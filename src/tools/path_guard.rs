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
    // 从可执行文件位置推断项目根：<proj>/target/release/gqy 或 <proj>/target/debug/gqy
    if let Ok(exe) = std::env::current_exe() {
        if let Some(target) = exe.parent().and_then(|p| p.parent()) {
            if target.file_name().is_some_and(|n| n == "target") {
                if let Some(project) = target.parent() {
                    if project.join("Cargo.toml").is_file() {
                        return project.to_path_buf();
                    }
                }
            }
        }
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

/// path 是否位于 dir 内（支持相对路径、符号链接与尚不存在的文件或多级未创建子目录）。
fn is_inside(path: &Path, dir: &Path) -> bool {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let norm_abs = normalize_lexical(&abs);
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if norm_abs.starts_with(&dir) {
        return true;
    }
    // 目标文件/子目录可能尚不存在：沿着父目录向上寻找第一个存在的祖先目录 canonicalize 后比较
    let mut current: &Path = &norm_abs;
    while let Some(parent) = current.parent() {
        if let Ok(canon_parent) = parent.canonicalize() {
            if canon_parent.starts_with(&dir) {
                return true;
            }
            break;
        }
        current = parent;
    }
    if let Ok(canon) = norm_abs.canonicalize() {
        if canon.starts_with(&dir) {
            return true;
        }
    }
    false
}

fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            c => normalized.push(c),
        }
    }
    normalized
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
    fn detects_non_existent_nested_subdir_inside_project() {
        let dir = std::env::temp_dir().join("gqy-guard-nested-test");
        std::fs::create_dir_all(&dir).unwrap();

        let nested_non_existent = dir.join("a/b/c/d/file.txt");
        assert!(is_inside(&nested_non_existent, &dir));

        let escaped_path = dir.join("a/b/../../../../outside.txt");
        assert!(!is_inside(&escaped_path, &dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writable_outside_project_passes() {
        let temp = std::env::temp_dir().join("gqy-writable-test.txt");
        ensure_writable(&temp).unwrap();
    }

    #[test]
    fn writable_inside_project_is_rejected() {
        // 用环境变量指向一个临时项目目录，验证护栏本身（不依赖真实仓库位置）
        let temp = std::env::temp_dir().join("gqy-guard-project-test");
        std::fs::create_dir_all(&temp).unwrap();
        let guarded_file = temp.join("src/main.rs");
        std::fs::create_dir_all(guarded_file.parent().unwrap()).unwrap();
        std::fs::write(&guarded_file, "").unwrap();

        unsafe { std::env::set_var("GQY_PROJECT_DIR", &temp) };
        let result = ensure_writable(&guarded_file);
        unsafe { std::env::remove_var("GQY_PROJECT_DIR") };
        assert!(result.is_err());
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("受保护"));

        std::fs::remove_dir_all(&temp).ok();
    }
}
