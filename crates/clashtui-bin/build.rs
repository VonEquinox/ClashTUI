//! 在编译期把版本号（含 git 短哈希与脏标记）嵌入二进制。
//! 容忍非 git 检出（如 crates.io 打包）。

use std::process::Command;

fn main() {
    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());

    let git = git_describe();
    let version = match git {
        Some(g) => format!("{pkg}-{g}"),
        None => pkg,
    };
    println!("cargo:rustc-env=CLASHTUI_VERSION={version}");

    // git HEAD 变化时重跑（若在 git 仓库内）。
    if let Some(git_dir) = find_git_dir() {
        println!("cargo:rerun-if-changed={}/HEAD", git_dir.display());
    }
}

/// 返回 "<short>" 或 "<short>-dirty"，非 git 环境返回 None。
fn git_describe() -> Option<String> {
    let short = run(&["rev-parse", "--short", "HEAD"])?;
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    Some(if dirty {
        format!("{short}-dirty")
    } else {
        short
    })
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn find_git_dir() -> Option<std::path::PathBuf> {
    let out = run(&["rev-parse", "--git-dir"])?;
    Some(std::path::PathBuf::from(out))
}
