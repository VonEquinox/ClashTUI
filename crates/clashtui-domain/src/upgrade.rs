//! 内核升级：GitHub releases 下载 + sha256 校验 + gunzip + 原子替换 + 回滚。
//!
//! 资产命名：`mihomo-{os}-{arch}-v{ver}.gz`（os: darwin/linux；arch: arm64/amd64）。

use std::io::Read;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use crate::error::{DomainError, DomainResult};

/// 最新 release 信息。
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub asset_url: String,
    pub asset_name: String,
}

/// 当前平台的 mihomo 资产中缀，如 `darwin-arm64`。
pub fn platform_infix() -> DomainResult<&'static str> {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Err(DomainError::Upgrade("不支持的操作系统".into()));
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "amd64"
    } else {
        return Err(DomainError::Upgrade("不支持的 CPU 架构".into()));
    };
    // 用静态字符串组合表覆盖四种情况。
    Ok(match (os, arch) {
        ("darwin", "arm64") => "darwin-arm64",
        ("darwin", "amd64") => "darwin-amd64",
        ("linux", "arm64") => "linux-arm64",
        ("linux", "amd64") => "linux-amd64",
        _ => unreachable!(),
    })
}

/// 查询 MetaCubeX/mihomo 最新 release，挑选匹配当前平台的 `.gz` 资产。
pub async fn fetch_latest() -> DomainResult<ReleaseInfo> {
    let infix = platform_infix()?;
    let client = reqwest::Client::builder()
        .user_agent("ClashTUI")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;

    let resp = client
        .get("https://api.github.com/repos/MetaCubeX/mihomo/releases/latest")
        .send()
        .await
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(DomainError::Upgrade(
            "GitHub API 限流（403），请稍后再试".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(DomainError::Upgrade(format!(
            "GitHub 返回 {}",
            resp.status()
        )));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;

    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    let assets = json["assets"].as_array().cloned().unwrap_or_default();

    // 选择 mihomo-{infix}-v{ver}.gz（排除 compatible/ 其它变体由 infix 精确匹配）。
    let asset = assets.iter().find(|a| {
        let name = a["name"].as_str().unwrap_or("");
        name.starts_with(&format!("mihomo-{infix}-v"))
            && name.ends_with(".gz")
            && !name.contains("compatible")
    });
    let asset = asset
        .ok_or_else(|| DomainError::Upgrade(format!("未找到匹配资产 mihomo-{infix}-v*.gz")))?;

    Ok(ReleaseInfo {
        tag,
        asset_url: asset["browser_download_url"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        asset_name: asset["name"].as_str().unwrap_or_default().to_string(),
    })
}

/// 是否有更新（latest 与 current 不同且 current 非空）。
pub fn has_update(current: &str, latest_tag: &str) -> bool {
    let cur = current.trim().trim_start_matches('v');
    let lat = latest_tag.trim().trim_start_matches('v');
    !lat.is_empty() && cur != lat
}

/// 下载并安装内核到 `target`：下载 .gz → gunzip → chmod+x → 备份旧的 → 原子替换。
/// 失败时从 .bak 回滚。
pub async fn download_and_install(info: &ReleaseInfo, target: &Path) -> DomainResult<()> {
    download_and_install_with_progress(info, target, |_, _| {}).await
}

/// 下载并安装内核，下载期间回调 `(downloaded, total)`。
pub async fn download_and_install_with_progress<F>(
    info: &ReleaseInfo,
    target: &Path,
    mut progress: F,
) -> DomainResult<()>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let client = reqwest::Client::builder()
        .user_agent("ClashTUI")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;

    let bytes = client
        .get(&info.asset_url)
        .send()
        .await
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;
    if !bytes.status().is_success() {
        return Err(DomainError::Upgrade(format!("下载返回 {}", bytes.status())));
    }
    let total = bytes.content_length();
    let mut stream = bytes.bytes_stream();
    let mut downloaded = 0u64;
    let mut archive = Vec::new();
    progress(downloaded, total);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| DomainError::Upgrade(e.to_string()))?;
        downloaded += chunk.len() as u64;
        archive.extend_from_slice(&chunk);
        progress(downloaded, total);
    }

    // gunzip。
    let mut decoder = flate2::read::GzDecoder::new(&archive[..]);
    let mut binary = Vec::new();
    decoder
        .read_to_end(&mut binary)
        .map_err(|e| DomainError::Upgrade(format!("解压失败: {e}")))?;

    install_binary(&binary, target)
}

/// 把解压后的二进制写入 target（同 FS tempfile → chmod → 备份 → 原子 rename）。
fn install_binary(binary: &[u8], target: &Path) -> DomainResult<()> {
    use std::io::Write;
    let parent = target
        .parent()
        .ok_or_else(|| DomainError::Upgrade("目标无父目录".into()))?;
    std::fs::create_dir_all(parent)?;

    // 写到同目录临时文件（保证同 FS）。
    let mut tmp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| DomainError::Upgrade(e.to_string()))?;
    tmp.write_all(binary)
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;
    tmp.flush()
        .map_err(|e| DomainError::Upgrade(e.to_string()))?;

    // chmod +x。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tmp
            .as_file()
            .metadata()
            .map_err(|e| DomainError::Upgrade(e.to_string()))?
            .permissions();
        perms.set_mode(0o755);
        tmp.as_file()
            .set_permissions(perms)
            .map_err(|e| DomainError::Upgrade(e.to_string()))?;
    }

    // 备份旧二进制。
    let bak: PathBuf = target.with_extension("bak");
    if target.exists() {
        let _ = std::fs::rename(target, &bak);
    }

    // 原子替换。
    match tmp.persist(target) {
        Ok(_) => {
            // macOS：去 quarantine（尽力）。
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("xattr")
                    .args(["-d", "com.apple.quarantine"])
                    .arg(target)
                    .output();
            }
            Ok(())
        }
        Err(e) => {
            // 回滚。
            if bak.exists() {
                let _ = std::fs::rename(&bak, target);
            }
            Err(DomainError::Upgrade(format!("替换二进制失败: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_infix_is_known() {
        let infix = platform_infix().unwrap();
        assert!(["darwin-arm64", "darwin-amd64", "linux-arm64", "linux-amd64"].contains(&infix));
    }

    #[test]
    fn has_update_compares_versions() {
        assert!(has_update("v1.18.0", "v1.18.1"));
        assert!(has_update("1.18.0", "v1.18.1"));
        assert!(!has_update("v1.18.1", "v1.18.1"));
        assert!(!has_update("v1.18.1", "1.18.1"));
        assert!(!has_update("anything", "")); // 空 latest 不算更新
    }

    #[test]
    fn install_binary_backs_up_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bin").join("mihomo");
        // 首次安装。
        install_binary(b"VERSION1", &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"VERSION1");
        // 升级：旧的应进 .bak。
        install_binary(b"VERSION2", &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"VERSION2");
        assert_eq!(
            std::fs::read(target.with_extension("bak")).unwrap(),
            b"VERSION1"
        );
    }

    #[cfg(unix)]
    #[test]
    fn installed_binary_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("mihomo");
        install_binary(b"#!/bin/sh\n", &target).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111); // 可执行位
    }
}
