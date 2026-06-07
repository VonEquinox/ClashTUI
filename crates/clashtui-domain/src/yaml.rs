//! 保序 YAML 抽象。
//!
//! mixin/profile/runtime 的所有 YAML 都经此处理，使依赖可替换（默认
//! `serde_yaml_ng`，feature `yaml-noya` 时换 `noyalib`）。代理组/规则顺序
//! 必须保真，否则路由会变，故 map 用保序表示。
//!
//! 这里用 serde_yaml_ng 的 `Value`（其 Mapping 内部保序）做深合并的载体。

use crate::error::{DomainError, DomainResult};

#[cfg(feature = "yaml-ng")]
pub use serde_yaml_ng as yaml_impl;

#[cfg(all(feature = "yaml-noya", not(feature = "yaml-ng")))]
pub use noyalib as yaml_impl;

/// YAML 值类型别名（保序）。
pub type Value = yaml_impl::Value;
/// YAML 映射类型别名（保序）。
pub type Mapping = yaml_impl::Mapping;

/// 解析 YAML 文本。
pub fn parse(text: &str) -> DomainResult<Value> {
    yaml_impl::from_str(text).map_err(|e| DomainError::Yaml(e.to_string()))
}

/// 序列化为 YAML 文本。
pub fn to_string(value: &Value) -> DomainResult<String> {
    yaml_impl::to_string(value).map_err(|e| DomainError::Yaml(e.to_string()))
}

/// 取 string key 对应的值（仅当 value 是 mapping）。
pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_mapping().and_then(|m| m.get(Value::from(key)))
}

/// 在 mapping 中设置 string key（保持已有顺序，新增追加到末尾）。
pub fn set(map: &mut Mapping, key: &str, val: Value) {
    map.insert(Value::from(key), val);
}

/// 深合并 `overlay` 到 `base`（overlay 优先）：
/// - 两者都是 mapping → 递归按 key 合并（保序：base 顺序在前，overlay 新 key 追加）。
/// - 两者都是 sequence → 默认 overlay 覆盖（具体 prepend/append 由 mixin 层处理）。
/// - 其它 → overlay 覆盖 base。
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base.as_mapping_mut(), overlay.as_mapping()) {
        (Some(base_map), Some(over_map)) => {
            for (k, v) in over_map {
                if let Some(existing) = base_map.get_mut(k) {
                    deep_merge(existing, v);
                } else {
                    base_map.insert(k.clone(), v.clone());
                }
            }
        }
        _ => {
            *base = overlay.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_roundtrip_preserves_key_order() {
        let text = "b: 1\na: 2\nc: 3\n";
        let v = parse(text).unwrap();
        let out = to_string(&v).unwrap();
        // 顺序应保持 b, a, c。
        let keys: Vec<&str> = out.lines().filter_map(|l| l.split(':').next()).collect();
        assert_eq!(keys, vec!["b", "a", "c"]);
    }

    #[test]
    fn deep_merge_overlay_wins_and_appends() {
        let mut base = parse("a: 1\nb:\n  x: 1\n  y: 2\n").unwrap();
        let overlay = parse("b:\n  y: 99\n  z: 3\nc: 5\n").unwrap();
        deep_merge(&mut base, &overlay);
        // b.x 保留, b.y 被覆盖为 99, b.z 新增, c 新增。
        let b = get(&base, "b").unwrap();
        assert_eq!(get(b, "x").unwrap().as_u64(), Some(1));
        assert_eq!(get(b, "y").unwrap().as_u64(), Some(99));
        assert_eq!(get(b, "z").unwrap().as_u64(), Some(3));
        assert_eq!(get(&base, "c").unwrap().as_u64(), Some(5));
    }

    #[test]
    fn deep_merge_scalar_overwrites() {
        let mut base = parse("a: 1\n").unwrap();
        let overlay = parse("a: 2\n").unwrap();
        deep_merge(&mut base, &overlay);
        assert_eq!(get(&base, "a").unwrap().as_u64(), Some(2));
    }
}
