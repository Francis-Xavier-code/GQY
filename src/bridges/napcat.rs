//! `gqy napcat`：NapCat (QQ) 桥接管理。
//!
//! 子命令：
//! - `status`：查看配置与自启动状态
//! - `install`：安装桥接 LaunchAgent（自启动，KeepAlive 托管）；`--napcat <dir>` 同时托管 NapCat 本体
//! - `uninstall`：移除自启动（不删配置与数据）
//! - `config <key> <value>`：设置 ws_url / self_id / bin / enabled
//!
//! NapCat 本体（QQ 客户端 + NapCat 插件）依赖本机 QQ：`/Applications/QQ.app`。
//! 未安装时可以让顾清影在对话里帮你下载部署（她有下载与文件工具）。

use super::NapcatConfig;
use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use std::path::{Path, PathBuf};

const BRIDGE_LABEL: &str = "com.gqy.napcat-bridge";
const NAPCAT_LABEL: &str = "com.gqy.napcat";

#[derive(Debug, Args)]
pub struct NapcatArgs {
    #[command(subcommand)]
    pub command: Option<NapcatCommand>,
}

#[derive(Debug, Subcommand)]
pub enum NapcatCommand {
    Status,
    Install {
        /// NapCat 本体目录（含 QQ 副本或 NapCat 文件）；提供后同时托管本体自启动
        #[arg(long, value_name = "DIR")]
        napcat: Option<PathBuf>,
    },
    Uninstall,
    /// 设置配置项：ws_url / self_id / bin / enabled
    Config {
        key: String,
        value: String,
    },
}

pub async fn run(paths: &GqyPaths, args: NapcatArgs) -> Result<()> {
    match args.command.unwrap_or(NapcatCommand::Status) {
        NapcatCommand::Status => run_status(paths),
        NapcatCommand::Install { napcat } => run_install(paths, napcat),
        NapcatCommand::Uninstall => run_uninstall(paths),
        NapcatCommand::Config { key, value } => run_config(paths, &key, &value),
    }
}

fn config_for(paths: &GqyPaths) -> Result<NapcatConfig> {
    let mut bridges = super::load(paths)?;
    let mut config = bridges.napcat.clone().unwrap_or(NapcatConfig {
        ws_url: super::default_ws_url(),
        self_id: String::new(),
        bin: super::node_bin(),
        install_dir: String::new(),
        enabled: true,
    });
    // bin 默认跟随当前 node
    if config.bin.is_empty() {
        config.bin = super::node_bin();
    }
    bridges.napcat = Some(config.clone());
    super::save(paths, &bridges)?;
    Ok(config)
}

fn run_status(paths: &GqyPaths) -> Result<()> {
    let bridges = super::load(paths)?;
    let config = bridges
        .napcat
        .clone()
        .unwrap_or_else(|| NapcatConfig {
            ws_url: super::default_ws_url(),
            self_id: String::new(),
            bin: super::node_bin(),
            install_dir: String::new(),
            enabled: true,
        });
    println!("NapCat 桥接配置:");
    println!("  ws_url:    {}", config.ws_url);
    println!(
        "  self_id:   {}",
        if config.self_id.is_empty() {
            "(未设置 —— 群聊 @ 响应不可用)"
        } else {
            &config.self_id
        }
    );
    println!("  node:      {}", config.bin);
    println!("  启用:      {}", if config.enabled { "是" } else { "否" });
    println!("  桥接脚本:  {}", super::bridge_script(paths, "napcat")?.display());
    println!(
        "  桥接自启动: {}",
        if super::launchctl_status(BRIDGE_LABEL)? { "运行中" } else { "未安装" }
    );
    println!(
        "  NapCat 本体: {}",
        if super::launchctl_status(NAPCAT_LABEL)? { "运行中" } else { "未托管" }
    );
    let bridge_log = super::bridge_log_path(paths, "napcat");
    if bridge_log.exists() {
        let last = std::fs::read_to_string(&bridge_log)
            .ok()
            .and_then(|text| text.lines().rev().next().map(str::to_string))
            .unwrap_or_default();
        println!("  最近日志:  {}", last.chars().take(120).collect::<String>());
    }
    Ok(())
}

fn run_install(paths: &GqyPaths, napcat_dir: Option<PathBuf>) -> Result<()> {
    let config = config_for(paths)?;
    if !config.enabled {
        println!("桥接当前处于禁用状态（enabled=false），仅写入自启动配置不加载。");
    }

    // 1. 桥接 LaunchAgent
    let script = super::bridge_script(paths, "napcat")?;
    let plist = bridge_plist(paths, &config, &script)?;
    let plist_path = launch_agents_dir()?.join(format!("{BRIDGE_LABEL}.plist"));
    super::write_plist(&plist_path, &plist)?;
    println!("已写入 {}", plist_path.display());
    if config.enabled {
        super::launchctl_unload(BRIDGE_LABEL).ok();
        super::launchctl_load(&plist_path)?;
        println!("✅ 桥接自启动已加载（KeepAlive 托管，进程崩溃自动重启）");
    }

    // 2. NapCat 本体（可选）
    if let Some(dir) = napcat_dir {
        install_napcat_daemon(paths, &config, &dir)?;
    } else {
        check_qq_available()?;
    }
    println!();
    println!("下一步：");
    if config.self_id.is_empty() {
        println!("  · 设置 QQ 号：gqy napcat config self_id <你的QQ号>");
    }
    println!("  · 查看状态：gqy napcat status");
    Ok(())
}

