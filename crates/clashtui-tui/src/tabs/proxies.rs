//! Proxies tab：TwoPane 组|节点，选节点 + 测速 + unfix。
//!
//! 左栏组列表（保序），右栏当前组的节点。`←/→` 切换聚焦栏，`↑/↓` 移动，
//! `Enter` 选节点（仅 Selector 组），`t` 测当前节点，`T` 测整组，`u` 解除固定。

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use clashtui_core_api::{Delay, Proxy};

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, TabId},
    theme::Theme,
    widgets::SelectableList,
};

/// 聚焦栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Groups,
    Nodes,
}

pub struct ProxiesTab {
    theme: Theme,
    controller: String,
    /// 组列表（保序）。
    groups: Vec<Proxy>,
    /// 所有 proxy 详情（含节点延迟）。
    all: HashMap<String, Proxy>,
    /// 单节点临时测速结果覆盖。
    delays: HashMap<String, Delay>,
    pane: Pane,
    group_list: SelectableList,
    node_list: SelectableList,
}

impl ProxiesTab {
    pub fn new(theme: Theme, controller: String) -> Self {
        ProxiesTab {
            theme,
            controller,
            groups: Vec::new(),
            all: HashMap::new(),
            delays: HashMap::new(),
            pane: Pane::Groups,
            group_list: SelectableList::new(0),
            node_list: SelectableList::new(0),
        }
    }

    fn current_group(&self) -> Option<&Proxy> {
        self.groups.get(self.group_list.selected)
    }

    /// 当前组的节点名列表。
    fn current_nodes(&self) -> Vec<String> {
        self.current_group()
            .map(|g| g.all.clone())
            .unwrap_or_default()
    }

    fn selected_node(&self) -> Option<String> {
        self.current_nodes().get(self.node_list.selected).cloned()
    }

    fn restore_selection_after_refresh(
        &mut self,
        previous_group: Option<String>,
        previous_node: Option<String>,
    ) {
        self.group_list
            .set_len_and_select_by(self.groups.len(), &self.groups, |group| {
                previous_group.as_deref() == Some(group.name.as_str())
            });

        let nodes = self.current_nodes();
        let preferred_node = previous_node
            .or_else(|| self.current_group().and_then(|group| group.now.clone()))
            .unwrap_or_default();
        self.node_list
            .set_len_and_select_value(nodes.len(), &nodes, &preferred_node);
    }

    fn groups_in_stable_order(&self, incoming: &[Proxy]) -> Vec<Proxy> {
        if self.groups.is_empty() {
            return incoming.to_vec();
        }

        let mut used = HashSet::new();
        let mut ordered = Vec::with_capacity(incoming.len());
        for old_group in &self.groups {
            if let Some(group) = incoming.iter().find(|group| group.name == old_group.name) {
                used.insert(group.name.clone());
                ordered.push(group.clone());
            }
        }
        for group in incoming {
            if used.insert(group.name.clone()) {
                ordered.push(group.clone());
            }
        }
        ordered
    }

