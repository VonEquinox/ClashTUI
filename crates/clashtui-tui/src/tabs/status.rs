//! Status tab：版本、内核状态、模式、端口、TUN、实时流量/内存。

use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::KeyEvent;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline, Widget, Wrap},
};

use clashtui_core_api::{Delay, GeneralConfig, Memory, Mode, Proxy, Traffic, Version};
use clashtui_domain::{AppConfig, CoreStatus};

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, TabId},
    theme::Theme,
    widgets::human_bytes,
};

const TRAFFIC_HISTORY: usize = 120;

/// Status tab 视图状态。
pub struct StatusTab {
    theme: Theme,
    status: CoreStatus,
    version: Option<Version>,
    config: Option<GeneralConfig>,
    app_config: AppConfig,
    traffic: Traffic,
    traffic_up: VecDeque<u64>,
    traffic_down: VecDeque<u64>,
    memory: Memory,
    current_profile: Option<String>,
    current_group: Option<String>,
    current_node: Option<String>,
    current_node_kind: Option<String>,
    current_node_delay: Option<Delay>,
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
            traffic_up: VecDeque::from(vec![0; TRAFFIC_HISTORY]),
            traffic_down: VecDeque::from(vec![0; TRAFFIC_HISTORY]),
            memory: Memory::default(),
            current_profile: None,
            current_group: None,
            current_node: None,
            current_node_kind: None,
            current_node_delay: None,
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

    fn push_traffic(&mut self, traffic: Traffic) {
        self.traffic = traffic;
        if self.traffic_up.len() >= TRAFFIC_HISTORY {
            self.traffic_up.pop_front();
            self.traffic_down.pop_front();
        }
        self.traffic_up.push_back(traffic.up);
        self.traffic_down.push_back(traffic.down);
    }

    fn apply_proxies(&mut self, groups: &[Proxy], all: &HashMap<String, Proxy>) {
        let group = preferred_group(groups);
        self.current_group = group.map(|group| group.name.clone());
        let node = group.and_then(|group| resolve_selected_node(group, all));
        self.current_node = node.as_ref().map(|node| node.name.clone());
        self.current_node_kind = node.as_ref().and_then(|node| node.kind.clone());
        self.current_node_delay = node.and_then(|node| node.delay);
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
            Effect::RefreshProxies,
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
            AppEvent::ProxiesLoaded { groups, all } => self.apply_proxies(groups, all),
            AppEvent::DelayResult { node, delay } if self.current_node.as_deref() == Some(node) => {
                self.current_node_delay = Some(*delay);
            }
            AppEvent::GroupDelayResult(map) => {
                if let Some(delay) = self.current_node.as_deref().and_then(|node| map.get(node)) {
                    self.current_node_delay = Some(Delay(*delay));
                }
            }
            AppEvent::WsTraffic(t) => self.push_traffic(*t),
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

        let delay = self
            .current_node_delay
            .map(|delay| delay.display())
            .unwrap_or_else(|| "-".to_string());

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
                "代理组",
                self.current_group.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            row(
                &self.theme,
                "节点名",
                self.current_node.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            row(
                &self.theme,
                "节点类型",
                self.current_node_kind.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            row(&self.theme, "节点延迟", &delay, self.theme.fg_style()),
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

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(lines.len() as u16),
                Constraint::Min(3),
                Constraint::Min(3),
            ])
            .split(inner);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(sections[0], buf);

        let up: Vec<u64> = self.traffic_up.iter().copied().collect();
        let down: Vec<u64> = self.traffic_down.iter().copied().collect();
        Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style(focused))
                    .title(Span::styled(" 上行 ↑ ", self.theme.ok_style())),
            )
            .data(&up)
            .style(self.theme.ok_style())
            .render(sections[1], buf);
        Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style(focused))
                    .title(Span::styled(" 下行 ↓ ", self.theme.accent_style())),
            )
            .data(&down)
            .style(self.theme.accent_style())
            .render(sections[2], buf);
    }

    fn footer_hints(&self) -> &str {
        "s 启动 · S 停止 · R 重启内核 · Ctrl+R 重启"
    }
}