fn install_napcat_daemon(paths: &GqyPaths, config: &NapcatConfig, dir: &Path) -> Result<()> {
    let qq_bin = dir.join("QQ.app/Contents/MacOS/QQ");
    if !qq_bin.is_file() {
        bail!(
            "{} 下没有 QQ.app（NapCat 需要 QQ 客户端承载插件）。请确认目录正确。",
            dir.display()
        );
    }
    if config.self_id.is_empty() {
        bail!("托管 NapCat 本体前请先设置 QQ 号：gqy napcat config self_id <你的QQ号>");
    }
    let plist = json!({
        "Label": NAPCAT_LABEL,
        "ProgramArguments": [
            qq_bin.to_str().unwrap_or_default(),
            "--no-sandbox",
            "-q",
            config.self_id,
        ],
        "RunAtLoad": true,
        "KeepAlive": true,
        "ProcessType": "Background",
        "WorkingDirectory": dir.to_str().unwrap_or_default(),
        "StandardOutPath": paths.logs_dir().join("napcat.launchd.log").to_str().unwrap_or_default(),
        "StandardErrorPath": paths.logs_dir().join("napcat.launchd.log").to_str().unwrap_or_default(),
    });
    let plist_path = launch_agents_dir()?.join(format!("{NAPCAT_LABEL}.plist"));
    super::write_plist(&plist_path, &plist)?;
    super::launchctl_unload(NAPCAT_LABEL).ok();
    super::launchctl_load(&plist_path)?;
    println!("✅ NapCat 本体自启动已加载（KeepAlive 托管）");
    Ok(())
}

fn check_qq_available() -> Result<()> {
    let candidates = [
        PathBuf::from("/Applications/QQ.app"),
        PathBuf::from(env_home().join("qq-napcat/QQ.app")),
    ];
    for candidate in &candidates {
        if candidate.join("Contents/MacOS/QQ").is_file() {
            println!("检测到 QQ 客户端：{}", candidate.display());
            println!(
                "提示：如需托管 NapCat 本体自启动，执行 gqy napcat install --napcat {}",
                candidate.display()
            );
            return Ok(());
        }
    }
    println!(
        "⚠ 未检测到 QQ 客户端（NapCat 本体依赖）。可以让顾清影在对话里帮你下载部署，"
    );
    println!("  或手动安装后执行 gqy napcat install --napcat <QQ所在目录>");
    Ok(())
}

fn env_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
}

fn run_uninstall(paths: &GqyPaths) -> Result<()> {
    let _ = paths;
    super::launchctl_unload(BRIDGE_LABEL)?;
    super::launchctl_unload(NAPCAT_LABEL).ok();
    for label in [BRIDGE_LABEL, NAPCAT_LABEL] {
        let plist = launch_agents_dir()?.join(format!("{label}.plist"));
        if plist.exists() {
            std::fs::remove_file(&plist)?;
        }
    }
    println!("✅ 已移除 NapCat 桥接与本体自启动（配置与数据保留）");
    Ok(())
}

fn run_config(paths: &GqyPaths, key: &str, value: &str) -> Result<()> {
    let mut bridges = super::load(paths)?;
    let mut config = bridges.napcat.clone().unwrap_or(NapcatConfig {
        ws_url: super::default_ws_url(),
        self_id: String::new(),
        bin: super::node_bin(),
        install_dir: String::new(),
        enabled: true,
    });
    match key {
        "ws_url" => config.ws_url = value.to_string(),
        "self_id" => config.self_id = value.to_string(),
        "bin" => config.bin = value.to_string(),
        "enabled" => {
            config.enabled = value == "true" || value == "1" || value == "yes";
        }
        _ => bail!("未知配置项 {key}（支持：ws_url / self_id / bin / enabled）"),
    }
    bridges.napcat = Some(config.clone());
    super::save(paths, &bridges)?;
    println!("napcat.{key} = {value}");
    Ok(())
}

fn bridge_plist(paths: &GqyPaths, config: &NapcatConfig, script: &Path) -> Result<serde_json::Value> {
    let log = super::bridge_log_path(paths, "napcat");
    Ok(json!({
        "Label": BRIDGE_LABEL,
        "ProgramArguments": [
            config.bin,
            script.to_str().unwrap_or_default(),
        ],
        "EnvironmentVariables": {
            "HOME": env_home().to_str().unwrap_or_default(),
            "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
            "GQY_HOME": paths.home_hint(),
            "GQY_WS_URL": config.ws_url,
            "GQY_SELF_ID": config.self_id,
            "GQY_BIN": paths.bin_hint(),
            "GQY_BRIDGE_LOG": log.to_str().unwrap_or_default(),
        },
        "RunAtLoad": true,
        "KeepAlive": true,
        "StandardOutPath": log.to_str().unwrap_or_default(),
        "StandardErrorPath": log.to_str().unwrap_or_default(),
    }))
}

fn launch_agents_dir() -> Result<PathBuf> {
    let home = env_home();
    let dir = home.join("Library/LaunchAgents");
    std::fs::create_dir_all(&dir).context("creating LaunchAgents directory")?;
    Ok(dir)
}
