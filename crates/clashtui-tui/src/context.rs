//! [`AppContext`]：UI 与后端之间共享的句柄集合（按引用传递，**非全局**）。
//!
//! 拥有 mihomo 客户端、内核管理器、应用配置、主题、路径与事件回灌端。
//! 异步副作用通过 `event_tx` 把结果以具体 [`AppEvent`] 回灌主循环。

use std::sync::Arc;

use clashtui_core_api::MihomoClient;
use clashtui_domain::{AppConfig, CoreManager, Paths, ProfileStore};
use tokio::sync::{mpsc, Mutex};

use crate::event::AppEvent;
use crate::theme::Theme;

/// 共享上下文。`Arc` 字段便于 spawn 异步任务时克隆。
#[derive(Clone)]
pub struct AppContext {
    pub client: MihomoClient,
    pub core: Arc<CoreManager>,
    pub config: Arc<AppConfig>,
    pub paths: Arc<Paths>,
    pub profiles: Arc<Mutex<ProfileStore>>,
    pub theme: Theme,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl AppContext {
    /// 回灌一个事件到主循环（异步副作用完成后调用）。
    pub fn emit(&self, ev: AppEvent) {
        let _ = self.event_tx.send(ev);
    }
}
