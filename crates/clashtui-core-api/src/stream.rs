//! [`StreamHub`]：4 路 mihomo WebSocket 流的弹性工厂。
//!
//! 每个流一个 tokio task，扇入单一 mpsc 通道（由调用方在主循环消费）。
//! 设计要点（核验过的协议陷阱与评审 grafted ideas）：
//! - **源头节流**：traffic 合并到 ~10fps（保留最新）、logs 直送（上层批处理）、
//!   connections 按服务端 `interval` 推送。
//! - **僵尸看门狗**：超过 [`STALE_TIMEOUT`] 无帧则主动重连（`/restart` 与
//!   `PUT /configs` 会静默断开本客户端的 WS）。
//! - **指数退避**重连：200ms..30s。
//! - WS url 形如 `ws://host:port/<path>?token=<secret>`。

use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::models::{ConnectionsSnapshot, LogEntry, Memory, Traffic};

/// 无帧超过此时长则判定流僵死并重连。
const STALE_TIMEOUT: Duration = Duration::from_secs(15);
/// 退避下限。
const BACKOFF_MIN: Duration = Duration::from_millis(200);
/// 退避上限。
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// traffic 合并窗口（~10fps）。
const TRAFFIC_COALESCE: Duration = Duration::from_millis(100);

/// 流类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Traffic,
    Logs,
    Connections,
    Memory,
}

impl StreamKind {
    fn path(self) -> &'static str {
        match self {
            StreamKind::Traffic => "/traffic",
            StreamKind::Logs => "/logs",
            StreamKind::Connections => "/connections",
            StreamKind::Memory => "/memory",
        }
    }
}

/// 从 WS 流解析出的消息，发往主循环。
#[derive(Debug, Clone)]
pub enum StreamMsg {
    Traffic(Traffic),
    Log(LogEntry),
    Connections(Box<ConnectionsSnapshot>),
    Memory(Memory),
    /// 某流已（重新）连接。
    Connected(StreamKind),
    /// 某流断开（将自动重连）。
    Disconnected(StreamKind),
}

/// 管理一组 WS 流的句柄。drop 时自动 abort 所有 task。
pub struct StreamHub {
    host_port: String,
    secret: String,
    tx: mpsc::UnboundedSender<StreamMsg>,
    tasks: Vec<(StreamKind, JoinHandle<()>)>,
}

impl StreamHub {
    /// 新建。`host_port` 如 `127.0.0.1:9090`；`tx` 为扇入通道发送端。
    pub fn new(
        host_port: impl Into<String>,
        secret: impl Into<String>,
        tx: mpsc::UnboundedSender<StreamMsg>,
    ) -> Self {
        StreamHub {
            host_port: host_port.into(),
            secret: secret.into(),
            tx,
            tasks: Vec::new(),
        }
    }

    /// 启动某个流（若已启动则忽略——调用方自行去重）。
    pub fn start(&mut self, kind: StreamKind) {
        let url = self.ws_url(kind);
        let tx = self.tx.clone();
        let handle = tokio::spawn(run_stream(kind, url, tx));
        self.tasks.push((kind, handle));
    }

    /// 停止指定类型的流（abort 其 task）。
    pub fn stop(&mut self, kind: StreamKind) {
        self.tasks.retain(|(k, h)| {
            if *k == kind {
                h.abort();
                false
            } else {
                true
            }
        });
    }

    /// 停止所有流（abort task）。
    pub fn stop_all(&mut self) {
        for (_, t) in self.tasks.drain(..) {
            t.abort();
        }
    }

    fn ws_url(&self, kind: StreamKind) -> String {
        let mut url = format!("ws://{}{}", self.host_port, kind.path());
        // 附带额外查询参数。
        let mut qs: Vec<String> = Vec::new();
        if !self.secret.is_empty() {
            qs.push(format!("token={}", urlencoding::encode(&self.secret)));
        }
        if kind == StreamKind::Connections {
            qs.push("interval=1000".to_string());
        }
        if !qs.is_empty() {
            url.push('?');
            url.push_str(&qs.join("&"));
        }
        url
    }
}

