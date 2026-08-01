use super::{ToolRegistry, ToolSpec};
use crate::alarm::{self, AlarmRecord, AlarmStatus};
use crate::i18n::agent_text as t;
use crate::paths::GqyPaths;
use anyhow::{bail, Result};
use chrono::Local;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub fn register(registry: &mut ToolRegistry, paths: GqyPaths) {
    let pomo_paths = paths.clone();
    registry.register(ToolSpec::new(
        "pomodoro",
        t(
            "Start a pomodoro focus cycle: work 25 minutes then break 5 minutes, repeating until cancelled. Rings with GQY's sound at each boundary. Cancel with cancel_alarm.",
            "开启番茄钟专注循环：工作 25 分钟后休息 5 分钟，循环直到取消。每个阶段切换时用 GQY 内置声音提醒。取消用 cancel_alarm。",
        ),
        json!({
            "type": "object",
            "properties": {
                "work_minutes": { "type": "integer", "description": t("Work duration in minutes, default 25.", "工作时长（分钟），默认 25。") },
                "break_minutes": { "type": "integer", "description": t("Break duration in minutes, default 5.", "休息时长（分钟），默认 5。") }
            },
            "required": [],
            "additionalProperties": false
        }),
        move |args| {
            let paths = pomo_paths.clone();
            async move { start_pomodoro(args, paths).await }
        },
    ).writes());
    let set_paths = paths.clone();
    registry.register(ToolSpec::new(
        "set_alarm",
        t(
            "Set a local alarm, countdown, or repeating reminder. Accepts duration like 30s, 10m, 1h 30m, or a time like 14:30. Set repeat to a duration to ring every interval (e.g. pomodoro 25m / break 5m). Runs in a background GQY process with GQY's embedded sound.",
            "设置本地闹钟、倒计时或周期提醒。支持 30s、10m、1h 30m 或 14:30。repeat 传时长可周期性响铃（如番茄钟 25m / 休息 5m）。在后台 GQY 进程运行并使用内置声音。",
        ),
        json!({
            "type": "object",
            "properties": {
                "time": { "type": "string", "description": t("Duration or clock time.", "时长或时钟时间。") },
                "label": { "type": "string", "description": t("Optional alarm label.", "可选闹钟标签。") },
                "repeat": { "type": "string", "description": t("Optional repeat interval (e.g. 25m, 1h). Rings every interval until cancelled.", "可选重复间隔（如 25m、1h）。按间隔周期性响铃直到取消。") },
                "audio_file": { "type": "string", "description": t("Optional local .wav or .mp3 audio file to play instead of GQY's built-in alarm sound.", "可选本地 .wav 或 .mp3 音频文件，用它替代 GQY 内置闹钟音。") }
            },
            "required": ["time"],
            "additionalProperties": false
        }),
        move |args| {
            let paths = set_paths.clone();
            async move { set_alarm(args, paths).await }
        },
    ).writes());
    let list_paths = paths.clone();
    registry.register(ToolSpec::new(
        "list_alarms",
        t(
            "List currently scheduled or ringing local alarms.",
            "列出当前已设定或正在响的本地闹钟。",
        ),
        json!({"type":"object","properties":{},"additionalProperties":false}),
        move |_args| {
            let paths = list_paths.clone();
            async move { list_alarms(paths).await }
        },
    ));
    let cancel_paths = paths.clone();
    registry.register(ToolSpec::new(
        "cancel_alarm",
        t(
            "Cancel a scheduled or ringing alarm by id. Use list_alarms first if the id is unknown.",
            "按 id 取消已设定或正在响的闹钟。不知道 id 时先用 list_alarms。",
        ),
        json!({"type":"object","properties":{"id":{"type":"string","description":t("Alarm id from set_alarm or list_alarms.","set_alarm 或 list_alarms 返回的闹钟 id。")}},"required":["id"],"additionalProperties":false}),
        move |args| {
            let paths = cancel_paths.clone();
            async move { cancel_alarm(args, paths).await }
        },
    ).writes());
}

/// 番茄钟：工作阶段闹钟（每 work+break 分钟响一次），随后自动排一个休息闹钟。
/// 两个闹钟都带 label 便于用户识别；取消任意一个即停止循环。
async fn start_pomodoro(args: Value, paths: GqyPaths) -> Result<String> {
    let work_minutes = args
        .get("work_minutes")
        .and_then(Value::as_u64)
        .unwrap_or(25)
        .clamp(1, 180);
    let break_minutes = args
        .get("break_minutes")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 60);
    let work_seconds = work_minutes * 60;
    let break_seconds = break_minutes * 60;

    // 工作结束（同时排下一个工作）= 每 (work+break) 分钟响一次
    let work_alarm = set_alarm_impl_full(
        &format!("{work_seconds}s"),
        "番茄钟：工作结束，休息一下",
        work_seconds + break_seconds,
        20,
        None,
        &paths,
    )
    .await?;
    // 休息结束 = 工作开始后 work 分钟响一次
    let break_alarm = set_alarm_impl_full(
        &format!("{work_seconds}s"),
        "番茄钟：休息结束，继续工作",
        work_seconds + break_seconds,
        20,
        None,
        &paths,
    )
    .await?;

    Ok(json!({
        "ok": true,
        "work_minutes": work_minutes,
        "break_minutes": break_minutes,
        "work_alarm": work_alarm,
        "break_alarm": break_alarm,
        "note": "每个阶段切换都会响铃；cancel_alarm 可停止任一个。"
    })
    .to_string())
}

