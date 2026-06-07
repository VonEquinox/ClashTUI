//! Settings tab：模式切换、TUN 开关、系统代理、内核升级、配置展示。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use clashtui_core_api::{GeneralConfig, Mode};
use clashtui_domain::AppConfig;

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, TabId},
    theme::Theme,
    widgets::Prompt,
};

/// 设置项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Mode,
    Tun,
    HttpPort,
    SocksPort,
    MixedPort,
    SysProxy,
    KeepCoreRunning,
    EditMixin,
    UpgradeKernel,
    RestartCore,
}

impl Item {
    const ALL: [Item; 10] = [
        Item::Mode,
        Item::Tun,
        Item::HttpPort,
        Item::SocksPort,
        Item::MixedPort,
        Item::SysProxy,
        Item::KeepCoreRunning,
        Item::EditMixin,
        Item::UpgradeKernel,
        Item::RestartCore,
    ];

    fn label(self) -> &'static str {
        match self {
            Item::Mode => "代理模式",
            Item::Tun => "TUN 模式",
            Item::HttpPort => "HTTP 端口",
            Item::SocksPort => "SOCKS 端口",
            Item::MixedPort => "Mixed 端口",
            Item::SysProxy => "系统代理",
            Item::KeepCoreRunning => "退出保留内核",
            Item::EditMixin => "编辑 Mixin",
            Item::UpgradeKernel => "升级内核",
            Item::RestartCore => "重启内核",
        }
    }
}

pub struct SettingsTab {
    theme: Theme,
    config: Option<GeneralConfig>,
    app_config: AppConfig,
    selected: usize,
    /// 已知系统代理开启状态（仅展示，真实状态由 OS 决定）。
    sysproxy_on: bool,
    controller: String,
    editing_port: Option<Item>,
    port_input: Prompt,
}

impl SettingsTab {
    pub fn new(theme: Theme, controller: String) -> Self {
        SettingsTab {
            theme,
            config: None,
            app_config: AppConfig::default(),
            selected: 0,
            sysproxy_on: false,
            controller,
            editing_port: None,
            port_input: Prompt::new(),
        }
    }

    fn current_mode(&self) -> Option<Mode> {
        self.config.as_ref().and_then(|c| c.mode)
    }

    fn tun_on(&self) -> bool {
        self.config.as_ref().map(|c| c.tun.enable).unwrap_or(false)
    }

    fn http_port(&self) -> u16 {
        self.config
            .as_ref()
            .map(|c| c.port)
            .filter(|p| *p > 0)
            .unwrap_or(self.app_config.system_proxy.http_port)
    }

    fn socks_port(&self) -> u16 {
        self.config
            .as_ref()
            .map(|c| c.socks_port)
            .filter(|p| *p > 0)
            .unwrap_or(self.app_config.system_proxy.socks_port)
    }

    fn mixed_port(&self) -> u16 {
        self.config
            .as_ref()
            .map(|c| c.mixed_port)
            .filter(|p| *p > 0)
            .unwrap_or(self.app_config.system_proxy.mixed_port)
    }

    fn keep_core_running(&self) -> bool {
        self.app_config.keep_core_running
    }

    fn activate(&mut self) -> Vec<Effect> {
        match Item::ALL[self.selected] {
            Item::Mode => {
                let next = self.current_mode().unwrap_or(Mode::Rule).next();
                vec![Effect::SwitchMode(next)]
            }
            Item::Tun => vec![Effect::ToggleTun(!self.tun_on())],
            Item::HttpPort => {
                self.start_edit_port(Item::HttpPort);
                vec![]
            }
            Item::SocksPort => {
                self.start_edit_port(Item::SocksPort);
                vec![]
            }
            Item::MixedPort => {
                self.start_edit_port(Item::MixedPort);
                vec![]
            }
            Item::SysProxy => vec![Effect::ToggleSysProxy],
            Item::KeepCoreRunning => {
                vec![Effect::SetKeepCoreRunning(!self.keep_core_running())]
            }
            Item::EditMixin => vec![Effect::EditMixin],
            Item::UpgradeKernel => vec![Effect::UpgradeKernel],
            Item::RestartCore => vec![Effect::RestartCore, Effect::ReconnectStreams],
        }
    }

    fn start_edit_port(&mut self, item: Item) {
        let value = match item {
            Item::HttpPort => self.http_port(),
            Item::SocksPort => self.socks_port(),
            Item::MixedPort => self.mixed_port(),
            _ => return,
        };
        self.editing_port = Some(item);
        self.port_input.set_text(&value.to_string());
    }

    fn finish_edit_port(&mut self) -> Vec<Effect> {
        let Some(item) = self.editing_port.take() else {
            return vec![];
        };
        let text = self.port_input.text();
        let Ok(port) = text.parse::<u16>() else {
            return vec![Effect::Toast("端口必须是 1-65535 的数字".into())];
        };
        if port == 0 {
            return vec![Effect::Toast("端口必须大于 0".into())];
        }
        let (http_port, socks_port, mixed_port) = match item {
            Item::HttpPort => (port, self.socks_port(), self.mixed_port()),
            Item::SocksPort => (self.http_port(), port, self.mixed_port()),
            Item::MixedPort => (self.http_port(), self.socks_port(), port),
            _ => return vec![],
        };
        if http_port == socks_port || http_port == mixed_port || socks_port == mixed_port {
            return vec![Effect::Toast("HTTP、SOCKS、Mixed 端口不能重复".into())];
        }
        match item {
            Item::HttpPort | Item::SocksPort | Item::MixedPort => vec![Effect::SetProxyPorts {
                http_port,
                socks_port,
                mixed_port,
            }],
            _ => vec![],
        }
    }
}

