//! 订阅自动更新调度：进程内 tokio interval。

use std::time::Duration;

/// 计算调度间隔（小时数下限 1，默认 24）。
pub fn interval_duration(interval_hours: u32) -> Duration {
    let hours = interval_hours.max(1) as u64;
    Duration::from_secs(hours * 3600)
}

/// 某 profile 是否到期需要更新。
/// `last_updated` / `now` 为 unix 秒；`interval_hours` 为间隔。
pub fn is_stale(last_updated: i64, now: i64, interval_hours: u32) -> bool {
    if last_updated <= 0 {
        return true; // 从未更新
    }
    let elapsed = now.saturating_sub(last_updated);
    elapsed >= (interval_hours.max(1) as i64) * 3600
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_has_floor() {
        assert_eq!(interval_duration(0), Duration::from_secs(3600));
        assert_eq!(interval_duration(24), Duration::from_secs(24 * 3600));
    }

    #[test]
    fn staleness_logic() {
        let now = 1_000_000;
        // 从未更新。
        assert!(is_stale(0, now, 24));
        // 刚更新。
        assert!(!is_stale(now - 100, now, 24));
        // 超过 24h。
        assert!(is_stale(now - 25 * 3600, now, 24));
        // 不足 24h。
        assert!(!is_stale(now - 23 * 3600, now, 24));
    }
}
