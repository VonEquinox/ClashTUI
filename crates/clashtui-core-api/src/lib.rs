//! clashtui-core-api：mihomo（Clash.Meta）外部控制器的纯异步客户端。
//!
//! 不依赖终端、不碰文件系统、不执行 OS 命令——只负责 HTTP + WebSocket 通信，
//! 因此可以脱离终端、对着 mock server 做单元/集成测试。

pub mod client;
pub mod error;
pub mod models;
pub mod stream;

pub use client::MihomoClient;
pub use error::{ApiError, ApiResult};
pub use models::*;
pub use stream::{StreamHub, StreamKind, StreamMsg};
