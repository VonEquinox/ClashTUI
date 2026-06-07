//! 配置分层管线：raw → mixin → override → 强制注入 → 运行时 config.yaml。
//!
//! M2 落地：深合并 + 强制注入 external-controller/secret/端口。
//! M8 扩展：数组的 prepend/append/override-by-name/inject 语义。

use crate::config::AppConfig;
use crate::error::DomainResult;
use crate::yaml::{self, Value};

/// 构建运行时配置。
///
/// 步骤：
/// 1. 解析原始订阅。
/// 2. 若有 mixin，则按精细语义应用（prepend/append/override-by-name/inject + 深合并）。
/// 3. 若有 override，则深合并（override 优先，blunt 覆盖）。
/// 4. 强制注入 external-controller / secret / 端口，保证 API 始终可控。
///
/// 返回最终 YAML 文本。
pub fn build_runtime(
    raw_yaml: &str,
    mixin_yaml: Option<&str>,
    override_yaml: Option<&str>,
    config: &AppConfig,
) -> DomainResult<String> {
    let mut base = yaml::parse(raw_yaml)?;

    if let Some(m) = mixin_yaml {
        if !m.trim().is_empty() {
            let mixin = yaml::parse(m)?;
            apply_mixin(&mut base, &mixin);
        }
    }
    if let Some(o) = override_yaml {
        if !o.trim().is_empty() {
            let over = yaml::parse(o)?;
            yaml::deep_merge(&mut base, &over);
        }
    }

    force_inject(&mut base, config);

    yaml::to_string(&base)
}

/// 三个支持数组精细操作的目标段。
const ARRAY_SECTIONS: [&str; 3] = ["rules", "proxies", "proxy-groups"];

/// 应用 mixin 的精细语义：
/// - `prepend-<section>`：把数组接到该段前面。
/// - `append-<section>`：把数组接到该段后面。
/// - `override-<section>`：按 name 主键替换同名项、新名追加（proxies/proxy-groups），
///   或对 rules 直接替换整段。
/// - 其它键：深合并（mixin 优先）。
pub fn apply_mixin(base: &mut Value, mixin: &Value) {
    let Some(mixin_map) = mixin.as_mapping() else {
        return;
    };

    // 先处理精细操作键，再深合并剩余普通键。
    let mut plain = yaml::Mapping::new();
    for (k, v) in mixin_map {
        let Some(key) = k.as_str() else {
            plain.insert(k.clone(), v.clone());
            continue;
        };
        if let Some(section) = key.strip_prefix("prepend-") {
            if ARRAY_SECTIONS.contains(&section) {
                splice_array(base, section, v, true);
                continue;
            }
        }
        if let Some(section) = key.strip_prefix("append-") {
            if ARRAY_SECTIONS.contains(&section) {
                splice_array(base, section, v, false);
                continue;
            }
        }
        if let Some(section) = key.strip_prefix("override-") {
            if ARRAY_SECTIONS.contains(&section) {
                override_section(base, section, v);
                continue;
            }
        }
        plain.insert(k.clone(), v.clone());
    }

    if !plain.is_empty() {
        yaml::deep_merge(base, &Value::Mapping(plain));
    }
}

/// 把 `items`（应为数组）接到 base 的 `section` 段前/后。
fn splice_array(base: &mut Value, section: &str, items: &Value, prepend: bool) {
    let Some(new_items) = items.as_sequence() else {
        return;
    };
    let Some(map) = base.as_mapping_mut() else {
        return;
    };
    let key = Value::from(section);
    let existing = map
        .get(&key)
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    let merged: Vec<Value> = if prepend {
        new_items.iter().chain(existing.iter()).cloned().collect()
    } else {
        existing.iter().chain(new_items.iter()).cloned().collect()
    };
    map.insert(key, Value::Sequence(merged));
}

/// 覆盖 `section`：proxies/proxy-groups 按 name 主键合并；rules 整段替换。
fn override_section(base: &mut Value, section: &str, items: &Value) {
    let Some(new_items) = items.as_sequence() else {
        return;
    };
    let Some(map) = base.as_mapping_mut() else {
        return;
    };
    let key = Value::from(section);
    if section == "rules" {
        map.insert(key, items.clone());
        return;
    }
    let existing = map
        .get(&key)
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    let merged = merge_by_name(&existing, new_items, "name");
    map.insert(key, Value::Sequence(merged));
}

/// 强制注入 API 控制所需字段，确保 ClashTUI 始终能连上内核。
fn force_inject(base: &mut Value, config: &AppConfig) {
    // 确保 base 是 mapping。
    if base.as_mapping().is_none() {
        *base = Value::Mapping(Default::default());
    }
    let map = base.as_mapping_mut().expect("已确保是 mapping");

    yaml::set(
        map,
        "external-controller",
        Value::from(config.external_controller.clone()),
    );
    if !config.secret.is_empty() {
        yaml::set(map, "secret", Value::from(config.secret.clone()));
    }
    // 代理端口注入（若用户配置了）。
    if config.system_proxy.http_port > 0 {
        yaml::set(map, "port", Value::from(config.system_proxy.http_port));
    }
    if config.system_proxy.socks_port > 0 {
        yaml::set(
            map,
            "socks-port",
            Value::from(config.system_proxy.socks_port),
        );
    }
    if config.system_proxy.mixed_port > 0 {
        yaml::set(
            map,
            "mixed-port",
            Value::from(config.system_proxy.mixed_port),
        );
    }
}

