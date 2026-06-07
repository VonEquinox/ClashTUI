//! Traffic tab：实时上下行速率曲线 + 内存。

use std::collections::VecDeque;

use crossterm::event::KeyEvent;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Sparkline, Widget},
};

use clashtui_core_api::{Memory, Traffic};

use crate::{
    component::Component,
    event::{AppEvent, Effect, StreamId, TabId},
    theme::Theme,
    widgets::human_bytes,
};

/// 曲线保留点数。
const HISTORY: usize = 120;

pub struct TrafficTab {
    theme: Theme,
    up: VecDeque<u64>,
    down: VecDeque<u64>,
    last: Traffic,
    memory: Memory,
}

impl TrafficTab {
    pub fn new(theme: Theme) -> Self {
        TrafficTab {
            theme,
            up: VecDeque::from(vec![0; HISTORY]),
            down: VecDeque::from(vec![0; HISTORY]),
            last: Traffic::default(),
            memory: Memory::default(),
        }
    }

    fn push(&mut self, t: Traffic) {
        self.last = t;
        if self.up.len() >= HISTORY {
            self.up.pop_front();
            self.down.pop_front();
        }
        self.up.push_back(t.up);
        self.down.push_back(t.down);
    }
}

impl Component for TrafficTab {
    fn id(&self) -> TabId {
        TabId::Traffic
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![
            Effect::StartStream(StreamId::Traffic),
            Effect::StartStream(StreamId::Memory),
        ]
    }

    fn handle_key(&mut self, _key: KeyEvent) -> (crate::component::Handled, Vec<Effect>) {
        (crate::component::Handled::No, vec![])
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::WsTraffic(t) => self.push(*t),
            AppEvent::WsMemory(m) => self.memory = *m,
            _ => {}
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 概要
                Constraint::Min(3),    // 上行图
                Constraint::Min(3),    // 下行图
            ])
            .split(area);

        // 概要行。
        let summary = Line::from(vec![
            Span::styled("  ↑ ", self.theme.tab_inactive()),
            Span::styled(
                format!("{}/s", human_bytes(self.last.up)),
                self.theme.ok_style(),
            ),
            Span::raw("    "),
            Span::styled("↓ ", self.theme.tab_inactive()),
            Span::styled(
                format!("{}/s", human_bytes(self.last.down)),
                self.theme.accent_style(),
            ),
            Span::raw("    "),
            Span::styled("内存 ", self.theme.tab_inactive()),
            Span::styled(human_bytes(self.memory.inuse), self.theme.fg_style()),
        ]);
        buf.set_line(rows[0].x, rows[0].y, &summary, rows[0].width);

        let up: Vec<u64> = self.up.iter().copied().collect();
        let down: Vec<u64> = self.down.iter().copied().collect();

        let up_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" 上行 ↑ ", self.theme.ok_style()));
        Sparkline::default()
            .block(up_block)
            .data(&up)
            .style(self.theme.ok_style())
            .render(rows[1], buf);

        let down_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(" 下行 ↓ ", self.theme.accent_style()));
        Sparkline::default()
            .block(down_block)
            .data(&down)
            .style(self.theme.accent_style())
            .render(rows[2], buf);
    }

    fn footer_hints(&self) -> &str {
        "实时流量（每秒）"
    }
}
