//! [`MihomoClient`]：mihomo 外部控制器的 HTTP 客户端。
//!
//! - 自动附带 `Authorization: Bearer <secret>`（secret 非空时）。
//! - 每个 path 段做 percent-encode（节点/组名可能含空格、CJK、符号）。
//! - 按调用类型设置不同超时（测速 = 用户超时+缓冲；reload/upgrade 用长超时）。

use std::time::Duration;

use reqwest::{Client, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{ApiError, ApiResult};
use crate::models::*;

/// mihomo REST 客户端。克隆代价低（内部 `reqwest::Client` 是 Arc）。
#[derive(Debug, Clone)]
pub struct MihomoClient {
    http: Client,
    /// 形如 `http://127.0.0.1:9090`（无尾斜杠）。
    base_url: String,
    /// external-controller secret，可能为空。
    secret: String,
}

impl MihomoClient {
    /// 构造客户端。`base` 形如 `127.0.0.1:9090` 或 `http://127.0.0.1:9090`。
    pub fn new(base: &str, secret: impl Into<String>) -> ApiResult<Self> {
        let base_url = normalize_base(base);
        let http = Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(ApiError::Http)?;
        Ok(MihomoClient {
            http,
            base_url,
            secret: secret.into(),
        })
    }

    /// 当前 base url（如 `http://127.0.0.1:9090`）。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// secret（可能为空）。
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// 取主机:端口（去掉 scheme），用于拼 WS url。
    pub fn host_port(&self) -> &str {
        self.base_url
            .strip_prefix("http://")
            .or_else(|| self.base_url.strip_prefix("https://"))
            .unwrap_or(&self.base_url)
    }

    // ---------- 通用请求 ----------

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn req(&self, method: Method, url: &str) -> reqwest::RequestBuilder {
        let mut b = self.http.request(method, url);
        if !self.secret.is_empty() {
            b = b.bearer_auth(&self.secret);
        }
        b
    }

    /// 发送请求并把状态码翻译为类型化错误。
    async fn send(&self, rb: reqwest::RequestBuilder) -> ApiResult<reqwest::Response> {
        let resp = rb.send().await.map_err(ApiError::Http)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(map_status(status, body))
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let resp = self
            .send(
                self.req(Method::GET, &self.url(path))
                    .timeout(Duration::from_secs(10)),
            )
            .await?;
        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))
    }

    // ---------- /version ----------

    /// `GET /version`。也用作 spawn-or-attach 的探测。
    pub async fn version(&self) -> ApiResult<Version> {
        self.get_json("/version").await
    }

    /// 仅探测内核是否在线（不解析）。
    pub async fn ping(&self) -> bool {
        let url = self.url("/version");
        match self
            .req(Method::GET, &url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(r) => r.status().is_success() || r.status() == StatusCode::UNAUTHORIZED,
            Err(_) => false,
        }
    }

    // ---------- /configs ----------

    /// `GET /configs`。
    pub async fn configs(&self) -> ApiResult<GeneralConfig> {
        self.get_json("/configs").await
    }

    /// `PATCH /configs`：部分更新（mode / tun / 端口等），无需 reload。
    pub async fn patch_configs(&self, patch: &ConfigPatch) -> ApiResult<()> {
        let url = self.url("/configs");
        self.send(
            self.req(Method::PATCH, &url)
                .json(patch)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }

    /// `PUT /configs?force=<force>`：重载配置文件（`path` 指向运行时 config.yaml）。
    /// 注意：会**断开本客户端的 WS 流**，调用方须随后重连。
    pub async fn reload_config(&self, path: &str, force: bool) -> ApiResult<()> {
        let url = format!("{}/configs?force={}", self.base_url, force);
        #[derive(Serialize)]
        struct Body<'a> {
            path: &'a str,
        }
        self.send(
            self.req(Method::PUT, &url)
                .json(&Body { path })
                .timeout(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }

    // ---------- /proxies & /group ----------

    /// `GET /proxies`（包裹）。
    pub async fn proxies(&self) -> ApiResult<ProxiesResponse> {
        self.get_json("/proxies").await
    }

    /// `GET /proxies/{name}`（**未包裹**的单个 Proxy）。
    pub async fn proxy(&self, name: &str) -> ApiResult<Proxy> {
        let path = format!("/proxies/{}", enc(name));
        self.get_json(&path).await
    }

    /// `PUT /proxies/{group}` 选节点。仅 Selector 组有效，否则 400 → [`ApiError::ProxyCantUpdate`]。
    pub async fn select_node(&self, group: &str, node: &str) -> ApiResult<()> {
        let url = self.url(&format!("/proxies/{}", enc(group)));
        let body = SelectBody {
            name: node.to_string(),
        };
        self.send(
            self.req(Method::PUT, &url)
                .json(&body)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }

    /// `DELETE /proxies/{name}`：清除该（非 Selector）组的固定选择（unfix）。
    pub async fn unfix(&self, name: &str) -> ApiResult<()> {
        let url = self.url(&format!("/proxies/{}", enc(name)));
        self.send(
            self.req(Method::DELETE, &url)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }

    /// `GET /proxies/{name}/delay?url=&timeout=` 单节点测速。0 == 超时。
    pub async fn proxy_delay(
        &self,
        name: &str,
        test_url: &str,
        timeout_ms: u32,
    ) -> ApiResult<Delay> {
        let url = format!(
            "{}/proxies/{}/delay?url={}&timeout={}",
            self.base_url,
            enc(name),
            enc(test_url),
            timeout_ms
        );
        let resp = self
            .send(
                self.req(Method::GET, &url)
                    .timeout(Duration::from_millis(timeout_ms as u64 + 5000)),
            )
            .await?;
        #[derive(serde::Deserialize)]
        struct DelayResp {
            #[serde(default)]
            delay: u16,
        }
        let d: DelayResp = resp
            .json()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(Delay(d.delay))
    }

    /// `GET /group`（仅代理组，保序）。
    pub async fn groups(&self) -> ApiResult<Vec<Proxy>> {
        let r: GroupsResponse = self.get_json("/group").await?;
        Ok(r.proxies)
    }

    /// `GET /group/{name}/delay?url=&timeout=` 整组测速 → name→delay。
    pub async fn group_delay(
        &self,
        group: &str,
        test_url: &str,
        timeout_ms: u32,
    ) -> ApiResult<std::collections::HashMap<String, u16>> {
        let url = format!(
            "{}/group/{}/delay?url={}&timeout={}",
            self.base_url,
            enc(group),
            enc(test_url),
            timeout_ms
        );
        let resp = self
            .send(
                self.req(Method::GET, &url)
                    .timeout(Duration::from_millis(timeout_ms as u64 + 10000)),
            )
            .await?;
        resp.json()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))
    }

    // ---------- /connections ----------

    /// `GET /connections` 快照。
    pub async fn connections(&self) -> ApiResult<ConnectionsSnapshot> {
        self.get_json("/connections").await
    }

    /// `DELETE /connections` 关闭全部连接。
    pub async fn close_all_connections(&self) -> ApiResult<()> {
        let url = self.url("/connections");
        self.send(
            self.req(Method::DELETE, &url)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }

    /// `DELETE /connections/{id}` 关闭单条连接。
    pub async fn close_connection(&self, id: &str) -> ApiResult<()> {
        let url = self.url(&format!("/connections/{}", enc(id)));
        self.send(
            self.req(Method::DELETE, &url)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }

    // ---------- 控制 ----------

    /// `POST /restart` 重启内核。会断开 WS。
    pub async fn restart(&self) -> ApiResult<()> {
        let url = self.url("/restart");
        self.send(
            self.req(Method::POST, &url)
                .timeout(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }

    /// `POST /upgrade?force=` 内核自升级（核自管二进制时）。
    pub async fn upgrade_core(&self, force: bool) -> ApiResult<()> {
        let url = format!("{}/upgrade?force={}", self.base_url, force);
        self.send(
            self.req(Method::POST, &url)
                .timeout(Duration::from_secs(120)),
        )
        .await?;
        Ok(())
    }

    /// `POST /cache/fakeip/flush` 刷新 fakeip 缓存。
    pub async fn flush_fakeip(&self) -> ApiResult<()> {
        let url = self.url("/cache/fakeip/flush");
        self.send(
            self.req(Method::POST, &url)
                .timeout(Duration::from_secs(10)),
        )
        .await?;
        Ok(())
    }
}

/// percent-encode 单个 path 段。
fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// 规范化 base：补 scheme、去尾斜杠。
fn normalize_base(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.starts_with("http://") || b.starts_with("https://") {
        b.to_string()
    } else {
        format!("http://{b}")
    }
}

/// 状态码 → 类型化错误。
fn map_status(status: StatusCode, body: String) -> ApiError {
    match status {
        StatusCode::UNAUTHORIZED => ApiError::Auth,
        StatusCode::NOT_FOUND => ApiError::NotFound(body),
        StatusCode::BAD_REQUEST if body.contains("Proxy can't update") => {
            ApiError::ProxyCantUpdate(body)
        }
        _ => ApiError::Status {
            status: status.as_u16(),
            body,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_adds_scheme_and_trims() {
        assert_eq!(normalize_base("127.0.0.1:9090"), "http://127.0.0.1:9090");
        assert_eq!(normalize_base("http://x:1/"), "http://x:1");
        assert_eq!(normalize_base("https://x:1"), "https://x:1");
    }

    #[test]
    fn host_port_strips_scheme() {
        let c = MihomoClient::new("127.0.0.1:9090", "").unwrap();
        assert_eq!(c.host_port(), "127.0.0.1:9090");
    }

    #[test]
    fn enc_encodes_special_chars() {
        assert_eq!(enc("香港 01"), "%E9%A6%99%E6%B8%AF%2001");
    }

    #[test]
    fn proxy_cant_update_mapped_from_400() {
        let e = map_status(StatusCode::BAD_REQUEST, "Proxy can't update".into());
        assert!(matches!(e, ApiError::ProxyCantUpdate(_)));
    }

    #[test]
    fn unauthorized_mapped() {
        let e = map_status(StatusCode::UNAUTHORIZED, String::new());
        assert!(matches!(e, ApiError::Auth));
    }
}
