//! Profiles tab：订阅列表 + 添加/删除/切换/更新。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, TabId},
    theme::Theme,
    widgets::{Prompt, SelectableList},
};

/// 添加订阅的输入步骤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddStep {
    /// 未在添加流程。
    Idle,
    /// 输入名称。
    Name,
    /// 输入 URL 或本地路径。
    Source,
}

/// 确认删除弹窗。
#[derive(Debug, Clone, Default)]
struct ConfirmDelete {
    open: bool,
    target: String,
}

pub struct ProfilesTab {
    theme: Theme,
    items: Vec<(String, bool)>, // (名称, 是否当前)
    list: SelectableList,
    add_step: AddStep,
    name_input: Prompt,
    source_input: Prompt,
    pending_name: String,
    confirm: ConfirmDelete,
}

impl ProfilesTab {
    pub fn new(theme: Theme) -> Self {
        ProfilesTab {
            theme,
            items: Vec::new(),
            list: SelectableList::new(0),
            add_step: AddStep::Idle,
            name_input: Prompt::new(),
            source_input: Prompt::new(),
            pending_name: String::new(),
            confirm: ConfirmDelete::default(),
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.items.get(self.list.selected).map(|(n, _)| n.clone())
    }

    fn begin_add(&mut self) {
        self.add_step = AddStep::Name;
        self.name_input.clear();
        self.source_input.clear();
    }

    fn cancel_add(&mut self) {
        self.add_step = AddStep::Idle;
    }
}

impl Component for ProfilesTab {
    fn id(&self) -> TabId {
        TabId::Profiles
    }

    fn capturing(&self) -> bool {
        self.add_step != AddStep::Idle || self.confirm.open
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![Effect::RefreshProfiles]
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        // 确认删除弹窗优先。
        if self.confirm.open {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let target = std::mem::take(&mut self.confirm.target);
                    self.confirm.open = false;
                    return (Handled::Yes, vec![Effect::DeleteProfile(target)]);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.confirm.open = false;
                    return (Handled::Yes, vec![]);
                }
                _ => return (Handled::Yes, vec![]),
            }
        }

        // 添加流程的输入捕获。
        match self.add_step {
            AddStep::Name => {
                match key.code {
                    KeyCode::Esc => self.cancel_add(),
                    KeyCode::Enter => {
                        if !self.name_input.is_empty() {
                            self.pending_name = self.name_input.text();
                            self.add_step = AddStep::Source;
                        }
                    }
                    _ => {
                        self.name_input.handle_key(key);
                    }
                }
                return (Handled::Yes, vec![]);
            }
            AddStep::Source => {
                match key.code {
                    KeyCode::Esc => self.cancel_add(),
                    KeyCode::Enter => {
                        if !self.source_input.is_empty() {
                            let source = self.source_input.text();
                            let is_url =
                                source.starts_with("http://") || source.starts_with("https://");
                            let name = std::mem::take(&mut self.pending_name);
                            self.add_step = AddStep::Idle;
                            return (
                                Handled::Yes,
                                vec![Effect::AddProfile {
                                    name,
                                    source,
                                    is_url,
                                }],
                            );
                        }
                    }
                    _ => {
                        self.source_input.handle_key(key);
                    }
                }
                return (Handled::Yes, vec![]);
            }
            AddStep::Idle => {}
        }

