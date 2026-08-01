//! `gqy tg`：Telegram 桥接管理。
//!
//! 子命令：
//! - `status`：查看配置与自启动状态
//! - `install`：安装桥接 LaunchAgent（自启动，KeepAlive 托管）
//! - `uninstall`：移除自启动（不删配置与数据）
//! - `token <token>`：设置 Bot Token（@BotFather 获取）
//! - `config <key> <value>`：设置 owner_id / bin / enabled
//!
//! Telegram 桥接只依赖 node 与网络（bot 在云端），不需要额外安装任何客户端。

use super::TgConfig;
use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use std::path::{Path, PathBuf};

const TG_LABEL: &str = "com.gqy.napcat-tg";

#[derive(Debug, Args)]
pub struct TgArgs {
    #[command(subcommand)]
    pub command: Option<TgCommand>,
}

#[derive(Debug, Subcommand)]
pub enum TgCommand {
    Status,
    Install,
    Uninstall,
    /// 设置 Telegram Bot Token（@BotFather 获取）
    Token { token: String },
    /// 设置配置项：owner_id / bin / enabled
    Config { key: String, value: String },
}

pub async fn run(paths: &GqyPaths, args: TgArgs) -> Result<()> {
    match args.command.unwrap_or(TgCommand::Status) {
        TgCommand::Status => run_status(paths),
        TgCommand::Install => run_install(paths),
        TgCommand::Uninstall => run_uninstall(paths),
        TgCommand::Token { token } => run_config(paths, "token", &token),
        TgCommand::Config { key, value } => run_config(paths, &key, &value),
    }
}

fn config_for(paths: &GqyPaths) -> Result<TgConfig> {
    let mut bridges = super::load(paths)?;
    let mut config = bridges.tg.clone().unwrap_or(TgConfig {
        token: String::new(),
        owner_id: String::new(),
        bin: super::node_bin(),
        enabled: true,
    });
    if config.bin.is_empty() {
        config.bin = super::node_bin();
    }
    bridges.tg = Some(config.clone());
    super::save(paths, &bridges)?;
    Ok(config)
}

fn run_status(paths: &GqyPaths) -> Result<()> {
    let bridges = super::load(paths)?;
    let config = bridges.tg.unwrap_or_else(|| TgConfig {
        token: String::new(),
        owner_id: String::new(),
        bin: super::node_bin(),
        enabled: true,
    });
    println!("Telegram 桥接配置:");
    let token_display = if config.token.is_empty() {
        "(未设置 —— 需要 gqy tg token <token>)".to_string()
    } else {
        let preview: String = config.token.chars().take(8).collect();
        format!("已配置（{preview}…）")
    };
    println!("  token:     {token_display}");
    println!(
        "  owner_id:  {}",
        if config.owner_id.is_empty() {
            "(未设置 —— 私聊按用户隔离，主人看不到主上下文)"
        } else {
            &config.owner_id
        }
    );
    println!("  node:      {}", config.bin);
    println!("  启用:      {}", if config.enabled { "是" } else { "否" });
    println!("  桥接脚本:  {}", super::bridge_script(paths, "tg")?.display());
    println!(
        "  自启动:    {}",
        if super::launchctl_status(TG_LABEL)? { "运行中" } else { "未安装" }
    );
    let log = super::bridge_log_path(paths, "tg");
    if log.exists() {
        let last = std::fs::read_to_string(&log)
            .ok()
            .and_then(|text| text.lines().rev().next().map(str::to_string))
            .unwrap_or_default();
        println!("  最近日志:  {}", last.chars().take(120).collect::<String>());
    }
    Ok(())
}

fn run_install(paths: &GqyPaths) -> Result<()> {
    let config = config_for(paths)?;
    if config.token.is_empty() {
        println!("⚠ token 未设置：先 gqy tg token <token>（@BotFather 获取）再安装。");
    }
    let script = super::bridge_script(paths, "tg")?;
    let plist = bridge_plist(paths, &config, &script)?;
    let plist_path = launch_agents_dir()?.join(format!("{TG_LABEL}.plist"));
    super::write_plist(&plist_path, &plist)?;
    if config.enabled {
        super::launchctl_unload(TG_LABEL).ok();
        super::launchctl_load(&plist_path)?;
        println!("✅ Telegram 桥接自启动已加载（KeepAlive 托管）");
    }
    println!("查看状态：gqy tg status");
    Ok(())
}

fn run_uninstall(paths: &GqyPaths) -> Result<()> {
    let _ = paths;
    super::launchctl_unload(TG_LABEL)?;
    let plist = launch_agents_dir()?.join(format!("{TG_LABEL}.plist"));
    if plist.exists() {
        std::fs::remove_file(&plist)?;
    }
    println!("✅ 已移除 Telegram 桥接自启动（配置与数据保留）");
    Ok(())
}

fn run_config(paths: &GqyPaths, key: &str, value: &str) -> Result<()> {
    let mut bridges = super::load(paths)?;
    let mut config = bridges.tg.clone().unwrap_or(TgConfig {
        token: String::new(),
        owner_id: String::new(),
        bin: super::node_bin(),
        enabled: true,
    });
    match key {
        "token" => config.token = value.to_string(),
        "owner_id" => config.owner_id = value.to_string(),
        "bin" => config.bin = value.to_string(),
        "enabled" => {
            config.enabled = value == "true" || value == "1" || value == "yes";
        }
        _ => bail!("未知配置项 {key}（支持：token / owner_id / bin / enabled）"),
    }
    bridges.tg = Some(config.clone());
    super::save(paths, &bridges)?;
    println!("tg.{key} = {}", if key == "token" { "(已设置)" } else { value });
    Ok(())
}

fn bridge_plist(paths: &GqyPaths, config: &TgConfig, script: &Path) -> Result<serde_json::Value> {
    let log = super::bridge_log_path(paths, "tg");
    Ok(json!({
        "Label": TG_LABEL,
        "ProgramArguments": [
            config.bin,
            script.to_str().unwrap_or_default(),
        ],
        "EnvironmentVariables": {
            "HOME": env_home().to_str().unwrap_or_default(),
            "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "GQY_TG_TOKEN": config.token,
            "GQY_TG_OWNER_ID": config.owner_id,
            "GQY_BRIDGE_LOG": log.to_str().unwrap_or_default(),
        },
        "RunAtLoad": true,
        "KeepAlive": true,
        "StandardOutPath": log.to_str().unwrap_or_default(),
        "StandardErrorPath": log.to_str().unwrap_or_default(),
    }))
}

fn env_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
}

fn launch_agents_dir() -> Result<PathBuf> {
    let dir = env_home().join("Library/LaunchAgents");
    std::fs::create_dir_all(&dir).context("creating LaunchAgents directory")?;
    Ok(dir)
}
