//! mihomo API 的类型化数据模型。
//!
//! 编码了核验过的协议不变量（务必靠类型 + 测试守住）：
//! - `mode` 小写（rule/global/direct）。
//! - 延迟是 `u16`，**0 == 超时/不可达**，绝不渲染成 0ms → 用 [`Delay`] newtype。
//! - 组字段 camelCase（testUrl/expectedStatus/...），代理字段 kebab-case（dialer-proxy/...）。
//! - `GET /proxies` 包裹在 `{proxies:{...}}`，而 `GET /proxies/{name}` 返回**未包裹**的单个 Proxy。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 内核运行模式。线上格式为小写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Rule,
    Global,
    Direct,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Rule => "rule",
            Mode::Global => "global",
            Mode::Direct => "direct",
        }
    }

    /// 用于 UI 轮换：rule → global → direct → rule。
    pub fn next(self) -> Mode {
        match self {
            Mode::Rule => Mode::Global,
            Mode::Global => Mode::Direct,
            Mode::Direct => Mode::Rule,
        }
    }

    pub const ALL: [Mode; 3] = [Mode::Rule, Mode::Global, Mode::Direct];
}

/// 延迟值。`0` 表示超时/不可达，而非 0 毫秒。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Delay(pub u16);

impl Delay {
    /// 是否为超时（0）。
    pub fn is_timeout(self) -> bool {
        self.0 == 0
    }

    /// 毫秒值；超时返回 None。
    pub fn millis(self) -> Option<u16> {
        if self.is_timeout() {
            None
        } else {
            Some(self.0)
        }
    }

    /// 渲染用文本。
    pub fn display(self) -> String {
        match self.millis() {
            Some(ms) => format!("{ms}ms"),
            None => "timeout".to_string(),
        }
    }
}

/// 单条延迟历史记录。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DelayHistory {
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub delay: u16,
}

/// 一个代理节点或代理组。
///
/// `/proxies` 里组与节点共用此结构：组会有 `all`/`now` 字段，节点没有。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proxy {
    pub name: String,
    /// 类型：Selector / URLTest / Direct / Shadowsocks / Vmess ...
    #[serde(rename = "type")]
    pub kind: String,
    /// 组内成员名（仅组有）。
    #[serde(default)]
    pub all: Vec<String>,
    /// 当前选中的成员名（仅 Selector/URLTest 等组有）。
    #[serde(default)]
    pub now: Option<String>,
    /// 延迟历史（最后一条是最新）。
    #[serde(default)]
    pub history: Vec<DelayHistory>,
    /// 是否 udp。
    #[serde(default)]
    pub udp: bool,
    /// 组测速用的 URL（camelCase）。
    #[serde(default)]
    pub test_url: Option<String>,
    /// 组期望状态码（camelCase）。
    #[serde(default)]
    pub expected_status: Option<String>,
}

impl Proxy {
    /// 是否为可选择节点的组（Selector）。只有 Selector 能 PUT 切换。
    pub fn is_selector(&self) -> bool {
        self.kind.eq_ignore_ascii_case("Selector")
    }

    /// 是否为某种代理组（含成员）。
    pub fn is_group(&self) -> bool {
        !self.all.is_empty()
            || matches!(
                self.kind.as_str(),
                "Selector" | "URLTest" | "Fallback" | "LoadBalance" | "Relay"
            )
    }

    /// 最近一次延迟（无历史时为超时）。
    pub fn latest_delay(&self) -> Delay {
        Delay(self.history.last().map(|h| h.delay).unwrap_or(0))
    }
}

/// `GET /proxies` 的包裹结构。
#[derive(Debug, Clone, Deserialize)]
pub struct ProxiesResponse {
    pub proxies: HashMap<String, Proxy>,
}

/// `GET /group` 的包裹结构（仅代理组，按顺序）。
#[derive(Debug, Clone, Deserialize)]
pub struct GroupsResponse {
    pub proxies: Vec<Proxy>,
}

/// 切换节点的请求体：`PUT /proxies/{group}` body `{"name": "<node>"}`。
#[derive(Debug, Clone, Serialize)]
pub struct SelectBody {
    pub name: String,
}

/// 内核版本信息。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    #[serde(default)]
    pub version: String,
    /// 是否 mihomo（meta）内核。
    #[serde(default)]
    pub meta: bool,
}

/// TUN 配置（GeneralConfig 内嵌）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TunConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default, rename = "device")]
    pub device: Option<String>,
}

/// `GET /configs` 返回的通用配置（只取我们关心的字段）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GeneralConfig {
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub port: u16,
    #[serde(default, rename = "socks-port")]
    pub socks_port: u16,
    #[serde(default, rename = "mixed-port")]
    pub mixed_port: u16,
    #[serde(default, rename = "redir-port")]
    pub redir_port: u16,
    #[serde(default, rename = "tproxy-port")]
    pub tproxy_port: u16,
    #[serde(default, rename = "allow-lan")]
    pub allow_lan: bool,
    #[serde(default, rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(default)]
    pub ipv6: bool,
    #[serde(default)]
    pub tun: TunConfig,
}

/// `PATCH /configs` 的部分更新构造器，发出 kebab-case 键、mode 小写。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConfigPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "mixed-port")]
    pub mixed_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "socks-port")]
    pub socks_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tun: Option<TunPatch>,
}

