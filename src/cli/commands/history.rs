use anyhow::Result;
use crate::cli::HistoryArgs;
use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::paths::GqyPaths;
use crate::render;
use crate::state::StateStore;

pub fn run_history(paths: &GqyPaths, args: HistoryArgs) -> Result<()> {
    let state = StateStore::new(paths)?;
    if let Some(query) = args.search.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        return run_history_search(paths, &state, query, args.limit, args.raw, args.no_thinking);
    }
    for entry in state.history(args.limit)? {
        if args.raw {
            println!("{}", serde_json::to_string(&entry)?);
            continue;
        }
        let display_role = if entry.role.ends_with("_clarification") {
            entry.role.trim_end_matches("_clarification")
        } else {
            entry.role.as_str()
        };
        println!("{} {display_role}", entry.timestamp);
        if entry.role.starts_with("assistant") {
            let response = crate::llm::ChatResult {
                content: entry.content,
                reasoning: if args.no_thinking {
                    None
                } else {
                    entry.reasoning
                },
                usage: None,
                usage_estimated: false,
                tool_calls: Vec::new(),
                provider_id: None,
                model: None,
            };
            render::print_assistant_response(&response, !args.no_thinking)?;
        } else {
            println!("{}", entry.content);
        }
        println!();
    }
    Ok(())
}

/// 关键词搜索会话记录：当前会话全部轮次 + 已归档轮次（evicted_turns）。
/// 只输出命中轮次，不占对话上下文；供 GQY 查「之前干了什么」。
fn run_history_search(
    paths: &GqyPaths,
    state: &StateStore,
    query: &str,
    limit: usize,
    raw: bool,
    no_thinking: bool,
) -> Result<()> {
    let needle = query.to_lowercase();
    let mut matched = Vec::new();

    // 当前会话全部轮次
    for entry in state.load_conversation()? {
        let hay = format!("{} {}", entry.role, entry.content)
            .to_lowercase();
        if hay.contains(&needle) {
            matched.push(entry);
        }
    }
    // 已归档轮次（记忆库 evicted_turns）
    if let Ok(config) = AppConfig::load(paths) {
        if config.memory_config().evicted_context_enabled {
            let store = crate::memory::MemoryStore::new(&config, paths);
            if let Ok(results) = store.search_evicted_context_readonly(query, 50) {
                if let Some(list) = results.get("results").and_then(serde_json::Value::as_array) {
                    for item in list {
                        // evicted 搜索结果用 snippet 字段承载命中片段
                        let content = item
                            .get("snippet")
                            .or_else(|| item.get("content"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        if content.to_lowercase().contains(&needle) {
                            matched.push(crate::state::StoredConversationEntry {
                                timestamp: item
                                    .get("timestamp")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                role: "archived".to_string(),
                                content: content.to_string(),
                                reasoning: None,
                            });
                        }
                    }
                }
            }
        }
    }

    let shown = matched.into_iter().rev().take(limit).rev().collect::<Vec<_>>();
    if shown.is_empty() {
        println!("{}", t("No matches.", "没有找到匹配的记录。"));
        return Ok(());
    }
    for entry in shown {
        if raw {
            println!("{}", serde_json::to_string(&entry)?);
            continue;
        }
        println!("{} {}", entry.timestamp, entry.role);
        if entry.role.starts_with("assistant") {
            let response = crate::llm::ChatResult {
                content: entry.content,
                reasoning: if no_thinking { None } else { entry.reasoning },
                usage: None,
                usage_estimated: false,
                tool_calls: Vec::new(),
                provider_id: None,
                model: None,
            };
            render::print_assistant_response(&response, !no_thinking)?;
        } else {
            println!("{}", entry.content);
        }
        println!();
    }
    Ok(())
}
