//! 应用配置 `config.toml`：原子持久化。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::util::atomic_write;

/// 系统代理配置（两维度）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemProxyConfig {
    /// 是否已开启 OS 服务级代理。
    #[serde(default)]
    pub service_enabled: bool,
    /// 是否已写出 env 级 source 片段。
    #[serde(default)]
    pub env_enabled: bool,
    /// HTTP/HTTPS 代理端口。
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    /// SOCKS 代理端口。
    #[serde(default = "default_socks_port")]
    pub socks_port: u16,
    /// Mixed 代理端口（HTTP + SOCKS 复用入口）。
    #[serde(default = "default_mixed_port")]
    pub mixed_port: u16,
    /// 绕过列表。
    #[serde(default = "default_bypass")]
    pub bypass: Vec<String>,
}

fn default_http_port() -> u16 {
    7890
}
fn default_socks_port() -> u16 {
    7891
}
fn default_mixed_port() -> u16 {
    7892
}
fn default_bypass() -> Vec<String> {
    vec![
        "127.0.0.1".into(),
        "localhost".into(),
        "*.local".into(),
        "10.0.0.0/8".into(),
        "172.16.0.0/12".into(),
        "192.168.0.0/16".into(),
    ]
}

impl Default for SystemProxyConfig {
    fn default() -> Self {
        SystemProxyConfig {
            service_enabled: false,
            env_enabled: false,
            http_port: default_http_port(),
            socks_port: default_socks_port(),
            mixed_port: default_mixed_port(),
            bypass: default_bypass(),
        }
    }
}

/// 自动更新配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoUpdateConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 间隔小时数（下限 24）。
    #[serde(default = "default_interval")]
    pub interval_hours: u32,
}

fn default_interval() -> u32 {
    24
}

impl Default for AutoUpdateConfig {
    fn default() -> Self {
        AutoUpdateConfig {
            enabled: false,
            interval_hours: 24,
        }
    }
}

/// 应用主配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// external-controller 地址，如 `127.0.0.1:9090`。
    #[serde(default = "default_controller")]
    pub external_controller: String,
    /// API secret（可能为空）。
    #[serde(default)]
    pub secret: String,
    /// mihomo 二进制路径（空 = 用默认 bin 目录）。
    #[serde(default)]
    pub mihomo_binary: String,
    /// 退出 TUI 后是否保留由 ClashTUI 启动的 mihomo 内核。
    #[serde(default)]
    pub keep_core_running: bool,
    /// 测速 URL。
    #[serde(default = "default_test_url")]
    pub test_url: String,
    /// 测速超时（毫秒）。
    #[serde(default = "default_test_timeout")]
    pub test_timeout_ms: u32,
    /// WS 日志级别（info/warning/error/debug/silent）。
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// 手动指定的 macOS 网络服务名（空 = 自动探测）。
    #[serde(default)]
    pub manual_network_service: String,
    /// 系统代理配置。
    #[serde(default)]
    pub system_proxy: SystemProxyConfig,
    /// 自动更新配置。
    #[serde(default)]
    pub auto_update: AutoUpdateConfig,
}

fn default_controller() -> String {
    "127.0.0.1:9090".into()
}
fn default_test_url() -> String {
    "http://www.gstatic.com/generate_204".into()
}
fn default_test_timeout() -> u32 {
    5000
}
fn default_log_level() -> String {
    "info".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            external_controller: default_controller(),
            secret: String::new(),
            mihomo_binary: String::new(),
            keep_core_running: false,
            test_url: default_test_url(),
            test_timeout_ms: default_test_timeout(),
            log_level: default_log_level(),
            manual_network_service: String::new(),
            system_proxy: SystemProxyConfig::default(),
            auto_update: AutoUpdateConfig::default(),
        }
    }
}

impl AppConfig {
    /// 从文件读取；不存在则返回默认值（不写盘）。
    pub fn load(path: &Path) -> DomainResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| DomainError::Config(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
            Err(e) => Err(DomainError::Io(e)),
        }
    }

    /// 原子写盘（temp + rename）。
    pub fn save(&self, path: &Path) -> DomainResult<()> {
        let body = toml::to_string_pretty(self).map_err(|e| DomainError::Config(e.to_string()))?;
        atomic_write(path, body.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip() {
        let c = AppConfig::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: AppConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.external_controller, "127.0.0.1:9090");
        assert!(!back.keep_core_running);
        assert_eq!(back.system_proxy.http_port, 7890);
        assert_eq!(back.system_proxy.socks_port, 7891);
        assert_eq!(back.system_proxy.mixed_port, 7892);
        assert_eq!(back.auto_update.interval_hours, 24);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s = r#"external_controller = "1.2.3.4:9090""#;
        let c: AppConfig = toml::from_str(s).unwrap();
        assert_eq!(c.external_controller, "1.2.3.4:9090");
        assert_eq!(c.test_timeout_ms, 5000);
        assert!(!c.system_proxy.service_enabled);
    }

    #[test]
    fn save_then_load(/* uses tempfile */) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let c = AppConfig {
            secret: "abc".into(),
            ..Default::default()
        };
        c.save(&path).unwrap();
        let back = AppConfig::load(&path).unwrap();
        assert_eq!(back.secret, "abc");
    }

    #[test]
    fn load_missing_returns_default() {
        let c = AppConfig::load(Path::new("/nonexistent/xyz/config.toml")).unwrap();
        assert_eq!(c.external_controller, "127.0.0.1:9090");
    }
}
