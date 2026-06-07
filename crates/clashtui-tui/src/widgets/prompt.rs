//! 极简单行文本输入（替代 tui-textarea，待其适配 ratatui 0.30）。

use crossterm::event::{KeyCode, KeyEvent};

/// 单行输入框状态：内容 + 光标位置（按字符计）。
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    chars: Vec<char>,
    cursor: usize,
}

impl Prompt {
    pub fn new() -> Self {
        Prompt::default()
    }

    /// 当前文本。
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    /// 设定初始内容，光标置末尾。
    pub fn set_text(&mut self, s: &str) {
        self.chars = s.chars().collect();
        self.cursor = self.chars.len();
    }

    /// 在当前光标位置插入一段文本。
    pub fn insert_str(&mut self, s: &str) {
        let inserted: Vec<char> = s.chars().collect();
        let len = inserted.len();
        self.chars.splice(self.cursor..self.cursor, inserted);
        self.cursor += len;
    }

    /// 光标字符索引（供渲染定位）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 处理一个按键，返回是否消费。
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
                true
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.chars.remove(self.cursor - 1);
                    self.cursor -= 1;
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
                true
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                if self.cursor < self.chars.len() {
                    self.cursor += 1;
                }
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.chars.len();
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::empty())
    }

    #[test]
    fn typing_and_editing() {
        let mut p = Prompt::new();
        for c in "hello".chars() {
            p.handle_key(k(KeyCode::Char(c)));
        }
        assert_eq!(p.text(), "hello");
        p.handle_key(k(KeyCode::Backspace));
        assert_eq!(p.text(), "hell");
        p.handle_key(k(KeyCode::Left));
        p.handle_key(k(KeyCode::Char('X')));
        assert_eq!(p.text(), "helXl");
    }

    #[test]
    fn home_end_delete() {
        let mut p = Prompt::new();
        p.set_text("abc");
        p.handle_key(k(KeyCode::Home));
        assert_eq!(p.cursor(), 0);
        p.handle_key(k(KeyCode::Delete));
        assert_eq!(p.text(), "bc");
        p.handle_key(k(KeyCode::End));
        assert_eq!(p.cursor(), 2);
    }
}
