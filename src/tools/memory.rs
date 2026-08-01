use super::{ToolRegistry, ToolSpec};
use crate::config::AppConfig;
use crate::i18n::agent_text as t;
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub fn register(registry: &mut ToolRegistry, config: AppConfig, paths: GqyPaths) {
    if !config.memory_config().enabled {
        return;
    }
    register_readonly(registry, config.clone(), paths.clone());
    registry.register(ToolSpec::new(
        "remember_fact",
        t("Save a durable memory fact or useful knowledge point for future association. Use only for reusable facts, preferences, methods, or stable discoveries.", "保存长期记忆事实或有用知识点，供之后联想使用。仅用于可复用事实、偏好、方法或稳定发现。"),
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": t("The concise fact or knowledge point to remember.", "要记住的简洁事实或知识点。") },
                "source": { "type": "string", "description": t("Optional source label.", "可选来源标签。") }
            },
            "required": ["content"],
            "additionalProperties": false
        }),
        {
            let config = config.clone();
            let paths = paths.clone();
            move |args| {
                let config = config.clone();
                let paths = paths.clone();
                async move { remember_fact(args, config, paths).await }
            }
        },
    ).writes());
    // 情绪感知：记录当前心情（心情日志），供后续回忆关联。
    // 只在情感场景激活，不参与代码任务；写入极简文本，不耗额外 token。
    let mood_config = config.clone();
    let mood_paths = paths.clone();
    registry.register(ToolSpec::new(
        "log_mood",
        t(
            "Record the current mood or emotional state as a diary entry. Use sparingly in emotional or personal conversations; not for coding tasks.",
            "把当前心情或情绪状态记入心情日志。仅在情感/闲聊场景使用，不要用于代码任务。",
        ),
        json!({
            "type": "object",
            "properties": {
                "mood": { "type": "string", "description": t("Mood in a few words, e.g. 平静 / 开心 / 疲惫 / 烦躁.", "几个字描述心情，如 平静 / 开心 / 疲惫 / 烦躁。") },
                "note": { "type": "string", "description": t("Optional short context note.", "可选简短背景说明。") }
            },
            "required": ["mood"],
            "additionalProperties": false
        }),
        {
            let config = mood_config;
            let paths = mood_paths;
            move |args| {
                let config = config.clone();
                let paths = paths.clone();
                async move { log_mood(args, config, paths).await }
            }
        },
    ).writes());
    // 情绪感知：查看最近心情记录（心情日志回看）。
    let recall_mood_config = config.clone();
    let recall_mood_paths = paths.clone();
    registry.register(ToolSpec::new(
        "recall_mood",
        t("Show recent mood diary entries, newest first.", "查看最近的心情日志记录，新的在前。"),
        json!({
            "type": "object",
            "properties": {
                "max_results": { "type": "integer", "description": t("Optional result limit.", "可选结果数量限制。") }
            },
            "required": [],
            "additionalProperties": false
        }),
        {
            let config = recall_mood_config;
            let paths = recall_mood_paths;
            move |args| {
                let config = config.clone();
                let paths = paths.clone();
                async move { recall_mood(args, config, paths).await }
            }
        },
    ));
}

pub fn register_readonly(registry: &mut ToolRegistry, config: AppConfig, paths: GqyPaths) {
    if !config.memory_config().enabled {
        return;
    }
    if config.memory_config().evicted_context_enabled {
        registry.register(ToolSpec::new(
            "search_evicted_context",
            t("Search conversation turns that were moved out of the active context window. Use this when the current context appears to be missing earlier discussion.", "搜索已经移出当前上下文窗口的对话轮次。当当前上下文明显缺少早前讨论时使用。"),
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": t("Search keywords or question.", "搜索关键词或问题。") },
                    "max_results": { "type": "integer", "description": t("Optional result limit.", "可选结果数量限制。") }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            {
                let config = config.clone();
                let paths = paths.clone();
                move |args| {
                    let config = config.clone();
                    let paths = paths.clone();
                    async move { search_evicted_context(args, config, paths).await }
                }
            },
        ));
    }
    registry.register(ToolSpec::new(
        "recall_past_events",
        t("Search the assistant's diary-like memory of things that happened in previous conversations.", "搜索助手对过往对话事件的日记式记忆。"),
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": t("Search keywords or question.", "搜索关键词或问题。") },
                "max_results": { "type": "integer", "description": t("Optional result limit.", "可选结果数量限制。") }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        {
            let config = config.clone();
            let paths = paths.clone();
            move |args| {
                let config = config.clone();
                let paths = paths.clone();
                async move { recall_past_events(args, config, paths).await }
            }
        },
    ));
    registry.register(ToolSpec::new(
        "recall_memories",
        t("Search remembered facts and past events, including forgotten memories when requested. This read-only tool does not change memory state.", "搜索已记住的事实和过往事件；需要时也可包含已遗忘记忆。此只读工具不会改变记忆状态。"),
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": t("Search keywords or question.", "搜索关键词或问题。") },
                "max_results": { "type": "integer", "description": t("Optional result limit.", "可选结果数量限制。") },
                "include_forgotten": { "type": "boolean", "description": t("Whether to include forgotten memories.", "是否包含已遗忘记忆。") }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        {
            let config = config.clone();
            let paths = paths.clone();
            move |args| {
                let config = config.clone();
                let paths = paths.clone();
                async move { recall_memories(args, config, paths).await }
            }
        },
    ));
}

