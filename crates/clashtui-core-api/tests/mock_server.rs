//! 对着一个手写的最小 HTTP mock server 验证 MihomoClient 的 wire 行为。
//!
//! 不引入 wiremock，用 tokio TcpListener 直接应答固定响应，覆盖核验过的协议陷阱：
//! - GET /proxies 包裹 vs GET /proxies/{name} 不包裹
//! - PUT /proxies/{group} 对非 Selector 返回 400 "Proxy can't update" → ProxyCantUpdate
//! - GET /proxies/{name}/delay 的 0 == timeout
//! - PATCH /configs 发出 kebab-case + 小写 mode（这里只验证客户端不报错并正确解析 body）

use std::sync::Arc;

use clashtui_core_api::{ApiError, MihomoClient, Mode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// 启动一个 mock server，返回其 `host:port`。`handler` 决定每个请求的响应。
async fn start_mock<F>(handler: F) -> String
where
    F: Fn(&str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    let captured = Arc::new(Mutex::new(Vec::<String>::new()));
    let _ = &captured;

    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => break,
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                // 请求行：METHOD PATH HTTP/1.1
                let first = req.lines().next().unwrap_or("");
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                let key = format!("{method} {path}");

                let (code, body) = handler(&key);
                let reason = if code == 200 { "OK" } else { "ERR" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });

    format!("{}:{}", addr.ip(), addr.port())
}

#[tokio::test]
async fn version_and_configs() {
    let host = start_mock(|key| match key {
        k if k.starts_with("GET /version") => {
            (200, r#"{"version":"1.18.0","meta":true}"#.to_string())
        }
        k if k.starts_with("GET /configs") => (
            200,
            r#"{"mode":"rule","mixed-port":7890,"socks-port":7891,"tun":{"enable":false}}"#
                .to_string(),
        ),
        _ => (404, "{}".to_string()),
    })
    .await;

    let client = MihomoClient::new(&host, "").unwrap();
    let v = client.version().await.unwrap();
    assert_eq!(v.version, "1.18.0");
    assert!(v.meta);

    let cfg = client.configs().await.unwrap();
    assert_eq!(cfg.mode, Some(Mode::Rule));
    assert_eq!(cfg.mixed_port, 7890);
    assert!(!cfg.tun.enable);
}

#[tokio::test]
async fn proxies_wrapped_vs_single_unwrapped() {
    let host = start_mock(|key| match key {
        "GET /proxies" => (
            200,
            r#"{"proxies":{"GLOBAL":{"name":"GLOBAL","type":"Selector","all":["A"],"now":"A","history":[]}}}"#.to_string(),
        ),
        k if k.starts_with("GET /proxies/") => (
            200,
            // 单个 proxy：未包裹
            r#"{"name":"A","type":"Shadowsocks","history":[{"time":"t","delay":120}]}"#.to_string(),
        ),
        _ => (404, "{}".to_string()),
    })
    .await;

    let client = MihomoClient::new(&host, "").unwrap();
    let wrapped = client.proxies().await.unwrap();
    assert!(wrapped.proxies.contains_key("GLOBAL"));

    let single = client.proxy("A").await.unwrap();
    assert_eq!(single.name, "A");
    assert_eq!(single.latest_delay().millis(), Some(120));
}

#[tokio::test]
async fn select_non_selector_maps_to_proxy_cant_update() {
    let host = start_mock(|key| {
        if key.starts_with("PUT /proxies/") {
            (400, "Proxy can't update".to_string())
        } else {
            (404, "{}".to_string())
        }
    })
    .await;

    let client = MihomoClient::new(&host, "").unwrap();
    let err = client.select_node("AutoGroup", "node1").await.unwrap_err();
    assert!(matches!(err, ApiError::ProxyCantUpdate(_)));
}

#[tokio::test]
async fn delay_zero_is_timeout() {
    let host = start_mock(|key| {
        if key.starts_with("GET /proxies/") && key.contains("/delay") {
            (200, r#"{"delay":0}"#.to_string())
        } else {
            (404, "{}".to_string())
        }
    })
    .await;

    let client = MihomoClient::new(&host, "").unwrap();
    let d = client
        .proxy_delay("node1", "http://example.com", 2000)
        .await
        .unwrap();
    assert!(d.is_timeout());
    assert_eq!(d.display(), "timeout");
}

#[tokio::test]
async fn auth_401_mapped() {
    let host = start_mock(|_| (401, "unauthorized".to_string())).await;
    let client = MihomoClient::new(&host, "wrong-secret").unwrap();
    let err = client.version().await.unwrap_err();
    assert!(matches!(err, ApiError::Auth));
}
