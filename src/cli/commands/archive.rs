use anyhow::Result;
use crate::cli::ArchiveArgs;
use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::paths::GqyPaths;
use crate::state::StateStore;

/// 记忆定期归档：把超过保留期的可见轮次归档到 evicted_context.db
/// （不占对话上下文，随时可用 `gqy history --search` 或 recall 检索）。
/// 返回归档的轮次数。
pub fn run_archive(paths: &GqyPaths, args: ArchiveArgs) -> Result<()> {
    let config = AppConfig::load(paths)?;
    let state = StateStore::new(paths)?;
    let memory = crate::memory::MemoryStore::new(&config, paths);

    let turns = state.load_visible_turns()?;
    let non_summary = turns
        .iter()
        .filter(|turn| !turn.is_summary)
        .collect::<Vec<_>>();
    if non_summary.is_empty() {
        println!("{}", t("Nothing to archive.", "没有可归档的轮次。"));
        return Ok(());
    }

    let cutoff = chrono::Local::now().timestamp() - (args.keep_days as i64 * 86400);
    let mut to_archive = Vec::new();
    for turn in &non_summary {
        let ts = turn
            .user_timestamp
            .parse::<i64>()
            .or_else(|_| {
                chrono::DateTime::parse_from_rfc3339(&turn.user_timestamp)
                    .map(|dt| dt.timestamp())
            })
            .unwrap_or(i64::MAX);
        if ts < cutoff {
            to_archive.push(*turn);
        }
    }
    if to_archive.is_empty() && !args.force {
        println!(
            "{}",
            t(
                "No turns older than the retention window.",
                "没有超过保留期的轮次。"
            )
        );
        return Ok(());
    }
    if to_archive.is_empty() {
        // force：归档最旧的一半（保守）
        let half = non_summary.len() / 2;
        to_archive = non_summary[..half.max(1)].to_vec();
    }

    let owned = to_archive.iter().map(|turn| (*turn).clone()).collect::<Vec<_>>();
    let (_, evicted) = crate::agent::evicted_turn_entries_for_archive(&owned);
    let turn_ids = to_archive
        .iter()
        .map(|turn| turn.turn_id.clone())
        .collect::<Vec<_>>();
    let archive_db = memory.prepare_evicted_context_db()?;
    let archived = match archive_db {
        Some(db) => state.archive_and_delete_visible_turns(&db, &evicted, &turn_ids, None)?,
        None => {
            state.delete_visible_turns(&turn_ids)?;
            to_archive.len()
        }
    };
    if crate::i18n::is_zh() {
        println!("已归档 {archived} 个轮次（早于 {} 天保留期）。", args.keep_days);
    } else {
        println!(
            "Archived {archived} turns (older than {}-day window).",
            args.keep_days
        );
    }
    crate::activity::record(
        paths,
        "archive",
        &serde_json::json!({ "turns": archived, "keep_days": args.keep_days }),
    );
    Ok(())
}

/// 自动归档（对话轮次开始前调用）：距上次归档超过阈值且有过期轮次时执行。
/// 节流标记在 state 目录，默认 7 天检查一次；静默失败（不阻塞对话）。
pub fn maybe_auto_archive(paths: &GqyPaths, keep_days: u64) {
    let marker = paths.state_dir.join("last_archive_check");
    let interval_secs = (keep_days.max(1) as i64) * 86400;
    if let Ok(text) = std::fs::read_to_string(&marker) {
        if let Ok(last) = text.trim().parse::<i64>() {
            if chrono::Local::now().timestamp() - last < interval_secs {
                return;
            }
        }
    }
    let _ = std::fs::write(&marker, chrono::Local::now().timestamp().to_string());
    let _ = run_archive(
        paths,
        ArchiveArgs {
            keep_days,
            force: false,
        },
    );
}
