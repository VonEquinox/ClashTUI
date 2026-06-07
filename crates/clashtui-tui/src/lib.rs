//! clashtui-tui：ratatui + crossterm 前端。
//!
//! 导出 [`App`]、[`Component`] trait、[`Effect`]/[`AppEvent`] 事件词汇、路由器与 tab。
//! 只依赖 ratatui + crossterm，不含业务逻辑（业务在 clashtui-domain）。

pub mod app;
pub mod component;
pub mod context;
pub mod effect_runner;
pub mod event;
pub mod router;
pub mod tabs;
pub mod theme;
pub mod tui;
pub mod widgets;

pub use app::App;
pub use component::{Component, Handled};
pub use context::AppContext;
pub use event::{AppEvent, Effect, TabId};

use std::sync::Arc;

use clashtui_core_api::MihomoClient;
use clashtui_domain::{AppConfig, CoreManager, Paths, ProfileStore};
use tokio::sync::{mpsc, Mutex};

use crate::theme::Theme;

/// 启动 TUI：解析配置、构建上下文与内核管理器、跑主循环、最后恢复终端。
pub async fn run() -> color_eyre::Result<()> {
    // 路径与配置。
    let paths = Paths::resolve();
    paths.ensure_dirs()?;
    let config = AppConfig::load(&paths.config_file())?;

    // mihomo 客户端。
    let client = MihomoClient::new(&config.external_controller, config.secret.clone())
        .map_err(|e| color_eyre::eyre::eyre!("构建 mihomo 客户端失败: {e}"))?;

    // 内核管理器。
    let binary = if config.mihomo_binary.is_empty() {
        paths.default_binary()
    } else {
        std::path::PathBuf::from(&config.mihomo_binary)
    };
    let mut core = CoreManager::new(
        client.clone(),
        binary,
        paths.core_dir(),
        paths.runtime_config(),
        config.keep_core_running,
    );

    // 中央事件总线。
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // 内核子进程日志 → 转成 WsLog 事件回灌。
    let (log_tx, mut log_rx) = mpsc::unbounded_channel::<String>();
    core.set_log_sender(log_tx);
    {
        let etx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(line) = log_rx.recv().await {
                let _ = etx.send(AppEvent::WsLog {
                    level: "core".into(),
                    payload: line,
                });
            }
        });
    }

    // Profile 存储。
    let profiles = ProfileStore::load(paths.clone())?;

    let ctx = AppContext {
        client,
        core: Arc::new(core),
        config: Arc::new(config),
        paths: Arc::new(paths),
        profiles: Arc::new(Mutex::new(profiles)),
        theme: Theme::default(),
        event_tx: event_tx.clone(),
    };

    // 自动更新调度（M9）：周期检查 staleness，到期则更新订阅。
    if ctx.config.auto_update.enabled {
        let sctx = ctx.clone();
        tokio::spawn(async move {
            use clashtui_domain::schedule;
            let interval = schedule::interval_duration(sctx.config.auto_update.interval_hours);
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // 首次 tick 立即触发，跳过。
            tick.tick().await;
            loop {
                tick.tick().await;
                let _ = sctx.event_tx.send(AppEvent::Toast("自动更新订阅…".into()));
                // 触发"更新全部"。复用 effect 路径需要 App，这里直接发事件由 App 处理。
                let _ = sctx
                    .event_tx
                    .send(AppEvent::SubUpdated("__auto_update_all__".into()));
            }
        });
    }

    // WS 流扇入通道 → 转成 AppEvent 回灌。
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<clashtui_core_api::StreamMsg>();
    {
        let fctx = ctx.clone();
        tokio::spawn(async move {
            while let Some(msg) = stream_rx.recv().await {
                effect_runner::forward_stream_msg(&fctx, msg);
            }
        });
    }

    // 终端。
    let terminal = tui::init()?;
    tui::install_panic_hook();
    let (events, _requester) = tui::TuiEventStream::new();

    let app = App::new(ctx, stream_tx);
    let result = app.run(terminal, events, event_rx).await;

    let _ = tui::restore();
    result.map_err(Into::into)
}
