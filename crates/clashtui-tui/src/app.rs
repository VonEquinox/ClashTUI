//! App 壳：拥有全部 UI 状态与 [`AppContext`]，跑主 `select!` 循环，集中应用 [`Effect`]。
//!
//! 并发模型：单 UI 任务上的 `loop { tokio::select! { ... } }`，**固定 3 个 arm**：
//! 1. [`AppEvent`] mpsc 接收（中央总线：WS 帧 / 数据加载 / Toast / Quit …）
//! 2. [`TuiEventStream`] 输入 + 重绘请求合流
//! 3. 动画 tick interval（仅在有 spinner 时启用）
//!
//! WS 流由 [`StreamHub`] 扇入单独通道，再由转发任务回灌 [`AppEvent`] 总线。

use std::collections::HashSet;
use std::time::Duration;

use clashtui_core_api::{StreamHub, StreamKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use tokio::sync::mpsc;

use crate::{
    component::Component,
    context::AppContext,
    effect_runner,
    event::{AppEvent, Effect, ProgressUpdate, TabId},
    router::{self, Chord, GlobalAction, Routed},
    tabs::build_tabs,
};

/// 应用主结构。
pub struct App {
    ctx: AppContext,
    tabs: Vec<Box<dyn Component>>,
    active: usize,
    pub help_open: bool,
    pub should_quit: bool,
    toast: Option<String>,
    progress: Option<ProgressUpdate>,
    inflight_effects: HashSet<String>,
    /// 已启动的 WS 流类型集合（去重）。
    active_streams: HashSet<StreamKind>,
    /// WS 流扇入通道发送端（用于重建 hub）。
    stream_tx: mpsc::UnboundedSender<clashtui_core_api::StreamMsg>,
    /// WS hub。
    hub: StreamHub,
    /// 待执行的 $EDITOR 编辑 mixin 请求（由主循环消费）。
    pending_edit_mixin: bool,
}

impl App {
    /// 用上下文构造。`stream_tx` 是 StreamHub 扇入端，对应的 rx 在 run 里转发。
    pub fn new(
        ctx: AppContext,
        stream_tx: mpsc::UnboundedSender<clashtui_core_api::StreamMsg>,
    ) -> Self {
        let tabs = build_tabs(&ctx.theme, ctx.client.base_url());
        let hub = StreamHub::new(
            ctx.client.host_port(),
            ctx.client.secret(),
            stream_tx.clone(),
        );
        App {
            ctx,
            tabs,
            active: 0,
            help_open: false,
            should_quit: false,
            toast: None,
            progress: None,
            inflight_effects: HashSet::new(),
            active_streams: HashSet::new(),
            stream_tx,
            hub,
            pending_edit_mixin: false,
        }
    }

    /// 请求用 $EDITOR 编辑 mixin（由主循环在挂起 TUI 后执行）。
    pub fn request_edit_mixin(&mut self) {
        self.pending_edit_mixin = true;
    }

    /// 共享上下文克隆（供 effect_runner spawn 异步任务）。
    pub fn ctx(&self) -> AppContext {
        self.ctx.clone()
    }

    pub fn active_tab(&self) -> TabId {
        TabId::ORDER[self.active]
    }

    pub fn switch_tab(&mut self, tab: TabId) {
        let new = tab.index();
        if new == self.active {
            return;
        }
        let blur = self.tabs[self.active].on_blur();
        self.active = new;
        let focus = self.tabs[self.active].on_focus();
        self.apply_effects(blur);
        self.apply_effects(focus);
    }

    pub fn set_toast(&mut self, msg: String) {
        self.toast = Some(msg);
    }

    fn apply_effects(&mut self, effects: impl IntoIterator<Item = Effect>) -> bool {
        let mut redraw = false;
        for effect in effects {
            redraw |= self.apply_effect(effect);
        }
        redraw
    }

    fn apply_effect(&mut self, effect: Effect) -> bool {
        if let Some(key) = effect.inflight_key() {
            if !self.inflight_effects.insert(key.clone()) {
                self.set_toast("任务正在进行中，请等待完成".into());
                return true;
            }
        }
        effect_runner::apply(self, effect)
    }

    fn focus(&self) -> router::Focus {
        router::Focus {
            tab: self.active_tab(),
            popup_open: false,
            help_open: self.help_open,
        }
    }

    // ---------- 流管理 ----------

    /// 启动某 WS 流（去重）。
    pub fn start_stream(&mut self, kind: StreamKind) {
        if self.active_streams.insert(kind) {
            self.hub.start(kind);
        }
    }

    /// 停止某 WS 流。
    pub fn stop_stream(&mut self, kind: StreamKind) {
        if self.active_streams.remove(&kind) {
            self.hub.stop(kind);
        }
    }

    /// 重连所有流：重建 hub 并重启已激活的流（restart/reload 后调用）。
    pub fn reconnect_streams(&mut self) {
        let kinds: Vec<StreamKind> = self.active_streams.iter().copied().collect();
        self.hub = StreamHub::new(
            self.ctx.client.host_port(),
            self.ctx.client.secret(),
            self.stream_tx.clone(),
        );
        for k in kinds {
            self.hub.start(k);
        }
    }

    /// 运行主循环。
    pub async fn run(
        mut self,
        mut terminal: crate::tui::Tui,
        mut events: crate::tui::TuiEventStream,
        mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
    ) -> std::io::Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(16));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // 进入首个 tab 触发其 on_focus。
        let focus_effects = self.tabs[self.active].on_focus();
        self.apply_effects(focus_effects);

        terminal.draw(|f| self.draw(f))?;

        loop {
            if self.should_quit {
                break;
            }
            let mut need_redraw = false;

            tokio::select! {
                Some(ev) = event_rx.recv() => {
                    need_redraw |= self.on_app_event(ev);
                }
                maybe = events.next() => {
                    match maybe {
                        Some(ev) => need_redraw |= self.on_app_event(ev),
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    if self.tabs[self.active].tick() {
                        need_redraw = true;
                    }
                }
            }

            // 处理挂起 TUI 去编辑 mixin 的请求。
            if self.pending_edit_mixin {
                self.pending_edit_mixin = false;
                self.edit_mixin(&mut terminal)?;
                terminal.draw(|f| self.draw(f))?;
                continue;
            }

            if need_redraw && !self.should_quit {
                terminal.draw(|f| self.draw(f))?;
            }
        }
        Ok(())
    }

    /// 挂起 TUI，启动 $EDITOR 编辑 mixin.yaml，返回后恢复终端。
    fn edit_mixin(&mut self, terminal: &mut crate::tui::Tui) -> std::io::Result<()> {
        let path = self.ctx.paths.mixin_file();
        // 不存在则写入模板。
        if !path.exists() {
            let _ = std::fs::write(&path, MIXIN_TEMPLATE);
        }
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "vi".to_string());

        // 离开 alternate screen + raw mode。
        crate::tui::restore()?;
        let status = std::process::Command::new(&editor).arg(&path).status();
        // 重新进入 TUI。
        *terminal = crate::tui::init()?;

        match status {
            Ok(s) if s.success() => self.set_toast("mixin 已保存，切换/更新配置后生效".into()),
            Ok(_) => self.set_toast("编辑器异常退出".into()),
            Err(e) => self.set_toast(format!("无法启动编辑器 {editor}: {e}")),
        }
        Ok(())
    }

    fn on_app_event(&mut self, ev: AppEvent) -> bool {
        match ev {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Paste(text) => self.on_paste(text),
            AppEvent::Resize(_, _) | AppEvent::Draw => true,
            AppEvent::Tick => self.tabs[self.active].tick(),
            AppEvent::Toast(msg) | AppEvent::UpgradeProgress(msg) => {
                self.set_toast(msg);
                true
            }
            AppEvent::Error(msg) => {
                self.progress = None;
                self.set_toast(msg);
                true
            }
            AppEvent::Progress(update) => {
                if update.done {
                    if self.progress.as_ref().map(|p| p.id.as_str()) == Some(update.id.as_str()) {
                        self.progress = None;
                    }
                } else {
                    self.progress = Some(update);
                }
                true
            }
            AppEvent::TaskDone(key) => {
                self.inflight_effects.remove(&key);
                false
            }
            AppEvent::Quit => {
                self.should_quit = true;
                false
            }
            AppEvent::Mouse(_) => false,
            // 自动更新调度的哨兵：触发"更新全部"。
            AppEvent::SubUpdated(name) if name == "__auto_update_all__" => {
                self.apply_effect(Effect::UpdateAllProfiles);
                false
            }
            // 数据 / WS 事件：分发给所有 tab 的 apply_event（它们各取所需）。
            other => self.dispatch_to_tabs(other),
        }
    }

    /// 把数据/WS 事件分发给所有 tab，收集其链式 Effect。返回是否重绘。
    fn dispatch_to_tabs(&mut self, ev: AppEvent) -> bool {
        let mut effects = Vec::new();
        for tab in &mut self.tabs {
            effects.extend(tab.apply_event(&ev));
        }
        // 分发后通常需要重绘。
        let redraw = self.apply_effects(effects);
        let _ = redraw;
        true
    }

    fn on_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        // 组件独占输入（文本框）时，所有键直送组件，绕过路由器。
        if self.tabs[self.active].capturing() {
            let (_handled, effects) = self.tabs[self.active].handle_key(key);
            self.apply_effects(effects);
            return true; // 输入态每次按键都重绘
        }
        match router::route(self.focus(), key) {
            Routed::Popup(_) => false,
            Routed::GlobalChord(chord) => self.on_chord(chord),
            Routed::DismissHelp => {
                self.help_open = false;
                true
            }
            Routed::Global(action) => self.on_global(action),
            Routed::ActiveTab => {
                let (handled, effects) = self.tabs[self.active].handle_key(key);
                let mut redraw = handled.is_handled();
                redraw |= self.apply_effects(effects);
                if !handled.is_handled() {
                    if let Some(a) = router::global_fallback(self.focus(), key) {
                        redraw |= self.on_global(a);
                    }
                }
                redraw
            }
            Routed::Ignore => false,
        }
    }

    fn on_paste(&mut self, text: String) -> bool {
        let (handled, effects) = self.tabs[self.active].handle_paste(text);
        let mut redraw = handled.is_handled();
        redraw |= self.apply_effects(effects);
        redraw
    }

    fn on_chord(&mut self, chord: Chord) -> bool {
        match chord {
            Chord::RestartCore => {
                self.apply_effect(Effect::RestartCore);
                self.apply_effect(Effect::ReconnectStreams);
                self.set_toast("重启内核…".into());
                true
            }
            Chord::ToggleSysProxy => {
                self.apply_effect(Effect::ToggleSysProxy);
                true
            }
        }
    }

    fn on_global(&mut self, action: GlobalAction) -> bool {
        match action {
            GlobalAction::SwitchTab(tab) => self.apply_effect(Effect::SwitchTab(tab)),
            GlobalAction::OpenHelp => {
                self.help_open = true;
                true
            }
            GlobalAction::Quit => {
                self.should_quit = true;
                false
            }
            GlobalAction::Refresh => {
                // 刷新当前 tab：触发其 on_focus 的刷新类 Effect。
                let effects = self.tabs[self.active].on_focus();
                self.apply_effects(effects);
                self.set_toast("刷新…".into());
                true
            }
        }
    }

    // ---------- 渲染 ----------

    fn draw(&self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        self.draw_tab_bar(f, chunks[0]);
        self.tabs[self.active].draw(chunks[1], f.buffer_mut(), true);
        self.draw_status_bar(f, chunks[2]);

        if self.help_open {
            self.draw_help(f, area);
        }
    }

    fn draw_tab_bar(&self, f: &mut Frame, area: Rect) {
        let mut spans: Vec<Span> = Vec::new();
        for (i, &id) in TabId::ORDER.iter().enumerate() {
            let style = if i == self.active {
                self.ctx.theme.tab_active()
            } else {
                self.ctx.theme.tab_inactive()
            };
            spans.push(Span::styled(format!(" {} ", id.title()), style));
            spans.push(Span::styled("│", self.ctx.theme.tab_inactive()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let hints = self.tabs[self.active].footer_hints();
        let left = match (&self.progress, &self.toast) {
            (Some(p), _) => format!(" {} ", format_progress(p, area.width as usize)),
            (None, Some(t)) => format!(" {t} "),
            (None, None) => format!(" {hints} "),
        };
        let right = " Tab/←→ 切换 · ? 帮助 · q 退出 ";
        let used = left.chars().count() + right.chars().count();
        let pad = (area.width as usize).saturating_sub(used);
        let line = Line::from(vec![
            Span::styled(left, self.ctx.theme.accent_style()),
            Span::raw(" ".repeat(pad)),
            Span::styled(right, self.ctx.theme.tab_inactive()),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let popup = centered_rect(60, 60, area);
        f.render_widget(Clear, popup);
        let t = &self.ctx.theme;
        let text = vec![
            Line::from(Span::styled("ClashTUI — 帮助", t.tab_active())),
            Line::from(""),
            Line::from("  ←/→ 或 Tab/Shift-Tab   切换 tab"),
            Line::from("  1-7                      跳到对应 tab"),
            Line::from("  ↑/↓                      列表内移动"),
            Line::from("  Enter                    选择/确认"),
            Line::from("  Ctrl+R                   重启内核"),
            Line::from("  Ctrl+P                   切换系统代理"),
            Line::from("  F5                       刷新当前 tab"),
            Line::from("  ?                        打开/关闭帮助"),
            Line::from("  q / Ctrl+C               退出"),
            Line::from(""),
            Line::from(Span::styled("  任意键关闭帮助", t.tab_inactive())),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(t.border_style(true));
        f.render_widget(
            Paragraph::new(text).block(block).wrap(Wrap { trim: false }),
            popup,
        );
        let _ = Style::default();
    }
}

/// mixin.yaml 初始模板。
const MIXIN_TEMPLATE: &str = "# ClashTUI mixin 配置：在不改原始订阅的前提下扩展运行时配置。
# 普通键深合并（mixin 优先）；数组段支持精细操作：
#   prepend-rules / append-rules
#   prepend-proxies / append-proxies / override-proxies（按 name）
#   prepend-proxy-groups / append-proxy-groups / override-proxy-groups（按 name）
#
# 示例：
# append-rules:
#   - 'DOMAIN-SUFFIX,example.com,DIRECT'
# log-level: info
";

fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

fn format_progress(p: &ProgressUpdate, area_width: usize) -> String {
    let available = area_width.saturating_sub(34).max(20);
    let label_limit = available.saturating_sub(34).clamp(8, 36);
    let label = truncate_chars(&p.label, label_limit);
    let bar = progress_bar(p.current, p.total, 14);
    match p.total {
        Some(total) if total > 0 => {
            let percent = ((p.current.min(total) as f64 / total as f64) * 100.0).round() as u64;
            format!(
                "{label} {bar} {percent:>3}% {}/{}",
                format_amount(p.current, total),
                format_amount(total, total)
            )
        }
        _ => format!("{label} {bar} {}", format_amount(p.current, 0)),
    }
}

fn progress_bar(current: u64, total: Option<u64>, width: usize) -> String {
    let filled = match total {
        Some(total) if total > 0 => {
            ((current.min(total) as f64 / total as f64) * width as f64).round() as usize
        }
        _ => ((current / 8192) as usize % width).saturating_add(1),
    }
    .min(width);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(width - filled))
}

fn format_amount(value: u64, total: u64) -> String {
    if total > 1024 * 1024 || value > 1024 * 1024 {
        format!("{:.1} MB", value as f64 / 1024.0 / 1024.0)
    } else if total > 1024 || value > 1024 {
        format!("{:.1} KB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
