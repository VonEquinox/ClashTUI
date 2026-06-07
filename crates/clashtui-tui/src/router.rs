//! 纯按键路由阶梯（取自评审方案 P1 的 grafted idea）。
//!
//! 输入路由是一个**纯函数** `route(focus, key) -> Routed`：不碰终端、不做 I/O，
//! 因此整套优先级逻辑可以 headless 单测。优先级阶梯，stop-at-first-match：
//!
//! ```text
//! Popup  →  GlobalChord  →  Help  →  ReservedGlobal  →  ActiveTab  →  GlobalFallback
//! ```
//!
//! 三类普通按键的区分是关键：
//! - **ReservedGlobal**（`?`/`q`/`Ctrl+C`/数字键 1-7）：永远归全局，即使在 tab 内，
//!   避免被组件贪婪吃掉导致无法切 tab / 退出。
//! - **ActiveTab**（方向键、Enter、Esc、动作字符 t/T/u/a/d/p…）：先交给当前组件。
//! - **GlobalFallback**（Tab/Shift-Tab/F5/r）：tab 切换与刷新，组件不参与。
//!
//! `ActiveTab` 表示"该问当前聚焦的 Component"——组件 `handle_key` 若返回未消费，
//! 主循环可再调 [`global_fallback`] 做二次兜底（如方向键在无选择时无意义则忽略）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::event::TabId;

/// 路由判定结果：这个按键应由哪一层处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// 模态弹窗吞掉所有按键（Confirm / Prompt / Help 的 dismiss 等）。
    Popup(PopupKey),
    /// 全局 chord 组合键（如 Ctrl+R 重启核、Ctrl+P 切系统代理）。
    GlobalChord(Chord),
    /// 关闭 Help 覆盖层（Help 打开时任意键都关）。
    DismissHelp,
    /// 全局动作（保留全局键或全局回退键）。
    Global(GlobalAction),
    /// 交给当前聚焦的 tab Component 处理。
    ActiveTab,
    /// 无对应动作，丢弃。
    Ignore,
}

/// 模态层的按键语义（路由器只做粗分类，具体由弹窗组件解释）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupKey {
    Confirm,
    Cancel,
    /// 其它按键透传给弹窗内部（如 Prompt 的文本输入）。
    Passthrough(KeyEvent),
}

/// 全局 chord 组合键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chord {
    RestartCore,
    ToggleSysProxy,
}

/// 全局兜底动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalAction {
    /// 切到指定 tab（Tab/Shift-Tab/数字键）。
    SwitchTab(TabId),
    /// 打开 Help 覆盖层。
    OpenHelp,
    /// 退出应用。
    Quit,
    /// 刷新当前 tab 数据。
    Refresh,
}

/// 路由所需的最小焦点上下文。保持精简，便于单测构造。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focus {
    /// 当前激活的 tab。
    pub tab: TabId,
    /// 是否有模态弹窗打开。
    pub popup_open: bool,
    /// Help 覆盖层是否打开。
    pub help_open: bool,
}

/// 路由阶梯核心。纯函数，无副作用。
pub fn route(focus: Focus, key: KeyEvent) -> Routed {
    // (0) Popup 层：任何弹窗打开时，所有按键先归弹窗。
    if focus.popup_open {
        return Routed::Popup(classify_popup(key));
    }

    // (0.5) GlobalChord 层：带 Ctrl 的已知全局组合键优先于一切普通按键。
    if let Some(chord) = classify_chord(key) {
        return Routed::GlobalChord(chord);
    }

    // (1) Help 层：Help 打开时，任意键都关闭它。
    if focus.help_open {
        return Routed::DismissHelp;
    }

    // (2) ReservedGlobal 层：保留全局键，永远归全局（即使在 tab 内）。
    if let Some(action) = classify_reserved_global(key) {
        return Routed::Global(action);
    }

    // (3) ActiveTab 层：二级导航键与组件动作键先交给当前组件。
    if is_active_tab_key(key) {
        return Routed::ActiveTab;
    }

    // (4) GlobalFallback 层：tab 切换与刷新。
    if let Some(action) = global_fallback(focus, key) {
        return Routed::Global(action);
    }

    Routed::Ignore
}

