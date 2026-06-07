//! clashtui-domain：业务逻辑层。
//!
//! config / profile / mixin / lifecycle / sysproxy / upgrade / schedule。
//! 不拥有任何终端或 UI 类型，可独立测试，并被 TUI 与 CLI 复用。

pub mod config;
pub mod error;
pub mod lifecycle;
pub mod mixin;
pub mod paths;
pub mod profile;
pub mod schedule;
pub mod sysproxy;
pub mod upgrade;
pub mod util;
pub mod yaml;

pub use config::AppConfig;
pub use error::{DomainError, DomainResult};
pub use lifecycle::{CoreManager, CoreStatus};
pub use paths::Paths;
pub use profile::{ProfileKind, ProfileMeta, ProfileStore, SubscriptionInfo};
