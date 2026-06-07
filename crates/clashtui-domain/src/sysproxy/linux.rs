//! Linux 系统代理：GNOME `gsettings`（主），其它桌面回退 env-only。

use std::process::Command;

use crate::error::{DomainError, DomainResult};
use crate::sysproxy::{ProxySettings, ServiceProxy};

/// Linux 系统代理实现（GNOME gsettings）。
pub struct LinuxProxy;

impl LinuxProxy {
    pub fn new() -> Self {
        LinuxProxy
    }
}

impl Default for LinuxProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceProxy for LinuxProxy {
    fn enable(&self, s: &ProxySettings) -> DomainResult<()> {
        let http_port = s.http_port.to_string();
        let socks_port = s.socks_port.to_string();
        gset(&["set", "org.gnome.system.proxy", "mode", "manual"])?;
        for proto in ["http", "https"] {
            gset(&[
                "set",
                &format!("org.gnome.system.proxy.{proto}"),
                "host",
                &s.host,
            ])?;
            gset(&[
                "set",
                &format!("org.gnome.system.proxy.{proto}"),
                "port",
                &http_port,
            ])?;
        }
        gset(&["set", "org.gnome.system.proxy.socks", "host", &s.host])?;
        gset(&["set", "org.gnome.system.proxy.socks", "port", &socks_port])?;
        // ignore-hosts。
        let list = format!(
            "[{}]",
            s.bypass
                .iter()
                .map(|h| format!("'{h}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        gset(&["set", "org.gnome.system.proxy", "ignore-hosts", &list])?;
        Ok(())
    }

    fn disable(&self) -> DomainResult<()> {
        gset(&["set", "org.gnome.system.proxy", "mode", "none"])
    }

    fn is_enabled(&self) -> DomainResult<bool> {
        let out = Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy", "mode"])
            .output()
            .map_err(|e| DomainError::SysProxy(format!("gsettings 失败: {e}")))?;
        let s = String::from_utf8_lossy(&out.stdout);
        Ok(s.contains("manual"))
    }
}

fn gset(args: &[&str]) -> DomainResult<()> {
    let status = Command::new("gsettings")
        .args(args)
        .status()
        .map_err(|e| DomainError::SysProxy(format!("gsettings 执行失败（需 GNOME）: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(DomainError::SysProxy(format!(
            "gsettings {args:?} 返回 {status}"
        )))
    }
}
