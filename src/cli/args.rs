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

pub(crate) fn parse_args(mut args: Vec<std::ffi::OsString>) -> std::result::Result<Cli, clap::Error> {
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

pub(crate) fn localized_command() -> clap::Command {
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
    command = localize_top_args(command);
    command = localize_subcommands(command);
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

fn localize_top_args(command: clap::Command) -> clap::Command {
    command
        .mut_arg("plan", |arg| {
            arg.help(t("Run in read-only planning mode", "使用只读计划模式运行"))
        })
        .mut_arg("debug", |arg| {
            arg.help(t(
                "Write detailed diagnostics to the GQY log directory",
                "将详细诊断信息写入 GQY 日志目录",
            ))
        })
        .mut_arg("stdout", |arg| {
            arg.help(t(
                "Plain output mode (no colors, no TUI); pipe-friendly for stdout redirection",
                "纯文本输出模式（无颜色、无 TUI）；适合管道重定向",
            ))
        })
        .mut_arg("message", |arg| {
            arg.help(t(
                "Message to send; omitted to enter REPL",
                "要发送的消息；省略则进入 REPL",
            ))
        })
}

fn localize_subcommands(mut command: clap::Command) -> clap::Command {
    let descriptions = [
        (
            "ask",
            "Send one message to the assistant",
            "向助手发送一条消息",
        ),
        (
            "init",
            "Create default config and state files",
            "创建默认配置和状态文件",
        ),
        (
            "paths",
            "Show app config, data, and cache paths",
            "显示应用配置、数据和缓存路径",
        ),
        ("config", "Open or manage configuration", "打开或管理配置"),
        ("models", "List or switch models", "列出或切换模型"),
        (
            "variant",
            "View or switch thinking level",
            "查看或切换思考档位",
        ),
        (
            "fish-init",
            "Integrate with fish so you can chat in natural language directly in the terminal",
            "集成到 fish，集成后可在终端直接使用自然语言交流。",
        ),
        (
            "bash-init",
            "Integrate with bash so you can chat in natural language directly in the terminal",
            "集成到 bash，集成后可在终端直接使用自然语言交流。",
        ),
        (
            "zsh-init",
            "Integrate with zsh so you can chat in natural language directly in the terminal",
            "集成到 zsh，集成后可在终端直接使用自然语言交流。",
        ),
        (
            "remove-shell-hook",
            "Safely remove installed GQY shell hooks",
            "安全删除已安装的 GQY shell hook",
        ),
        ("history", "Show conversation history", "显示会话历史"),
        (
            "pop",
            "Move conversation turns out of active context",
            "将对话轮次移出当前上下文",
        ),
        ("kb", "Manage local knowledge base", "管理本地知识库"),
        (
            "memory",
            "Inspect or edit assistant memory",
            "查看或编辑助手记忆",
        ),
        (
            "backup",
            "Snapshot and sync portable assistant state",
            "快照并同步助理的独立状态",
        ),
        ("skills", "Manage assistant skills", "管理助手 skills"),
        (
            "reset",
            "Clear current conversation history",
            "清空当前会话历史",
        ),
        ("web", "Start the local GQY WebUI", "启动本地 GQY WebUI"),
    ];
    for (name, en, zh) in descriptions {
        command = command.mut_subcommand(name, |subcommand| subcommand.about(t(en, zh)));
    }
    command = command
        .mut_subcommand("ask", localize_ask_command)
        .mut_subcommand("models", localize_models_command)
        .mut_subcommand("variant", localize_variant_command)
        .mut_subcommand("history", localize_history_command)
        .mut_subcommand("pop", localize_pop_command)
        .mut_subcommand("kb", localize_kb_command)
        .mut_subcommand("memory", localize_memory_command)
        .mut_subcommand("backup", localize_backup_command)
        .mut_subcommand("skills", localize_skills_command)
        .mut_subcommand("config", localize_config_command)
        .mut_subcommand("reset", localize_reset_command)
        .mut_subcommand("web", localize_web_command);
    command
}

fn localize_ask_command(command: clap::Command) -> clap::Command {
    command.mut_arg("message", |arg| {
        arg.help(t("Message to send", "要发送的消息"))
    })
}

fn localize_models_command(command: clap::Command) -> clap::Command {
    command.mut_arg("index", |arg| {
        arg.help(t("Model list index to activate", "要激活的模型列表序号"))
    })
}

fn localize_variant_command(command: clap::Command) -> clap::Command {
    command.mut_arg("name", |arg| {
        arg.help(t(
            "Thinking level to select; omit to choose interactively",
            "要选择的思考档位；省略则进入交互选择",
        ))
    })
}

fn localize_history_command(command: clap::Command) -> clap::Command {
    command
        .mut_arg("limit", |arg| {
            arg.help(t("Number of history entries to show", "显示的历史条数"))
        })
        .mut_arg("raw", |arg| {
            arg.help(t("Print raw JSONL entries", "输出原始 JSONL 条目"))
        })
        .mut_arg("no_thinking", |arg| {
            arg.help(t("Hide stored reasoning", "隐藏已保存的思考内容"))
        })
}

fn localize_pop_command(command: clap::Command) -> clap::Command {
    command.mut_arg("count", |arg| {
        arg.help(t(
            "Number of oldest turns to pop; omit to select interactively",
            "要弹出的最旧轮次数；省略则进入交互多选",
        ))
    })
}

fn localize_config_command(command: clap::Command) -> clap::Command {
    command
        .mut_subcommand("validate", |subcommand| {
            subcommand.about(t("Validate configuration", "校验配置"))
        })
        .mut_subcommand("paths", |subcommand| {
            subcommand.about(t("Show configuration paths", "显示配置路径"))
        })
        .mut_subcommand("set", |subcommand| {
            subcommand
                .about(t(
                    "Set a config value non-interactively",
                    "免交互设置配置项",
                ))
                .mut_arg("key", |arg| {
                    arg.help(t(
                        "Dotted path, e.g. display.language",
                        "点号路径，如 display.language",
                    ))
                })
                .mut_arg("value", |arg| {
                    arg.help(t(
                        "Value; JSON types are auto-detected",
                        "值；自动识别 JSON 类型",
                    ))
                })
        })
        .mut_subcommand("get", |subcommand| {
            subcommand
                .about(t("Read a config value (secrets redacted)", "读取配置项（密钥脱敏）"))
                .mut_arg("key", |arg| {
                    arg.help(t(
                        "Dotted path; omit to dump everything",
                        "点号路径；省略时输出全部",
                    ))
                })
        })
}

fn localize_reset_command(command: clap::Command) -> clap::Command {
    command.mut_arg("scope", |arg| {
        arg.help(t(
            "all also clears long-term memory",
            "all 同时清空长期记忆",
        ))
    })
}

fn localize_web_command(command: clap::Command) -> clap::Command {
    command
        .mut_arg("port", |arg| arg.help(t("Local TCP port", "本地 TCP 端口")))
        .mut_arg("host", |arg| {
            arg.help(t(
                "Bind address; non-loopback addresses require a password",
                "监听地址；绑定非回环地址必须设置密码",
            ))
        })
        .mut_arg("no_open", |arg| {
            arg.help(t(
                "Do not open the WebUI in a browser",
                "不自动在浏览器中打开 WebUI",
            ))
        })
        .mut_arg("password", |arg| {
            arg.help(t(
                "Require a password; omit the value to enter it securely",
                "要求访问密码；省略参数值时安全输入",
            ))
        })
        .mut_arg("password_file", |arg| {
            arg.help(t(
                "Read the WebUI password from a file",
                "从文件读取 WebUI 访问密码",
            ))
        })
}

fn localize_kb_command(mut command: clap::Command) -> clap::Command {
    let descriptions = [
        ("add", "Add a file or directory", "添加文件或目录"),
        ("list", "List indexed files", "列出已索引文件"),
        ("search", "Search knowledge base content", "搜索知识库内容"),
        ("find", "Find files by name", "按文件名查找文件"),
        ("read", "Read a knowledge base file", "读取知识库文件"),
        ("remove", "Remove a knowledge base file", "移除知识库文件"),
        (
            "reindex",
            "Rebuild keyword index on demand",
            "按需重建关键词索引",
        ),
        ("stats", "Show knowledge base statistics", "显示知识库统计"),
        ("embed", "Manage semantic embeddings", "管理语义嵌入"),
    ];
    for (name, en, zh) in descriptions {
        command = command.mut_subcommand(name, |subcommand| subcommand.about(t(en, zh)));
    }
    command
        .mut_subcommand("add", |subcommand| {
            subcommand
                .mut_arg("path", |arg| arg.help(t("Path to add", "要添加的路径")))
                .mut_arg("recursive", |arg| {
                    arg.help(t(
                        "Compatibility flag; directories are recursive by default",
                        "兼容参数；目录默认递归导入",
                    ))
                })
        })
        .mut_subcommand("search", |subcommand| {
            subcommand
                .mut_arg("query", |arg| arg.help(t("Search query", "搜索查询")))
                .mut_arg("limit", |arg| arg.help(t("Maximum results", "最大结果数")))
        })
        .mut_subcommand("find", |subcommand| {
            subcommand
                .mut_arg("query", |arg| arg.help(t("Filename query", "文件名查询")))
                .mut_arg("limit", |arg| arg.help(t("Maximum results", "最大结果数")))
        })
        .mut_subcommand("read", |subcommand| {
            subcommand
                .mut_arg("file", |arg| {
                    arg.help(t("Knowledge base file name", "知识库文件名"))
                })
                .mut_arg("start", |arg| arg.help(t("Starting line", "起始行")))
                .mut_arg("lines", |arg| arg.help(t("Number of lines", "读取行数")))
        })
        .mut_subcommand("remove", |subcommand| {
            subcommand.mut_arg("file", |arg| arg.help(t("File to remove", "要移除的文件")))
        })
}

fn localize_memory_command(mut command: clap::Command) -> clap::Command {
    let descriptions = [
        ("stats", "Show memory statistics", "显示记忆统计"),
        ("reset", "Clear assistant memory", "清空助手记忆"),
        ("search", "Search memories", "搜索记忆"),
        ("remember", "Save a manual fact", "手动保存事实"),
    ];
    for (name, en, zh) in descriptions {
        command = command.mut_subcommand(name, |subcommand| subcommand.about(t(en, zh)));
    }
    command
        .mut_subcommand("reset", |subcommand| {
            subcommand.mut_arg("include_skills", |arg| {
                arg.help(t(
                    "Also remove generated skills",
                    "同时移除自动生成的 skills",
                ))
            })
        })
        .mut_subcommand("search", |subcommand| {
            subcommand
                .mut_arg("query", |arg| arg.help(t("Search query", "搜索查询")))
                .mut_arg("limit", |arg| arg.help(t("Maximum results", "最大结果数")))
                .mut_arg("forgotten", |arg| {
                    arg.help(t("Include forgotten memories", "包含已遗忘记忆"))
                })
        })
        .mut_subcommand("remember", |subcommand| {
            subcommand
                .mut_arg("content", |arg| arg.help(t("Fact content", "事实内容")))
                .mut_arg("source", |arg| arg.help(t("Source label", "来源标签")))
        })
}

fn localize_backup_command(mut command: clap::Command) -> clap::Command {
    let descriptions = [
        (
            "init",
            "Configure an isolated Git backup",
            "配置独立的 Git 备份",
        ),
        (
            "now",
            "Create and optionally push a snapshot",
            "立即创建并推送快照",
        ),
        (
            "status",
            "Show backup configuration and Git status",
            "显示备份配置与 Git 状态",
        ),
        (
            "restore",
            "Restore state from the backup remote",
            "从备份远程恢复状态",
        ),
        (
            "remote",
            "Attach or replace the backup remote",
            "绑定或更换备份远程仓库",
        ),
    ];
    for (name, en, zh) in descriptions {
        command = command.mut_subcommand(name, |subcommand| subcommand.about(t(en, zh)));
    }
    command
}

fn localize_skills_command(mut command: clap::Command) -> clap::Command {
    let descriptions = [
        ("list", "List skills", "列出 skills"),
        ("show", "Show a skill", "显示 skill"),
        ("enable", "Enable a skill", "启用 skill"),
        ("disable", "Disable a skill", "禁用 skill"),
        ("remove", "Remove a skill", "移除 skill"),
        ("stats", "Show skill statistics", "显示 skill 统计"),
        (
            "prune",
            "Remove disabled generated skills",
            "清理已禁用的自动 skills",
        ),
    ];
    for (name, en, zh) in descriptions {
        command = command.mut_subcommand(name, |subcommand| subcommand.about(t(en, zh)));
    }
    for name in ["show", "enable", "disable", "remove"] {
        command = command.mut_subcommand(name, |subcommand| {
            subcommand.mut_arg("name", |arg| arg.help(t("Skill name", "skill 名称")))
        });
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
    /// 浏览内置供应商模板目录
    Templates {
        /// 按区域筛选：china / overseas / local / default
        #[arg(long)]
        category: Option<String>,
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
