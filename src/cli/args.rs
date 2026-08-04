//! CLI 参数定义模块
//!
//! 包含所有 clap 参数结构体和子命令枚举。

use clap::{Arg, ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;

use crate::bridges::napcat::NapcatArgs;
use crate::bridges::tg::TgArgs;
use crate::i18n::{is_zh, text as t};

// ── CLI 入口 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "gqy", version, about = "GQY CLI AI Agent")]
pub struct Cli {
    #[arg(long)]
    pub plan: bool,

    #[arg(long, global = true)]
    pub debug: bool,

    #[arg(long)]
    pub stdout: bool,

    /// 显示工具调用计划而不实际执行
    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, hide = true)]
    pub shell_intercept: bool,

    #[arg(long, hide = true)]
    pub shell_classify: bool,

    #[arg(long, hide = true)]
    pub shell: Option<String>,

    #[arg(long, hide = true)]
    pub stdin: bool,

    #[arg(long, hide = true)]
    pub clipboard_paste: bool,

    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub message: Vec<String>,
}

pub fn parse() -> Cli {
    parse_args(std::env::args_os().collect()).unwrap_or_else(|err| err.exit())
}

fn parse_args(mut args: Vec<std::ffi::OsString>) -> std::result::Result<Cli, clap::Error> {
    let debug = extract_debug_flag(&mut args);
    let matches = localized_command().try_get_matches_from(args)?;
    let mut cli = Cli::from_arg_matches(&matches)?;
    cli.debug |= debug;
    Ok(cli)
}

fn extract_debug_flag(args: &mut Vec<std::ffi::OsString>) -> bool {
    let mut found = false;
    let mut seen_separator = false;
    args.retain(|arg| {
        if seen_separator {
            return true;
        }
        if arg == "--" {
            seen_separator = true;
            return true;
        }
        if arg == "--debug" {
            found = true;
            false
        } else {
            true
        }
    });
    found
}

fn localized_command() -> clap::Command {
    let mut command = Cli::command();
    command = command
        .about(t("GQY CLI AI Agent", "GQY 命令行 AI 助手"))
        .override_usage(t(
            "gqy [OPTIONS] [MESSAGE]... [COMMAND]",
            "gqy [选项] [消息]... [命令]",
        ));
    if is_zh() {
        command = command
            .subcommand_help_heading("命令")
            .arg_required_else_help(false)
            .next_help_heading("选项")
            .help_template("{about}\n\n用法: {usage}\n\n命令:\n{subcommands}\n参数:\n{positionals}\n选项:\n{options}\n{after-help}")
            .after_help("提示：不带参数进入 REPL；直接输入消息会发送一次对话。可在配置界面设置语言，GQY_LANG 可临时覆盖。")
            .disable_help_subcommand(true);
    } else {
        command = command
            .after_help(
                "Tip: run without arguments to enter the REPL; pass MESSAGE to send one chat turn. Set the language in the configuration UI; GQY_LANG is a temporary override.",
            )
            .disable_help_subcommand(true);
    }
    command = apply_localized_help_flags(command, true);
    if is_zh() {
        command = apply_chinese_help_template(command);
    }
    command
}

fn apply_localized_help_flags(mut command: clap::Command, root: bool) -> clap::Command {
    command = command.disable_help_flag(true).arg(
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t("Print help", "显示帮助"))
            .action(ArgAction::Help),
    );
    if root {
        command = command.disable_version_flag(true).arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .help(t("Print version", "显示版本"))
                .action(ArgAction::Version),
        );
    }
    let subcommands = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<Vec<_>>();
    for name in subcommands {
        command = command.mut_subcommand(&name, |subcommand| {
            apply_localized_help_flags(subcommand, false)
        });
    }
    command
}

fn apply_chinese_help_template(mut command: clap::Command) -> clap::Command {
    let has_subcommands = command.get_subcommands().next().is_some();
    command = if has_subcommands {
        command.help_template(
            "{about}\n\n用法: {usage}\n\n命令:\n{subcommands}\n参数:\n{positionals}\n选项:\n{options}\n{after-help}",
        )
    } else {
        command.help_template(
            "{about}\n\n用法: {usage}\n\n参数:\n{positionals}\n选项:\n{options}\n{after-help}",
        )
    };
    let subcommands = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<Vec<_>>();
    for name in subcommands {
        command = command.mut_subcommand(&name, apply_chinese_help_template);
    }
    command
}

