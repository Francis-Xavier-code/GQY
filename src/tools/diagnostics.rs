use super::{ToolRegistry, ToolSpec};
use crate::config::{AppConfig, DiagnosticsPluginConfig};
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

pub fn register(registry: &mut ToolRegistry, config: AppConfig) {
    registry.register(ToolSpec::new(
        "inspect_issue",
        "Collect read-only local facts for a reported computer issue. Covers app startup, input method, display or screen sharing, audio, package updates, GPU or driver, network, storage, and general system context. This only gathers evidence; it does not diagnose or produce final advice. After using it, combine the result with knowledge base, memory, and web search/fetch when needed. It does not modify the system.",
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Optional original user request. Used only as fallback when mode is auto or omitted." },
                "mode": { "type": "string", "enum": ["auto", "system", "app", "input_method", "display", "audio", "package_update", "gpu", "network", "storage"], "description": "Probe mode. Use auto only when passing query and no structured mode is obvious." },
                "target": { "type": "string", "description": "Optional target app, process, command, or subsystem, for example qq or opencode." },
                "symptom": { "type": "string", "description": "Optional symptom such as cannot_start, app_cannot_input_chinese, no_audio, screen_share_failed." },
                "depth": { "type": "string", "enum": ["quick", "normal", "full"], "description": "Probe depth. Start with quick or normal; full may run slower probes." },
                "recent_minutes": { "type": "integer", "description": "Recent log window in minutes, clamped to 1..1440." },
                "platform": { "type": "string", "enum": ["auto", "linux", "macos"], "description": "Platform override. Prefer auto." }
            },
            "required": [],
            "additionalProperties": false
        }),
        move |args| {
            let config = config.clone();
            async move { inspect_issue(args, config.plugins.diagnostics.clone()).await }
        },
    ));
}

