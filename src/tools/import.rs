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

#[derive(Debug, Default)]
pub struct ImportResult {
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    /// 检测到的许可证 SPDX 标识（"unknown" = 未识别 / 无许可证文件）。
    pub license: Option<String>,
}

/// 仓库许可证检测结果。
#[derive(Debug, Clone)]
pub struct LicenseInfo {
    pub spdx: String,
    /// 许可宽松度：permissive（MIT/Apache/BSD 等可自由再分发）、
    /// copyleft（GPL/AGPL 等传染性）、restricted（自定义/无许可证）。
    pub kind: LicenseKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseKind {
    Permissive,
    Copyleft,
    Restricted,
}

/// 扫描仓库根目录的许可证文件并识别 SPDX 标识。
/// 兼容常见文件名：LICENSE/LICENSE.md/LICENSE.txt/COPYING/LICENCE 等。
pub fn detect_license(dir: &Path) -> Result<LicenseInfo> {
    const FILE_NAMES: &[&str] = &[
        "LICENSE",
        "LICENSE.md",
        "LICENSE.txt",
        "LICENCE",
        "LICENCE.md",
        "COPYING",
        "COPYING.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
    ];
    for name in FILE_NAMES {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        let text = match fs::read_to_string(&candidate) {
            Ok(text) => text,
            Err(_) => continue,
        };
        if let Some(spdx) = identify_license(&text) {
            let kind = if is_permissive(&spdx) {
                LicenseKind::Permissive
            } else if is_copyleft(&spdx) {
                LicenseKind::Copyleft
            } else {
                LicenseKind::Restricted
            };
            return Ok(LicenseInfo { spdx, kind });
        }
    }
    Ok(LicenseInfo {
        spdx: "unknown".to_string(),
        kind: LicenseKind::Restricted,
    })
}

fn identify_license(text: &str) -> Option<String> {
    let head = &text[..text.len().min(2000)];
    if head.contains("GNU AFFERO") {
        return Some(if head.contains("Version 3") { "AGPL-3.0" } else { "AGPL-3.0" }.into());
    }
    if head.contains("GNU GENERAL PUBLIC LICENSE") {
        if head.contains("Version 3") {
            return Some("GPL-3.0".into());
        }
        if head.contains("Version 2") {
            return Some("GPL-2.0".into());
        }
        return Some("GPL-3.0".into());
    }
    if head.contains("GNU LESSER GENERAL PUBLIC LICENSE") || head.contains("GNU LIBRARY GENERAL PUBLIC LICENSE") {
        return Some("LGPL-3.0".into());
    }
    if head.contains("MOZILLA PUBLIC LICENSE") {
        return Some("MPL-2.0".into());
    }
    if head.contains("Apache License") && head.contains("Version 2.0") {
        return Some("Apache-2.0".into());
    }
    if head.contains("MIT License") || head.contains("Permission is hereby granted, free of charge")
    {
        return Some("MIT".into());
    }
    if head.contains("BSD 3-Clause") || head.contains("Redistribution and use in source and binary forms")
    {
        return Some("BSD-3-Clause".into());
    }
    if head.contains("BSD 2-Clause") {
        return Some("BSD-2-Clause".into());
    }
    if head.contains("ISC License") || head.contains("Permission to use, copy, modify")
    {
        return Some("ISC".into());
    }
    if head.contains("This is free and unencumbered software") {
        return Some("Unlicense".into());
    }
    if head.contains("CC0 1.0") {
        return Some("CC0-1.0".into());
    }
    None
}

/// 宽松许可：允许自由再分发（兼容 GPL-3.0 项目）。
fn is_permissive(spdx: &str) -> bool {
    matches!(
        spdx,
        "MIT" | "Apache-2.0" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC" | "Unlicense" | "CC0-1.0" | "MPL-2.0"
    )
}

/// 传染性许可：再分发需保持同一许可证（GPL-3.0 兼容 GPL-3.0/AGPL-3.0，但与 GPL-2.0 不兼容）。
fn is_copyleft(spdx: &str) -> bool {
    matches!(spdx, "GPL-2.0" | "GPL-3.0" | "AGPL-3.0" | "LGPL-3.0")
}

/// 解析并准备仓库/目录（git URL 克隆到 workspace，本地路径规范化）。
fn resolve_source(source: &str) -> Result<PathBuf> {
    let workspace = crate::tools::path_guard::workspace_dir().join("tool-imports");
    if is_git_url(source) {
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
        return Ok(target);
    } else {
        let path = PathBuf::from(source);
        if !path.is_dir() {
            bail!("工具目录不存在：{source}");
        }
        return Ok(path.canonicalize()?);
    }
}

/// 先理解再导入：列出候选可执行脚本与头部摘要，供 GQY/用户判断核心功能。
/// 只读仓库头部几行，不导入任何东西。
pub fn inspect_source(source: &str) -> Result<Vec<(String, String)>> {
    let dir = resolve_source(source)?;
    let dir = dir.canonicalize().unwrap_or(dir);
    let entries = auto_scan(&dir)?;
    let mut result = Vec::new();
    for entry in entries {
        let path = dir.join(&entry.path);
        let header = read_header_lines(&path, 3);
        result.push((entry.path, header));
    }
    Ok(result)
}

fn read_header_lines(path: &Path, limit: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    text.lines()
        .take(limit)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
        .chars()
        .take(160)
        .collect()
}

/// 导入工具包：source 为本地目录或 Git 仓库 URL（https/git@）。
/// `only` 非空时只导入指定的候选（GQY 理解项目后挑核心功能）。
pub fn import_tools(
    paths: &GqyPaths,
    source: &str,
    name: Option<&str>,
    only: Option<&[String]>,
) -> Result<ImportResult> {
    let dir = resolve_source(source)?;

    // 统一解析真实路径：/tmp 在 macOS 上是 /private/tmp 的符号链接，
    // 不 canonicalize 会导致后续 starts_with 越界误判
    let dir = dir.canonicalize().unwrap_or(dir);

    // 先理解许可证：识别仓库 LICENSE，随包保留来源（GPL 合规），
    // restricted（自定义/无许可证）时警告但不阻止
    let license = detect_license(&dir)?;
    if license.kind == LicenseKind::Restricted && license.spdx != "unknown" {
        eprintln!(
            "⚠ 许可证未识别（{spdx}），请确认来源可自由再分发后再使用",
            spdx = license.spdx
        );
    } else if license.spdx == "unknown" {
        eprintln!("⚠ 仓库没有 LICENSE 文件（默认为「保留所有权利」），导入仅供个人使用");
    } else if license.kind == LicenseKind::Copyleft {
        eprintln!(
            "ℹ 许可证 {spdx} 为传染性许可：随工具包分发需保持同一许可证（GQY 本体 GPL-3.0）",
            spdx = license.spdx
        );
    }

    let mut entries = load_entries(&dir)?;
    if let Some(only) = only {
        // 先理解再导入：只保留指定候选（按文件名/相对路径匹配）
        let wanted: Vec<&str> = only.iter().map(String::as_str).collect();
        entries.retain(|entry| {
            let file_name = Path::new(&entry.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            wanted.iter().any(|w| *w == entry.path || *w == file_name)
        });
        if entries.is_empty() {
            bail!("--only 指定的候选都不存在（先 gqy tools inspect 查看候选）");
        }
    }
    if entries.is_empty() {
        bail!(
            "{} 里没有找到工具（需要 gqy-tools.json/manifest.json 清单，或可执行文件）",
            dir.display()
        );
    }

    // 校验并复制到用户脚本目录（config/scripts/<包名>/<文件>），
    // 清单合并写入 config/scripts/index.json（扫描只认根目录清单）
    let package_name = sanitize_name(name.unwrap_or(
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("imported"),
    ));
    let scripts_root = paths.config_dir.join("scripts");
    let package_dir = scripts_root.join(&package_name);
    fs::create_dir_all(&package_dir)?;

    // 许可证随包保留：复制 LICENSE 到包目录（GPL/MIT 都要求保留来源与版权声明）
    if license.spdx != "unknown" {
        if let Some(license_file) = find_license_file(&dir) {
            let _ = fs::copy(&license_file, package_dir.join("LICENSE"));
        }
    }
    let mut installed = Vec::new();
    let mut new_entries = Vec::new();
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
        let dest = package_dir.join(file_name);
        fs::copy(&canonical, &dest)?;
        let mut permissions = fs::metadata(&dest)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
        fs::set_permissions(&dest, permissions)?;

        // id 规范化：脚本文件名不能直接当工具 id（须字母开头、无点号）
        let tool_id = normalize_tool_id(file_name);
        let mut installed_entry = entry.clone();
        installed_entry.id = tool_id.clone();
        installed_entry.path = format!("{package_name}/{file_name}");
        new_entries.push(installed_entry);
        installed.push(tool_id);
    }

    // 合并已有根清单（用户原有脚本保留）
    let index_path = scripts_root.join("index.json");
    let mut merged: Vec<Value> = Vec::new();
    if let Ok(text) = fs::read_to_string(&index_path) {
        if let Ok(existing) = serde_json::from_str::<Manifest>(&text) {
            merged = existing
                .scripts
                .into_iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or_default())
                .collect();
        }
    }
    for entry in new_entries {
        merged.push(serde_json::to_value(entry)?);
    }
    let manifest = json!({ "scripts": merged });
    fs::write(index_path, serde_json::to_string_pretty(&manifest)?)?;