    /// 某节点的延迟（优先临时结果，其次详情历史）。
    fn node_delay(&self, name: &str) -> Delay {
        if let Some(d) = self.delays.get(name) {
            return *d;
        }
        self.all
            .get(name)
            .map(|p| p.latest_delay())
            .unwrap_or(Delay(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, now: &str) -> Proxy {
        Proxy {
            name: name.to_string(),
            kind: "Selector".to_string(),
            all: vec!["A".to_string(), "B".to_string()],
            now: Some(now.to_string()),
            history: Vec::new(),
            udp: false,
            test_url: None,
            expected_status: None,
        }
    }

    #[test]
    fn refresh_keeps_group_order_and_selection_by_name() {
        let mut tab = ProxiesTab::new(Theme::default(), "http://127.0.0.1:9090".into());
        tab.groups = vec![group("Group A", "A"), group("Group B", "A")];
        tab.group_list = SelectableList::new(2);
        tab.group_list.selected = 1;
        tab.node_list = SelectableList::new(2);
        tab.node_list.selected = 1;

        tab.apply_event(&AppEvent::ProxiesLoaded {
            groups: vec![
                group("Group B", "B"),
                group("Group A", "A"),
                group("Group C", "A"),
            ],
            all: HashMap::new(),
        });

        let names: Vec<_> = tab.groups.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, vec!["Group A", "Group B", "Group C"]);
        assert_eq!(
            tab.current_group().map(|group| group.name.as_str()),
            Some("Group B")
        );
        assert_eq!(tab.selected_node().as_deref(), Some("B"));
    }
}

impl Component for ProxiesTab {
    fn id(&self) -> TabId {
        TabId::Proxies
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![Effect::RefreshProxies]
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        match key.code {
            KeyCode::Left => {
                self.pane = Pane::Groups;
                (Handled::Yes, vec![])
            }
            KeyCode::Right => {
                self.pane = Pane::Nodes;
                (Handled::Yes, vec![])
            }
            KeyCode::Up => {
                match self.pane {
                    Pane::Groups => {
                        self.group_list.up();
                        self.node_list = SelectableList::new(self.current_nodes().len());
                    }
                    Pane::Nodes => self.node_list.up(),
                }
                (Handled::Yes, vec![])
            }
            KeyCode::Down => {
                match self.pane {
                    Pane::Groups => {
                        self.group_list.down();
                        self.node_list = SelectableList::new(self.current_nodes().len());
                    }
                    Pane::Nodes => self.node_list.down(),
                }
                (Handled::Yes, vec![])
            }
            KeyCode::Enter => {
                // 选节点：仅 Selector 组。
                if let Some(group) = self.current_group() {
                    if !group.is_selector() {
                        return (
                            Handled::Yes,
                            vec![Effect::Toast(format!(
                                "{} 是 {} 组，不能手动选节点",
                                group.name, group.kind
                            ))],
                        );
                    }
                    let gname = group.name.clone();
                    if let Some(node) = self.selected_node() {
                        return (
                            Handled::Yes,
                            vec![Effect::SelectNode { group: gname, node }],
                        );
                    }
                }
                (Handled::Yes, vec![])
            }
            KeyCode::Char('t') => match self.selected_node() {
                Some(n) => (Handled::Yes, vec![Effect::TestNode(n)]),
                None => (Handled::Yes, vec![]),
            },
            KeyCode::Char('T') => match self.current_group() {
                Some(g) => (Handled::Yes, vec![Effect::TestGroup(g.name.clone())]),
                None => (Handled::Yes, vec![]),
            },
            KeyCode::Char('u') => match self.current_group() {
                Some(g) => (Handled::Yes, vec![Effect::UnfixGroup(g.name.clone())]),
                None => (Handled::Yes, vec![]),
            },
            _ => (Handled::No, vec![]),
        }
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::ProxiesLoaded { groups, all } => {
                let previous_group = self.current_group().map(|group| group.name.clone());
                let previous_node = self.selected_node();
                self.groups = self.groups_in_stable_order(groups);
                self.all = all.clone();
                self.restore_selection_after_refresh(previous_group, previous_node);
            }
            AppEvent::DelayResult { node, delay } => {
                self.delays.insert(node.clone(), *delay);
            }
            AppEvent::GroupDelayResult(map) => {
                for (k, v) in map {
                    self.delays.insert(k.clone(), Delay(*v));
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        self.draw_groups(cols[0], buf, focused && self.pane == Pane::Groups);
        self.draw_nodes(cols[1], buf, focused && self.pane == Pane::Nodes);
    }

    fn footer_hints(&self) -> &str {
        "←/→ 切栏 · ↑/↓ 移动 · Enter 选节点 · t 测节点 · T 测整组 · u 解除固定"
    }
}

impl ProxiesTab {
    fn draw_groups(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" 组 ", self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.groups.is_empty() {
            self.draw_empty_groups(inner, buf);
            return;
        }

        let viewport = inner.height as usize;
        let mut list = self.group_list.clone();
        let offset = list.adjust_offset(viewport);
        for (row, (i, g)) in self
            .groups
            .iter()
            .enumerate()
            .skip(offset)
            .take(viewport)
            .enumerate()
        {
            let y = inner.y + row as u16;
            let selected = i == self.group_list.selected;
            let style = if selected {
                self.theme.selected()
            } else {
                self.theme.fg_style()
            };
            let now = g.now.as_deref().unwrap_or("-");
            let text = format!("{} [{}] → {}", g.name, g.kind, now);
            let line = Line::from(Span::styled(text, style));
            buf.set_line(inner.x + 1, y, &line, inner.width.saturating_sub(1));
        }
    }

    fn draw_nodes(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let title = self
            .current_group()
            .map(|g| format!(" 节点：{} ", g.name))
            .unwrap_or_else(|| " 节点 ".into());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(title, self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        let nodes = self.current_nodes();
        if nodes.is_empty() {
            let line = if self.groups.is_empty() {
                "等待代理组加载"
            } else {
                "当前组没有可显示的节点"
            };
            Paragraph::new(Line::from(Span::styled(line, self.theme.tab_inactive())))
                .wrap(Wrap { trim: true })
                .render(inner, buf);
            return;
        }

        let now = self.current_group().and_then(|g| g.now.clone());
        let viewport = inner.height as usize;
        let mut list = self.node_list.clone();
        let offset = list.adjust_offset(viewport);

        for (row, (i, name)) in nodes
            .iter()
            .enumerate()
            .skip(offset)
            .take(viewport)
            .enumerate()
        {
            let y = inner.y + row as u16;
            let selected = i == self.node_list.selected;
            let is_now = now.as_deref() == Some(name.as_str());
            let base_style = if selected {
                self.theme.selected()
            } else {
                self.theme.fg_style()
            };
            let marker = if is_now { "●" } else { " " };
            let cursor = if selected { "›" } else { " " };
            let delay = self.node_delay(name);
            let delay_txt = delay.display();
            let delay_len = delay_txt.len() as u16;
            let delay_style = self.theme.delay_style(delay.millis());

            // 渲染名称（左）+ 右对齐延迟。
            let prefix = Span::styled(format!("{cursor} {marker} {name}"), base_style);
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(prefix),
                inner.width.saturating_sub(1),
            );
            let dx = inner.x + inner.width.saturating_sub(delay_len + 1);
            buf.set_line(
                dx,
                y,
                &Line::from(Span::styled(delay_txt, delay_style)),
                delay_len + 1,
            );
        }
    }

    fn draw_empty_groups(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            Line::from(Span::styled("暂无代理组", self.theme.tab_active())),
            Line::from(""),
            Line::from(Span::styled(
                "通常是 mihomo 外部控制器还没连上。",
                self.theme.fg_style(),
            )),
            Line::from(Span::styled(
                format!("当前 API: {}", self.controller),
                self.theme.tab_inactive(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "可在 Profiles 选中订阅按 Enter 加载。",
                self.theme.fg_style(),
            )),
            Line::from(Span::styled(
                "也可以去 Status 按 s 启动内核。",
                self.theme.fg_style(),
            )),
        ];
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .render(area, buf);
    }
}
