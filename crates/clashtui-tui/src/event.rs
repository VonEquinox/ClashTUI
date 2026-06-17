//! 两个核心事件词汇表（取自评审方案 P2 的 grafted idea）。
//!
//! - [`AppEvent`]：引擎 → UI 的单向总线（输入、重绘、tick、WS 帧、数据加载结果、提示、错误）。
//! - [`Effect`]：UI → 引擎的声明式副作用（取自 P1，替代不可测的 `Box<dyn FnOnce>` 闭包）。
//!
//! 两者都 `Debug`，可写入 `clashtui.log` 做重放调试，也可在 headless 单测中直接断言。

use crossterm::event::{KeyEvent, MouseEvent};

use clashtui_core_api::{Connection, Delay, GeneralConfig, Memory, Mode, Proxy, Traffic, Version};
use clashtui_domain::{AppConfig, CoreStatus};

/// 顶层 tab，决定一级导航顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabId {
    Status,
    Proxies,
    Profiles,
    Connections,
    Logs,
    Settings,
}

impl TabId {
    /// tab 栏从左到右的顺序，也用于数字键 1-6 直跳。
    pub const ORDER: [TabId; 6] = [
        TabId::Status,
        TabId::Proxies,
        TabId::Profiles,
        TabId::Connections,
        TabId::Logs,
        TabId::Settings,
    ];

    /// tab 栏显示标题。
    pub fn title(self) -> &'static str {
        match self {
            TabId::Status => "Status",
            TabId::Proxies => "Proxies",
            TabId::Profiles => "Profiles",
            TabId::Connections => "Connections",
            TabId::Logs => "Logs",
            TabId::Settings => "Settings",
        }
    }

    /// 该 tab 在 [`TabId::ORDER`] 中的下标。
    pub fn index(self) -> usize {
        TabId::ORDER.iter().position(|&t| t == self).unwrap_or(0)
    }

    /// 由下标取 tab（用于数字键 1-6）。越界返回 None。
    pub fn from_index(i: usize) -> Option<TabId> {
        TabId::ORDER.get(i).copied()
    }

    /// 循环切到下一个 / 上一个 tab。
    pub fn cycle(self, forward: bool) -> TabId {
        let n = TabId::ORDER.len();
        let i = self.index();
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        TabId::ORDER[next]
    }
}

/// 长任务进度。`total = None` 表示总量未知，只展示已完成量和活动条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressUpdate {
    /// 任务标识，用于后续完成/覆盖同一任务。
    pub id: String,
    /// 状态栏显示文本。
    pub label: String,
    /// 已完成数量（字节数、条目数或其它单位）。
    pub current: u64,
    /// 总量；未知时为 None。
    pub total: Option<u64>,
    /// true 表示任务完成，UI 应清除该进度。
    pub done: bool,
}

/// 引擎 → UI 的事件总线。所有进入主 `select!` 循环的东西都是一个 `AppEvent`。
#[derive(Debug)]
pub enum AppEvent {
    // ---- 输入与渲染 ----
    /// 终端按键。
    Key(KeyEvent),
    /// 终端鼠标事件（暂保留）。
    Mouse(MouseEvent),
    /// 终端 bracketed paste 文本。
    Paste(String),
    /// 终端尺寸变化，需重绘。
    Resize(u16, u16),
    /// 显式重绘请求（由 [`crate::tui::FrameRequester`] 触发）。
    Draw,
    /// 动画 tick（仅在有 spinner 时启用）。
    Tick,

    // ---- 提示 ----
    /// 短暂的状态提示（顶部 toast）。
    Toast(String),
    /// 非致命错误提示。
    Error(String),
    /// 长任务进度。
    Progress(ProgressUpdate),
    /// 某个异步 Effect 已结束，可释放 in-flight 锁。
    TaskDone(String),

    // ---- 数据加载结果（异步副作用回灌） ----
    /// 内核状态更新。
    CoreStatus(CoreStatus),
    /// 版本信息。
    Version(Option<Version>),
    /// 通用配置加载完成。
    ConfigLoaded(Box<GeneralConfig>),
    /// 应用配置加载完成。
    AppConfigLoaded(Box<AppConfig>),
    /// 代理组与节点加载完成（组列表保序，详情字典）。
    ProxiesLoaded {
        groups: Vec<Proxy>,
        all: std::collections::HashMap<String, Proxy>,
    },
    /// 单节点测速结果。
    DelayResult {
        node: String,
        delay: Delay,
    },
    /// 整组测速结果。
    GroupDelayResult(std::collections::HashMap<String, u16>),
    /// Profile 列表发生变化（名称, 是否当前）。
    ProfilesChanged(Vec<(String, bool)>),
    /// 订阅更新完成。
    SubUpdated(String),

    // ---- WS 流 ----
    /// 实时流量帧。
    WsTraffic(Traffic),
    /// 实时日志条目。
    WsLog {
        level: String,
        payload: String,
    },
    /// 实时连接快照。
    WsConnections {
        download_total: u64,
        upload_total: u64,
        connections: Vec<Connection>,
    },
    /// 实时内存帧。
    WsMemory(Memory),
    /// 某流（重新）连接 / 断开（kind 名称）。
    WsConnected(String),
    WsDisconnected(String),

    // ---- 升级 ----
    /// 内核升级进度文本。
    UpgradeProgress(String),

    /// 请求退出主循环。
    Quit,
}