#[derive(Debug, Clone)]
struct DiagnoseArgs {
    query: Option<String>,
    mode: Mode,
    target: Option<String>,
    symptom: Option<String>,
    depth: Depth,
    recent_minutes: u64,
    platform: PlatformArg,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    System,
    App,
    InputMethod,
    Display,
    Audio,
    PackageUpdate,
    Gpu,
    Network,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Depth {
    Quick,
    Normal,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformArg {
    Auto,
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Platform {
    Linux,
    Macos,
    Unsupported,
}

#[derive(Debug, Serialize)]
struct DiagnosticReport {
    ok: bool,
    platform: Platform,
    query: Option<String>,
    mode: Mode,
    target: Option<String>,
    symptom: Option<String>,
    depth: Depth,
    summary: String,
    facts: BTreeMap<String, Value>,
    checks: Vec<Check>,
    logs: Vec<LogExcerpt>,
    findings: Vec<Finding>,
    next_questions: Vec<String>,
    output_instruction: String,
}

#[derive(Debug, Serialize)]
struct Check {
    id: String,
    status: CheckStatus,
    detail: String,
    evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Ok,
    Warn,
    Error,
    Unknown,
}

#[derive(Debug, Serialize)]
struct LogExcerpt {
    source: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct Finding {
    severity: Severity,
    title: String,
    evidence: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Severity {
    Medium,
    High,
}

#[derive(Debug)]
struct ProbeOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
    timed_out: bool,
}

async fn inspect_issue(args: Value, config: DiagnosticsPluginConfig) -> Result<String> {
    if !config.enabled {
        bail!("diagnostics plugin is disabled");
    }
    let args = parse_args(args)?;
    let platform = detect_platform(args.platform);
    let mut report = DiagnosticReport {
        ok: true,
        platform,
        query: args.query.clone(),
        mode: args.mode,
        target: args.target.clone(),
        symptom: args.symptom.clone(),
        depth: args.depth,
        summary: String::new(),
        facts: BTreeMap::new(),
        checks: Vec::new(),
        logs: Vec::new(),
        findings: Vec::new(),
        next_questions: Vec::new(),
        output_instruction: "Treat this as local issue context only, not a diagnosis. Before the final answer, combine it with relevant knowledge_base or memory results when available; use web_search/web_fetch only for current or external facts. Final answer should explain root cause, evidence, and next steps in user-facing language instead of dumping raw JSON.".to_string(),
    };
    match platform {
        Platform::Linux => run_linux_plan(&args, &config, &mut report).await,
        Platform::Macos => run_macos_plan(&args, &config, &mut report).await,
        Platform::Unsupported => {
            report.ok = false;
            report.summary = "unsupported platform".to_string();
            report.checks.push(Check {
                id: "platform.supported".to_string(),
                status: CheckStatus::Error,
                detail: "only linux and macos are supported by diagnostics".to_string(),
                evidence: vec![std::env::consts::OS.to_string()],
            });
        }
    }
    finalize_summary(&mut report);
    Ok(serde_json::to_string_pretty(&report)?)
}

fn parse_args(args: Value) -> Result<DiagnoseArgs> {
    let query = optional_string(&args, "query", 500);
    let mode_raw = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .trim();
    let mut target = optional_string(&args, "target", 160);
    let mut symptom = optional_string(&args, "symptom", 200);
    let mode = if mode_raw == "auto" {
        let inferred =
            infer_probe_request(query.as_deref(), target.as_deref(), symptom.as_deref())?;
        if target.is_none() {
            target = inferred.target;
        }
        if symptom.is_none() {
            symptom = inferred.symptom;
        }
        inferred.mode
    } else {
        parse_mode(mode_raw)?
    };
    let depth = parse_depth(
        args.get("depth")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .trim(),
    )?;
    let recent_minutes = args
        .get("recent_minutes")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 1440);
    let platform = parse_platform_arg(
        args.get("platform")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .trim(),
    )?;
    Ok(DiagnoseArgs {
        query,
        mode,
        target,
        symptom,
        depth,
        recent_minutes,
        platform,
    })
}

fn optional_string(args: &Value, name: &str, max_chars: usize) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(max_chars).collect())
}

struct InferredProbeRequest {
    mode: Mode,
    target: Option<String>,
    symptom: Option<String>,
}

fn infer_probe_request(
    query: Option<&str>,
    target: Option<&str>,
    symptom: Option<&str>,
) -> Result<InferredProbeRequest> {
    let text = query.unwrap_or_default().trim();
    let lower = text.to_ascii_lowercase();
    let mode = if contains_any(
        text,
        &[
            "输入法",
            "打不了中文",
            "候选框",
            "fcitx",
            "fcitx5",
            "ibus",
            "拼音",
        ],
    ) || contains_any(&lower, &["ime", "input method"])
    {
        Mode::InputMethod
    } else if contains_any(text, &["没声音", "声音", "麦克风", "耳机", "音频"])
        || contains_any(
            &lower,
            &["audio", "sound", "microphone", "pipewire", "wireplumber"],
        )
    {
        Mode::Audio
    } else if contains_any(
        text,
        &["屏幕分享", "黑屏", "截图", "录屏", "显示器", "窗口", "闪屏"],
    ) || contains_any(
        &lower,
        &["display", "screen", "wayland", "xwayland", "portal"],
    ) {
        Mode::Display
    } else if contains_any(text, &["更新", "安装包", "依赖", "滚挂", "包管理"])
        || contains_any(
            &lower,
            &["pacman", "yay", "paru", "aur", "dnf", "apt", "brew"],
        )
    {
        Mode::PackageUpdate
    } else if contains_any(text, &["显卡", "驱动", "独显", "核显"])
        || contains_any(&lower, &["gpu", "nvidia", "amd", "mesa", "vulkan"])
    {
        Mode::Gpu
    } else if contains_any(
        text,
        &["网络", "联网", "断网", "dns", "网卡", "wifi", "wi-fi"],
    ) || contains_any(&lower, &["network", "internet", "wifi", "wi-fi", "dns"])
    {
        Mode::Network
    } else if contains_any(text, &["磁盘", "硬盘", "空间", "挂载", "btrfs", "快照"])
        || contains_any(&lower, &["disk", "storage", "mount", "btrfs", "filesystem"])
    {
        Mode::Storage
    } else if target.is_some()
        || contains_any(text, &["打不开", "启动不了", "闪退", "崩溃", "报错"])
        || contains_any(&lower, &["crash", "cannot start", "won't open", "not open"])
    {
        Mode::App
    } else if text.is_empty() {
        bail!("mode is auto but query is empty; provide query or a structured mode")
    } else {
        Mode::System
    };
    Ok(InferredProbeRequest {
        mode,
        target: target
            .map(ToString::to_string)
            .or_else(|| infer_target(text)),
        symptom: symptom
            .map(ToString::to_string)
            .or_else(|| infer_symptom(text, mode)),
    })
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn infer_target(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for (needle, target) in [
        ("opencode", "opencode"),
        ("open code", "opencode"),
        ("linuxqq", "qq"),
        ("qq", "qq"),
        ("微信", "wechat"),
        ("wechat", "wechat"),
        ("steam", "steam"),
        ("firefox", "firefox"),
        ("chrome", "chrome"),
        ("chromium", "chromium"),
        ("wps", "wps"),
        ("vscode", "code"),
        ("code", "code"),
    ] {
        if lower.contains(needle) || text.contains(needle) {
            return Some(target.to_string());
        }
    }
    None
}

fn infer_symptom(text: &str, mode: Mode) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    match mode {
        Mode::InputMethod => Some("app_cannot_input_chinese".to_string()),
        Mode::App
            if contains_any(text, &["打不开", "启动不了", "闪退", "崩溃"])
                || contains_any(&lower, &["crash", "cannot start", "won't open", "not open"]) =>
        {
            Some("cannot_start".to_string())
        }
        Mode::Audio => Some("audio_problem".to_string()),
        Mode::Display => Some("display_problem".to_string()),
        Mode::Network => Some("network_problem".to_string()),
        Mode::PackageUpdate => Some("package_update_problem".to_string()),
        Mode::Storage => Some("storage_problem".to_string()),
        Mode::Gpu => Some("gpu_problem".to_string()),
        _ => None,
    }
}

fn parse_mode(value: &str) -> Result<Mode> {
    match value {
        "system" => Ok(Mode::System),
        "app" => Ok(Mode::App),
        "input_method" => Ok(Mode::InputMethod),
        "display" => Ok(Mode::Display),
        "audio" => Ok(Mode::Audio),
        "package_update" => Ok(Mode::PackageUpdate),
        "gpu" => Ok(Mode::Gpu),
        "network" => Ok(Mode::Network),
        "storage" => Ok(Mode::Storage),
        _ => bail!("unsupported diagnostic mode: {value}"),
    }
}

fn parse_depth(value: &str) -> Result<Depth> {
    match value {
        "quick" => Ok(Depth::Quick),
        "normal" => Ok(Depth::Normal),
        "full" => Ok(Depth::Full),
        _ => bail!("unsupported diagnostic depth: {value}"),
    }
}

fn parse_platform_arg(value: &str) -> Result<PlatformArg> {
    match value {
        "auto" => Ok(PlatformArg::Auto),
        "linux" => Ok(PlatformArg::Linux),
        "macos" => Ok(PlatformArg::Macos),
        _ => bail!("unsupported diagnostic platform: {value}"),
    }
}

fn detect_platform(arg: PlatformArg) -> Platform {
    match arg {
        PlatformArg::Linux => Platform::Linux,
        PlatformArg::Macos => Platform::Macos,
        PlatformArg::Auto => match std::env::consts::OS {
            "linux" => Platform::Linux,
            "macos" => Platform::Macos,
            _ => Platform::Unsupported,
        },
    }
}

async fn run_linux_plan(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    linux_system_facts(config, report).await;
    match args.mode {
        Mode::System => linux_system_checks(config, report).await,
        Mode::App => linux_app_checks(args, config, report).await,
        Mode::InputMethod => linux_input_method_checks(args, config, report).await,
        Mode::Display => linux_display_checks(args, config, report).await,
        Mode::Audio => linux_audio_checks(args, config, report).await,
        Mode::PackageUpdate => linux_package_checks(args, config, report).await,
        Mode::Gpu => linux_gpu_checks(config, report).await,
        Mode::Network => linux_network_checks(config, report).await,
        Mode::Storage => linux_storage_checks(config, report).await,
    }
}

async fn run_macos_plan(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    macos_system_facts(config, report).await;
    match args.mode {
        Mode::System => macos_system_checks(config, report).await,
        Mode::App => macos_app_checks(args, config, report).await,
        Mode::InputMethod => macos_input_method_checks(args, config, report).await,
        Mode::Display => macos_display_checks(config, report).await,
        Mode::Audio => macos_audio_checks(config, report).await,
        Mode::PackageUpdate => macos_package_checks(config, report).await,
        Mode::Network => macos_network_checks(config, report).await,
        Mode::Storage => macos_storage_checks(config, report).await,
        Mode::Gpu => macos_display_checks(config, report).await,
    }
}

async fn linux_system_facts(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    fact_env(report, "env.shell", "SHELL");
    fact_env(report, "env.term", "TERM");
    fact_env(report, "env.lang", "LANG");
    for key in [
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "DESKTOP_SESSION",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "GTK_IM_MODULE",
        "QT_IM_MODULE",
        "XMODIFIERS",
    ] {
        fact_env(report, &format!("env.{key}"), key);
    }
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        if let Some(name) = os_release_value(&text, "PRETTY_NAME") {
            report
                .facts
                .insert("os.pretty_name".to_string(), json!(name));
        }
    }
    let uname = run_command(config, "uname", &["-a"], 2).await;
    if !uname.stdout.trim().is_empty() {
        report
            .facts
            .insert("kernel.uname".to_string(), json!(uname.stdout.trim()));
    }
}