/// 底层调度实现：解析 time/repeat，spawn 后台 worker，落库。
async fn set_alarm_impl(
    time: &str,
    label: &str,
    repeat_seconds: u64,
    audio_file: Option<PathBuf>,
    paths: &GqyPaths,
) -> Result<String> {
    set_alarm_impl_full(time, label, repeat_seconds, 20, audio_file, paths).await
}

async fn set_alarm_impl_full(
    time: &str,
    label: &str,
    repeat_seconds: u64,
    max_rings: u64,
    audio_file: Option<PathBuf>,
    paths: &GqyPaths,
) -> Result<String> {
    let due_at = alarm::due_at_from_time(time)?;
    let id = format!(
        "alarm-{}-{}",
        Local::now().timestamp_millis(),
        std::process::id()
    );
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg("__alarm-worker")
        .arg("--id")
        .arg(&id)
        .arg("--time")
        .arg(time)
        .arg("--label")
        .arg(label)
        .arg("--state-dir")
        .arg(&paths.state_dir)
        .arg("--cache-dir")
        .arg(&paths.cache_dir)
        .arg("--repeat")
        .arg(repeat_seconds.to_string())
        .arg("--max-rings")
        .arg(max_rings.to_string())
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    if let Some(path) = &audio_file {
        command.arg("--audio-file").arg(path);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pid = child.id();
    alarm::upsert(
        paths,
        AlarmRecord {
            id: id.clone(),
            label: label.to_string(),
            time: time.to_string(),
            audio_file: audio_file.clone(),
            due_at,
            pid,
            status: AlarmStatus::Scheduled,
            repeat_seconds,
            max_rings,
        },
    )?;
    Ok(id)
}

async fn set_alarm(args: Value, paths: GqyPaths) -> Result<String> {
    let time = args
        .get("time")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if time.is_empty() {
        bail!("time is required")
    }
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("GQY alarm")
        .trim();
    let audio_file = args
        .get("audio_file")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(resolve_audio_file)
        .transpose()?;
    let repeat_seconds = args
        .get("repeat")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| alarm::parse_alarm_seconds(value).map_err(anyhow::Error::from))
        .transpose()?
        .unwrap_or(0);
    let max_rings = args
        .get("max_rings")
        .and_then(Value::as_u64)
        .unwrap_or(20);
    let id = set_alarm_impl_full(
        time,
        label,
        repeat_seconds,
        max_rings,
        audio_file.clone(),
        &paths,
    )
    .await?;
    let due_at = alarm::due_at_from_time(time)?;
    Ok(json!({
        "ok": true,
        "id": id,
        "time": time,
        "label": label,
        "repeat": repeat_seconds,
        "audio_file": audio_file,
        "due_at": due_at,
        "due_at_local": alarm::format_due_at(due_at),
    })
    .to_string())
}

async fn list_alarms(paths: GqyPaths) -> Result<String> {
    let records = alarm::cleanup_dead(&paths)?;
    let alarms = records
        .into_iter()
        .map(|record| {
            json!({
                "id": record.id,
                "label": record.label,
                "time": record.time,
                "audio_file": record.audio_file,
                "due_at": record.due_at,
                "due_at_local": alarm::format_due_at(record.due_at),
                "pid": record.pid,
                "status": record.status,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"ok": true, "alarms": alarms}).to_string())
}

async fn cancel_alarm(args: Value, paths: GqyPaths) -> Result<String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if id.is_empty() {
        bail!("id is required")
    }
    let cancelled = alarm::cancel(&paths, id)?;
    Ok(json!({"ok": cancelled, "id": id, "removed": cancelled}).to_string())
}

fn resolve_audio_file(value: &str) -> Result<PathBuf> {
    let path = expand_path(value.trim());
    let canonical = path.canonicalize()?;
    if !canonical.is_file() {
        bail!("audio_file is not a regular file: {}", path.display())
    }
    let extension = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "wav" | "mp3") {
        bail!("audio_file must be a .wav or .mp3 file")
    }
    Ok(canonical)
}

fn expand_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
            return home.join(rest);
        }
    }
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
