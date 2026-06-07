//! Status tab：版本、内核状态、模式、端口、TUN、实时流量/内存。

use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::KeyEvent;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
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
        let group = preferred_group(groups, all);
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
            .unwrap_or_else(|| "未测速".to_string());

        let left_rows = vec![
            info_row("内核", self.status.label(), status_style),
            info_row("版本", ver, self.theme.fg_style()),
            info_row("模式", self.mode_str(), self.theme.fg_style()),
            info_row("端口", ports, self.theme.fg_style()),
            info_row("TUN", tun, self.theme.fg_style()),
            info_row(
                "当前配置",
                self.current_profile.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
        ];
        let right_rows = vec![
            info_row(
                "代理组",
                self.current_group.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            info_row(
                "节点名",
                self.current_node.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            info_row(
                "节点类型",
                self.current_node_kind.as_deref().unwrap_or("-"),
                self.theme.fg_style(),
            ),
            info_row("节点延迟", delay, self.theme.fg_style()),
            info_row(
                "流量",
                format!(
                    "↑ {}/s  ↓ {}/s",
                    human_bytes(self.traffic.up),
                    human_bytes(self.traffic.down)
                ),
                self.theme.fg_style(),
            ),
            info_row(
                "内存",
                human_bytes(self.memory.inuse),
                self.theme.fg_style(),
            ),
        ];
        let info_rows =
            left_rows.len().max(right_rows.len()) as u16 + u16::from(!self.stream_note.is_empty());

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(info_rows),
                Constraint::Min(3),
                Constraint::Min(3),
            ])
            .split(inner);

        render_info_grid(
            sections[0],
            buf,
            &self.theme,
            &left_rows,
            &right_rows,
            self.stream_note.as_str(),
        );

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

fn preferred_group<'a>(groups: &'a [Proxy], all: &HashMap<String, Proxy>) -> Option<&'a Proxy> {
    const PREFERRED_NAMES: [&str; 5] = ["节点选择", "🚀 节点选择", "PROXY", "Proxy", "代理"];
    for name in PREFERRED_NAMES {
        if let Some(group) = groups
            .iter()
            .find(|group| group.name == name && resolves_to_real_node(group, all))
        {
            return Some(group);
        }
    }
    groups
        .iter()
        .find(|group| {
            !group.name.eq_ignore_ascii_case("GLOBAL")
                && group.is_selector()
                && resolves_to_real_node(group, all)
        })
        .or_else(|| {
            groups.iter().find(|group| {
                !group.name.eq_ignore_ascii_case("GLOBAL") && resolves_to_real_node(group, all)
            })
        })
        .or_else(|| {
            groups
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case("GLOBAL") && group.now.is_some())
        })
        .or_else(|| groups.iter().find(|group| group.now.is_some()))
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
                delay: proxy.and_then(latest_known_delay),
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
                    delay: latest_known_delay(proxy),
                });
            }
            Some(proxy) => {
                return Some(SelectedNode {
                    name: proxy.name.clone(),
                    kind: Some(proxy.kind.clone()),
                    delay: latest_known_delay(proxy),
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

fn latest_known_delay(proxy: &Proxy) -> Option<Delay> {
    proxy.history.last().map(|history| Delay(history.delay))
}

fn resolves_to_real_node(group: &Proxy, all: &HashMap<String, Proxy>) -> bool {
    resolve_selected_node(group, all)
        .as_ref()
        .is_some_and(|node| !node.is_builtin())
}

impl SelectedNode {
    fn is_builtin(&self) -> bool {
        let upper_name = self.name.to_ascii_uppercase();
        if matches!(
            upper_name.as_str(),
            "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS"
        ) {
            return true;
        }

        self.kind
            .as_deref()
            .is_some_and(|kind| matches!(kind, "Direct" | "Reject" | "Pass"))
    }
}

struct InfoRow {
    label: &'static str,
    value: String,
    value_style: Style,
}

fn info_row(label: &'static str, value: impl Into<String>, value_style: Style) -> InfoRow {
    InfoRow {
        label,
        value: value.into(),
        value_style,
    }
}

fn render_info_grid(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    left_rows: &[InfoRow],
    right_rows: &[InfoRow],
    stream_note: &str,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    render_info_column(columns[0], buf, theme, left_rows);
    render_info_column(columns[1], buf, theme, right_rows);

    if !stream_note.is_empty() {
        let y = area.y + left_rows.len().max(right_rows.len()) as u16;
        if y < area.bottom() {
            Paragraph::new(Line::from(vec![
                Span::styled("数据流", theme.tab_inactive()),
                Span::raw("  "),
                Span::styled(stream_note.to_string(), theme.tab_inactive()),
            ]))
            .render(Rect::new(area.x, y, area.width, 1), buf);
        }
    }
}

fn render_info_column(area: Rect, buf: &mut Buffer, theme: &Theme, rows: &[InfoRow]) {
    const LABEL_WIDTH: u16 = 10;
    for (idx, row) in rows.iter().enumerate() {
        let y = area.y + idx as u16;
        if y >= area.bottom() {
            break;
        }
        let row_area = Rect::new(area.x, y, area.width, 1);
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(LABEL_WIDTH), Constraint::Min(1)])
            .split(row_area);
        Paragraph::new(row.label)
            .style(theme.tab_inactive())
            .render(cells[0], buf);
        Paragraph::new(row.value.as_str())
            .style(row.value_style)
            .wrap(Wrap { trim: true })
            .render(cells[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clashtui_core_api::DelayHistory;

    fn group(name: &str, now: Option<&str>) -> Proxy {
        Proxy {
            name: name.to_string(),
            kind: "Selector".to_string(),
            all: now.map(|node| vec![node.to_string()]).unwrap_or_default(),
            now: now.map(str::to_string),
            history: Vec::new(),
            udp: false,
            test_url: None,
            expected_status: None,
        }
    }

    fn builtin(name: &str, kind: &str) -> Proxy {
        Proxy {
            name: name.to_string(),
            kind: kind.to_string(),
            all: vec![],
            now: None,
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
    fn preferred_group_uses_real_selector_before_global_direct() {
        let groups = vec![
            group("GLOBAL", Some("DIRECT")),
            group("节点选择", Some("Node A")),
        ];
        let mut all = HashMap::new();
        all.insert("DIRECT".to_string(), builtin("DIRECT", "Direct"));
        all.insert("Node A".to_string(), node("Node A"));

        assert_eq!(
            preferred_group(&groups, &all).map(|group| group.name.as_str()),
            Some("节点选择")
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

    #[test]
    fn status_marks_missing_delay_as_untested() {
        let mut node = node("Node A");
        node.history.clear();

        let mut tab = StatusTab::new(Theme::default());
        let groups = vec![group("节点选择", Some("Node A"))];
        let mut all = HashMap::new();
        all.insert("Node A".to_string(), node);

        tab.apply_proxies(&groups, &all);

        assert_eq!(tab.current_group.as_deref(), Some("节点选择"));
        assert_eq!(tab.current_node.as_deref(), Some("Node A"));
        assert_eq!(tab.current_node_delay, None);
    }
}