async fn linux_system_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    for command in ["systemctl", "journalctl", "loginctl", "ip", "df"] {
        command_exists_check(config, report, command).await;
    }
}

async fn linux_app_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    let Some(target) = args.target.as_deref() else {
        report
            .next_questions
            .push("which app should I probe?".to_string());
        return;
    };
    match command_path(config, target).await {
        Some(path) => {
            report
                .facts
                .insert("app.command_path".to_string(), json!(path.clone()));
            report.checks.push(Check {
                id: "app.command_exists".to_string(),
                status: CheckStatus::Ok,
                detail: format!("{target} exists in PATH"),
                evidence: vec![path.clone()],
            });
            app_probe_version(config, report, target).await;
            app_probe_help(config, report, target).await;
            linux_package_owner(config, report, &path).await;
            node_runtime_if_relevant(config, report, target, &path).await;
        }
        None => {
            report.checks.push(Check {
                id: "app.command_exists".to_string(),
                status: CheckStatus::Error,
                detail: format!("{target} was not found in PATH"),
                evidence: Vec::new(),
            });
            report.findings.push(Finding {
                severity: Severity::High,
                title: format!("{target} is not available in the current PATH"),
                evidence: "command -v returned no path".to_string(),
            });
        }
    }
    linux_recent_logs(args, config, report, &[target, "node", "error", "failed"]).await;
}

