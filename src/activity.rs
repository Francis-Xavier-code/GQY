//! 活动日志（activity log）：记录 GQY 干了什么（工具调用、子代理、关键事件）。
//!
//! 设计原则：**默认不进 LLM 上下文**（零 token 开销），落盘为追加式 JSONL，
//! 需要时通过 `gqy activity [--search <词>]` 查询。供顾清影复盘、排查、自我回顾用。

use crate::paths::GqyPaths;
use anyhow::Result;
use chrono::Local;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

/// 活动日志文件：`<cache>/logs/activity.jsonl`（与 tracing 日志同目录，互不干扰）。
pub fn activity_log_file(paths: &GqyPaths) -> PathBuf {
    paths.logs_dir().join("activity.jsonl")
}

/// 追加一条活动记录。任何失败都静默忽略（日志不能影响主流程）。
pub fn record(paths: &GqyPaths, event: &str, detail: &Value) {
    let file = activity_log_file(paths);
    if std::fs::create_dir_all(file.parent().unwrap_or(std::path::Path::new("."))).is_err()
    {
        return;
    }
    let entry = json!({
        "ts": Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "event": event,
        "detail": detail,
    });
    let mut handle = match OpenOptions::new().create(true).append(true).open(&file) {
        Ok(handle) => handle,
        Err(_) => return,
    };
    let _ = writeln!(handle, "{}", serde_json::to_string(&entry).unwrap_or_default());
}

/// 便捷记录：工具调用。
pub fn record_tool(paths: &GqyPaths, name: &str, ok: bool) {
    record(
        paths,
        "tool",
        &json!({ "name": name, "ok": ok }),
    );
}

/// 便捷记录：子代理完成。
pub fn record_subagent(paths: &GqyPaths, kind: &str, steps: usize, ok: bool) {
    record(
        paths,
        "subagent",
        &json!({ "kind": kind, "steps": steps, "ok": ok }),
    );
}

/// 查询活动日志：倒序（新的在前），可选关键词过滤，limit 条。
pub fn query(paths: &GqyPaths, search: Option<&str>, limit: usize) -> Result<Vec<Value>> {
    let file = activity_log_file(paths);
    if !file.is_file() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&file)?;
    let needle = search.map(str::to_lowercase);
    let mut entries = Vec::new();
    for line in content.lines().rev() {
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            if let Some(needle) = &needle {
                let hay = serde_json::to_string(&entry).unwrap_or_default().to_lowercase();
                if !hay.contains(needle) {
                    continue;
                }
            }
            entries.push(entry);
            if entries.len() >= limit {
                break;
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_queries_with_search() {
        let root = tempfile::tempdir().unwrap().into_path();
        let _env_guard = crate::paths::test_env::GQY_HOME_LOCK.lock().unwrap();
        let old = std::env::var_os("GQY_HOME");
        std::env::set_var("GQY_HOME", &root);
        let paths = crate::paths::GqyPaths::new().unwrap();

        record_tool(&paths, "read_file", true);
        record_tool(&paths, "apply_patch", false);
        record_subagent(&paths, "explore", 7, true);

        let all = query(&paths, None, 20).unwrap();
        assert_eq!(all.len(), 3);
        // 倒序：最新的在前
        assert_eq!(all[0]["event"], "subagent");
        assert_eq!(all[0]["detail"]["kind"], "explore");

        let tools = query(&paths, Some("apply_patch"), 20).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["detail"]["ok"], false);

        if let Some(v) = old {
            std::env::set_var("GQY_HOME", v);
        } else {
            std::env::remove_var("GQY_HOME");
        }
    }
}