/// 组件未消费 ActiveTab 键时，主循环可调此做二次全局兜底。
/// 也用于阶梯第 (4) 层。返回 None 表示无全局含义、应忽略。
pub fn global_fallback(focus: Focus, key: KeyEvent) -> Option<GlobalAction> {
    match key.code {
        KeyCode::Tab => Some(GlobalAction::SwitchTab(focus.tab.cycle(true))),
        KeyCode::BackTab => Some(GlobalAction::SwitchTab(focus.tab.cycle(false))),
        KeyCode::F(5) => Some(GlobalAction::Refresh),
        KeyCode::Char('r') if key.modifiers.is_empty() => Some(GlobalAction::Refresh),
        _ => None,
    }
}

/// 弹窗层按键分类。
fn classify_popup(key: KeyEvent) -> PopupKey {
    match key.code {
        KeyCode::Enter => PopupKey::Confirm,
        KeyCode::Esc => PopupKey::Cancel,
        _ => PopupKey::Passthrough(key),
    }
}

/// 全局 chord 识别（带 Ctrl 修饰）。仅识别已知组合，未知的返回 None 继续下沉。
fn classify_chord(key: KeyEvent) -> Option<Chord> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char('r') => Some(Chord::RestartCore),
        KeyCode::Char('p') => Some(Chord::ToggleSysProxy),
        _ => None,
    }
}

/// 保留全局键：永远归全局，不下发给组件。
fn classify_reserved_global(key: KeyEvent) -> Option<GlobalAction> {
    match key.code {
        // 数字直跳 1-7
        KeyCode::Char(c @ '1'..='7') => {
            let idx = (c as u8 - b'1') as usize;
            TabId::from_index(idx).map(GlobalAction::SwitchTab)
        }
        // 退出：裸 q 或 Ctrl+C
        KeyCode::Char('q') if key.modifiers.is_empty() => Some(GlobalAction::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(GlobalAction::Quit)
        }
        // Help
        KeyCode::Char('?') => Some(GlobalAction::OpenHelp),
        _ => None,
    }
}