async fn linux_input_method_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    for name in ["fcitx5", "ibus-daemon"] {
        process_check(config, report, name).await;
    }
    command_exists_check(config, report, "fcitx5-remote").await;
    if command_path(config, "fcitx5-remote").await.is_some() {
        let output = run_command(config, "fcitx5-remote", &[], 2).await;
        report.checks.push(Check {
            id: "input_method.fcitx5_remote".to_string(),
            status: if output.status == Some(0) {
                CheckStatus::Ok
            } else {
                CheckStatus::Warn
            },
            detail: "fcitx5-remote status probe".to_string(),
            evidence: compact_evidence(&output),
        });
    }
    if let Some(target) = args.target.as_deref() {
        let pids = process_check(config, report, target).await;
        linux_app_input_env(report, target, &pids);
        linux_fcitx_package_checks(config, report).await;
        linux_recent_logs(
            args,
            config,
            report,
            &[target, "fcitx", "ibus", "qt", "gtk", "xwayland"],
        )
        .await;
    }
    if std::env::var("QT_IM_MODULE").ok().as_deref() != Some("fcitx") {
        report.findings.push(Finding {
            severity: Severity::Medium,
            title: "QT_IM_MODULE is not set to fcitx in current environment".to_string(),
            evidence: "Qt apps may not load fcitx without this variable or equivalent desktop environment setup".to_string(),
        });
    }
}

async fn linux_display_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    for service in [
        "xdg-desktop-portal.service",
        "pipewire.service",
        "wireplumber.service",
    ] {
        systemd_user_active_check(config, report, service).await;
    }
    process_check(config, report, "Xwayland").await;
    linux_gpu_checks(config, report).await;
    linux_recent_logs(
        args,
        config,
        report,
        &["portal", "pipewire", "wireplumber", "wayland", "xwayland"],
    )
    .await;
}

async fn linux_audio_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    for service in [
        "pipewire.service",
        "wireplumber.service",
        "pipewire-pulse.service",
    ] {
        systemd_user_active_check(config, report, service).await;
    }
    command_exists_check(config, report, "wpctl").await;
    if command_path(config, "wpctl").await.is_some() {
        let output = run_command(config, "wpctl", &["status"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "wpctl status".to_string(),
                message: clip(&output.stdout, 2_000),
            });
        }
    }
    linux_recent_logs(
        args,
        config,
        report,
        &["pipewire", "wireplumber", "pulse", "audio"],
    )
    .await;
}

async fn linux_package_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    command_exists_check(config, report, "pacman").await;
    if std::path::Path::new("/var/lib/pacman/db.lck").exists() {
        report.findings.push(Finding {
            severity: Severity::High,
            title: "pacman database lock exists".to_string(),
            evidence: "/var/lib/pacman/db.lck exists".to_string(),
        });
    }
    if command_path(config, "pacman").await.is_some() {
        let output = run_command(config, "pacman", &["-Q", "archlinux-keyring"], 3).await;
        if output.status == Some(0) && !output.stdout.trim().is_empty() {
            report.facts.insert(
                "package.archlinux_keyring".to_string(),
                json!(output.stdout.trim()),
            );
        }
    }
    linux_recent_logs(
        args,
        config,
        report,
        &["pacman", "error", "failed", "warning"],
    )
    .await;
}

async fn linux_gpu_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    command_exists_check(config, report, "lspci").await;
    if command_path(config, "lspci").await.is_some() {
        let output = run_command(config, "lspci", &["-nnk"], 4).await;
        let gpu_lines = extract_lspci_gpu_blocks(&output.stdout);
        if !gpu_lines.is_empty() {
            report
                .facts
                .insert("gpu.lspci".to_string(), json!(gpu_lines));
        }
    }
    command_exists_check(config, report, "nvidia-smi").await;
}

async fn linux_network_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    for command in ["ip", "resolvectl", "ping"] {
        command_exists_check(config, report, command).await;
    }
    if command_path(config, "ip").await.is_some() {
        let output = run_command(config, "ip", &["-brief", "addr"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "ip -brief addr".to_string(),
                message: clip(&mask_network_addresses(&output.stdout), 2_000),
            });
        }
    }
    if command_path(config, "resolvectl").await.is_some() {
        let output = run_command(config, "resolvectl", &["status"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "resolvectl status".to_string(),
                message: clip(&output.stdout, 2_000),
            });
        }
    }
}