fn preferred_group(groups: &[Proxy]) -> Option<&Proxy> {
    const PREFERRED_NAMES: [&str; 7] = [
        "GLOBAL",
        "Global",
        "PROXY",
        "Proxy",
        "代理",
        "节点选择",
        "🚀 节点选择",
    ];
    for name in PREFERRED_NAMES {
        if let Some(group) = groups
            .iter()
            .find(|group| group.name == name && group.now.is_some())
        {
            return Some(group);
        }
    }
    groups.iter().find(|group| group.now.is_some())
}

struct SelectedNode {
    name: String,
    kind: Option<String>,
    delay: Option<Delay>,
}

fn resolve_selected_node(group: &Proxy, all: &HashMap<String, Proxy>) -> Option<SelectedNode> {
    let mut name = group.now.as_deref()?;
    let mut seen = HashSet::new();
    loop {
        let proxy = all.get(name);
        if !seen.insert(name.to_string()) {
            return Some(SelectedNode {
                name: name.to_string(),
                kind: proxy.map(|proxy| proxy.kind.clone()),
                delay: proxy.map(|proxy| proxy.latest_delay()),
            });
        }

        match proxy {
            Some(proxy) if proxy.is_group() => {
                if let Some(next) = proxy.now.as_deref() {
                    name = next;
                    continue;
                }
                return Some(SelectedNode {
                    name: proxy.name.clone(),
                    kind: Some(proxy.kind.clone()),
                    delay: Some(proxy.latest_delay()),
                });
            }
            Some(proxy) => {
                return Some(SelectedNode {
                    name: proxy.name.clone(),
                    kind: Some(proxy.kind.clone()),
                    delay: Some(proxy.latest_delay()),
                });
            }
            None => {
                return Some(SelectedNode {
                    name: name.to_string(),
                    kind: None,
                    delay: None,
                });
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clashtui_core_api::DelayHistory;

    fn group(name: &str, now: Option<&str>) -> Proxy {
        Proxy {
            name: name.to_string(),
            kind: "Selector".to_string(),
            all: vec![],
            now: now.map(str::to_string),
            history: Vec::new(),
            udp: false,
            test_url: None,
            expected_status: None,
        }
    }

    fn node(name: &str) -> Proxy {
        Proxy {
            name: name.to_string(),
            kind: "Shadowsocks".to_string(),
            all: vec![],
            now: None,
            history: vec![DelayHistory {
                time: "t".to_string(),
                delay: 123,
            }],
            udp: true,
            test_url: None,
            expected_status: None,
        }
    }

    #[test]
    fn preferred_group_uses_global_when_available() {
        let groups = vec![group("节点选择", Some("A")), group("GLOBAL", Some("B"))];
        assert_eq!(
            preferred_group(&groups).map(|group| group.name.as_str()),
            Some("GLOBAL")
        );
    }

    #[test]
    fn status_extracts_current_node_from_proxies() {
        let mut tab = StatusTab::new(Theme::default());
        let groups = vec![group("GLOBAL", Some("Node A"))];
        let mut all = HashMap::new();
        all.insert("Node A".to_string(), node("Node A"));

        tab.apply_proxies(&groups, &all);

        assert_eq!(tab.current_group.as_deref(), Some("GLOBAL"));
        assert_eq!(tab.current_node.as_deref(), Some("Node A"));
        assert_eq!(tab.current_node_kind.as_deref(), Some("Shadowsocks"));
        assert_eq!(tab.current_node_delay.map(|delay| delay.0), Some(123));
    }

    #[test]
    fn status_resolves_nested_group_to_leaf_node() {
        let mut tab = StatusTab::new(Theme::default());
        let groups = vec![group("GLOBAL", Some("节点选择"))];
        let mut all = HashMap::new();
        all.insert("节点选择".to_string(), group("节点选择", Some("Node A")));
        all.insert("Node A".to_string(), node("Node A"));

        tab.apply_proxies(&groups, &all);

        assert_eq!(tab.current_group.as_deref(), Some("GLOBAL"));
        assert_eq!(tab.current_node.as_deref(), Some("Node A"));
        assert_eq!(tab.current_node_kind.as_deref(), Some("Shadowsocks"));
        assert_eq!(tab.current_node_delay.map(|delay| delay.0), Some(123));
    }
}