async fn search_evicted_context(
    args: Value,
    config: AppConfig,
    paths: GqyPaths,
) -> Result<String> {
    let query = required_str(&args, "query")?;
    let limit = optional_limit(&args);
    let store = MemoryStore::new(&config, &paths);
    Ok(store
        .search_evicted_context_readonly(query, limit)?
        .to_string())
}

async fn recall_past_events(args: Value, config: AppConfig, paths: GqyPaths) -> Result<String> {
    let query = required_str(&args, "query")?;
    let limit = optional_limit(&args);
    let store = MemoryStore::new(&config, &paths);
    Ok(store.recall_past_events_readonly(query, limit)?.to_string())
}

async fn remember_fact(args: Value, config: AppConfig, paths: GqyPaths) -> Result<String> {
    let content = required_str(&args, "content")?;
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("conversation");
    let store = MemoryStore::new(&config, &paths);
    let id = store.remember_fact(content, source)?;
    Ok(json!({
        "ok": true,
        "id": id,
        "source": source.trim(),
        "content": content.trim(),
        "message": t("Memory saved. The saved content is included here so the current conversation can refer to it accurately.", "记忆已保存。这里包含已保存内容，方便当前对话准确引用。")
    })
    .to_string())
}

async fn recall_memories(args: Value, config: AppConfig, paths: GqyPaths) -> Result<String> {
    let query = required_str(&args, "query")?;
    let limit = optional_limit(&args);
    let include_forgotten = args
        .get("include_forgotten")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let store = MemoryStore::new(&config, &paths);
    Ok(store
        .recall_memories_readonly(query, limit, include_forgotten)?
        .to_string())
}

fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    let value = args
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if value.is_empty() {
        bail!("{}: {name}", t("required argument missing", "缺少必需参数"));
    }
    Ok(value)
}

fn optional_limit(args: &Value) -> usize {
    args.get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 50) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn test_paths() -> GqyPaths {
        let root = PathBuf::from("/tmp/gqy-memory-tool-test");
        GqyPaths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.jsonc"),
            skills_dir: root.join("config/skills"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            state_dir: root.join("state"),
            pictures_dir: root.join("pictures"),
            fish_hook_file: root.join("fish-hook.fish"),
            bash_hook_file: root.join("bash-hook.sh"),
            zsh_hook_file: root.join("zsh-hook.zsh"),
            scripts_dir: root.join("config/scripts"),
            system_scripts_dir: PathBuf::new(),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        }
    }

    fn tool_names(registry: &ToolRegistry) -> BTreeSet<String> {
        registry
            .lazy_definitions(&BTreeSet::new())
            .into_iter()
            .map(|definition| definition.function.name)
            .collect()
    }

    #[test]
    fn search_evicted_context_is_available_for_manual_pop_with_compact_overflow() {
        let paths = test_paths();
        let compact_config = AppConfig::default();
        let mut compact_registry = ToolRegistry::new();
        register_readonly(&mut compact_registry, compact_config, paths.clone());
        assert!(tool_names(&compact_registry).contains("search_evicted_context"));

        let mut pop_config = AppConfig::default();
        pop_config.context.on_overflow = "pop".to_string();
        let mut pop_registry = ToolRegistry::new();
        register_readonly(&mut pop_registry, pop_config, paths);
        assert!(tool_names(&pop_registry).contains("search_evicted_context"));
    }
}

async fn log_mood(args: Value, config: AppConfig, paths: GqyPaths) -> Result<String> {
    let mood = args
        .get("mood")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if mood.is_empty() {
        bail!("mood is required")
    }
    let note = args
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let content = match note {
        Some(note) => format!("心情：{mood}（{note}）"),
        None => format!("心情：{mood}"),
    };
    let store = MemoryStore::new(&config, &paths);
    let id = store.remember_episode(&content, "mood")?;
    Ok(json!({ "ok": true, "id": id, "content": content }).to_string())
}

async fn recall_mood(args: Value, config: AppConfig, paths: GqyPaths) -> Result<String> {
    let limit = optional_limit(&args).max(1).min(50);
    let store = MemoryStore::new(&config, &paths);
    Ok(store.recent_moods(limit)?.to_string())
}
