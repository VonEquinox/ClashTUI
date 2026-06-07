//! ClashTUI 入口：解析 CLI，分派到子命令或启动 TUI。

mod cli;

use clap::Parser;
use cli::{Cli, Command};

/// 编译期嵌入的版本号（见 build.rs）。
const VERSION: &str = env!("CLASHTUI_VERSION");

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    // 文件级日志（绝不写终端，避免污染 TUI 画面）。后续里程碑接入文件 appender。
    init_tracing(cli.verbose);

    match cli.command {
        Some(Command::Version) => {
            println!("clashtui {VERSION}");
            Ok(())
        }
        Some(Command::Sysproxy { action }) => run_sysproxy(action).await,
        // 其余子命令（profile/mode/proxy/service/upgrade）在后续里程碑实现。
        Some(other) => {
            eprintln!("子命令 {other:?} 尚未实现（计划于后续里程碑）。");
            Ok(())
        }
        // 无子命令 → 启动 TUI。
        None => clashtui_tui::run().await,
    }
}

/// 处理 `sysproxy` 子命令（复用 domain 层）。
async fn run_sysproxy(action: cli::SysproxyAction) -> color_eyre::Result<()> {
    use clashtui_domain::sysproxy::{env_snippet, service_proxy, ProxySettings};
    use clashtui_domain::{AppConfig, Paths};

    let paths = Paths::resolve();
    let config = AppConfig::load(&paths.config_file())?;
    let sp = &config.system_proxy;
    let settings = ProxySettings::new(sp.http_port, sp.socks_port, sp.bypass.clone());

    match action {
        cli::SysproxyAction::On => {
            let manual = (!config.manual_network_service.is_empty())
                .then(|| config.manual_network_service.clone());
            service_proxy(manual)
                .enable(&settings)
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
            println!("系统代理已开启");
        }
        cli::SysproxyAction::Off => {
            let manual = (!config.manual_network_service.is_empty())
                .then(|| config.manual_network_service.clone());
            service_proxy(manual)
                .disable()
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
            println!("系统代理已关闭");
        }
        cli::SysproxyAction::Env { off } => {
            // 仅打印到 stdout，供 eval 使用（不写日志、不加其它输出）。
            print!("{}", env_snippet(&settings, !off));
        }
    }
    Ok(())
}

/// 初始化日志（M0：仅 stderr，TUI 启动前/CLI 模式下可见；TUI 模式后续切文件）。
fn init_tracing(verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let default = if verbose { "debug" } else { "warn" };
    let filter =
        EnvFilter::try_from_env("CLASHTUI_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    // 忽略重复初始化错误。
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
