//! CLI 定义。M0 仅声明骨架，子命令具体逻辑在后续里程碑实现，
//! 复用 clashtui-domain + clashtui-core-api（与 TUI 共享同一套后端）。

use clap::{Parser, Subcommand};

/// ClashTUI：纯终端的 mihomo（Clash.Meta）管理器。
#[derive(Debug, Parser)]
#[command(name = "clashtui", version, about)]
pub struct Cli {
    /// 详细日志（等价 CLASHTUI_LOG=debug）。
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// 子命令。无子命令时启动 TUI。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 打印版本号。
    Version,
    /// 订阅 / Profile 管理（M2）。
    Profile,
    /// 模式切换 rule/global/direct（M7）。
    Mode,
    /// 代理测速 / 选节点（M3）。
    Proxy,
    /// 内核服务 start/stop/restart（M1）。
    Service,
    /// 系统代理 on/off/env。
    Sysproxy {
        #[command(subcommand)]
        action: SysproxyAction,
    },
    /// 内核升级（M11）。
    Upgrade,
}

/// 系统代理子动作。
#[derive(Debug, Subcommand)]
pub enum SysproxyAction {
    /// 开启系统代理（service 级）。
    On,
    /// 关闭系统代理（service 级）。
    Off,
    /// 打印可 source 的 env 片段：`eval $(clashtui sysproxy env)`。
    Env {
        /// 打印关闭（unset）片段。
        #[arg(long)]
        off: bool,
    },
}
