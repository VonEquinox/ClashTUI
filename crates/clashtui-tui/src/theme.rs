//! 主题：颜色调色板。v1 只给一套合理默认值。
//!
//! 关键：主题经 `AppContext` 按引用传递，**不是全局** OnceLock/RwLock——
//! 这样后续加 TOML 覆盖时无需改动调用方，也避免跨 await 持锁的风险。

use ratatui::style::{Color, Modifier, Style};

/// 全应用调色板。后续可由 `theme.toml` 反序列化覆盖。
#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub bg_selected: Color,
    pub border: Color,
    pub border_focused: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            accent: Color::Cyan,
            fg: Color::Reset,
            fg_dim: Color::DarkGray,
            bg_selected: Color::Indexed(236),
            border: Color::DarkGray,
            border_focused: Color::Cyan,
            ok: Color::Green,
            warn: Color::Yellow,
            err: Color::Red,
        }
    }
}

impl Theme {
    /// 激活 tab 标题样式。
    pub fn tab_active(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// 非激活 tab 标题样式。
    pub fn tab_inactive(&self) -> Style {
        Style::default().fg(self.fg_dim)
    }

    /// 列表选中行样式。
    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.bg_selected)
            .add_modifier(Modifier::BOLD)
    }

    /// 聚焦 / 非聚焦边框样式。
    pub fn border_style(&self, focused: bool) -> Style {
        Style::default().fg(if focused {
            self.border_focused
        } else {
            self.border
        })
    }

    /// 普通前景文本样式。
    pub fn fg_style(&self) -> Style {
        Style::default().fg(self.fg)
    }

    /// 成功 / 正常状态样式。
    pub fn ok_style(&self) -> Style {
        Style::default().fg(self.ok)
    }

    /// 警告样式。
    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.warn)
    }

    /// 错误样式。
    pub fn err_style(&self) -> Style {
        Style::default().fg(self.err)
    }

    /// 强调样式。
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    /// 根据延迟毫秒上色：绿(<200) 黄(<500) 红(其余/超时)。
    pub fn delay_style(&self, ms: Option<u16>) -> Style {
        match ms {
            Some(d) if d < 200 => self.ok_style(),
            Some(d) if d < 500 => self.warn_style(),
            _ => self.err_style(),
        }
    }
}