/// TUN 部分更新。
#[derive(Debug, Clone, Serialize)]
pub struct TunPatch {
    pub enable: bool,
}

impl ConfigPatch {
    /// 仅切换模式。
    pub fn mode(mode: Mode) -> Self {
        ConfigPatch {
            mode: Some(mode),
            ..Default::default()
        }
    }

    /// 仅切换 TUN。
    pub fn tun(enable: bool) -> Self {
        ConfigPatch {
            tun: Some(TunPatch { enable }),
            ..Default::default()
        }
    }
}

/// 一条日志（WS `/logs` 帧）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    #[serde(rename = "type", default)]
    pub level: String,
    #[serde(default)]
    pub payload: String,
}

/// 流量帧（WS `/traffic`）：每秒上下行字节。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Traffic {
    #[serde(default)]
    pub up: u64,
    #[serde(default)]
    pub down: u64,
}

/// 内存帧（WS `/memory`）。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Memory {
    #[serde(default)]
    pub inuse: u64,
    #[serde(default)]
    pub oslimit: u64,
}

/// 连接元数据。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionMeta {
    #[serde(default)]
    pub network: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub source_ip: String,
    #[serde(default)]
    pub destination_ip: String,
    #[serde(default)]
    pub source_port: String,
    #[serde(default)]
    pub destination_port: String,
    #[serde(default)]
    pub host: String,
}

/// 单条连接（`GET /connections` 或 WS 帧内 `connections[]`）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub metadata: ConnectionMeta,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default, rename = "rulePayload")]
    pub rule_payload: String,
}

/// 连接快照（WS `/connections` 帧）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionsSnapshot {
    #[serde(default)]
    pub download_total: u64,
    #[serde(default)]
    pub upload_total: u64,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Mode::Rule).unwrap(), "\"rule\"");
        assert_eq!(serde_json::to_string(&Mode::Global).unwrap(), "\"global\"");
        let m: Mode = serde_json::from_str("\"direct\"").unwrap();
        assert_eq!(m, Mode::Direct);
    }

    #[test]
    fn delay_zero_is_timeout() {
        assert!(Delay(0).is_timeout());
        assert_eq!(Delay(0).millis(), None);
        assert_eq!(Delay(0).display(), "timeout");
        assert_eq!(Delay(120).display(), "120ms");
        assert_eq!(Delay(120).millis(), Some(120));
    }

    #[test]
    fn proxy_camelcase_group_fields() {
        let json = r#"{
            "name": "Proxy",
            "type": "Selector",
            "all": ["A", "B"],
            "now": "A",
            "testUrl": "http://www.gstatic.com/generate_204",
            "expectedStatus": "204",
            "history": [{"time": "t", "delay": 100}]
        }"#;
        let p: Proxy = serde_json::from_str(json).unwrap();
        assert_eq!(p.name, "Proxy");
        assert!(p.is_selector());
        assert!(p.is_group());
        assert_eq!(p.now.as_deref(), Some("A"));
        assert_eq!(
            p.test_url.as_deref(),
            Some("http://www.gstatic.com/generate_204")
        );
        assert_eq!(p.expected_status.as_deref(), Some("204"));
        assert_eq!(p.latest_delay(), Delay(100));
    }

    #[test]
    fn non_selector_group_detected() {
        let json = r#"{"name":"Auto","type":"URLTest","all":["A"],"now":"A","history":[]}"#;
        let p: Proxy = serde_json::from_str(json).unwrap();
        assert!(!p.is_selector());
        assert!(p.is_group());
    }

    #[test]
    fn proxies_response_is_wrapped() {
        let json =
            r#"{"proxies":{"GLOBAL":{"name":"GLOBAL","type":"Selector","all":[],"history":[]}}}"#;
        let r: ProxiesResponse = serde_json::from_str(json).unwrap();
        assert!(r.proxies.contains_key("GLOBAL"));
    }

    #[test]
    fn config_patch_emits_kebab_and_lowercase() {
        let patch = ConfigPatch::mode(Mode::Global);
        let s = serde_json::to_string(&patch).unwrap();
        assert_eq!(s, r#"{"mode":"global"}"#);

        let tun = ConfigPatch::tun(true);
        let s2 = serde_json::to_string(&tun).unwrap();
        assert_eq!(s2, r#"{"tun":{"enable":true}}"#);

        let ports = ConfigPatch {
            port: Some(7890),
            socks_port: Some(7891),
            mixed_port: Some(7892),
            ..Default::default()
        };
        let p = serde_json::to_value(&ports).unwrap();
        assert_eq!(p["port"], 7890);
        assert_eq!(p["socks-port"], 7891);
        assert_eq!(p["mixed-port"], 7892);
    }

    #[test]
    fn general_config_parses_kebab_ports_and_nested_tun() {
        let json = r#"{
            "mode": "rule",
            "port": 7889,
            "mixed-port": 7890,
            "socks-port": 7891,
            "log-level": "info",
            "tun": {"enable": true, "stack": "gvisor"}
        }"#;
        let c: GeneralConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.mode, Some(Mode::Rule));
        assert_eq!(c.port, 7889);
        assert_eq!(c.mixed_port, 7890);
        assert_eq!(c.socks_port, 7891);
        assert_eq!(c.log_level.as_deref(), Some("info"));
        assert!(c.tun.enable);
        assert_eq!(c.tun.stack.as_deref(), Some("gvisor"));
    }
}
