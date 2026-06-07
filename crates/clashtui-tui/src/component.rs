//! `Component` trait：每个 tab 是一个自洽组件。
//!
//! 加一个功能 = 实现 `Component` + 在 app 注册，只动一个文件（取自评审方案 P3）。
//!
//! 关键纪律：
//! - `draw` 是 `&self`、**render-no-mutate**（不改状态、不 await、不持锁跨 await）。
//! - `handle_key` 返回 `(Handled, Vec<Effect>)`——副作用是声明式 [`Effect`]（取自 P1），
//!   绝不用 `Box<dyn FnOnce>` 闭包，从而可在 headless 单测里断言"按键 → Effect"。
//! - `apply_event` 把引擎回灌的 [`AppEvent`] 消化进组件视图状态，同样返回 `Vec<Effect>`
//!   以便链式触发（如数据加载完成后自动起流）。

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::event::{AppEvent, Effect, TabId};

/// 组件是否消费了某个按键。未消费则主循环做全局二次兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

impl Handled {
    pub fn is_handled(self) -> bool {
        matches!(self, Handled::Yes)
    }
}

/// 每个 tab 实现的组件接口。
pub trait Component {
    /// 该组件对应的 tab。
    fn id(&self) -> TabId;

    /// 处理一个按键，返回是否消费 + 要执行的副作用。
    ///
    /// 默认实现：不消费、无副作用。
    fn handle_key(&mut self, _key: KeyEvent) -> (Handled, Vec<Effect>) {
        (Handled::No, Vec::new())
    }

    /// 处理 bracketed paste 文本，返回是否消费 + 要执行的副作用。
    ///
    /// 默认实现：不消费、无副作用。
    fn handle_paste(&mut self, _text: String) -> (Handled, Vec<Effect>) {
        (Handled::No, Vec::new())
    }

    /// 消化引擎回灌的事件（WS 帧、数据加载结果等），更新视图状态。
    ///
    /// 默认实现：忽略、无后续副作用。
    fn apply_event(&mut self, _event: &AppEvent) -> Vec<Effect> {
        Vec::new()
    }

    /// 获得焦点时调用（如 Connections tab 在此发 StartStream）。
    fn on_focus(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// 失去焦点时调用（如 Connections tab 在此发 StopStream）。
    fn on_blur(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// 动画 tick（仅在需要 spinner 动画时返回 true 以保持 tick 启用）。
    fn tick(&mut self) -> bool {
        false
    }

    /// 组件是否处于"独占输入"模式（如文本输入框打开）。
    /// 为 true 时，主循环把**所有**按键直送本组件，绕过路由器的保留全局键与 tab 切换，
    /// 以免在输入 URL 时 `q`/`?`/数字键被全局拦截。
    fn capturing(&self) -> bool {
        false
    }

    /// 渲染。**只读 self**，不得修改状态。
    fn draw(&self, area: Rect, buf: &mut Buffer, focused: bool);

    /// 底部状态栏的上下文提示（按键说明）。
    fn footer_hints(&self) -> &str {
        ""
    }
}