        // 常规列表导航。
        match key.code {
            KeyCode::Up => {
                self.list.up();
                (Handled::Yes, vec![])
            }
            KeyCode::Down => {
                self.list.down();
                (Handled::Yes, vec![])
            }
            KeyCode::Enter => match self.selected_name() {
                Some(n) => (Handled::Yes, vec![Effect::SwitchProfile(n)]),
                None => (Handled::Yes, vec![]),
            },
            KeyCode::Char('a') => {
                self.begin_add();
                (Handled::Yes, vec![])
            }
            KeyCode::Char('d') => {
                if let Some(n) = self.selected_name() {
                    self.confirm = ConfirmDelete {
                        open: true,
                        target: n,
                    };
                }
                (Handled::Yes, vec![])
            }
            KeyCode::Char('u') => match self.selected_name() {
                Some(n) => (Handled::Yes, vec![Effect::UpdateProfile(n)]),
                None => (Handled::Yes, vec![]),
            },
            KeyCode::Char('P') => match self.selected_name() {
                Some(n) => (Handled::Yes, vec![Effect::UpdateProfileViaProxy(n)]),
                None => (Handled::Yes, vec![]),
            },
            KeyCode::Char('U') => (Handled::Yes, vec![Effect::UpdateAllProfiles]),
            _ => (Handled::No, vec![]),
        }
    }

    fn handle_paste(&mut self, text: String) -> (Handled, Vec<Effect>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return (Handled::Yes, vec![]);
        }

        match self.add_step {
            AddStep::Name => {
                self.name_input.insert_str(&text);
                (Handled::Yes, vec![])
            }
            AddStep::Source => {
                self.source_input.insert_str(&text);
                (Handled::Yes, vec![])
            }
            AddStep::Idle => {
                if is_url(&text) {
                    (
                        Handled::Yes,
                        vec![Effect::AddProfileFromUrl(text), Effect::RefreshProfiles],
                    )
                } else {
                    (Handled::No, vec![])
                }
            }
        }
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        if let AppEvent::ProfilesChanged(list) = event {
            self.items = list.clone();
            self.list.set_len(self.items.len());
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" Profiles ", self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.items.is_empty() {
            let hint = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  还没有订阅。按 a 添加。",
                    self.theme.tab_inactive(),
                )),
            ]);
            hint.render(inner, buf);
        } else {
            let viewport = inner.height as usize;
            let mut list = self.list.clone();
            let offset = list.adjust_offset(viewport);
            for (row, (i, (name, current))) in self
                .items
                .iter()
                .enumerate()
                .skip(offset)
                .take(viewport)
                .enumerate()
            {
                let y = inner.y + row as u16;
                let selected = i == self.list.selected;
                let style = if selected {
                    self.theme.selected()
                } else {
                    self.theme.fg_style()
                };
                let marker = if *current { "●" } else { " " };
                let cursor = if selected { "›" } else { " " };
                let line = Line::from(vec![Span::styled(
                    format!("{cursor} {marker} {name}"),
                    style,
                )]);
                buf.set_line(inner.x + 1, y, &line, inner.width.saturating_sub(1));
            }
        }

        // 叠加弹窗。
        if self.add_step != AddStep::Idle {
            self.draw_add_popup(area, buf);
        }
        if self.confirm.open {
            self.draw_confirm(area, buf);
        }
    }

    fn footer_hints(&self) -> &str {
        match self.add_step {
            AddStep::Name => "输入名称 · Enter 下一步 · Esc 取消",
            AddStep::Source => "输入 URL 或文件路径 · Enter 确认 · Esc 取消",
            AddStep::Idle => {
                if self.confirm.open {
                    "y 确认删除 · n/Esc 取消"
                } else {
                    "↑/↓ 选择 · Enter 切换 · a 添加 · d 删除 · u 更新 · P 代理更新 · U 全部更新"
                }
            }
        }
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

impl ProfilesTab {
    fn draw_add_popup(&self, area: Rect, buf: &mut Buffer) {
        let popup = centered(60, 7, area);
        Clear.render(popup, buf);
        let (label, value, active2) = match self.add_step {
            AddStep::Name => ("名称", self.name_input.text(), false),
            AddStep::Source => ("URL/路径", self.source_input.text(), true),
            AddStep::Idle => return,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(Span::styled(" 添加订阅 ", self.theme.tab_active()));
        let inner = block.inner(popup);
        block.render(popup, buf);

        let mut lines = vec![Line::from(vec![
            Span::styled(format!("  {label}: "), self.theme.tab_inactive()),
            Span::styled(format!("{value}_"), self.theme.accent_style()),
        ])];
        if active2 {
            lines.insert(
                0,
                Line::from(Span::styled(
                    format!("  名称: {}", self.pending_name),
                    self.theme.tab_inactive(),
                )),
            );
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }

    fn draw_confirm(&self, area: Rect, buf: &mut Buffer) {
        let popup = centered(50, 5, area);
        Clear.render(popup, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(Span::styled(" 确认删除 ", self.theme.err_style()));
        let inner = block.inner(popup);
        block.render(popup, buf);
        Paragraph::new(vec![
            Line::from(""),
            Line::from(format!("  删除订阅 \"{}\"？", self.confirm.target)),
            Line::from(Span::styled(
                "  y 确认 · n/Esc 取消",
                self.theme.tab_inactive(),
            )),
        ])
        .render(inner, buf);
    }
}

/// 居中矩形（按字符宽高）。
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
