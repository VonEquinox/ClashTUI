//! 内核生命周期管理：spawn-or-attach。
//!
//! 启动时探测 `GET /version`：
//! - 通 → [`CoreStatus::AttachedExternal`]（不拥有进程，restart 走 API）。
//! - 不通 → [`CoreStatus::Stopped`]，可由用户 spawn 子进程托管。
//!
//! 托管的子进程 stdout/stderr 行级喂入日志通道；意外退出标记 [`CoreStatus::Crashed`]。

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clashtui_core_api::MihomoClient;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};

use crate::error::{DomainError, DomainResult};

/// 内核状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreStatus {
    /// 连接到一个外部已运行的内核（非本进程托管）。
    AttachedExternal,
    /// 本进程托管的内核正在运行（pid）。
    ManagedRunning(u32),
    /// 未运行。
    Stopped,
    /// 托管进程异常退出。
    Crashed(String),
}

impl CoreStatus {
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            CoreStatus::AttachedExternal | CoreStatus::ManagedRunning(_)
        )
    }

    /// 是否由本进程托管（可 stop/kill）。
    pub fn is_managed(&self) -> bool {
        matches!(self, CoreStatus::ManagedRunning(_))
    }

    pub fn label(&self) -> String {
        match self {
            CoreStatus::AttachedExternal => "已连接（外部内核）".into(),
            CoreStatus::ManagedRunning(pid) => format!("运行中（托管 pid {pid}）"),
            CoreStatus::Stopped => "已停止".into(),
            CoreStatus::Crashed(e) => format!("已崩溃: {e}"),
        }
    }
}

/// 内核管理器。
pub struct CoreManager {
    client: MihomoClient,
    binary: PathBuf,
    core_dir: PathBuf,
    runtime_config: PathBuf,
    keep_running_on_drop: AtomicBool,
    /// 托管子进程句柄（None = 未托管）。
    child: Arc<Mutex<Option<Child>>>,
    status: Arc<Mutex<CoreStatus>>,
    /// 内核 stdout/stderr 日志行的发送端。
    log_tx: Option<mpsc::UnboundedSender<String>>,
}

