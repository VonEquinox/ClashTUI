//! Profile / 订阅管理：元数据 DB + 原始订阅存储。
//!
//! - `profiles.toml`：所有 profile 的元数据 + 当前指针。
//! - `profiles/{name}.yaml`：逐字节保存的原始订阅，**永不修改**。

pub mod download;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, DomainResult};
use crate::paths::Paths;
use crate::util::atomic_write;

/// profile 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    /// 本地文件导入。
    File,
    /// 远程订阅 URL。
    Url,
}

/// 订阅流量/到期信息（来自 subscription-userinfo 头）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionInfo {
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub total: u64,
    /// 到期 unix 时间戳（秒），0 表示无。
    #[serde(default)]
    pub expire: i64,
}

impl SubscriptionInfo {
    /// 已用流量（上行+下行）。
    pub fn used(&self) -> u64 {
        self.upload + self.download
    }

    /// 剩余流量；total 为 0 时返回 None（无限制/未知）。
    pub fn remaining(&self) -> Option<u64> {
        if self.total == 0 {
            None
        } else {
            Some(self.total.saturating_sub(self.used()))
        }
    }
}

/// 单个 profile 元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    pub kind: ProfileKind,
    /// URL（kind=Url）或源文件路径（kind=File，仅记录）。
    #[serde(default)]
    pub url: String,
    /// 最近更新 unix 时间戳（秒）。
    #[serde(default)]
    pub last_updated: i64,
    /// 订阅信息（仅 URL）。
    #[serde(default)]
    pub subscription_info: SubscriptionInfo,
    /// 是否启用 mixin。
    #[serde(default = "default_true")]
    pub mixin_enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 持久化的 profile DB。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileDb {
    /// 当前选中的 profile 名。
    #[serde(default)]
    pub current: Option<String>,
    /// 所有 profile（保序）。
    #[serde(default)]
    pub profiles: Vec<ProfileMeta>,
}

/// Profile 存储管理器。
pub struct ProfileStore {
    paths: Paths,
    db: ProfileDb,
}

impl ProfileStore {
    /// 从磁盘加载（不存在则空库）。
    pub fn load(paths: Paths) -> DomainResult<Self> {
        let db = match std::fs::read_to_string(paths.profiles_file()) {
            Ok(s) => toml::from_str(&s).map_err(|e| DomainError::Profile(e.to_string()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProfileDb::default(),
            Err(e) => return Err(DomainError::Io(e)),
        };
        Ok(ProfileStore { paths, db })
    }

    /// 持久化 DB。
    fn persist(&self) -> DomainResult<()> {
        let body =
            toml::to_string_pretty(&self.db).map_err(|e| DomainError::Profile(e.to_string()))?;
        atomic_write(&self.paths.profiles_file(), body.as_bytes())
    }

    /// 全部 profile（名称, 是否当前）。
    pub fn list(&self) -> Vec<(String, bool)> {
        self.db
            .profiles
            .iter()
            .map(|p| (p.name.clone(), self.db.current.as_deref() == Some(&p.name)))
            .collect()
    }

    /// 当前 profile 名。
    pub fn current(&self) -> Option<&str> {
        self.db.current.as_deref()
    }

    /// 取某 profile 元数据。
    pub fn get(&self, name: &str) -> Option<&ProfileMeta> {
        self.db.profiles.iter().find(|p| p.name == name)
    }

    /// 原始订阅 YAML 路径。
    pub fn raw_path(&self, name: &str) -> PathBuf {
        self.paths.profile_yaml(name)
    }

    /// 添加/更新一个 profile（写入原始 YAML + 元数据）。
    pub fn upsert(&mut self, meta: ProfileMeta, raw_yaml: &str) -> DomainResult<()> {
        // 写原始订阅（逐字节）。
        atomic_write(&self.paths.profile_yaml(&meta.name), raw_yaml.as_bytes())?;
        // 更新元数据（同名替换，保序）。
        if let Some(existing) = self.db.profiles.iter_mut().find(|p| p.name == meta.name) {
            *existing = meta;
        } else {
            self.db.profiles.push(meta);
        }
        // 首个 profile 自动设为当前。
        if self.db.current.is_none() {
            self.db.current = self.db.profiles.first().map(|p| p.name.clone());
        }
        self.persist()
    }

    /// 删除 profile（含原始文件）。
    pub fn delete(&mut self, name: &str) -> DomainResult<()> {
        self.db.profiles.retain(|p| p.name != name);
        let _ = std::fs::remove_file(self.paths.profile_yaml(name));
        if self.db.current.as_deref() == Some(name) {
            self.db.current = self.db.profiles.first().map(|p| p.name.clone());
        }
        self.persist()
    }

    /// 切换当前 profile。
    pub fn set_current(&mut self, name: &str) -> DomainResult<()> {
        if !self.db.profiles.iter().any(|p| p.name == name) {
            return Err(DomainError::Profile(format!("profile 不存在: {name}")));
        }
        self.db.current = Some(name.to_string());
        self.persist()
    }

    /// 读取某 profile 的原始 YAML 内容。
    pub fn read_raw(&self, name: &str) -> DomainResult<String> {
        std::fs::read_to_string(self.paths.profile_yaml(name)).map_err(DomainError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (ProfileStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        (ProfileStore::load(paths).unwrap(), dir)
    }

    fn meta(name: &str) -> ProfileMeta {
        ProfileMeta {
            name: name.into(),
            kind: ProfileKind::Url,
            url: "http://example.com/sub".into(),
            last_updated: 0,
            subscription_info: SubscriptionInfo::default(),
            mixin_enabled: true,
        }
    }

    #[test]
    fn upsert_sets_first_as_current() {
        let (mut s, _d) = store();
        s.upsert(meta("hk"), "proxies: []\n").unwrap();
        assert_eq!(s.current(), Some("hk"));
        s.upsert(meta("us"), "proxies: []\n").unwrap();
        assert_eq!(s.current(), Some("hk")); // 仍是第一个
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn switch_and_delete_current_repoints() {
        let (mut s, _d) = store();
        s.upsert(meta("a"), "x: 1\n").unwrap();
        s.upsert(meta("b"), "x: 1\n").unwrap();
        s.set_current("b").unwrap();
        assert_eq!(s.current(), Some("b"));
        s.delete("b").unwrap();
        // 当前应回退到剩余的 a。
        assert_eq!(s.current(), Some("a"));
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        {
            let mut s = ProfileStore::load(paths.clone()).unwrap();
            s.upsert(meta("hk"), "proxies: []\n").unwrap();
        }
        let s2 = ProfileStore::load(paths).unwrap();
        assert_eq!(s2.current(), Some("hk"));
        assert_eq!(s2.read_raw("hk").unwrap(), "proxies: []\n");
    }

    #[test]
    fn subscription_info_math() {
        let info = SubscriptionInfo {
            upload: 100,
            download: 200,
            total: 1000,
            expire: 0,
        };
        assert_eq!(info.used(), 300);
        assert_eq!(info.remaining(), Some(700));
        let unlimited = SubscriptionInfo::default();
        assert_eq!(unlimited.remaining(), None);
    }
}
