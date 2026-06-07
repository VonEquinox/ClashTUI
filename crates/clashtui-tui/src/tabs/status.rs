//! Status tab：版本、内核状态、模式、端口、TUN、实时流量/内存。

use crossterm::event::KeyEvent;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use clashtui_core_api::{GeneralConfig, Memory, Mode, Traffic, Version};
use clashtui_domain::{AppConfig, CoreStatus};

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, TabId},
    theme::Theme,
    widgets::human_bytes,
};

/// Status tab 视图状态。
pub struct StatusTab {
    theme: Theme,
    status: CoreStatus,
    version: Option<Version>,
    config: Option<GeneralConfig>,
    app_config: AppConfig,
    traffic: Traffic,
    memory: Memory,
    current_profile: Option<String>,
    stream_note: String,
}

impl StatusTab {
    pub fn new(theme: Theme) -> Self {
        StatusTab {
            theme,
            status: CoreStatus::Stopped,
            version: None,
            config: None,
            app_config: AppConfig::default(),
            traffic: Traffic::default(),
            memory: Memory::default(),
            current_profile: None,
            stream_note: String::new(),
        }
    }

    fn mode_str(&self) -> &str {
        self.config
            .as_ref()
            .and_then(|c| c.mode)
            .map(Mode::as_str)
            .unwrap_or("-")
    }
}

impl Component for StatusTab {
    fn id(&self) -> TabId {
        TabId::Status
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        use crossterm::event::KeyCode::*;
        match key.code {
            Char('s') => (Handled::Yes, vec![Effect::StartCore]),
            Char('S') => (Handled::Yes, vec![Effect::StopCore]),
            Char('R') => (Handled::Yes, vec![Effect::RestartCore]),
            _ => (Handled::No, vec![]),
        }
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        // 进入 Status 即刷新状态，并起 traffic/memory 流。
        vec![
            Effect::RefreshStatus,
            Effect::StartStream(crate::event::StreamId::Traffic),
            Effect::StartStream(crate::event::StreamId::Memory),
        ]
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::CoreStatus(s) => self.status = s.clone(),
            AppEvent::Version(v) => self.version = v.clone(),
            AppEvent::ConfigLoaded(c) => self.config = Some((**c).clone()),
            AppEvent::AppConfigLoaded(c) => self.app_config = (**c).clone(),
            AppEvent::WsTraffic(t) => self.traffic = *t,
            AppEvent::WsMemory(m) => self.memory = *m,
            AppEvent::WsConnected(k) => self.stream_note = format!("{k} 已连接"),
            AppEvent::WsDisconnected(k) => self.stream_note = format!("{k} 重连中…"),
            AppEvent::ProfilesChanged(list) => {
                self.current_profile = list.iter().find(|(_, cur)| *cur).map(|(n, _)| n.clone());
            }
            _ => {}
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" Status ", self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        let status_style = if self.status.is_running() {
            self.theme.ok_style()
        } else {
            self.theme.err_style()
        };

        let ver = self
            .version
            .as_ref()
            .map(|v| format!("{}{}", v.version, if v.meta { " (meta)" } else { "" }))
            .unwrap_or_else(|| "-".into());

        let ports = self
            .config
            .as_ref()
            .map(|c| {
                let sp = &self.app_config.system_proxy;
                let http = if c.port > 0 { c.port } else { sp.http_port };
                let socks = if c.socks_port > 0 {
                    c.socks_port
                } else {
                    sp.socks_port
                };
                let mixed = if c.mixed_port > 0 {
                    c.mixed_port
                } else {
                    sp.mixed_port
                };
                format!("http {http} / socks {socks} / mixed {mixed}")
            })
            .unwrap_or_else(|| "-".into());

        let tun = self
            .config
            .as_ref()
            .map(|c| if c.tun.enable { "on" } else { "off" })
            .unwrap_or("-");

        let rows = vec![
            row(&self.theme, "内核", &self.status.label(), status_style),
            row(&self.theme, "版本", &ver, self.theme.fg_style()),
            row(&self.theme, "模式", self.mode_str(), self.theme.fg_style()),
            row(&self.theme, "端口", &ports, self.theme.fg_style()),
            row(&self.theme, "TUN", tun, self.theme.fg_style()),
            row(
                &self.theme,
                "当前配置",
                self.current_profile.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            row(
                &self.theme,
                "流量",
                &format!(
                    "↑ {}/s  ↓ {}/s",
                    human_bytes(self.traffic.up),
                    human_bytes(self.traffic.down)
                ),
                self.theme.fg_style(),
            ),
            row(
                &self.theme,
                "内存",
                &human_bytes(self.memory.inuse),
                self.theme.fg_style(),
            ),
        ];

        let mut lines = rows;
        if !self.stream_note.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {}", self.stream_note),
                self.theme.tab_inactive(),
            )));
        }

        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        para.render(inner, buf);
    }

    fn footer_hints(&self) -> &str {
        "s 启动 · S 停止 · R 重启内核 · Ctrl+R 重启"
    }
}

fn row(
    theme: &Theme,
    label: &str,
    value: &str,
    value_style: ratatui::style::Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<10}"), theme.tab_inactive()),
        Span::styled(value.to_string(), value_style),
    ])
}