impl Drop for StreamHub {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// 单个流的运行循环：连接 → 读帧 → 节流 → 转发；断开则退避重连。
async fn run_stream(kind: StreamKind, url: String, tx: mpsc::UnboundedSender<StreamMsg>) {
    let mut backoff = BACKOFF_MIN;
    loop {
        match connect_and_pump(kind, &url, &tx).await {
            ConnOutcome::ChannelClosed => return, // 消费端已走，结束 task
            ConnOutcome::Reconnect => {
                let _ = tx.send(StreamMsg::Disconnected(kind));
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

enum ConnOutcome {
    /// 扇入通道关闭，应结束 task。
    ChannelClosed,
    /// 连接断开/超时，应退避重连。
    Reconnect,
}

async fn connect_and_pump(
    kind: StreamKind,
    url: &str,
    tx: &mpsc::UnboundedSender<StreamMsg>,
) -> ConnOutcome {
    let ws = match tokio_tungstenite::connect_async(url).await {
        Ok((ws, _resp)) => ws,
        Err(_) => return ConnOutcome::Reconnect,
    };
    if tx.send(StreamMsg::Connected(kind)).is_err() {
        return ConnOutcome::ChannelClosed;
    }

    let (_write, mut read) = ws.split();

    // traffic 合并状态。
    let mut pending_traffic: Option<Traffic> = None;
    let mut coalesce = tokio::time::interval(TRAFFIC_COALESCE);
    coalesce.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // 读下一帧（带僵尸超时）。
            frame = tokio::time::timeout(STALE_TIMEOUT, read.next()) => {
                match frame {
                    Err(_) => return ConnOutcome::Reconnect,      // 看门狗超时
                    Ok(None) => return ConnOutcome::Reconnect,    // 流结束
                    Ok(Some(Err(_))) => return ConnOutcome::Reconnect, // 读错误
                    Ok(Some(Ok(msg))) => {
                        match parse_frame(kind, msg) {
                            Some(ParsedFrame::Traffic(t)) => {
                                // 合并：仅保留最新，由 interval 节流发送。
                                pending_traffic = Some(t);
                            }
                            Some(ParsedFrame::Msg(m)) => {
                                if tx.send(m).is_err() {
                                    return ConnOutcome::ChannelClosed;
                                }
                            }
                            None => {} // ping/pong/无法解析，跳过
                        }
                    }
                }
            }
            // traffic 节流发送窗口。
            _ = coalesce.tick() => {
                if let Some(t) = pending_traffic.take() {
                    if tx.send(StreamMsg::Traffic(t)).is_err() {
                        return ConnOutcome::ChannelClosed;
                    }
                }
            }
        }
    }
}

enum ParsedFrame {
    Traffic(Traffic),
    Msg(StreamMsg),
}

/// 把一帧 WS 文本解析为对应类型。
fn parse_frame(kind: StreamKind, msg: Message) -> Option<ParsedFrame> {
    let text = match msg {
        Message::Text(t) => t.to_string(),
        Message::Binary(b) => String::from_utf8(b.to_vec()).ok()?,
        _ => return None, // ping/pong/close
    };
    match kind {
        StreamKind::Traffic => {
            let t: Traffic = serde_json::from_str(&text).ok()?;
            Some(ParsedFrame::Traffic(t))
        }
        StreamKind::Logs => {
            let l: LogEntry = serde_json::from_str(&text).ok()?;
            Some(ParsedFrame::Msg(StreamMsg::Log(l)))
        }
        StreamKind::Connections => {
            let c: ConnectionsSnapshot = serde_json::from_str(&text).ok()?;
            Some(ParsedFrame::Msg(StreamMsg::Connections(Box::new(c))))
        }
        StreamKind::Memory => {
            let m: Memory = serde_json::from_str(&text).ok()?;
            Some(ParsedFrame::Msg(StreamMsg::Memory(m)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_includes_token_and_interval() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let hub = StreamHub::new("127.0.0.1:9090", "sekret", tx);
        let conn = hub.ws_url(StreamKind::Connections);
        assert!(conn.starts_with("ws://127.0.0.1:9090/connections?"));
        assert!(conn.contains("token=sekret"));
        assert!(conn.contains("interval=1000"));

        let logs = hub.ws_url(StreamKind::Logs);
        assert_eq!(logs, "ws://127.0.0.1:9090/logs?token=sekret");
    }

    #[test]
    fn ws_url_without_secret() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let hub = StreamHub::new("127.0.0.1:9090", "", tx);
        assert_eq!(
            hub.ws_url(StreamKind::Traffic),
            "ws://127.0.0.1:9090/traffic"
        );
    }

    #[test]
    fn parse_traffic_frame() {
        let f = parse_frame(
            StreamKind::Traffic,
            Message::Text(r#"{"up":10,"down":20}"#.into()),
        );
        match f {
            Some(ParsedFrame::Traffic(t)) => {
                assert_eq!(t.up, 10);
                assert_eq!(t.down, 20);
            }
            _ => panic!("expected traffic"),
        }
    }

    #[test]
    fn parse_log_frame() {
        let f = parse_frame(
            StreamKind::Logs,
            Message::Text(r#"{"type":"info","payload":"hello"}"#.into()),
        );
        match f {
            Some(ParsedFrame::Msg(StreamMsg::Log(l))) => {
                assert_eq!(l.level, "info");
                assert_eq!(l.payload, "hello");
            }
            _ => panic!("expected log"),
        }
    }
}
