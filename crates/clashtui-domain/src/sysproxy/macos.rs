//! macOS 系统代理：`networksetup`。
//!
//! 活跃网络服务探测：bind UDP 到 1.1.1.1:80 取本机出口 IP → 在
//! `networksetup -listnetworkserviceorder` 里匹配对应硬件设备 → 服务名。
//! 兜底：取第一个已启用的服务。可用 config 手动覆盖。

use std::process::Command;

use crate::error::{DomainError, DomainResult};
use crate::sysproxy::{ProxySettings, ServiceProxy};

/// macOS 系统代理实现。
pub struct MacosProxy {
    manual_service: Option<String>,
}

impl MacosProxy {
    pub fn new(manual_service: Option<String>) -> Self {
        MacosProxy {
            manual_service: manual_service.filter(|s| !s.is_empty()),
        }
    }

    /// 解析要操作的网络服务名。
    fn service(&self) -> DomainResult<String> {
        if let Some(s) = &self.manual_service {
            return Ok(s.clone());
        }
        detect_active_service().or_else(|_| first_enabled_service())
    }
}

impl ServiceProxy for MacosProxy {
    fn enable(&self, s: &ProxySettings) -> DomainResult<()> {
        let svc = self.service()?;
        let http_port = s.http_port.to_string();
        let socks_port = s.socks_port.to_string();

        run(&["-setwebproxy", &svc, &s.host, &http_port])?;
        run(&["-setsecurewebproxy", &svc, &s.host, &http_port])?;
        run(&["-setsocksfirewallproxy", &svc, &s.host, &socks_port])?;
        run(&["-setwebproxystate", &svc, "on"])?;
        run(&["-setsecurewebproxystate", &svc, "on"])?;
        run(&["-setsocksfirewallproxystate", &svc, "on"])?;

        // 设置 bypass 列表。
        if !s.bypass.is_empty() {
            let mut args = vec!["-setproxybypassdomains".to_string(), svc.clone()];
            args.extend(s.bypass.iter().cloned());
            let arg_refs: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
            run(&arg_refs)?;
        }
        Ok(())
    }

    fn disable(&self) -> DomainResult<()> {
        let svc = self.service()?;
        run(&["-setwebproxystate", &svc, "off"])?;
        run(&["-setsecurewebproxystate", &svc, "off"])?;
        run(&["-setsocksfirewallproxystate", &svc, "off"])?;
        Ok(())
    }

    fn is_enabled(&self) -> DomainResult<bool> {
        let svc = self.service()?;
        let out = output(&["-getwebproxy", &svc])?;
        // 输出含 "Enabled: Yes"。
        Ok(out.lines().any(|l| {
            let l = l.trim();
            l.starts_with("Enabled:") && l.contains("Yes")
        }))
    }
}

/// 运行 networksetup，丢弃输出。
fn run(args: &[&str]) -> DomainResult<()> {
    let status = Command::new("networksetup")
        .args(args)
        .status()
        .map_err(|e| DomainError::SysProxy(format!("networksetup 执行失败: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(DomainError::SysProxy(format!(
            "networksetup {:?} 返回 {status}",
            args.first().unwrap_or(&"")
        )))
    }
}