async fn linux_storage_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    command_exists_check(config, report, "df").await;
    if command_path(config, "df").await.is_some() {
        let output = run_command(config, "df", &["-hT"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "df -hT".to_string(),
                message: clip(&output.stdout, 2_000),
            });
        }
    }
    command_exists_check(config, report, "btrfs").await;
}

async fn macos_system_facts(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    fact_env(report, "env.shell", "SHELL");
    fact_env(report, "env.term", "TERM");
    fact_env(report, "env.lang", "LANG");
    let sw_vers = run_command(config, "sw_vers", &[], 2).await;
    if !sw_vers.stdout.trim().is_empty() {
        report
            .facts
            .insert("os.sw_vers".to_string(), json!(sw_vers.stdout.trim()));
    }
    let arch = run_command(config, "uname", &["-m"], 2).await;
    if !arch.stdout.trim().is_empty() {
        report
            .facts
            .insert("hardware.arch".to_string(), json!(arch.stdout.trim()));
    }
}

async fn macos_system_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    for command in ["sw_vers", "launchctl", "log", "system_profiler", "df"] {
        command_exists_check(config, report, command).await;
    }
}

async fn macos_app_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    let Some(target) = args.target.as_deref() else {
        report
            .next_questions
            .push("which app should I probe?".to_string());
        return;
    };
    match command_path(config, target).await {
        Some(path) => {
            report
                .facts
                .insert("app.command_path".to_string(), json!(path.clone()));
            report.checks.push(Check {
                id: "app.command_exists".to_string(),
                status: CheckStatus::Ok,
                detail: format!("{target} exists in PATH"),
                evidence: vec![path.clone()],
            });
            app_probe_version(config, report, target).await;
            app_probe_help(config, report, target).await;
            macos_quarantine_check(config, report, &path).await;
            macos_codesign_check(config, report, &path).await;
            node_runtime_if_relevant(config, report, target, &path).await;
        }
        None => {
            report.checks.push(Check {
                id: "app.command_exists".to_string(),
                status: CheckStatus::Error,
                detail: format!("{target} was not found in PATH"),
                evidence: Vec::new(),
            });
        }
    }
    macos_recent_logs(args, config, report, &[target, "error", "failed"]).await;
}

async fn macos_input_method_checks(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    let output = run_command(
        config,
        "defaults",
        &["read", "com.apple.HIToolbox", "AppleSelectedInputSources"],
        3,
    )
    .await;
    if !output.stdout.trim().is_empty() {
        report.logs.push(LogExcerpt {
            source: "AppleSelectedInputSources".to_string(),
            message: clip(&output.stdout, 2_000),
        });
    }
    if let Some(target) = args.target.as_deref() {
        process_check(config, report, target).await;
        macos_recent_logs(args, config, report, &[target, "InputMethodKit", "TIS"]).await;
    }
}

async fn macos_display_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    system_profiler_check(
        config,
        report,
        "SPDisplaysDataType",
        "display.system_profiler",
    )
    .await;
}

async fn macos_audio_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    system_profiler_check(config, report, "SPAudioDataType", "audio.system_profiler").await;
}

async fn macos_package_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    for command in ["brew", "port", "nix"] {
        command_exists_check(config, report, command).await;
    }
}

async fn macos_network_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    for command in ["ifconfig", "scutil", "networksetup"] {
        command_exists_check(config, report, command).await;
    }
    if command_path(config, "scutil").await.is_some() {
        let output = run_command(config, "scutil", &["--dns"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "scutil --dns".to_string(),
                message: clip(&output.stdout, 2_000),
            });
        }
    }
}

async fn macos_storage_checks(config: &DiagnosticsPluginConfig, report: &mut DiagnosticReport) {
    command_exists_check(config, report, "df").await;
    if command_path(config, "df").await.is_some() {
        let output = run_command(config, "df", &["-h"], 3).await;
        if !output.stdout.trim().is_empty() {
            report.logs.push(LogExcerpt {
                source: "df -h".to_string(),
                message: clip(&output.stdout, 2_000),
            });
        }
    }
}

async fn command_exists_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    name: &str,
) {
    let path = command_path(config, name).await;
    report.checks.push(Check {
        id: format!("command.{name}.exists"),
        status: if path.is_some() {
            CheckStatus::Ok
        } else {
            CheckStatus::Unknown
        },
        detail: if path.is_some() {
            format!("{name} is available")
        } else {
            format!("{name} is not available")
        },
        evidence: path.into_iter().collect(),
    });
}

