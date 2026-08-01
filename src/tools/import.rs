//! 工具包导入：把一个目录/Git 仓库转换成 GQY 可长期使用的脚本工具。
//!
//! 标准（见 docs/02-设计/tool-package-standard.md）：
//! - 仓库根放 `gqy-tools.json`（或 manifest.json / index.json）声明工具清单，
//!   格式与 GQY 脚本工具一致：{ "scripts": [ { id, display_name, description,
//!   path, parameters, timeout_seconds, always_loaded, load_policy, groups } ] }
//! - 没有清单时自动扫描可执行文件，描述取文件头 `Description:` 注释
//!
//! 导入后写入 `GQY_HOME/config/scripts/<name>/`，随每轮对话自动扫描注册，
//! 长期可用；随备份快照，换机恢复后依然在。

use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScriptEntry {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    path: String,
    #[serde(default)]
    parameters: Value,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    always_loaded: bool,
    #[serde(default)]
    load_policy: String,
    #[serde(default)]
    groups: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    #[serde(default)]
    scripts: Vec<ScriptEntry>,
}

/// 导入工具包：source 为本地目录或 Git 仓库 URL（https/git@）。
pub fn import_tools(paths: &GqyPaths, source: &str, name: Option<&str>) -> Result<Vec<String>> {
    let workspace = crate::tools::path_guard::workspace_dir().join("tool-imports");
    let dir = if is_git_url(source) {
        let dir_name = source
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .unwrap_or("repo")
            .trim_end_matches(".git");
        let target = workspace.join(sanitize_name(dir_name));
        if target.join(".git").is_dir() {
            // 已有克隆：拉取更新
            let status = Command::new("git")
                .args(["-C", target.to_str().unwrap_or_default(), "pull", "--ff-only"])
                .status()
                .context("pulling tool repository")?;
            if !status.success() {
                bail!("git pull 失败：{}", target.display());
            }
        } else {
            fs::create_dir_all(&workspace)?;
            let status = Command::new("git")
                .args(["clone", "--depth", "1", source, target.to_str().unwrap_or_default()])
                .status()
                .context("cloning tool repository")?;
            if !status.success() {
                bail!("git clone 失败：{source}");
            }
        }
        target
    } else {
        let path = PathBuf::from(source);
        if !path.is_dir() {
            bail!("工具目录不存在：{source}");
        }
        path.canonicalize()?
    };

    let entries = load_entries(&dir)?;
    if entries.is_empty() {
        bail!(
            "{} 里没有找到工具（需要 gqy-tools.json/manifest.json 清单，或可执行文件）",
            dir.display()
        );
    }

    // 校验并复制到用户脚本目录
    let package_name = sanitize_name(name.unwrap_or(
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("imported"),
    ));
    let target_dir = paths.config_dir.join("scripts").join(&package_name);
    fs::create_dir_all(&target_dir)?;
    let mut installed = Vec::new();
    let mut scripts = Vec::new();
    for entry in &entries {
        let source_path = dir.join(&entry.path);
        // 路径穿越防护
        let canonical = source_path.canonicalize().with_context(|| {
            format!("工具文件不存在：{}", source_path.display())
        })?;
        if !canonical.starts_with(&dir) {
            bail!("工具路径越界：{}", entry.path);
        }
        let file_name = Path::new(&entry.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&entry.id);
        let dest = target_dir.join(file_name);
        fs::copy(&canonical, &dest)?;
        let mut permissions = fs::metadata(&dest)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
        fs::set_permissions(&dest, permissions)?;

        let mut installed_entry = entry.clone();
        installed_entry.path = file_name.to_string();
        scripts.push(installed_entry);
        installed.push(entry.id.clone());
    }
    let manifest = json!({ "scripts": scripts });
    fs::write(
        target_dir.join("index.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(installed)
}

/// 列出已导入的用户工具包。
pub fn list_tools(paths: &GqyPaths) -> Result<Vec<(String, usize)>> {
    let base = paths.config_dir.join("scripts");
    let mut result = Vec::new();
    if !base.is_dir() {
        return Ok(result);
    }
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let index = entry.path().join("index.json");
        if index.is_file() {
            if let Ok(text) = fs::read_to_string(&index) {
                if let Ok(manifest) = serde_json::from_str::<Manifest>(&text) {
                    result.push((
                        entry.file_name().to_string_lossy().to_string(),
                        manifest.scripts.len(),
                    ));
                }
            }
        }
    }
    Ok(result)
}

fn load_entries(dir: &Path) -> Result<Vec<ScriptEntry>> {
    for manifest_name in ["gqy-tools.json", "manifest.json", "index.json"] {
        let manifest_path = dir.join(manifest_name);
        if !manifest_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("解析 {} 失败", manifest_path.display()))?;
        if !manifest.scripts.is_empty() {
            return Ok(manifest.scripts);
        }
    }
    auto_scan(dir)
}

/// 自动扫描：可执行文件（有执行位）即工具；描述取文件头 `Description:` 注释。
fn auto_scan(dir: &Path) -> Result<Vec<ScriptEntry>> {
    let mut entries = Vec::new();
    let mut directories = vec![dir.to_path_buf()];
    let mut visited = 0usize;
    while let Some(current) = directories.pop() {
        if visited > 200 {
            break;
        }
        visited += 1;
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                if !path.file_name().is_some_and(|n| n == ".git") {
                    directories.push(path);
                }
                continue;
            }
            if !file_type.is_file() || !is_executable(&path) {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if id.starts_with('.') || id == "index.json" {
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            entries.push(ScriptEntry {
                description: read_description(&path),
                path: relative,
                id,
                ..ScriptEntry::default()
            });
        }
    }
    Ok(entries)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// 读文件头几行找 `Description:` 注释。
fn read_description(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    for line in text.lines().take(30) {
        let trimmed = line.trim_start_matches(['#', '/', ';', ' ', '\t']);
        if let Some(rest) = trimmed
            .strip_prefix("Description:")
            .or_else(|| trimmed.strip_prefix("description:"))
        {
            return rest.trim().to_string();
        }
    }
    String::new()
}

fn is_git_url(source: &str) -> bool {
    source.starts_with("https://") || source.starts_with("git@") || source.starts_with("git://")
}

fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "imported".to_string()
    } else {
        trimmed
    }
}

impl Default for ScriptEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            description: String::new(),
            path: String::new(),
            parameters: json!({}),
            timeout_seconds: None,
            always_loaded: false,
            load_policy: "group".to_string(),
            groups: Vec::new(),
        }
    }
}
