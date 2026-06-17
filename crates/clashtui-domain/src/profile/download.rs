//! 订阅下载与 `subscription-userinfo` 头解析。

use futures_util::StreamExt;

use crate::error::{DomainError, DomainResult};
use crate::profile::SubscriptionInfo;

/// 下载结果：原始 YAML 文本 + 订阅信息。
#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub body: String,
    pub info: SubscriptionInfo,
}

/// 下载一个订阅 URL。
///
/// - User-Agent 设为 `clash.meta`（多数机场据此返回 clash 格式）。
/// - 解析 `subscription-userinfo` 响应头。
/// - 校验 body 能被 YAML 解析（避免存入坏配置）。
pub async fn download(url: &str) -> DomainResult<DownloadResult> {
    download_with_progress(url, |_, _| {}).await
}

/// 通过 HTTP 代理下载一个订阅 URL。
pub async fn download_via_proxy(url: &str, proxy: &str) -> DomainResult<DownloadResult> {
    download_with_proxy_and_progress(url, Some(proxy), |_, _| {}).await
}

/// 下载一个订阅 URL，下载期间回调 `(downloaded, total)`。
pub async fn download_with_progress<F>(url: &str, progress: F) -> DomainResult<DownloadResult>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    download_with_proxy_and_progress(url, None, progress).await
}

/// 下载一个订阅 URL，可选通过 HTTP 代理，下载期间回调 `(downloaded, total)`。
pub async fn download_with_proxy_and_progress<F>(
    url: &str,
    proxy: Option<&str>,
    mut progress: F,
) -> DomainResult<DownloadResult>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let mut builder = reqwest::Client::builder()
        .user_agent("clash.meta")
        .timeout(std::time::Duration::from_secs(30));
    if let Some(proxy) = proxy {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|e| DomainError::Http(e.to_string()))?);
    }
    let client = builder
        .build()
        .map_err(|e| DomainError::Http(e.to_string()))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| DomainError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(DomainError::Http(format!("订阅返回状态 {}", resp.status())));
    }

    let info = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(parse_userinfo)
        .unwrap_or_default();

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut downloaded = 0u64;
    let mut bytes = Vec::new();
    progress(downloaded, total);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| DomainError::Http(e.to_string()))?;
        downloaded += chunk.len() as u64;
        bytes.extend_from_slice(&chunk);
        progress(downloaded, total);
    }
    let body = String::from_utf8(bytes)
        .map_err(|e| DomainError::Profile(format!("订阅不是 UTF-8 文本: {e}")))?;

    // 校验可解析。
    crate::yaml::parse(&body)
        .map_err(|e| DomainError::Profile(format!("订阅不是合法 YAML: {e}")))?;

    Ok(DownloadResult { body, info })
}

/// 解析 `subscription-userinfo` 头，形如：
/// `upload=123; download=456; total=789; expire=1700000000`
pub fn parse_userinfo(s: &str) -> SubscriptionInfo {
    let mut info = SubscriptionInfo::default();
    for part in s.split(';') {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "upload" => info.upload = val.parse().unwrap_or(0),
            "download" => info.download = val.parse().unwrap_or(0),
            "total" => info.total = val.parse().unwrap_or(0),
            "expire" => info.expire = val.parse().unwrap_or(0),
            _ => {}
        }
    }
    info
}

/// 从本地文件读取订阅。
pub fn read_local(path: &str) -> DomainResult<DownloadResult> {
    let body = std::fs::read_to_string(path).map_err(DomainError::Io)?;
    crate::yaml::parse(&body)
        .map_err(|e| DomainError::Profile(format!("文件不是合法 YAML: {e}")))?;
    Ok(DownloadResult {
        body,
        info: SubscriptionInfo::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_userinfo() {
        let s = "upload=100; download=200; total=1000; expire=1700000000";
        let info = parse_userinfo(s);
        assert_eq!(info.upload, 100);
        assert_eq!(info.download, 200);
        assert_eq!(info.total, 1000);
        assert_eq!(info.expire, 1700000000);
    }

    #[test]
    fn parse_partial_and_malformed() {
        let info = parse_userinfo("download=50; garbage; total=");
        assert_eq!(info.download, 50);
        assert_eq!(info.total, 0);
        assert_eq!(info.upload, 0);
    }

    #[test]
    fn parse_empty() {
        let info = parse_userinfo("");
        assert_eq!(info, SubscriptionInfo::default());
    }

    #[test]
    fn read_local_validates_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.yaml");
        std::fs::write(&good, "proxies: []\n").unwrap();
        assert!(read_local(good.to_str().unwrap()).is_ok());

        let bad = dir.path().join("bad.yaml");
        std::fs::write(&bad, "key: : :\n  - broken").unwrap();
        assert!(read_local(bad.to_str().unwrap()).is_err());
    }
}
