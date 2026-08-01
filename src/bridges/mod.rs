//! 消息平台桥接管理（gqy napcat / gqy tg）。
//!
//! 把 communication/ 里的桥接从「手工部署」变成 CLI 可管理：
//! - 配置统一存 GQY_HOME/config/bridges.json（token/self_id/ws 等）
//! - LaunchAgent 自启动（install/uninstall/status，KeepAlive 托管）
//! - napcat 本体支持自动下载安装（从官方 GitHub Release 获取）
//!
//! 桥接脚本来源：brew 安装时为 share/gqy/bridges/，源码开发时为仓库 communication/。

pub mod napcat;
pub mod tg;

use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const BRIDGES_FILE: &str = "bridges.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub napcat: Option<NapcatConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tg: Option<TgConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NapcatConfig {
    #[serde(default = "default_ws_url")]
    pub ws_url: String,
    #[serde(default)]
    pub self_id: String,
    #[serde(default)]
    pub bin: String,
    #[serde(default)]
    pub install_dir: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgConfig {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub bin: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_ws_url() -> String {
    "ws://127.0.0.1:3001".to_string()
}

fn default_enabled() -> bool {
    true
}

pub fn bridges_file(paths: &GqyPaths) -> PathBuf {
    paths.config_dir.join(BRIDGES_FILE)
}

pub fn load(paths: &GqyPaths) -> Result<BridgesConfig> {
    let file = bridges_file(paths);
    if !file.exists() {
        return Ok(BridgesConfig::default());
    }
    let text = std::fs::read_to_string(&file)?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))
}

pub fn save(paths: &GqyPaths, config: &BridgesConfig) -> Result<()> {
    std::fs::create_dir_all(&paths.config_dir)?;
    let file = bridges_file(paths);
    let temp = tempfile::NamedTempFile::new_in(&paths.config_dir)?;
    std::fs::write(temp.path(), serde_json::to_string_pretty(config)?)?;
    temp.persist(file)?;
    Ok(())
}

/// 桥接脚本目录：brew 安装 = share/gqy/bridges；源码开发 = 仓库 communication/。
pub fn bridges_dir(paths: &GqyPaths) -> PathBuf {
    let share_bridges = paths.share_dir.join("bridges");
    if share_bridges.join("napcat/bridge.cjs").is_file() {
        return share_bridges;
    }
    // 源码树：share_dir 在源码模式下指向仓库根
    let source_bridges = paths.share_dir.join("communication");
    if source_bridges.join("napcat/bridge.cjs").is_file() {
        return source_bridges;
    }
    share_bridges
}

pub fn bridge_script(paths: &GqyPaths, platform: &str) -> Result<PathBuf> {
    let path = bridges_dir(paths).join(platform).join("bridge.cjs");
    if !path.is_file() {
        bail!(
            "找不到桥接脚本 {}（brew 安装请确认随包文件，或从源码克隆 communication/）",
            path.display()
        );
    }
    Ok(path)
}

pub fn node_bin() -> String {
    std::env::var("GQY_NODE_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/opt/homebrew/bin/node".to_string())
}

// ─────────────────────────── LaunchAgent 管理 ───────────────────────────

pub fn launchctl_target() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

pub fn launchctl_status(label: &str) -> Result<bool> {
    let output = Command::new("/bin/launchctl")
        .args(["print", &format!("{}/{}", launchctl_target(), label)])
        .output()
        .context("running launchctl print")?;
    Ok(output.status.success())
}

pub fn launchctl_load(plist: &Path) -> Result<()> {
    let output = Command::new("/bin/launchctl")
        .args(["bootstrap", &launchctl_target(), plist.to_str().unwrap_or_default()])
        .output()
        .context("running launchctl bootstrap")?;
    if !output.status.success() {
        // 已加载时 bootstrap 会报错，视为成功
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("service already loaded") || stderr.contains("already bootstrapped") {
            return Ok(());
        }
        bail!("launchctl bootstrap 失败: {}", stderr.trim());
    }
    Ok(())
}

pub fn launchctl_unload(label: &str) -> Result<()> {
    let output = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{}/{}", launchctl_target(), label)])
        .output()
        .context("running launchctl bootout")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Could not find service") || stderr.contains("No such process") {
            return Ok(());
        }
        bail!("launchctl bootout 失败: {}", stderr.trim());
    }
    Ok(())
}

pub fn write_plist(path: &Path, plist: &serde_json::Value) -> Result<()> {
    let xml = plist_to_xml(plist).context("serializing LaunchAgent plist")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, xml)?;
    Ok(())
}

/// 把 JSON 结构转成 LaunchAgent XML plist（只支持本模块用到的类型）。
pub fn plist_to_xml(value: &serde_json::Value) -> Result<String> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n",
    );
    append_plist_value(&mut out, value, 1)?;
    out.push_str("</plist>\n");
    Ok(out)
}

fn append_plist_value(out: &mut String, value: &serde_json::Value, depth: usize) -> Result<()> {
    let indent = "    ".repeat(depth);
    match value {
        serde_json::Value::Object(map) => {
            out.push_str(&format!("{indent}<dict>\n"));
            for (key, value) in map {
                out.push_str(&format!("{}{indent}<key>{}</key>\n", "    ", key));
                append_plist_value(out, value, depth + 1)?;
            }
            out.push_str(&format!("{indent}</dict>\n"));
        }
        serde_json::Value::Array(items) => {
            out.push_str(&format!("{indent}<array>\n"));
            for item in items {
                append_plist_value(out, item, depth + 1)?;
            }
            out.push_str(&format!("{indent}</array>\n"));
        }
        serde_json::Value::String(text) => {
            out.push_str(&format!("{indent}<string>{}</string>\n", xml_escape(text)));
        }
        serde_json::Value::Bool(flag) => {
            out.push_str(&format!(
                "{indent}<{} />\n",
                if *flag { "true" } else { "false" }
            ));
        }
        serde_json::Value::Number(number) => {
            out.push_str(&format!("{indent}<integer>{number}</integer>\n"));
        }
        _ => bail!("unsupported plist value: {value}"),
    }
    Ok(())
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 平台标签 -> 日志文件（放 GQY_HOME/cache/logs/ 下，随备份）。
pub fn bridge_log_path(paths: &GqyPaths, platform: &str) -> PathBuf {
    paths.logs_dir().join(format!("{platform}-bridge.log"))
}