async fn process_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    name: &str,
) -> Vec<u32> {
    let output = run_command(config, "pgrep", &["-af", name], 2).await;
    let matches = filtered_process_matches(&output.stdout, name);
    let found = output.status == Some(0) && !matches.is_empty();
    report.checks.push(Check {
        id: format!("process.{name}.running"),
        status: if found {
            CheckStatus::Ok
        } else {
            CheckStatus::Unknown
        },
        detail: if found {
            format!("process matching {name} is running")
        } else {
            format!("no process matching {name} was found")
        },
        evidence: if found {
            vec![clip(&matches.join("\n"), 1_000)]
        } else {
            Vec::new()
        },
    });
    matches
        .iter()
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect()
}

fn filtered_process_matches(output: &str, name: &str) -> Vec<String> {
    let mut matches = output
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains(&name.to_ascii_lowercase())
                && !lower.contains("pgrep -af")
                && !line_starts_with_pid(line, std::process::id())
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    matches.sort_by_key(|line| std::cmp::Reverse(process_match_score(line, name)));
    matches
}

fn line_starts_with_pid(line: &str, pid: u32) -> bool {
    line.split_whitespace()
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        == Some(pid)
}

fn process_match_score(line: &str, name: &str) -> usize {
    let lower = line.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut score = 0usize;
    if lower.contains(&format!("/{name} ")) || lower.ends_with(&format!("/{name}")) {
        score += 100;
    }
    if lower.contains(&format!(" {name} ")) || lower.ends_with(&format!(" {name}")) {
        score += 50;
    }
    if lower.contains("--type=zygote") || lower.contains("--type=renderer") {
        score = score.saturating_sub(30);
    }
    if lower.contains("clipsync") || lower.contains("helper") {
        score = score.saturating_sub(20);
    }
    if lower.contains("/tmp/.mount_") {
        score = score.saturating_sub(10);
    }
    score
}

fn linux_app_input_env(report: &mut DiagnosticReport, target: &str, pids: &[u32]) {
    let Some(pid) = pids.first() else {
        return;
    };
    let path = format!("/proc/{pid}/environ");
    let Ok(raw) = std::fs::read(&path) else {
        report.checks.push(Check {
            id: "input_method.app_env".to_string(),
            status: CheckStatus::Unknown,
            detail: format!("could not read environment for {target} pid {pid}"),
            evidence: Vec::new(),
        });
        return;
    };
    let mut picked = BTreeMap::new();
    for item in raw.split(|byte| *byte == 0) {
        let entry = String::from_utf8_lossy(item);
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        if matches!(
            key,
            "GTK_IM_MODULE"
                | "QT_IM_MODULE"
                | "XMODIFIERS"
                | "SDL_IM_MODULE"
                | "GLFW_IM_MODULE"
                | "XDG_SESSION_TYPE"
                | "WAYLAND_DISPLAY"
                | "DISPLAY"
        ) {
            picked.insert(key.to_string(), redact(value));
        }
    }
    let qt_ok = picked.get("QT_IM_MODULE").map(String::as_str) == Some("fcitx");
    report
        .facts
        .insert("input_method.app_env".to_string(), json!(picked));
    report.checks.push(Check {
        id: "input_method.app_env_qt_im_module".to_string(),
        status: if qt_ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: format!("checked input method environment for {target} pid {pid}"),
        evidence: vec![format!(
            "QT_IM_MODULE={}",
            if qt_ok {
                "fcitx"
            } else {
                "missing-or-different"
            }
        )],
    });
}

async fn linux_fcitx_package_checks(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
) {
    if command_path(config, "pacman").await.is_none() {
        return;
    }
    for package in ["fcitx5", "fcitx5-qt", "fcitx5-gtk"] {
        let output = run_command(config, "pacman", &["-Q", package], 2).await;
        report.checks.push(Check {
            id: format!("input_method.package.{package}"),
            status: if output.status == Some(0) {
                CheckStatus::Ok
            } else {
                CheckStatus::Warn
            },
            detail: if output.status == Some(0) {
                format!("{package} is installed")
            } else {
                format!("{package} is not confirmed installed")
            },
            evidence: compact_evidence(&output),
        });
    }
}

