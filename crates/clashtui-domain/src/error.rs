//! domain 层错误类型。

use thiserror::Error;

/// 业务逻辑错误。
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("YAML 错误: {0}")]
    Yaml(String),

    #[error("Profile 错误: {0}")]
    Profile(String),

    #[error("内核错误: {0}")]
    Core(String),

    #[error("系统代理错误: {0}")]
    SysProxy(String),

    #[error("升级错误: {0}")]
    Upgrade(String),

    #[error("网络错误: {0}")]
    Http(String),

    #[error("API 错误: {0}")]
    Api(#[from] clashtui_core_api::ApiError),
}

/// domain 层 Result。
pub type DomainResult<T> = Result<T, DomainError>;