impl CoreManager {
    pub fn new(
        client: MihomoClient,
        binary: PathBuf,
        core_dir: PathBuf,
        runtime_config: PathBuf,
        keep_running_on_drop: bool,
    ) -> Self {
        CoreManager {
            client,
            binary,
            core_dir,
            runtime_config,
            keep_running_on_drop: AtomicBool::new(keep_running_on_drop),
            child: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(CoreStatus::Stopped)),
            log_tx: None,
        }
    }

    /// 设置内核日志行接收通道。
    pub fn set_log_sender(&mut self, tx: mpsc::UnboundedSender<String>) {
        self.log_tx = Some(tx);
    }

    /// 当前状态快照。
    pub async fn status(&self) -> CoreStatus {
        self.status.lock().await.clone()
    }

    /// 设置后续启动托管内核时，退出 TUI 是否保留该内核。
    pub fn set_keep_running_on_drop(&self, keep: bool) {
        self.keep_running_on_drop.store(keep, Ordering::Relaxed);
    }

    /// 探测并确定初始状态（spawn-or-attach 的 attach 探测部分）。
    pub async fn probe(&self) -> CoreStatus {
        let st = if self.client.ping().await {
            CoreStatus::AttachedExternal
        } else {
            // 若本进程已托管，则保留托管状态。
            let guard = self.child.lock().await;
            if guard.is_some() {
                let cur = self.status.lock().await.clone();
                if cur.is_managed() {
                    cur
                } else {
                    CoreStatus::Stopped
                }
            } else {
                CoreStatus::Stopped
            }
        };
        *self.status.lock().await = st.clone();
        st
    }

    /// spawn 托管子进程：`mihomo -d <core_dir> -f <runtime_config>`。
    pub async fn start(&self) -> DomainResult<CoreStatus> {
        {
            // 已在运行（外部或托管）则不重复 spawn。
            let cur = self.status.lock().await.clone();
            if cur.is_running() {
                return Ok(cur);
            }
        }
        if !self.binary.exists() {
            return Err(DomainError::Core(format!(
                "找不到内核二进制: {}（请先在 Settings 升级/下载内核）",
                self.binary.display()
            )));
        }

        // macOS：去除 quarantine 属性，避免 Gatekeeper 拦截。
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&self.binary)
                .output()
                .await;
        }

        let keep_running = self.keep_running_on_drop.load(Ordering::Relaxed);
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-d")
            .arg(&self.core_dir)
            .arg("-f")
            .arg(&self.runtime_config);
        if keep_running {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            cmd.kill_on_drop(true);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| DomainError::Core(format!("启动内核失败: {e}")))?;

        let pid = child.id().unwrap_or(0);

        // 转发 stdout/stderr 到日志通道。
        if !keep_running {
            if let Some(tx) = &self.log_tx {
                if let Some(out) = child.stdout.take() {
                    spawn_log_forwarder(out, tx.clone());
                }
                if let Some(err) = child.stderr.take() {
                    spawn_log_forwarder(err, tx.clone());
                }
            }
        }

        *self.child.lock().await = Some(child);
        let st = CoreStatus::ManagedRunning(pid);
        *self.status.lock().await = st.clone();
        Ok(st)
    }

    /// 停止托管子进程（SIGTERM → 等待 → SIGKILL 兜底）。外部内核不可停。
    pub async fn stop(&self) -> DomainResult<CoreStatus> {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            // tokio 的 Child::kill 发送 SIGKILL；先尝试优雅，再兜底。
            #[cfg(unix)]
            {
                if let Some(pid) = child.id() {
                    // SIGTERM
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGTERM,
                    );
                    // 给 1.5s 优雅退出
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_millis(1500), child.wait())
                            .await;
                }
            }
            let _ = child.kill().await; // 兜底
            let _ = child.wait().await;
        }
        *self.status.lock().await = CoreStatus::Stopped;
        Ok(CoreStatus::Stopped)
    }

    /// 重启：托管内核 = stop+start；外部内核 = `POST /restart`。
    /// 两种情况都需调用方随后触发 WS 重连。
    pub async fn restart(&self) -> DomainResult<CoreStatus> {
        let cur = self.status.lock().await.clone();
        match cur {
            CoreStatus::ManagedRunning(_) => {
                self.stop().await?;
                self.start().await
            }
            CoreStatus::AttachedExternal => {
                self.client.restart().await?;
                Ok(CoreStatus::AttachedExternal)
            }
            _ => self.start().await,
        }
    }

    /// 通过 `PUT /configs` 重载运行时配置（保进程、断连接）。
    pub async fn reload(&self) -> DomainResult<()> {
        let path = self.runtime_config.to_string_lossy().to_string();
        self.client.reload_config(&path, true).await?;
        Ok(())
    }

    /// 检查托管子进程是否仍存活；若已退出则更新为 Crashed/Stopped。
    pub async fn check_alive(&self) -> CoreStatus {
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // 已退出
                    guard.take();
                    let st = if status.success() {
                        CoreStatus::Stopped
                    } else {
                        CoreStatus::Crashed(format!("退出码 {status}"))
                    };
                    *self.status.lock().await = st.clone();
                    return st;
                }
                Ok(None) => {} // 仍在运行
                Err(e) => {
                    let st = CoreStatus::Crashed(e.to_string());
                    *self.status.lock().await = st.clone();
                    return st;
                }
            }
        }
        self.status.lock().await.clone()
    }
}

/// 把一个异步可读流按行转发到日志通道。
fn spawn_log_forwarder<R>(reader: R, tx: mpsc::UnboundedSender<String>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels_and_predicates() {
        assert!(CoreStatus::AttachedExternal.is_running());
        assert!(!CoreStatus::AttachedExternal.is_managed());
        assert!(CoreStatus::ManagedRunning(42).is_managed());
        assert!(CoreStatus::ManagedRunning(42).is_running());
        assert!(!CoreStatus::Stopped.is_running());
        assert!(CoreStatus::ManagedRunning(7).label().contains("7"));
    }

    #[tokio::test]
    async fn start_without_binary_errors() {
        let client = MihomoClient::new("127.0.0.1:59999", "").unwrap();
        let mgr = CoreManager::new(
            client,
            PathBuf::from("/nonexistent/mihomo"),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/config.yaml"),
            false,
        );
        let r = mgr.start().await;
        assert!(r.is_err());
    }
}
