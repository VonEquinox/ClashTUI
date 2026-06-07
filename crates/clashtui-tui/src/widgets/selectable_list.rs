//! 一个带选中与滚动的简单列表状态（无状态渲染由调用方完成）。

/// 列表选中与滚动偏移管理。与具体渲染解耦，便于单测。
#[derive(Debug, Clone, Default)]
pub struct SelectableList {
    pub selected: usize,
    pub offset: usize,
    len: usize,
}

impl SelectableList {
    pub fn new(len: usize) -> Self {
        SelectableList {
            selected: 0,
            offset: 0,
            len,
        }
    }

    /// 更新元素数量，保持选中合法。
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
        if self.offset >= len {
            self.offset = len.saturating_sub(1);
        }
    }

    /// 更新长度，并按元素谓词恢复选中项。找不到时只夹紧原 index。
    pub fn set_len_and_select_by<T, F>(&mut self, len: usize, items: &[T], mut pred: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.set_len(len);
        if let Some(index) = items.iter().position(|item| pred(item)) {
            self.selected = index;
        }
    }

    /// 更新长度，并按字符串值恢复选中项。找不到时只夹紧原 index。
    pub fn set_len_and_select_value(&mut self, len: usize, items: &[String], value: &str) {
        self.set_len_and_select_by(len, items, |item| item == value);
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 上移（环绕）。
    pub fn up(&mut self) {
        if self.len == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            self.len - 1
        } else {
            self.selected - 1
        };
    }

    /// 下移（环绕）。
    pub fn down(&mut self) {
        if self.len == 0 {
            return;
        }
        self.selected = (self.selected + 1) % self.len;
    }

    /// 根据可视高度调整滚动偏移，保证选中可见。返回偏移。
    pub fn adjust_offset(&mut self, viewport: usize) -> usize {
        if viewport == 0 {
            return 0;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + viewport {
            self.offset = self.selected + 1 - viewport;
        }
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_around() {
        let mut l = SelectableList::new(3);
        l.up();
        assert_eq!(l.selected, 2);
        l.down();
        assert_eq!(l.selected, 0);
        l.down();
        l.down();
        assert_eq!(l.selected, 2);
    }

    #[test]
    fn set_len_clamps() {
        let mut l = SelectableList::new(5);
        l.selected = 4;
        l.offset = 4;
        l.set_len(2);
        assert_eq!(l.selected, 1);
        assert_eq!(l.offset, 1);
    }

    #[test]
    fn set_len_and_select_by_prefers_identity() {
        let mut l = SelectableList::new(3);
        l.selected = 0;
        let items = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        l.set_len_and_select_value(items.len(), &items, "C");
        assert_eq!(l.selected, 2);
    }

    #[test]
    fn offset_follows_selection() {
        let mut l = SelectableList::new(20);
        l.selected = 15;
        let off = l.adjust_offset(10);
        assert_eq!(off, 6); // 15 可见：offset 6..16
        l.selected = 2;
        let off = l.adjust_offset(10);
        assert_eq!(off, 2);
    }

    #[test]
    fn empty_is_safe() {
        let mut l = SelectableList::new(0);
        l.up();
        l.down();
        assert!(l.is_empty());
        assert_eq!(l.adjust_offset(5), 0);
    }
}