/// 是否属于"该交给当前 tab 组件"的按键（二级导航 / 组件动作）。
/// 注意：调用前保留全局键已被 [`classify_reserved_global`] 截走。
fn is_active_tab_key(key: KeyEvent) -> bool {
    // 带 Ctrl/Alt 的组合不属于 tab 内普通导航（已知 chord 更早处理）。
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Char(_) // 组件动作键（t/T/u/a/d/p…）与过滤输入
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn focus() -> Focus {
        Focus {
            tab: TabId::Status,
            popup_open: false,
            help_open: false,
        }
    }

    // ---- (0) Popup 层吞键，优先级最高 ----

    #[test]
    fn popup_swallows_all_keys() {
        let f = Focus {
            popup_open: true,
            ..focus()
        };
        assert_eq!(
            route(f, key(KeyCode::Enter)),
            Routed::Popup(PopupKey::Confirm)
        );
        assert_eq!(route(f, key(KeyCode::Esc)), Routed::Popup(PopupKey::Cancel));
        assert!(matches!(
            route(f, key(KeyCode::Char('x'))),
            Routed::Popup(PopupKey::Passthrough(_))
        ));
        // 即便是 chord，弹窗打开时也归弹窗（Popup 优先于 GlobalChord）。
        assert!(matches!(
            route(f, ctrl(KeyCode::Char('r'))),
            Routed::Popup(_)
        ));
    }

    // ---- (0.5) GlobalChord 层优先于 Help / Reserved / ActiveTab ----

    #[test]
    fn chord_beats_help_and_tab() {
        let f = Focus {
            help_open: true,
            ..focus()
        };
        assert_eq!(
            route(f, ctrl(KeyCode::Char('r'))),
            Routed::GlobalChord(Chord::RestartCore)
        );
        assert_eq!(
            route(focus(), ctrl(KeyCode::Char('p'))),
            Routed::GlobalChord(Chord::ToggleSysProxy)
        );
    }

    #[test]
    fn unknown_chord_falls_through_to_quit() {
        // Ctrl+X 不是已知 chord，下沉；非保留键、非 tab 键 → Ignore。
        assert_eq!(route(focus(), ctrl(KeyCode::Char('x'))), Routed::Ignore);
        // Ctrl+C 下沉到保留全局键 → Quit。
        assert_eq!(
            route(focus(), ctrl(KeyCode::Char('c'))),
            Routed::Global(GlobalAction::Quit)
        );
    }

    // ---- (1) Help 层：打开时任意键关闭 ----

    #[test]
    fn help_dismisses_on_any_key() {
        let f = Focus {
            help_open: true,
            ..focus()
        };
        assert_eq!(route(f, key(KeyCode::Up)), Routed::DismissHelp);
        assert_eq!(route(f, key(KeyCode::Char('a'))), Routed::DismissHelp);
    }

    // ---- (2) ReservedGlobal 层：永远全局，优先于 ActiveTab ----

    #[test]
    fn number_keys_jump_directly_even_over_active_tab() {
        assert_eq!(
            route(focus(), key(KeyCode::Char('3'))),
            Routed::Global(GlobalAction::SwitchTab(TabId::Profiles))
        );
        assert_eq!(
            route(focus(), key(KeyCode::Char('7'))),
            Routed::Global(GlobalAction::SwitchTab(TabId::Settings))
        );
    }

    #[test]
    fn reserved_quit_and_help() {
        assert_eq!(
            route(focus(), key(KeyCode::Char('q'))),
            Routed::Global(GlobalAction::Quit)
        );
        assert_eq!(
            route(focus(), key(KeyCode::Char('?'))),
            Routed::Global(GlobalAction::OpenHelp)
        );
    }

    // ---- (3) ActiveTab 层：导航键交给组件 ----

    #[test]
    fn active_tab_owns_arrows_and_action_keys() {
        assert_eq!(route(focus(), key(KeyCode::Up)), Routed::ActiveTab);
        assert_eq!(route(focus(), key(KeyCode::Down)), Routed::ActiveTab);
        assert_eq!(route(focus(), key(KeyCode::Left)), Routed::ActiveTab);
        assert_eq!(route(focus(), key(KeyCode::Right)), Routed::ActiveTab);
        assert_eq!(route(focus(), key(KeyCode::Enter)), Routed::ActiveTab);
        assert_eq!(route(focus(), key(KeyCode::Char('t'))), Routed::ActiveTab);
        // 'a' 是组件动作键（Profiles 的 add），归 ActiveTab。
        assert_eq!(route(focus(), key(KeyCode::Char('a'))), Routed::ActiveTab);
    }

    // ---- (4) GlobalFallback 层：tab 切换 / 刷新 ----

    #[test]
    fn tab_and_backtab_cycle() {
        assert_eq!(
            route(focus(), key(KeyCode::Tab)),
            Routed::Global(GlobalAction::SwitchTab(TabId::Proxies))
        );
        // Status 的上一个应回环到 Settings。
        assert_eq!(
            route(focus(), key(KeyCode::BackTab)),
            Routed::Global(GlobalAction::SwitchTab(TabId::Settings))
        );
    }

    #[test]
    fn refresh_keys() {
        assert_eq!(
            route(focus(), key(KeyCode::F(5))),
            Routed::Global(GlobalAction::Refresh)
        );
        // 'r' 是 ActiveTab 字符键，先归组件；二次兜底才是刷新。
        assert_eq!(route(focus(), key(KeyCode::Char('r'))), Routed::ActiveTab);
        assert_eq!(
            global_fallback(focus(), key(KeyCode::Char('r'))),
            Some(GlobalAction::Refresh)
        );
    }

    // ---- 阶梯顺序整体校验 ----

    #[test]
    fn stop_at_first_match_ordering() {
        // help_open=true 且按普通键 → DismissHelp（Help 先于 Reserved/ActiveTab）。
        let f = Focus {
            help_open: true,
            ..focus()
        };
        assert_eq!(route(f, key(KeyCode::Up)), Routed::DismissHelp);
        // popup 同时打开时 Popup 仍最优先。
        let f2 = Focus {
            help_open: true,
            popup_open: true,
            ..focus()
        };
        assert!(matches!(route(f2, key(KeyCode::Up)), Routed::Popup(_)));
    }

    #[test]
    fn from_index_bounds() {
        assert_eq!(TabId::from_index(0), Some(TabId::Status));
        assert_eq!(TabId::from_index(6), Some(TabId::Settings));
        assert_eq!(TabId::from_index(7), None);
    }
}
