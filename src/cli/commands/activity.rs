use anyhow::Result;
use crate::cli::ActivityArgs;
use crate::i18n::text as t;
use crate::paths::GqyPaths;

/// 查看活动日志（`gqy activity`）：GQY 干了什么的流水账。
/// 默认不进对话上下文（零 token），需要时查询。
pub fn run_activity(paths: &GqyPaths, args: ActivityArgs) -> Result<()> {
    let entries = crate::activity::query(paths, args.search.as_deref(), args.limit)?;
    if entries.is_empty() {
        println!("{}", t("No activity recorded yet.", "还没有活动记录。"));
        return Ok(());
    }
    for entry in entries {
        let ts = entry.get("ts").and_then(serde_json::Value::as_str).unwrap_or("");
        let event = entry.get("event").and_then(serde_json::Value::as_str).unwrap_or("");
        let detail = entry.get("detail").cloned().unwrap_or(serde_json::Value::Null);
        let detail = if detail.is_null() {
            String::new()
        } else {
            format!(" {}", serde_json::to_string(&detail).unwrap_or_default())
        };
        println!("{ts} [{event}]{detail}");
    }
    Ok(())
}