/// 按名称合并两个数组（M8 inject/override-by-name 的基础）。
/// 以 `key_field`（如 "name"）为主键：overlay 中同名项替换 base，新名追加。
pub fn merge_by_name(base: &[Value], overlay: &[Value], key_field: &str) -> Vec<Value> {
    let name_of = |v: &Value| -> Option<String> {
        yaml::get(v, key_field).and_then(|n| n.as_str().map(|s| s.to_string()))
    };
    let mut result: Vec<Value> = base.to_vec();
    for item in overlay {
        if let Some(name) = name_of(item) {
            if let Some(existing) = result
                .iter_mut()
                .find(|v| name_of(v).as_deref() == Some(name.as_str()))
            {
                *existing = item.clone();
                continue;
            }
        }
        result.push(item.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        AppConfig {
            external_controller: "127.0.0.1:9090".into(),
            secret: "sk".into(),
            ..Default::default()
        }
    }

    #[test]
    fn force_inject_controller_and_secret() {
        let raw = "proxies: []\nmode: rule\n";
        let out = build_runtime(raw, None, None, &cfg()).unwrap();
        assert!(out.contains("external-controller: 127.0.0.1:9090"));
        assert!(out.contains("secret: sk"));
        assert!(out.contains("port: 7890"));
        assert!(out.contains("socks-port: 7891"));
        assert!(out.contains("mixed-port: 7892"));
        // 原有字段保留。
        assert!(out.contains("mode: rule"));
    }

    #[test]
    fn mixin_overrides_raw() {
        let raw = "mode: rule\nlog-level: info\n";
        let mixin = "log-level: debug\n";
        let out = build_runtime(raw, Some(mixin), None, &cfg()).unwrap();
        assert!(out.contains("log-level: debug"));
    }

    #[test]
    fn override_beats_mixin() {
        let raw = "mode: rule\n";
        let mixin = "mode: global\n";
        let over = "mode: direct\n";
        let out = build_runtime(raw, Some(mixin), Some(over), &cfg()).unwrap();
        // 解析回来检查最终 mode = direct。
        let v = yaml::parse(&out).unwrap();
        assert_eq!(yaml::get(&v, "mode").unwrap().as_str(), Some("direct"));
    }

    #[test]
    fn build_runtime_is_idempotent() {
        let raw = "proxies: []\nmode: rule\n";
        let out1 = build_runtime(raw, None, None, &cfg()).unwrap();
        let out2 = build_runtime(&out1, None, None, &cfg()).unwrap();
        assert_eq!(out1, out2);
    }

    #[test]
    fn mixin_prepend_and_append_rules() {
        let raw = "rules:\n  - RULE-A\n  - RULE-B\n";
        let mixin = "prepend-rules:\n  - FIRST\nappend-rules:\n  - LAST\n";
        let out = build_runtime(raw, Some(mixin), None, &cfg()).unwrap();
        let v = yaml::parse(&out).unwrap();
        let rules: Vec<String> = yaml::get(&v, "rules")
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(rules, vec!["FIRST", "RULE-A", "RULE-B", "LAST"]);
    }

    #[test]
    fn mixin_override_proxies_by_name() {
        let raw = "proxies:\n  - {name: A, server: old}\n  - {name: B, server: keep}\n";
        let mixin = "override-proxies:\n  - {name: A, server: new}\n  - {name: C, server: added}\n";
        let out = build_runtime(raw, Some(mixin), None, &cfg()).unwrap();
        let v = yaml::parse(&out).unwrap();
        let proxies = yaml::get(&v, "proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 3);
        let a = proxies
            .iter()
            .find(|p| yaml::get(p, "name").unwrap().as_str() == Some("A"))
            .unwrap();
        assert_eq!(yaml::get(a, "server").unwrap().as_str(), Some("new"));
    }

    #[test]
    fn mixin_plain_keys_still_deep_merge() {
        let raw = "mode: rule\ndns:\n  enable: false\n";
        let mixin = "dns:\n  enable: true\n  listen: '0.0.0.0:53'\n";
        let out = build_runtime(raw, Some(mixin), None, &cfg()).unwrap();
        let v = yaml::parse(&out).unwrap();
        let dns = yaml::get(&v, "dns").unwrap();
        assert_eq!(yaml::get(dns, "enable").unwrap().as_bool(), Some(true));
        assert!(yaml::get(dns, "listen").is_some());
    }

    #[test]
    fn merge_by_name_replaces_and_appends() {
        let base = yaml::parse("- {name: A, v: 1}\n- {name: B, v: 2}\n").unwrap();
        let overlay = yaml::parse("- {name: B, v: 99}\n- {name: C, v: 3}\n").unwrap();
        let base_seq = base.as_sequence().unwrap();
        let over_seq = overlay.as_sequence().unwrap();
        let merged = merge_by_name(base_seq, over_seq, "name");
        assert_eq!(merged.len(), 3);
        // B 被替换为 99。
        let b = merged
            .iter()
            .find(|v| yaml::get(v, "name").unwrap().as_str() == Some("B"))
            .unwrap();
        assert_eq!(yaml::get(b, "v").unwrap().as_u64(), Some(99));
    }
}