    // 识别仓库的 GQY Skill 结构（skills/<name>/SKILL.md），一并导入
    // 到标准 skills 目录，load_skill 即可加载，长期可用
    let mut imported_skills = Vec::new();
    let skills_root = dir.join("skills");
    if skills_root.is_dir() {
        for entry in fs::read_dir(&skills_root)? {
            let entry = entry?;
            let skill_dir = entry.path();
            if !skill_dir.is_dir() || !skill_dir.join("SKILL.md").is_file() {
                continue;
            }
            let skill_name = entry.file_name().to_string_lossy().to_string();
            let target = paths.skills_dir.join(&skill_name);
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            copy_dir(&skill_dir, &target)?;
            imported_skills.push(skill_name);
        }
    }

    Ok(ImportResult {
        tools: installed,
        skills: imported_skills,
        license: (license.spdx != "unknown").then_some(license.spdx),
    })
}

/// 返回仓库根目录的许可证文件路径（与 detect_license 的文件名清单一致）。
fn find_license_file(dir: &Path) -> Option<PathBuf> {
    const FILE_NAMES: &[&str] = &[
        "LICENSE",
        "LICENSE.md",
        "LICENSE.txt",
        "LICENCE",
        "LICENCE.md",
        "COPYING",
        "COPYING.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
    ];
    FILE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// 把文件名规范化为合法工具 id：字母开头，非法字符转下划线，去扩展名。
fn normalize_tool_id(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let mut id = String::new();
    for (index, ch) in stem.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            id.push(ch);
        } else {
            id.push('_');
        }
        if index == 0 && !ch.is_ascii_alphabetic() {
            id.insert(0, 't');
        }
    }
    if id.is_empty() {
        "tool".to_string()
    } else {
        id
    }
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if entry.file_name() != ".git" {
                copy_dir(&from, &to)?;
            }
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 列出已导入的用户工具包（按根清单的 path 前缀分组），
/// 附带包内 LICENSE 文件识别的许可证（无则 "unknown"）。
pub fn list_tools(paths: &GqyPaths) -> Result<Vec<(String, usize, String)>> {
    let base = paths.config_dir.join("scripts");
    let mut result = Vec::new();
    let index = base.join("index.json");
    if !index.is_file() {
        return Ok(result);
    }
    let text = fs::read_to_string(&index)?;
    let manifest: Manifest = serde_json::from_str(&text)?;
    let mut packages: std::collections::BTreeMap<String, (usize, String)> =
        std::collections::BTreeMap::new();
    for script in manifest.scripts {
        let package = script
            .path
            .split('/')
            .next()
            .unwrap_or("default")
            .to_string();
        let entry = packages.entry(package.clone()).or_insert_with(|| {
            let license = find_license_file(&base.join(&package))
                .and_then(|path| fs::read_to_string(path).ok())
                .and_then(|text| identify_license(&text))
                .unwrap_or_else(|| "unknown".to_string());
            (0, license)
        });
        entry.0 += 1;
    }
    for (name, (count, license)) in packages {
        result.push((name, count, license));
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
            let description = read_description(&path);
            // GQY 要求工具描述非空（否则注册被拒）；没有 Description 注释时
            // 生成带文件名的默认描述，保证工具可用
            let description = if description.is_empty() {
                format!(
                    "Execute the bundled script {} from the imported package. Pass arguments as JSON via stdin; check the script source for exact usage.",
                    id
                )
            } else {
                description
            };
            entries.push(ScriptEntry {
                description,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_license(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn detects_mit_license() {
        let dir = tempfile::tempdir().unwrap();
        write_license(dir.path(), "LICENSE", "MIT License\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software...");
        let info = detect_license(dir.path()).unwrap();
        assert_eq!(info.spdx, "MIT");
        assert_eq!(info.kind, LicenseKind::Permissive);
    }

    #[test]
    fn detects_gpl3_as_copyleft() {
        let dir = tempfile::tempdir().unwrap();
        write_license(dir.path(), "COPYING", "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007");
        let info = detect_license(dir.path()).unwrap();
        assert_eq!(info.spdx, "GPL-3.0");
        assert_eq!(info.kind, LicenseKind::Copyleft);
    }

    #[test]
    fn detects_apache_license_md() {
        let dir = tempfile::tempdir().unwrap();
        write_license(dir.path(), "LICENSE.md", "Apache License\nVersion 2.0, January 2004");
        let info = detect_license(dir.path()).unwrap();
        assert_eq!(info.spdx, "Apache-2.0");
    }

    #[test]
    fn no_license_file_is_restricted_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let info = detect_license(dir.path()).unwrap();
        assert_eq!(info.spdx, "unknown");
        assert_eq!(info.kind, LicenseKind::Restricted);
    }

    #[test]
    fn license_copied_into_package_dir_on_import() {
        let root = tempfile::tempdir().unwrap().into_path();
        let _env_guard = crate::paths::test_env::GQY_HOME_LOCK.lock().unwrap();
        let old = std::env::var_os("GQY_HOME");
        std::env::set_var("GQY_HOME", &root);
        let paths = crate::paths::GqyPaths::new().unwrap();

        // 模拟一个带 LICENSE 的本地工具目录
        let src = root.join("src-repo");
        std::fs::create_dir_all(&src).unwrap();
        write_license(&src, "LICENSE", "MIT License\nPermission is hereby granted, free of charge...");
        std::fs::write(src.join("hello.sh"), "#!/bin/sh\necho hello\n").unwrap();
        std::fs::write(
            src.join("gqy-tools.json"),
            r#"{"scripts":[{"id":"hello","description":"say hello","path":"hello.sh"}]}"#,
        )
        .unwrap();

        let result = import_tools(&paths, src.to_str().unwrap(), Some("testpkg"), None).unwrap();
        assert_eq!(result.license.as_deref(), Some("MIT"));
        let pkg_dir = paths.config_dir.join("scripts/testpkg");
        assert!(pkg_dir.join("LICENSE").is_file(), "LICENSE should be copied into package dir");

        // 列表应显示许可证
        let listed = list_tools(&paths).unwrap();
        assert!(listed.iter().any(|(name, _, license)| name == "testpkg" && license == "MIT"));

        if let Some(v) = old {
            std::env::set_var("GQY_HOME", v);
        } else {
            std::env::remove_var("GQY_HOME");
        }
    }
}
