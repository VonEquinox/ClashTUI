//! 跨平台路径解析：macOS Application Support / Linux XDG。

use std::path::PathBuf;

use directories::ProjectDirs;

/// ClashTUI 的存储根与子路径。
#[derive(Debug, Clone)]
pub struct Paths {
    /// 配置根目录（macOS `~/Library/Application Support/ClashTUI`，Linux `~/.config/clashtui`）。
    pub config_dir: PathBuf,
}

impl Paths {
    /// 解析标准路径；无法定位时回退到当前目录下的 `.clashtui`。
    pub fn resolve() -> Self {
        let config_dir = ProjectDirs::from("", "", "ClashTUI")
            .map(|p| p.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".clashtui"));
        Paths { config_dir }
    }

    /// 显式指定根目录（测试用）。
    pub fn with_root(root: PathBuf) -> Self {
        Paths { config_dir: root }
    }

    /// 应用配置文件 `config.toml`。
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Profile 元数据 DB `profiles.toml`。
    pub fn profiles_file(&self) -> PathBuf {
        self.config_dir.join("profiles.toml")
    }

    /// 原始订阅存放目录 `profiles/`。
    pub fn profiles_dir(&self) -> PathBuf {
        self.config_dir.join("profiles")
    }

    /// 某 profile 的原始 YAML 路径。
    pub fn profile_yaml(&self, name: &str) -> PathBuf {
        self.profiles_dir().join(format!("{name}.yaml"))
    }

    /// 内核工作目录 `core/`（存运行时 config.yaml、geo 数据等）。
    pub fn core_dir(&self) -> PathBuf {
        self.config_dir.join("core")
    }

    /// 内核加载的运行时配置 `core/config.yaml`。
    pub fn runtime_config(&self) -> PathBuf {
        self.core_dir().join("config.yaml")
    }

    /// mixin 配置 `mixin.yaml`。
    pub fn mixin_file(&self) -> PathBuf {
        self.config_dir.join("mixin.yaml")
    }

    /// 覆写配置 `override.yaml`。
    pub fn override_file(&self) -> PathBuf {
        self.config_dir.join("override.yaml")
    }

    /// env 级系统代理 source 片段 `proxy.sh`。
    pub fn proxy_env_file(&self) -> PathBuf {
        self.config_dir.join("proxy.sh")
    }

    /// 内核二进制默认存放路径 `bin/mihomo`。
    pub fn default_binary(&self) -> PathBuf {
        self.config_dir.join("bin").join(mihomo_bin_name())
    }

    /// 确保所有需要的目录存在。
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(self.profiles_dir())?;
        std::fs::create_dir_all(self.core_dir())?;
        std::fs::create_dir_all(self.config_dir.join("bin"))?;
        Ok(())
    }
}

/// 当前平台的 mihomo 二进制文件名。
pub fn mihomo_bin_name() -> &'static str {
    if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpaths_compose() {
        let p = Paths::with_root(PathBuf::from("/tmp/ct"));
        assert_eq!(p.config_file(), PathBuf::from("/tmp/ct/config.toml"));
        assert_eq!(
            p.profile_yaml("hk"),
            PathBuf::from("/tmp/ct/profiles/hk.yaml")
        );
        assert_eq!(
            p.runtime_config(),
            PathBuf::from("/tmp/ct/core/config.yaml")
        );
    }
}
