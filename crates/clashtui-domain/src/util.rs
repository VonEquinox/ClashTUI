//! 通用工具：原子写文件。

use std::io::Write;
use std::path::Path;

use crate::error::{DomainError, DomainResult};

/// 原子写：先写到同目录临时文件，再 rename 覆盖目标。
/// 保证读者要么看到旧内容、要么看到完整新内容，不会读到半截。
pub fn atomic_write(path: &Path, contents: &[u8]) -> DomainResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DomainError::Config(format!("路径无父目录: {}", path.display())))?;
    std::fs::create_dir_all(parent)?;

    // 临时文件必须与目标同文件系统，故放在同目录。
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.flush()?;
    // persist 做 rename；失败时返回内部 io error。
    tmp.persist(path).map_err(|e| DomainError::Io(e.error))?;
    Ok(())
}

/// 当前进程是否具有 root/管理员权限（TUN 需要内核侧持有，此处用于给出警告）。
#[cfg(unix)]
pub fn is_elevated() -> bool {
    nix::unistd::geteuid().is_root()
}

/// 非 Unix 平台保守返回 false。
#[cfg(not(unix))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sub").join("x.txt");
        atomic_write(&f, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello");
        atomic_write(&f, b"world").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "world");
    }
}
