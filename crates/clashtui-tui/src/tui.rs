//! 终端生命周期与事件流。
//!
//! - [`init`] / [`restore`]：进入/退出 raw mode + alternate screen。
//! - [`install_panic_hook`]：panic 时先恢复终端再展示报错（否则终端会乱）。
//! - [`TuiEventStream`]：把 crossterm 异步输入事件与 [`FrameRequester`] 的重绘
//!   请求合流，供主 `select!` 循环消费。
//! - [`FrameRequester`]：任意后台任务/组件可借此请求一次重绘，无需拥有终端。

use std::io::{self, Stdout};

use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, EventStream, KeyEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::event::AppEvent;

/// 终端类型别名。
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// 进入 raw mode + alternate screen，返回可用的终端。
pub fn init() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

/// 退出 alternate screen + raw mode。幂等，失败也尽量继续。
pub fn restore() -> io::Result<()> {
    // 即使其中一步失败也尽量把能恢复的恢复掉。
    let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
    disable_raw_mode()?;
    Ok(())
}

/// 安装 panic hook：先恢复终端，再调用原 hook 打印 backtrace。
///
/// 配合 color-eyre 使用：调用方应先 `color_eyre::install()`，再调本函数包裹其 hook。
pub fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        original(info);
    }));
}

/// 可克隆的重绘请求句柄。
#[derive(Debug, Clone)]
pub struct FrameRequester {
    tx: mpsc::UnboundedSender<()>,
}

impl FrameRequester {
    /// 请求一次重绘。终端已关闭时静默失败。
    pub fn request(&self) {
        let _ = self.tx.send(());
    }
}

/// 合并 crossterm 输入与重绘请求的异步事件源。
pub struct TuiEventStream {
    input: EventStream,
    redraw_rx: mpsc::UnboundedReceiver<()>,
    requester: FrameRequester,
}

impl TuiEventStream {
    /// 创建事件流，并返回一个可克隆的 [`FrameRequester`]。
    pub fn new() -> (Self, FrameRequester) {
        let (tx, rx) = mpsc::unbounded_channel();
        let requester = FrameRequester { tx };
        let stream = TuiEventStream {
            input: EventStream::new(),
            redraw_rx: rx,
            requester: requester.clone(),
        };
        (stream, requester)
    }

    /// 取得内部的重绘句柄克隆。
    pub fn requester(&self) -> FrameRequester {
        self.requester.clone()
    }

    /// 取下一个事件。把 crossterm 原始事件翻译为 [`AppEvent`]。
    ///
    /// 返回 `None` 表示输入流结束（终端关闭）。
    pub async fn next(&mut self) -> Option<AppEvent> {
        loop {
            tokio::select! {
                // 重绘请求
                Some(()) = self.redraw_rx.recv() => {
                    return Some(AppEvent::Draw);
                }
                // 终端输入
                maybe = self.input.next() => {
                    match maybe {
                        Some(Ok(ev)) => {
                            if let Some(app_ev) = translate(ev) {
                                return Some(app_ev);
                            }
                            // 不关心的事件（如 KeyEventKind::Release）跳过，继续取。
                            continue;
                        }
                        Some(Err(_)) => continue, // 单次读取错误，跳过
                        None => return None,       // 输入流结束
                    }
                }
            }
        }
    }
}

/// 把 crossterm 事件翻译为 [`AppEvent`]；不关心的返回 None。
fn translate(ev: CtEvent) -> Option<AppEvent> {
    match ev {
        CtEvent::Key(k) => {
            // 只处理按下（Press）/重复（Repeat），忽略松开，避免重复触发。
            if matches!(k.kind, KeyEventKind::Release) {
                None
            } else {
                Some(AppEvent::Key(k))
            }
        }
        CtEvent::Mouse(m) => Some(AppEvent::Mouse(m)),
        CtEvent::Paste(s) => Some(AppEvent::Paste(s)),
        CtEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
        _ => None, // FocusGained/Lost 暂不处理
    }
}
