//! Embedded mihomo bootstrap.

use std::io::Write;
use std::path::Path;

use clashtui_domain::Paths;

const EMBEDDED_MIHOMO: &[u8] = include_bytes!(env!("CLASHTUI_EMBEDDED_MIHOMO_PATH"));

pub fn install_default_if_missing(paths: &Paths) -> color_eyre::Result<()> {
    if EMBEDDED_MIHOMO.is_empty() {
        return Ok(());
    }

    let target = paths.default_binary();
    if target.exists() {
        return Ok(());
    }

    install_binary(EMBEDDED_MIHOMO, &target)?;
    Ok(())
}

fn install_binary(binary: &[u8], target: &Path) -> color_eyre::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| color_eyre::eyre::eyre!("mihomo target has no parent directory"))?;
    std::fs::create_dir_all(parent)?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(binary)?;
    tmp.flush()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tmp.as_file().metadata()?.permissions();
        perms.set_mode(0o755);
        tmp.as_file().set_permissions(perms)?;
    }

    tmp.persist(target)
        .map_err(|e| color_eyre::eyre::eyre!("install embedded mihomo failed: {e}"))?;

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(target)
            .output();
    }

    Ok(())
}