async fn systemd_user_active_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    service: &str,
) {
    if command_path(config, "systemctl").await.is_none() {
        return;
    }
    let output = run_command(config, "systemctl", &["--user", "is-active", service], 2).await;
    let active = output.stdout.trim() == "active";
    report.checks.push(Check {
        id: format!("systemd_user.{service}.active"),
        status: if active {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: format!("{service} is {}", output.stdout.trim()),
        evidence: compact_evidence(&output),
    });
}

async fn app_probe_version(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    target: &str,
) {
    let output = run_command(config, target, &["--version"], 3).await;
    report.checks.push(Check {
        id: "app.version_probe".to_string(),
        status: if output.status == Some(0) {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: format!("ran {target} --version"),
        evidence: compact_evidence(&output),
    });
}

async fn app_probe_help(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    target: &str,
) {
    let output = run_command(config, target, &["--help"], 3).await;
    report.checks.push(Check {
        id: "app.help_probe".to_string(),
        status: if output.status == Some(0) {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: format!("ran {target} --help"),
        evidence: compact_evidence(&output),
    });
    if output.status != Some(0) && !output.stderr.trim().is_empty() {
        report.findings.push(Finding {
            severity: Severity::High,
            title: format!("{target} returned an error during startup probe"),
            evidence: clip(&output.stderr, 1_000),
        });
    }
}

async fn linux_package_owner(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    path: &str,
) {
    if command_path(config, "pacman").await.is_none() {
        return;
    }
    let output = run_command(config, "pacman", &["-Qo", path], 3).await;
    if output.status == Some(0) {
        report
            .facts
            .insert("app.package_owner".to_string(), json!(output.stdout.trim()));
    }
}

async fn node_runtime_if_relevant(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    target: &str,
    path: &str,
) {
    let lower = format!("{} {}", target, path).to_ascii_lowercase();
    if !(lower.contains("node") || lower.contains("npm") || lower.contains("opencode")) {
        return;
    }
    for command in ["node", "npm", "pnpm", "bun"] {
        command_exists_check(config, report, command).await;
        if command_path(config, command).await.is_some() {
            let output = run_command(config, command, &["--version"], 3).await;
            report.facts.insert(
                format!("runtime.{command}.version"),
                json!(clip(output.stdout.trim(), 200)),
            );
        }
    }
}

async fn linux_recent_logs(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    keywords: &[&str],
) {
    if args.depth == Depth::Quick || command_path(config, "journalctl").await.is_none() {
        return;
    }
    let since = format!("-{}min", args.recent_minutes);
    let output = run_command(
        config,
        "journalctl",
        &["--user", "--since", &since, "--no-pager", "-n", "300"],
        5,
    )
    .await;
    push_filtered_log(report, "journalctl --user", &output.stdout, keywords);
}

async fn macos_recent_logs(
    args: &DiagnoseArgs,
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    keywords: &[&str],
) {
    if args.depth == Depth::Quick || command_path(config, "log").await.is_none() {
        return;
    }
    let last = format!("{}m", args.recent_minutes);
    let predicate = keywords
        .iter()
        .map(|keyword| format!("eventMessage CONTAINS[c] '{}'", keyword.replace('\'', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let output = run_command(
        config,
        "log",
        &[
            "show",
            "--last",
            &last,
            "--style",
            "compact",
            "--predicate",
            &predicate,
        ],
        6,
    )
    .await;
    push_filtered_log(report, "log show", &output.stdout, keywords);
}

fn push_filtered_log(report: &mut DiagnosticReport, source: &str, text: &str, keywords: &[&str]) {
    let mut lines = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if keywords
            .iter()
            .any(|keyword| lower.contains(&keyword.to_ascii_lowercase()))
        {
            lines.push(line);
        }
        if lines.len() >= 20 {
            break;
        }
    }
    if !lines.is_empty() {
        report.logs.push(LogExcerpt {
            source: source.to_string(),
            message: clip(&lines.join("\n"), 4_000),
        });
    }
}

async fn macos_quarantine_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    path: &str,
) {
    if command_path(config, "xattr").await.is_none() {
        return;
    }
    let output = run_command(config, "xattr", &["-p", "com.apple.quarantine", path], 2).await;
    if output.status == Some(0) && !output.stdout.trim().is_empty() {
        report.findings.push(Finding {
            severity: Severity::Medium,
            title: "target has macOS quarantine attribute".to_string(),
            evidence: output.stdout.trim().to_string(),
        });
    }
}

async fn macos_codesign_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    path: &str,
) {
    if command_path(config, "codesign").await.is_none() {
        return;
    }
    let output = run_command(config, "codesign", &["--verify", "--verbose", path], 4).await;
    report.checks.push(Check {
        id: "macos.codesign.verify".to_string(),
        status: if output.status == Some(0) {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: "codesign verification probe".to_string(),
        evidence: compact_evidence(&output),
    });
}

async fn system_profiler_check(
    config: &DiagnosticsPluginConfig,
    report: &mut DiagnosticReport,
    data_type: &str,
    source: &str,
) {
    if command_path(config, "system_profiler").await.is_none() {
        return;
    }
    let output = run_command(config, "system_profiler", &[data_type], 8).await;
    if !output.stdout.trim().is_empty() {
        report.logs.push(LogExcerpt {
            source: source.to_string(),
            message: clip(&output.stdout, 4_000),
        });
    }
}

async fn command_path(config: &DiagnosticsPluginConfig, command: &str) -> Option<String> {
    if !safe_command_name(command) {
        return None;
    }
    let script = format!("command -v {}", shell_escape(command));
    let output = run_command(config, "sh", &["-c", &script], 2).await;
    if output.status == Some(0) && !output.stdout.trim().is_empty() {
        return Some(output.stdout.trim().to_string());
    }
    let output = run_command(config, "which", &[command], 2).await;
    (output.status == Some(0) && !output.stdout.trim().is_empty())
        .then(|| output.stdout.trim().to_string())
}

async fn run_command(
    config: &DiagnosticsPluginConfig,
    program: &str,
    args: &[&str],
    timeout_seconds: u64,
) -> ProbeOutput {
    let seconds = timeout_seconds
        .max(1)
        .min(config.command_timeout_seconds.max(1));
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ProbeOutput {
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(error.to_string()),
                timed_out: false,
            };
        }
    };
    match timeout(Duration::from_secs(seconds), child.wait_with_output()).await {
        Ok(Ok(output)) => ProbeOutput {
            status: output.status.code(),
            stdout: redact(&clip(
                &String::from_utf8_lossy(&output.stdout),
                config.max_stdout_chars,
            )),
            stderr: redact(&clip(
                &String::from_utf8_lossy(&output.stderr),
                config.max_stderr_chars,
            )),
            error: None,
            timed_out: false,
        },
        Ok(Err(error)) => ProbeOutput {
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.to_string()),
            timed_out: false,
        },
        Err(_) => ProbeOutput {
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("command timed out after {seconds}s")),
            timed_out: true,
        },
    }
}

