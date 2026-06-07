//! 各 tab 组件。随里程碑逐个替换占位为真实实现。

mod connections;
mod logs;
mod profiles;
mod proxies;
mod settings;
mod status;

pub use connections::ConnectionsTab;
pub use logs::LogsTab;
pub use profiles::ProfilesTab;
pub use proxies::ProxiesTab;
pub use settings::SettingsTab;
pub use status::StatusTab;

use crate::component::Component;
use crate::event::TabId;
use crate::theme::Theme;

/// 构建全部 tab，顺序与 [`TabId::ORDER`] 一致。
/// `controller` 为 external-controller 地址（供 Settings 展示）。
pub fn build_tabs(theme: &Theme, controller: &str) -> Vec<Box<dyn Component>> {
    TabId::ORDER
        .iter()
        .map(|&id| build_tab(id, theme, controller))
        .collect()
}

fn build_tab(id: TabId, theme: &Theme, controller: &str) -> Box<dyn Component> {
    match id {
        TabId::Status => Box::new(StatusTab::new(theme.clone())),
        TabId::Profiles => Box::new(ProfilesTab::new(theme.clone())),
        TabId::Proxies => Box::new(ProxiesTab::new(theme.clone(), controller.to_string())),
        TabId::Logs => Box::new(LogsTab::new(theme.clone())),
        TabId::Connections => Box::new(ConnectionsTab::new(theme.clone())),
        TabId::Settings => Box::new(SettingsTab::new(theme.clone(), controller.to_string())),
    }
}
