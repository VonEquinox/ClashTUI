//! Connections tab：消费 `/connections` WS 流，表格展示 + 关闭单条/全部。
//!
//! 聚焦时起流、失焦停流（重流量数据仅在需要时拉取）。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

use clashtui_core_api::Connection;

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, StreamId, TabId},
    theme::Theme,
    widgets::{human_bytes, SelectableList},
};

pub struct ConnectionsTab {
    theme: Theme,
    conns: Vec<Connection>,
    list: SelectableList,
    download_total: u64,
    upload_total: u64,
    paused: bool,
    /// 待确认关闭全部。
    confirm_close_all: bool,
    /// 'd' 的第一击，等待第二击 'd'（dd 关单条）。
    pending_d: bool,
}

impl ConnectionsTab {
    pub fn new(theme: Theme) -> Self {
        ConnectionsTab {
            theme,
            conns: Vec::new(),
            list: SelectableList::new(0),
            download_total: 0,
            upload_total: 0,
            paused: false,
            confirm_close_all: false,
            pending_d: false,
        }
    }

    fn selected_id(&self) -> Option<String> {
        self.conns.get(self.list.selected).map(|c| c.id.clone())
    }
}

impl Component for ConnectionsTab {
    fn id(&self) -> TabId {
        TabId::Connections
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![Effect::StartStream(StreamId::Connections)]
    }

    fn on_blur(&mut self) -> Vec<Effect> {
        vec![Effect::StopStream(StreamId::Connections)]
    }

    fn capturing(&self) -> bool {
        self.confirm_close_all
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        if self.confirm_close_all {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.confirm_close_all = false;
                    return (Handled::Yes, vec![Effect::CloseAllConns]);
                }
                _ => {
                    self.confirm_close_all = false;
                    return (Handled::Yes, vec![]);
                }
            }
        }

        // dd 关单条：第一击 'd' 后等第二击。
        if self.pending_d {
            self.pending_d = false;
            if key.code == KeyCode::Char('d') {
                if let Some(id) = self.selected_id() {
                    return (Handled::Yes, vec![Effect::CloseConn(id)]);
                }
                return (Handled::Yes, vec![]);
            }
            // 非第二击 'd'，落到普通处理。
        }

        match key.code {
            KeyCode::Up => {
                self.list.up();
                (Handled::Yes, vec![])
            }
            KeyCode::Down => {
                self.list.down();
                (Handled::Yes, vec![])
            }
            KeyCode::Char('d') => {
                self.pending_d = true;
                (Handled::Yes, vec![])
            }
            KeyCode::Char('a') => {
                self.confirm_close_all = true;
                (Handled::Yes, vec![])
            }
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                (Handled::Yes, vec![])
            }
            _ => (Handled::No, vec![]),
        }
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        if let AppEvent::WsConnections {
            download_total,
            upload_total,
            connections,
        } = event
        {
            if !self.paused {
                self.download_total = *download_total;
                self.upload_total = *upload_total;
                self.conns = connections.clone();
                self.list.set_len(self.conns.len());
            }
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let title = format!(
            " Connections ({})  ↑{}  ↓{} ",
            self.conns.len(),
            human_bytes(self.upload_total),
            human_bytes(self.download_total),
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(title, self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        let viewport = inner.height as usize;
        if viewport == 0 {
            return;
        }
        let mut list = self.list.clone();
        let offset = list.adjust_offset(viewport);

        for (row, (i, c)) in self
            .conns
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
            let cursor = if selected { "›" } else { " " };
            let host = if c.metadata.host.is_empty() {
                format!(
                    "{}:{}",
                    c.metadata.destination_ip, c.metadata.destination_port
                )
            } else {
                format!("{}:{}", c.metadata.host, c.metadata.destination_port)
            };
            let chain = c.chains.first().cloned().unwrap_or_default();
            let text = format!(
                "{cursor} {:<28} {:<10} ↑{} ↓{}  [{}]",
                truncate(&host, 28),
                truncate(&c.rule, 10),
                human_bytes(c.upload),
                human_bytes(c.download),
                chain,
            );
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(Span::styled(text, style)),
                inner.width.saturating_sub(1),
            );
        }

        if self.confirm_close_all {
            let msg = Line::from(Span::styled(
                " 关闭全部连接？ y 确认 · 其它键取消 ",
                self.theme.err_style(),
            ));
            buf.set_line(inner.x + 1, inner.y, &msg, inner.width.saturating_sub(1));
        }
    }

    fn footer_hints(&self) -> &str {
        "↑/↓ 选择 · dd 关单条 · a 关全部 · p 暂停"
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
