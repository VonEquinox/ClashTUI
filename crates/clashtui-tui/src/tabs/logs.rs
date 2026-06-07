//! Logs tab：消费 `/logs` WS 流，有界环 + level 过滤 + 滚动 + 暂停。

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

use crate::{
    component::{Component, Handled},
    event::{AppEvent, Effect, StreamId, TabId},
    theme::Theme,
};

/// 日志环最大容量。
const MAX_LOGS: usize = 5000;

struct LogLine {
    level: String,
    payload: String,
}

pub struct LogsTab {
    theme: Theme,
    logs: VecDeque<LogLine>,
    /// level 过滤（空 = 全部）。
    filter: Option<String>,
    /// 是否暂停接收（暂停时仍入环，但不自动滚到底）。
    paused: bool,
    /// 自底向上的滚动偏移（0 = 跟随最新）。
    scroll: usize,
}

impl LogsTab {
    pub fn new(theme: Theme) -> Self {
        LogsTab {
            theme,
            logs: VecDeque::with_capacity(MAX_LOGS),
            filter: None,
            paused: false,
            scroll: 0,
        }
    }

    fn push(&mut self, level: String, payload: String) {
        if self.logs.len() >= MAX_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(LogLine { level, payload });
    }

    /// 过滤后的行（引用）。
    fn filtered(&self) -> Vec<&LogLine> {
        self.logs
            .iter()
            .filter(|l| match &self.filter {
                Some(f) => l.level.eq_ignore_ascii_case(f),
                None => true,
            })
            .collect()
    }

    fn level_style(&self, level: &str) -> Style {
        match level.to_ascii_lowercase().as_str() {
            "error" | "err" => self.theme.err_style(),
            "warning" | "warn" => self.theme.warn_style(),
            "info" => self.theme.ok_style(),
            "core" => self.theme.accent_style(),
            _ => self.theme.tab_inactive(),
        }
    }

    fn cycle_filter(&mut self) {
        // None → info → warning → error → core → None
        self.filter = match self.filter.as_deref() {
            None => Some("info".into()),
            Some("info") => Some("warning".into()),
            Some("warning") => Some("error".into()),
            Some("error") => Some("core".into()),
            _ => None,
        };
    }
}

impl Component for LogsTab {
    fn id(&self) -> TabId {
        TabId::Logs
    }

    fn on_focus(&mut self) -> Vec<Effect> {
        vec![Effect::StartStream(StreamId::Logs)]
    }

    fn handle_key(&mut self, key: KeyEvent) -> (Handled, Vec<Effect>) {
        match key.code {
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_add(1);
                (Handled::Yes, vec![])
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_sub(1);
                (Handled::Yes, vec![])
            }
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                (
                    Handled::Yes,
                    vec![Effect::Toast(if self.paused {
                        "日志已暂停".into()
                    } else {
                        "日志继续".into()
                    })],
                )
            }
            KeyCode::Char('c') => {
                self.logs.clear();
                self.scroll = 0;
                (Handled::Yes, vec![])
            }
            KeyCode::Char('f') => {
                self.cycle_filter();
                (Handled::Yes, vec![])
            }
            _ => (Handled::No, vec![]),
        }
    }

    fn apply_event(&mut self, event: &AppEvent) -> Vec<Effect> {
        if let AppEvent::WsLog { level, payload } = event {
            self.push(level.clone(), payload.clone());
        }
        Vec::new()
    }

    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let filter_label = self.filter.as_deref().unwrap_or("all");
        let title = format!(
            " Logs [{}]{} ",
            filter_label,
            if self.paused { " ⏸" } else { "" }
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(focused))
            .title(Span::styled(title, self.theme.tab_active()));
        let inner = block.inner(area);
        block.render(area, buf);

        let lines = self.filtered();
        let viewport = inner.height as usize;
        if viewport == 0 || lines.is_empty() {
            return;
        }
        // 自底显示：end 受 scroll 影响。
        let total = lines.len();
        let end = total.saturating_sub(self.scroll);
        let start = end.saturating_sub(viewport);

        for (row, line) in lines[start..end].iter().enumerate() {
            let y = inner.y + row as u16;
            let lvl = format!("{:<5}", short_level(&line.level));
            let spans = Line::from(vec![
                Span::styled(format!("{lvl} "), self.level_style(&line.level)),
                Span::styled(line.payload.clone(), self.theme.fg_style()),
            ]);
            buf.set_line(inner.x + 1, y, &spans, inner.width.saturating_sub(1));
        }
    }

    fn footer_hints(&self) -> &str {
        "↑/↓ 滚动 · f 切换级别过滤 · p 暂停 · c 清空"
    }
}

fn short_level(level: &str) -> &str {
    match level.to_ascii_lowercase().as_str() {
        "warning" => "WARN",
        "error" => "ERR",
        "info" => "INFO",
        "debug" => "DBG",
        "core" => "CORE",
        _ => level,
    }
}
