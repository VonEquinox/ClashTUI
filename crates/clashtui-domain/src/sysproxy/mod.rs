//! 系统代理：两维度。
//!
//! - **service 级**：改 OS 设置（macOS `networksetup`，Linux `gsettings`）。
//! - **env 级**：生成可 `source` 的 shell 片段（`http_proxy` 等），因无法直接改父 shell。
//!
//! 退出时须恢复 service 级设置（见 [`SysProxyGuard`]），避免崩溃后网络中断。

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::error::DomainResult;

/// 系统代理设置参数。
#[derive(Debug, Clone)]
pub struct ProxySettings {
    pub host: String,
    pub http_port: u16,
    pub socks_port: u16,
    pub bypass: Vec<String>,
}

impl ProxySettings {
    pub fn new(http_port: u16, socks_port: u16, bypass: Vec<String>) -> Self {
        ProxySettings {
            host: "127.0.0.1".into(),
            http_port,
            socks_port,
            bypass,
        }
    }
}

/// service 级系统代理操作接口。
pub trait ServiceProxy {
    /// 开启系统代理。
    fn enable(&self, s: &ProxySettings) -> DomainResult<()>;
    /// 关闭系统代理。
    fn disable(&self) -> DomainResult<()>;
    /// 查询是否已开启（尽力而为）。
    fn is_enabled(&self) -> DomainResult<bool>;
}

/// 取当前平台的 service 级实现。
pub fn service_proxy(manual_service: Option<String>) -> Box<dyn ServiceProxy> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosProxy::new(manual_service))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = manual_service;
        Box::new(linux::LinuxProxy::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = manual_service;
        Box::new(NoopProxy)
    }
}

/// 不支持平台的占位实现。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct NoopProxy;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl ServiceProxy for NoopProxy {
    fn enable(&self, _s: &ProxySettings) -> DomainResult<()> {
        Err(crate::error::DomainError::SysProxy(
            "当前平台不支持系统代理".into(),
        ))
    }
    fn disable(&self) -> DomainResult<()> {
        Ok(())
    }
    fn is_enabled(&self) -> DomainResult<bool> {
        Ok(false)
    }
}

/// 生成 env 级可 source 的 shell 片段。
pub fn env_snippet(s: &ProxySettings, enable: bool) -> String {
    if enable {
        let http = format!("http://{}:{}", s.host, s.http_port);
        let socks = format!("socks5://{}:{}", s.host, s.socks_port);
        let no_proxy = s.bypass.join(",");
        format!(
            "# ClashTUI 系统代理（env 级）\n\
             export http_proxy=\"{http}\"\n\
             export https_proxy=\"{http}\"\n\
             export all_proxy=\"{socks}\"\n\
             export HTTP_PROXY=\"{http}\"\n\
             export HTTPS_PROXY=\"{http}\"\n\
             export ALL_PROXY=\"{socks}\"\n\
             export no_proxy=\"{no_proxy}\"\n\
             export NO_PROXY=\"{no_proxy}\"\n"
        )
    } else {
        "# ClashTUI 取消系统代理（env 级）\n\
         unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY no_proxy NO_PROXY\n"
            .to_string()
    }
}

/// 退出时恢复 service 级代理的守卫。drop 时关闭代理（若曾开启）。
pub struct SysProxyGuard {
    proxy: Box<dyn ServiceProxy>,
    was_enabled: bool,
}

impl SysProxyGuard {
    pub fn new(proxy: Box<dyn ServiceProxy>, was_enabled: bool) -> Self {
        SysProxyGuard { proxy, was_enabled }
    }
}

impl Drop for SysProxyGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            let _ = self.proxy.disable();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_snippet_enable_contains_exports() {
        let s = ProxySettings::new(7890, 7891, vec!["localhost".into(), "127.0.0.1".into()]);
        let snip = env_snippet(&s, true);
        assert!(snip.contains("export http_proxy=\"http://127.0.0.1:7890\""));
        assert!(snip.contains("export all_proxy=\"socks5://127.0.0.1:7891\""));
        assert!(snip.contains("no_proxy=\"localhost,127.0.0.1\""));
    }

    #[test]
    fn env_snippet_disable_unsets() {
        let s = ProxySettings::new(7890, 7891, vec![]);
        let snip = env_snippet(&s, false);
        assert!(snip.contains("unset http_proxy"));
    }
}