// ── 子命令定义 ────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(name = "__alarm-worker", hide = true)]
    AlarmWorker(AlarmWorkerArgs),
    #[command(name = "__tool", hide = true)]
    Tool(ToolArgs),
    #[command(name = "__preview", hide = true)]
    Preview,
    Ask(MessageArgs),
    Init,
    Paths,
    Config(ConfigArgs),
    Models(ModelsArgs),
    Variant(VariantArgs),
    FishInit,
    BashInit,
    ZshInit,
    RemoveShellHook,
    History(HistoryArgs),
    Activity(ActivityArgs),
    Archive(ArchiveArgs),
    Pop(PopArgs),
    Memes(MemesArgs),
    Kb(KbArgs),
    Memory(MemoryArgs),
    Backup(BackupArgs),
    Skills(SkillsArgs),
    Tools(ToolsArgs),
    Reset(ResetArgs),
    Web(WebArgs),
    Balance,
    Alarm(AlarmArgs),
    Watch(WatchArgs),
    Tts(TtsArgs),
    Stt(SttArgs),
    Napcat(NapcatArgs),
    Tg(TgArgs),
    Provider(ProviderArgs),
}

// ── 参数结构体 ────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct MessageArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub message: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ResetArgs {
    pub scope: Option<String>,
}

#[derive(Args)]
pub struct WebArgs {
    #[arg(long, default_value_t = 4096)]
    pub port: u16,

    /// 监听地址。默认仅本机；绑定非回环地址必须设置密码
    #[arg(long, default_value = "127.0.0.1", value_name = "HOST")]
    pub host: String,

    #[arg(long)]
    pub no_open: bool,

    #[arg(short = 'p', long, num_args = 0..=1, default_missing_value = "")]
    pub password: Option<String>,

    #[arg(long, value_name = "PATH", conflicts_with = "password")]
    pub password_file: Option<PathBuf>,
}

impl std::fmt::Debug for WebArgs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebArgs")
            .field("port", &self.port)
            .field("host", &self.host)
            .field("no_open", &self.no_open)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("password_file", &self.password_file)
            .finish()
    }
}

#[derive(Debug, Args)]
pub struct AlarmWorkerArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub time: String,
    #[arg(long, default_value = "GQY alarm")]
    pub label: String,
    #[arg(long)]
    pub state_dir: PathBuf,
    #[arg(long)]
    pub cache_dir: PathBuf,
    #[arg(long)]
    pub audio_file: Option<PathBuf>,
    /// 周期重复间隔秒数（0 = 一次性）
    #[arg(long, default_value_t = 0)]
    pub repeat: u64,
    /// 周期闹钟最大响铃次数（0 = 不限，默认 20，防止无限响铃）
    #[arg(long, default_value_t = 20)]
    pub max_rings: u64,
    /// 父进程 PID：周期闹钟检测父进程退出（孤儿保护）
    #[arg(long, default_value_t = 0)]
    pub parent_pid: u32,
}

#[derive(Debug, Args)]
pub struct ToolArgs {
    pub name: String,
    pub arguments: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    #[arg(short, long, default_value_t = 20)]
    pub limit: usize,

    #[arg(long)]
    pub raw: bool,

    #[arg(long)]
    pub no_thinking: bool,

    /// 关键词搜索（当前会话全部轮次 + 已归档轮次），不设则按时间倒序显示最近记录
    #[arg(long)]
    pub search: Option<String>,
}

#[derive(Debug, Args)]
pub struct ActivityArgs {
    /// 关键词过滤活动日志
    #[arg(long)]
    pub search: Option<String>,

    /// 显示条数，默认 20
    #[arg(short, long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ArchiveArgs {
    /// 保留最近 N 天的轮次，更早的归档到 evicted_context（默认 7 天）
    #[arg(long, default_value_t = 7)]
    pub keep_days: u64,

    /// 强制归档（即使没有新轮次）
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct PopArgs {
    #[arg(value_parser = parse_positive_pop_count)]
    pub count: Option<usize>,
}

fn parse_positive_pop_count(s: &str) -> std::result::Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("expected a positive integer, got `{s}`"))?;
    if n == 0 {
        Err("count must be >= 1".into())
    } else {
        Ok(n)
    }
}

#[derive(Debug, Args)]
pub struct ModelsArgs {
    pub index: Option<usize>,
}

