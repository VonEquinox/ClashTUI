//! API 错误类型，区分可恢复（重连/超时）与致命。

use thiserror::Error;

/// mihomo 外部控制器调用的错误。
#[derive(Debug, Error)]
pub enum ApiError {
    /// HTTP 传输层错误（连接失败、超时等）。
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),

    /// 认证失败（secret 错误 / 401）。
    #[error("认证失败 (401)：external-controller secret 不正确")]
    Auth,

    /// 选节点针对非 Selector 组返回 400 "Proxy can't update"。
    /// 这是核验过的协议陷阱：只有 Selector 组能 PUT 选节点。
    #[error("无法切换该组的节点（非 Selector 组）：{0}")]
    ProxyCantUpdate(String),

    /// 资源不存在（404）。
    #[error("资源不存在 (404): {0}")]
    NotFound(String),

    /// 服务端返回其它非 2xx 状态。
    #[error("服务端错误 {status}: {body}")]
    Status { status: u16, body: String },

    /// 响应体解析失败。
    #[error("响应解析失败: {0}")]
    Decode(String),

    /// WebSocket 错误。
    #[error("WebSocket 错误: {0}")]
    Ws(String),

    /// URL 构造失败。
    #[error("URL 非法: {0}")]
    Url(String),
}

impl ApiError {
    /// 是否为可恢复错误（值得重试 / 重连）。
    pub fn is_recoverable(&self) -> bool {
        matches!(self, ApiError::Http(_) | ApiError::Ws(_))
    }
}

/// core-api 的 Result 别名。
pub type ApiResult<T> = Result<T, ApiError>;