fn compact_evidence(output: &ProbeOutput) -> Vec<String> {
    let mut evidence = Vec::new();
    if !output.stdout.trim().is_empty() {
        evidence.push(clip(output.stdout.trim(), 1_000));
    }
    if !output.stderr.trim().is_empty() {
        evidence.push(clip(output.stderr.trim(), 1_000));
    }
    if let Some(error) = &output.error {
        evidence.push(error.clone());
    }
    if output.timed_out && output.error.is_none() {
        evidence.push("command timed out".to_string());
    }
    evidence
}

fn fact_env(report: &mut DiagnosticReport, fact: &str, key: &str) {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            report.facts.insert(fact.to_string(), json!(redact(&value)));
        }
    }
}

fn os_release_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name == key {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn extract_lspci_gpu_blocks(text: &str) -> String {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        let starts_device = line
            .chars()
            .next()
            .map(|ch| ch.is_ascii_hexdigit())
            .unwrap_or(false);
        if starts_device && !current.is_empty() {
            maybe_push_gpu_block(&mut blocks, &current);
            current.clear();
        }
        current.push(line.to_string());
    }
    if !current.is_empty() {
        maybe_push_gpu_block(&mut blocks, &current);
    }
    blocks.join("\n\n")
}

fn maybe_push_gpu_block(blocks: &mut Vec<String>, block: &[String]) {
    let header = block.first().map(String::as_str).unwrap_or_default();
    let lower = header.to_ascii_lowercase();
    if lower.contains("vga compatible controller")
        || lower.contains("3d controller")
        || lower.contains("display controller")
    {
        blocks.push(block.join("\n"));
    }
}

fn finalize_summary(report: &mut DiagnosticReport) {
    if !report.summary.is_empty() {
        return;
    }
    let errors = report
        .checks
        .iter()
        .filter(|check| matches!(check.status, CheckStatus::Error))
        .count();
    let warnings = report
        .checks
        .iter()
        .filter(|check| matches!(check.status, CheckStatus::Warn))
        .count();
    report.summary = if errors > 0 {
        format!(
            "context collection completed with {errors} error check(s) and {warnings} warning check(s)"
        )
    } else if warnings > 0 {
        format!("context collection completed with {warnings} warning check(s)")
    } else {
        "context collection completed without obvious errors in the selected probes".to_string()
    };
}

fn safe_command_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | '/'))
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}\n...[truncated]",
            text.chars().take(max_chars).collect::<String>()
        )
    }
}

fn redact(text: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    let mut output = text.to_string();
    if !home.is_empty() {
        output = output.replace(&home, "$HOME");
    }
    if !user.is_empty() {
        output = output.replace(&format!("/{user}/"), "/$USER/");
        output = output.replace(&format!("{user}@"), "$USER@");
    }
    for marker in [
        "TOKEN=",
        "API_KEY=",
        "PASSWORD=",
        "SECRET=",
        "ACCESS_TOKEN=",
        "AUTH=",
    ] {
        output = redact_after_marker(&output, marker);
    }
    output
}

fn redact_after_marker(input: &str, marker: &str) -> String {
    let mut output = String::new();
    for line in input.lines() {
        if let Some(pos) = line.find(marker) {
            output.push_str(&line[..pos + marker.len()]);
            output.push_str("[REDACTED]\n");
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    output.trim_end_matches('\n').to_string()
}

fn mask_network_addresses(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split_whitespace()
                .map(|token| {
                    if token.contains('/') && (token.contains('.') || token.contains(':')) {
                        "[ip/prefix]"
                    } else {
                        token
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