#[derive(Debug, Args)]
pub struct VariantArgs {
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct KbArgs {
    #[command(subcommand)]
    pub command: KbCommand,
}

#[derive(Debug, Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    Stats,
    Reset(MemoryResetArgs),
    Search(MemorySearchArgs),
    Remember(MemoryRememberArgs),
}

#[derive(Debug, Args)]
pub struct MemoryResetArgs {
    #[arg(long)]
    pub include_skills: bool,
}

#[derive(Debug, Args)]
pub struct MemorySearchArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub forgotten: bool,
}

#[derive(Debug, Args)]
pub struct MemoryRememberArgs {
    pub content: Vec<String>,
    #[arg(short, long, default_value = "manual")]
    pub source: String,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommand,
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    Init(BackupInitArgs),
    Now(BackupNowArgs),
    Status,
    Restore(BackupRestoreArgs),
    Remote(BackupRemoteArgs),
}

#[derive(Debug, Args)]
pub struct BackupInitArgs {
    #[arg(long)]
    pub remote: Option<String>,
    #[arg(long, default_value = "main")]
    pub branch: String,
    #[arg(long, default_value = "GQY Memory")]
    pub name: String,
    #[arg(long, default_value = "gqy@localhost")]
    pub email: String,
    #[arg(long)]
    pub ssh_key: Option<PathBuf>,
    #[arg(long)]
    pub no_auto_push: bool,
}

#[derive(Debug, Args)]
pub struct BackupNowArgs {
    #[arg(long)]
    pub no_push: bool,
}

#[derive(Debug, Args)]
pub struct BackupRestoreArgs {
    #[arg(long)]
    pub remote: String,
    #[arg(long, default_value = "main")]
    pub branch: String,
    #[arg(long, default_value = "GQY Restore")]
    pub name: String,
    #[arg(long, default_value = "gqy@localhost")]
    pub email: String,
    #[arg(long)]
    pub ssh_key: Option<PathBuf>,
    #[arg(long)]
    pub no_auto_push: bool,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct BackupRemoteArgs {
    #[arg(value_name = "URL")]
    pub url: String,
    #[arg(long)]
    pub ssh_key: Option<PathBuf>,
    #[arg(long, conflicts_with = "no_auto_push")]
    pub auto_push: bool,
    #[arg(long)]
    pub no_auto_push: bool,
}

#[derive(Debug, Args)]
pub struct MemesArgs {
    #[command(subcommand)]
    pub command: Option<MemesCommand>,
}

#[derive(Debug, Subcommand)]
pub enum MemesCommand {
    /// 列出表情库（内置 + 用户覆盖层）
    List,
    /// 统计表情库（数量/格式/大小）
    Stats,
}

#[derive(Debug, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: SkillsCommand,
}

#[derive(Debug, Args)]
pub struct AlarmArgs {
    #[command(subcommand)]
    pub command: AlarmCommand,
}

#[derive(Debug, Subcommand)]
pub enum AlarmCommand {
    /// 列出当前闹钟/番茄钟/周期提醒
    List,
    /// 按 id 取消闹钟
    Cancel { id: String },
    /// 全局停止：终止所有运行中的闹钟 worker（孤儿兜底）
    Stop {
        /// 停止全部闹钟（唯一模式）
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Args)]
pub struct ProviderArgs {
    #[command(subcommand)]
    pub action: ProviderAction,
}

#[derive(Debug, Subcommand)]
pub enum ProviderAction {
    /// 列出全部供应商（key 脱敏）与当前激活项
    List,
    /// 添加/更新 OpenAI 兼容供应商（自动发现模型并激活默认模型）
    Add {
        /// 供应商 id（小写字母/数字/连字符；不填从 base_url 推断）
        #[arg(long)]
        id: Option<String>,
        /// 显示名
        #[arg(long)]
        name: Option<String>,
        /// OpenAI 兼容端点，如 https://api.deepseek.com/v1
        base_url: String,
        /// API Key（本地服务可填占位符）
        #[arg(long)]
        api_key: String,
        /// 要激活的模型；不填自动发现并选第一个
        #[arg(long)]
        model: Option<String>,
    },
    /// 热切换激活指定供应商（可指定模型）
    Switch {
        provider_id: String,
        /// 要激活的模型；不填用其 default_model 或第一个
        #[arg(long)]
        model: Option<String>,
    },
    /// 移除供应商
    Remove {
        provider_id: String,
    },
}