impl Component for SettingsTab {
    fn id(&self) -> TabId {
        TabId::Settings
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![Effect::RefreshStatus]
    }

    fn capturing(&self) -> bool {
        self.editing_port.is_some()
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        if self.editing_port.is_some() {
            match key.code {
                KeyCode::Enter => return (Handled::Yes, self.finish_edit_port()),
                KeyCode::Esc => {
                    self.editing_port = None;
                    return (Handled::Yes, vec![]);
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    self.port_input.handle_key(key);
                    return (Handled::Yes, vec![]);
                }
                KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End => {
                    self.port_input.handle_key(key);
                    return (Handled::Yes, vec![]);
                }
                _ => return (Handled::Yes, vec![]),
            }
        }

        match key.code {
            KeyCode::Up => {
                self.selected = if self.selected == 0 {
                    Item::ALL.len() - 1
                } else {
                    self.selected - 1
                };
                (Handled::Yes, vec![])
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1) % Item::ALL.len();
                (Handled::Yes, vec![])
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ') => {
                let effects = self.activate();
                (Handled::Yes, effects)
            }
            _ => (Handled::No, vec![]),
        }
    }

    fn handle_paste(&mut self, text: String) -> (Handled, Vec<Effect>) {
        if self.editing_port.is_some() {
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            self.port_input.insert_str(&digits);
            return (Handled::Yes, vec![]);
        }
        (Handled::No, vec![])
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::ConfigLoaded(c) => self.config = Some((**c).clone()),
            AppEvent::AppConfigLoaded(c) => self.app_config = (**c).clone(),
            AppEvent::Toast(msg) if msg.contains("系统代理已开启") => {
                self.sysproxy_on = true
            }
            AppEvent::Toast(msg) if msg.contains("系统代理已关闭") => {
                self.sysproxy_on = false
            }
            _ => {}
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" Settings ", self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        for (i, item) in Item::ALL.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let selected = i == self.selected;
            let cursor = if selected { "›" } else { " " };
            let value = match item {
                Item::Mode => self
                    .current_mode()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "-".into()),
                Item::Tun => if self.tun_on() { "on" } else { "off" }.into(),
                Item::HttpPort => self.http_port().to_string(),
                Item::SocksPort => self.socks_port().to_string(),
                Item::MixedPort => self.mixed_port().to_string(),
                Item::SysProxy => if self.sysproxy_on { "on" } else { "off" }.into(),
                Item::KeepCoreRunning => if self.keep_core_running() {
                    "on"
                } else {
                    "off"
                }
                .into(),
                Item::EditMixin => "↵ $EDITOR".into(),
                Item::UpgradeKernel => "↵ 执行".into(),
                Item::RestartCore => "↵ 执行".into(),
            };
            let style = if selected {
                self.theme.selected()
            } else {
                self.theme.fg_style()
            };
            let line = Line::from(vec![
                Span::styled(format!("{cursor} {:<10}", item.label()), style),
                Span::styled(value, self.theme.accent_style()),
            ]);
            buf.set_line(inner.x + 1, y, &line, inner.width.saturating_sub(1));
        }

        // 底部展示只读信息。
        let info_y = inner.y + Item::ALL.len() as u16 + 1;
        if info_y < inner.y + inner.height {
            let info = Line::from(Span::styled(
                format!("  external-controller: {}", self.controller),
                self.theme.tab_inactive(),
            ));
            buf.set_line(inner.x + 1, info_y, &info, inner.width.saturating_sub(1));
        }

        if self.editing_port.is_some() {
            self.draw_port_popup(area, buf);
        }
    }

    fn footer_hints(&self) -> &str {
        if self.editing_port.is_some() {
            "输入端口或粘贴数字 · Enter 保存 · Esc 取消"
        } else {
            "↑/↓ 选择 · Enter/→ 切换或执行"
        }
    }
}

impl SettingsTab {
    fn draw_port_popup(&self, area: Rect, buf: &mut Buffer) {
        let popup = centered(52, 5, area);
        Clear.render(popup, buf);
        let title = match self.editing_port {
            Some(Item::HttpPort) => " 设置 HTTP 端口 ",
            Some(Item::SocksPort) => " 设置 SOCKS 端口 ",
            Some(Item::MixedPort) => " 设置 Mixed 端口 ",
            _ => " 设置端口 ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(Span::styled(title, self.theme.tab_active()));
        let inner = block.inner(popup);
        block.render(popup, buf);
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("  端口: ", self.theme.tab_inactive()),
                Span::styled(
                    format!("{}_", self.port_input.text()),
                    self.theme.accent_style(),
                ),
            ]),
            Line::from(Span::styled(
                "  Enter 保存 · Esc 取消",
                self.theme.tab_inactive(),
            )),
        ])
        .wrap(Wrap { trim: false })
        .render(inner, buf);
    }
}

fn centered(w: u16, h: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}