/// 运行 networksetup 并取 stdout。
fn output(args: &[&str]) -> DomainResult<String> {
    let out = Command::new("networksetup")
        .args(args)
        .output()
        .map_err(|e| DomainError::SysProxy(format!("networksetup 执行失败: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 通过出口 IP 对应的硬件设备探测活跃服务名。
fn detect_active_service() -> DomainResult<String> {
    let local_ip = local_egress_ip()?;
    // 找到该 IP 所属的设备名（en0 等）。
    let device = device_for_ip(&local_ip)?;
    // 在服务顺序里把设备映射回服务名。
    service_for_device(&device)
}

/// bind 一个 UDP socket 到公网地址，取本机出口 IP（不实际发包）。
fn local_egress_ip() -> DomainResult<String> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| DomainError::SysProxy(format!("bind 失败: {e}")))?;
    sock.connect("1.1.1.1:80")
        .map_err(|e| DomainError::SysProxy(format!("connect 失败: {e}")))?;
    let addr = sock
        .local_addr()
        .map_err(|e| DomainError::SysProxy(e.to_string()))?;
    Ok(addr.ip().to_string())
}

/// 用 `ifconfig` 找出哪个设备拥有该 IP。
fn device_for_ip(ip: &str) -> DomainResult<String> {
    let out = Command::new("ifconfig")
        .output()
        .map_err(|e| DomainError::SysProxy(format!("ifconfig 失败: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut current_dev = String::new();
    for line in text.lines() {
        if !line.starts_with('\t') && !line.starts_with(' ') {
            // 设备行，形如 "en0: flags=..."
            if let Some(dev) = line.split(':').next() {
                current_dev = dev.to_string();
            }
        } else if line.trim_start().starts_with("inet ") && line.contains(ip) {
            return Ok(current_dev);
        }
    }
    Err(DomainError::SysProxy(format!("未找到 IP {ip} 对应设备")))
}

/// 在 `-listnetworkserviceorder` 输出里把设备映射回服务名。
fn service_for_device(device: &str) -> DomainResult<String> {
    let text = output(&["-listnetworkserviceorder"])?;
    parse_service_for_device(&text, device)
        .ok_or_else(|| DomainError::SysProxy(format!("未找到设备 {device} 的服务")))
}

/// 纯解析：从 `-listnetworkserviceorder` 文本里找设备对应的服务名。
///
/// 文本块形如：
/// ```text
/// (1) Wi-Fi
/// (Hardware Port: Wi-Fi, Device: en0)
/// ```
fn parse_service_for_device(text: &str, device: &str) -> Option<String> {
    let mut last_service = String::new();
    for line in text.lines() {
        let l = line.trim();
        // 服务标题行 "(N) 名称"。
        if let Some(name) = parse_service_title(l) {
            last_service = name;
        } else if l.contains(&format!("Device: {device})")) {
            // 精确匹配 "Device: en0)"，避免 en0 命中 en00。
            return Some(last_service.clone());
        }
    }
    None
}

/// 解析 "(1) Wi-Fi" → "Wi-Fi"；非标题行返回 None。
fn parse_service_title(line: &str) -> Option<String> {
    let rest = line.strip_prefix('(')?;
    let close = rest.find(')')?;
    // 括号内必须是数字编号。
    if !rest[..close].chars().all(|c| c.is_ascii_digit()) || close == 0 {
        return None;
    }
    let name = rest[close + 1..].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 兜底：第一个已启用的网络服务。
fn first_enabled_service() -> DomainResult<String> {
    let text = output(&["-listallnetworkservices"])?;
    // 首行是说明，带 `*` 前缀表示禁用。
    for line in text.lines().skip(1) {
        if !line.starts_with('*') && !line.trim().is_empty() {
            return Ok(line.trim().to_string());
        }
    }
    Err(DomainError::SysProxy("未找到可用网络服务".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "An asterisk (*) denotes that a network service is disabled.\n\
(1) Wi-Fi\n\
(Hardware Port: Wi-Fi, Device: en0)\n\
\n\
(2) Thunderbolt Bridge\n\
(Hardware Port: Thunderbolt Bridge, Device: bridge0)\n";

    #[test]
    fn parse_title() {
        assert_eq!(parse_service_title("(1) Wi-Fi"), Some("Wi-Fi".into()));
        assert_eq!(
            parse_service_title("(10) USB 10/100 LAN"),
            Some("USB 10/100 LAN".into())
        );
        assert_eq!(
            parse_service_title("(Hardware Port: Wi-Fi, Device: en0)"),
            None
        );
        assert_eq!(parse_service_title("random"), None);
    }

    #[test]
    fn maps_device_to_service() {
        assert_eq!(
            parse_service_for_device(SAMPLE, "en0"),
            Some("Wi-Fi".into())
        );
        assert_eq!(
            parse_service_for_device(SAMPLE, "bridge0"),
            Some("Thunderbolt Bridge".into())
        );
        assert_eq!(parse_service_for_device(SAMPLE, "en9"), None);
    }

    #[test]
    fn device_exact_match_avoids_prefix_collision() {
        let text = "(1) Ethernet\n(Hardware Port: Ethernet, Device: en00)\n";
        // 找 en0 不应误匹配 en00。
        assert_eq!(parse_service_for_device(text, "en0"), None);
    }
}