/// UI → 引擎的声明式副作用。
///
/// [`crate::component::Component::handle_key`] 返回 `Vec<Effect>`，由
/// [`crate::effect_runner`] 集中执行：同步副作用直接改 UI 状态，
/// 异步副作用 spawn 出去、完成后以一个**具体的 `AppEvent`**（而非闭包）回灌总线。
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    // ---- UI 同步 ----
    /// 切换到指定 tab。
    SwitchTab(TabId),
    /// 打开 / 关闭 Help 覆盖层。
    ToggleHelp,
    /// 弹出一个一次性提示。
    Toast(String),
    /// 退出应用。
    Quit,

    // ---- 数据刷新（异步） ----
    /// 刷新内核状态 + 版本 + 配置。
    RefreshStatus,
    /// 刷新代理组与节点。
    RefreshProxies,
    /// 刷新 Profile 列表。
    RefreshProfiles,

    // ---- 内核生命周期 ----
    StartCore,
    StopCore,
    RestartCore,

    // ---- 配置 ----
    /// 切换模式。
    SwitchMode(Mode),
    /// 切换 TUN。
    ToggleTun(bool),
    /// 设置代理端口（HTTP, SOCKS, Mixed）。
    SetProxyPorts {
        http_port: u16,
        socks_port: u16,
        mixed_port: u16,
    },
    /// 设置退出 TUI 后是否保留托管内核。
    SetKeepCoreRunning(bool),
    /// 设置组测速并发数（0 = mihomo 整组接口）。
    SetGroupDelayConcurrency(usize),

    // ---- 代理 ----
    /// 选节点（组, 节点）。
    SelectNode {
        group: String,
        node: String,
    },
    /// 解除固定（组名）。
    UnfixGroup(String),
    /// 单节点测速。
    TestNode(String),
    /// 整组测速。
    TestGroup(String),

    // ---- Profile ----
    /// 添加订阅（名称, URL 或本地路径, 是否 URL）。
    AddProfile {
        name: String,
        source: String,
        is_url: bool,
    },
    /// 直接从订阅 URL 添加 profile，名称自动生成，成功后切换并加载。
    AddProfileFromUrl(String),
    /// 切换当前 profile。
    SwitchProfile(String),
    /// 删除 profile。
    DeleteProfile(String),
    /// 更新一个订阅。
    UpdateProfile(String),
    /// 使用本机代理更新一个订阅。
    UpdateProfileViaProxy(String),
    /// 更新全部订阅。
    UpdateAllProfiles,

    // ---- 系统代理 ----
    /// 切换系统代理（service 维度）。
    ToggleSysProxy,

    // ---- 流 ----
    /// 启动某 WS 流。
    StartStream(StreamId),
    /// 停止某 WS 流。
    StopStream(StreamId),
    /// 重连所有流（restart/reload 后必发）。
    ReconnectStreams,

    // ---- 连接 ----
    /// 关闭单条连接。
    CloseConn(String),
    /// 关闭全部连接。
    CloseAllConns,

    // ---- 升级 ----
    /// 升级内核。
    UpgradeKernel,

    // ---- Mixin ----
    /// 用外部 $EDITOR 编辑 mixin.yaml（会临时挂起 TUI）。
    EditMixin,
}

impl Effect {
    /// 可异步执行的 Effect 的去重键。返回 None 表示同步/幂等操作，不参与 in-flight 锁。
    pub fn inflight_key(&self) -> Option<String> {
        match self {
            Effect::RefreshStatus => Some("refresh_status".into()),
            Effect::RefreshProxies => Some("refresh_proxies".into()),
            Effect::RefreshProfiles => Some("refresh_profiles".into()),
            Effect::StartCore | Effect::StopCore | Effect::RestartCore => {
                Some("core_action".into())
            }
            Effect::SwitchMode(_) => Some("switch_mode".into()),
            Effect::ToggleTun(_) => Some("toggle_tun".into()),
            Effect::SetProxyPorts { .. } => Some("set_proxy_ports".into()),
            Effect::SetKeepCoreRunning(_) => Some("set_keep_core_running".into()),
            Effect::SetGroupDelayConcurrency(_) => Some("set_group_delay_concurrency".into()),
            Effect::SelectNode { group, .. } => Some(format!("select_node:{group}")),
            Effect::UnfixGroup(group) => Some(format!("unfix_group:{group}")),
            Effect::TestNode(_) | Effect::TestGroup(_) => Some("proxy_delay".into()),
            Effect::AddProfile { name, .. } => Some(format!("add_profile:{name}")),
            Effect::AddProfileFromUrl(url) => Some(format!("add_profile_url:{url}")),
            Effect::SwitchProfile(name) => Some(format!("switch_profile:{name}")),
            Effect::DeleteProfile(name) => Some(format!("delete_profile:{name}")),
            Effect::UpdateProfile(name) => Some(format!("update_profile:{name}")),
            Effect::UpdateProfileViaProxy(name) => Some(format!("update_profile_via_proxy:{name}")),
            Effect::UpdateAllProfiles => Some("update_all_profiles".into()),
            Effect::ToggleSysProxy => Some("toggle_sysproxy".into()),
            Effect::CloseConn(id) => Some(format!("close_conn:{id}")),
            Effect::CloseAllConns => Some("close_all_conns".into()),
            Effect::UpgradeKernel => Some("upgrade_kernel".into()),
            Effect::SwitchTab(_)
            | Effect::ToggleHelp
            | Effect::Toast(_)
            | Effect::Quit
            | Effect::StartStream(_)
            | Effect::StopStream(_)
            | Effect::ReconnectStreams
            | Effect::EditMixin => None,
        }
    }
}

/// WS 流标识（用于 Start/StopStream）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamId {
    Traffic,
    Logs,
    Connections,
    Memory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_delay_effects_share_one_inflight_key() {
        assert_eq!(
            Effect::TestNode("A".to_string()).inflight_key().as_deref(),
            Some("proxy_delay")
        );
        assert_eq!(
            Effect::TestGroup("Group".to_string())
                .inflight_key()
                .as_deref(),
            Some("proxy_delay")
        );
    }
}