#[derive(Debug, Args)]
pub struct TtsArgs {
    /// 要朗读的文字
    pub text: String,

    /// 语音（如 Ting-Ting、Samantha），默认系统语音
    #[arg(long)]
    pub voice: Option<String>,

    /// 输出到文件而非播放（如 out.aiff / out.m4a）
    #[arg(short, long)]
    pub output: Option<String>,

    /// 列出可用语音
    #[arg(long)]
    pub list: bool,

    /// 用顾清影克隆音色朗读（Qwen3-TTS 本地服务，需先启动 scripts/tts-server.py）
    #[arg(long)]
    pub clone: bool,
}

#[derive(Debug, Args)]
pub struct SttArgs {
    /// 音频文件路径（m4a/wav 等）
    pub audio: String,

    /// 识别语言，默认 zh-Hans
    #[arg(long, default_value = "zh-Hans")]
    pub locale: String,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    /// 采样间隔（如 30s、5m），默认 60s
    #[arg(long, default_value = "60s")]
    pub every: String,

    /// 只采样一次并输出报告（不循环）
    #[arg(long)]
    pub once: bool,
}

#[derive(Debug, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    pub command: Option<ToolsCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// 先理解再导入：列出仓库候选脚本与头部摘要，不导入
    Inspect {
        source: String,
    },
    /// 导入工具包：本地目录或 Git 仓库 URL（自动扫描或按清单）
    Import {
        source: String,
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// 只导入指定候选（逗号分隔，配合 inspect 使用；如 --only download.sh,install.sh）
        #[arg(long, value_name = "IDS")]
        only: Option<String>,
    },
    /// 列出已导入的用户工具包
    List,
    /// 删除已导入的工具包（连同其在 index.json 中的注册）
    Remove {
        /// 工具包名（import 时的 --name，或包目录名）
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// 查看工具包详情（工具 id / 显示名 / 描述 / 禁用状态）
    Show {
        /// 工具包名
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// 禁用已导入的工具（扫描时跳过）
    Disable {
        /// 工具 id 或路径（如 today，或 today-demo/today.sh）
        #[arg(value_name = "ID")]
        id: String,
    },
    /// 重新启用被禁用的工具
    Enable {
        /// 工具 id 或路径
        #[arg(value_name = "ID")]
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    List,
    Show(SkillNameArgs),
    Enable(SkillNameArgs),
    Disable(SkillNameArgs),
    Remove(SkillNameArgs),
    Stats,
    Prune,
}

#[derive(Debug, Args)]
pub struct SkillNameArgs {
    pub name: String,
}

#[derive(Debug, Subcommand)]
pub enum KbCommand {
    Add(KbAddArgs),
    List,
    Search(KbSearchArgs),
    Find(KbFindArgs),
    Read(KbReadArgs),
    Remove(KbRemoveArgs),
    Reindex,
    Stats,
    Embed(KbEmbedArgs),
}

#[derive(Debug, Args)]
pub struct KbAddArgs {
    pub path: PathBuf,
    #[arg(
        short,
        long,
        help = "Compatibility flag; directories are recursive by default"
    )]
    pub recursive: bool,
}

#[derive(Debug, Args)]
pub struct KbSearchArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbFindArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbReadArgs {
    pub file: String,
    #[arg(long, default_value_t = 1)]
    pub start: usize,
    #[arg(long)]
    pub lines: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbRemoveArgs {
    pub file: String,
}

#[derive(Debug, Args)]
pub struct KbEmbedArgs {
    #[command(subcommand)]
    pub command: KbEmbedCommand,
}

#[derive(Debug, Subcommand)]
pub enum KbEmbedCommand {
    Reindex(KbEmbedReindexArgs),
}

#[derive(Debug, Args)]
pub struct KbEmbedReindexArgs {
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Validate,
    Paths,
    /// 免交互设置配置项（支持点号路径，如 display.language zh / active_provider deepseek）
    Set(ConfigSetArgs),
    /// 读取配置项（脱敏输出；不带 key 时输出全部）
    Get(ConfigGetArgs),
    #[command(hide = true)]
    PromptSource,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    /// 点号路径，如 display.language、tools.max_rounds
    pub key: String,
    /// 值：自动识别 JSON（true/数字/数组/对象），其余按字符串处理
    pub value: String,
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    /// 点号路径；省略时输出全部配置（密钥已脱敏）
    pub key: Option<String>,
}
