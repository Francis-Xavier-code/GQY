//! 地盘护栏：代码级约束，防止 GQY 把文件写进项目源码目录等受保护区。
//!
//! 提示词里的「工作纪律」是软约束；这里是硬约束，任何写文件工具
//! （write_file / edit_string / apply_patch）在落盘前都会经过本模块检查。
//!
//! 环境变量：
//! - `GQY_PROJECT_DIR`：项目源码目录（默认 `~/GQY`）
//! - `GQY_WORKSPACE`：她的临时工作区（默认 `~/gqy-workspace`）
//! - `GQY_ALLOW_PROJECT_WRITES=1`：开发模式，放行项目目录写入（主人专用）

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub fn project_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GQY_PROJECT_DIR") {
        return PathBuf::from(dir);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join("GQY"))
        .unwrap_or_else(|| PathBuf::from("/Users/Shared/GQY"))
}

/// 她的临时工作区：下载、解压、草稿等临时文件放这里。
pub fn workspace_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GQY_WORKSPACE") {
        return PathBuf::from(dir);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join("gqy-workspace"))
        .unwrap_or_else(|| PathBuf::from("/tmp/gqy-workspace"))
}

/// 写文件前的护栏：目标在项目源码目录内且未显式放行时拒绝。
pub fn ensure_writable(path: &Path) -> Result<()> {
    if !is_inside(path, &project_dir()) {
        return Ok(());
    }
    if std::env::var_os("GQY_ALLOW_PROJECT_WRITES").is_some() {
        return Ok(());
    }
    bail!(
        "路径位于项目源码目录（{}）内，受保护。\
         下载/临时文件请放到 {}；如需修改项目本身，请设置 GQY_ALLOW_PROJECT_WRITES=1",
        project_dir().display(),
        workspace_dir().display()
    )
}

/// path 是否位于 dir 内（支持相对路径、符号链接与尚不存在的文件）。
fn is_inside(path: &Path, dir: &Path) -> bool {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    // 目录先 canonicalize（处理 /var -> /private/var 等符号链接）
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if abs.starts_with(&dir) {
        return true;
    }
    // 目标文件可能不存在：沿父目录 canonicalize 后比较
    if let Some(parent) = abs.parent() {
        if let Ok(parent) = parent.canonicalize() {
            if parent.starts_with(&dir) {
                return true;
            }
        }
    }
    if let Ok(abs) = abs.canonicalize() {
        if abs.starts_with(&dir) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_project_inside_paths() {
        let dir = std::env::temp_dir().join("gqy-guard-test");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        assert!(is_inside(&dir.join("src/main.rs"), &dir));
        assert!(is_inside(&dir.join("Cargo.toml"), &dir));
        assert!(!is_inside(&std::env::temp_dir().join("other.txt"), &dir));
        // 兄弟目录（../outside.rs）不算 inside
        assert!(!is_inside(&dir.parent().unwrap().join("gqy-outside.txt"), &dir));
    }

    #[test]
    fn writable_outside_project_passes() {
        let temp = std::env::temp_dir().join("gqy-writable-test.txt");
        ensure_writable(&temp).unwrap();
    }
}
