mod alarm;
mod apply_patch;
mod ask_question;
mod calculator;
mod clipboard;
mod deep_research;
mod deepseek_status;
mod default_tools;
mod diagnostics;
mod edit_replace;
mod exchange_rate;
mod hash_codec;
mod image_generation;
mod kitty_image;
pub mod import;
pub mod knowledge_base;
mod load_tools;
mod local_vision;
mod speech;
mod man;
mod mcp;
pub(crate) mod memes;
mod memory;
mod moegirl;
pub(crate) mod path_guard;
mod patch_preview;
pub(crate) mod providers;
mod registry;
mod scripts;
mod skills;
mod subagent_runner;
mod task;
mod todowrite;
pub mod tool_descriptions;
pub mod vision;
mod weather;
mod web;
mod web_images;
mod write;
mod xuanxue;

use crate::config::AppConfig;
use crate::i18n::{is_zh, text as t};
use crate::paths::GqyPaths;
use std::collections::HashMap;
use std::sync::RwLock;

#[allow(unused_imports)]
pub use registry::{
    empty_parameters, CommandOutputStream, ToolPermission, ToolProgress, ToolProgressEvent,
    ToolRegistry, ToolSpec,
};
pub(crate) use scripts::rescan_scripts;
pub use skills::register_skills;

pub fn register_ask_question(registry: &mut ToolRegistry) {
    ask_question::register(registry);
}

static SCRIPT_DISPLAY_NAMES: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

pub fn register_script_display_names(registry: &ToolRegistry) {
    let mut map = HashMap::new();
    for name in registry.tool_names() {
        if let Some(dn) = registry.display_name(&name) {
            map.insert(name, dn);
        }
    }
    *SCRIPT_DISPLAY_NAMES.write().unwrap() = Some(map);
}

pub fn readable_tool_name(name: &str) -> String {
    if let Some(skill) = name.strip_prefix("load_skill:") {
        return if is_zh() {
            format!("加载技能：{skill}")
        } else {
            format!("Load skill: {skill}")
        };
    }
    if let Some(tools) = name.strip_prefix("load_tools:") {
        let targets = tools
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        let display = targets
            .iter()
            .map(|name| readable_load_target_name(name))
            .collect::<Vec<_>>()
            .join(if is_zh() { "、" } else { ", " });
        return if is_zh() {
            format!("加载：{display}")
        } else {
            format!("Load: {display}")
        };
    }
    if let Some(display_name) = builtin_readable_tool_name(name) {
        return display_name.to_string();
    }
    if let Ok(guard) = SCRIPT_DISPLAY_NAMES.read() {
        if let Some(map) = guard.as_ref() {
            if let Some(dn) = map.get(name) {
                return dn.clone();
            }
        }
    }
    name.to_string()
}

fn readable_load_target_name(name: &str) -> String {
    if let Some(group) = name.strip_prefix("group:") {
        return builtin_readable_group_name(group)
            .map(str::to_string)
            .unwrap_or_else(|| format!("group:{group}"));
    }
    readable_tool_name(name)
}

fn builtin_readable_tool_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "run_command" => t("Run command", "运行命令"),
        "apply_patch" => t("Apply patch", "应用补丁"),
        "ask_question" => t("Ask user", "询问用户"),
        "task" => t("Subagent", "子代理"),
        "read_file" => t("Read file", "读取文件"),
        "write_file" => t("Write file", "写入文件"),
        "edit_file" => t("Edit file", "编辑文件"),
        "edit_string" => t("Edit string", "字符串编辑"),
        "list_directory" => t("List directory", "列目录"),
        "create_directory" => t("Create directory", "创建目录"),
        "trash_path" => t("Move to trash", "移入回收站"),
        "glob" => t("Find files", "查找文件"),
        "grep" => t("Search text", "搜索文本"),
        "get_current_directory" => t("Current directory", "当前目录"),
        "get_current_time" => t("Current time", "当前时间"),
        "check_issue" => t("Check issue", "检查问题"),
        "check_os_info" => t("System information", "查看系统信息"),
        "read_clipboard" => t("Read clipboard", "读取剪贴板"),
        "web_search" => t("Web search", "网页搜索"),
        "web_fetch" => t("Fetch webpage", "读取网页"),
        "search_web_images" => t("Search images", "搜索图片"),
        "analyze_image" | "vision_analyze" => t("Analyze image", "分析图片"),
        "print_image" => t("Display image", "显示图片"),
        "generate_image" => t("Generate image", "生成图片"),
        "search_meme" => t("Search memes", "搜索表情包"),
        "show_meme" => t("Send meme", "发送表情"),
        "add_meme" => t("Add meme", "添加表情包"),
        "update_meme" => t("Update meme", "更新表情包"),
        "delete_meme" => t("Delete meme", "删除表情包"),
        "deep_research" => t("Deep research", "深度研究"),
        "upload_knowledge_base_file" | "upload_text_to_knowledge_base" => {
            t("Import knowledge base", "导入知识库")
        }
        "read_knowledge_base_file" => t("Read knowledge base", "读取知识库"),
        "search_knowledge_base" => t("Search knowledge base", "搜索知识库"),
        "search_knowledge_base_by_name" => t("Search knowledge base by name", "按名称搜索知识库"),
        "edit_knowledge_base_file" => t("Edit knowledge base", "编辑知识库"),
        "remove_knowledge_base_file" => t("Remove from knowledge base", "移除知识库"),
        "list_knowledge_base_files" => t("List knowledge base", "列出知识库"),
        "set_alarm" => t("Set alarm", "设置闹钟"),
        "list_alarms" => t("List alarms", "列出闹钟"),
        "cancel_alarm" => t("Cancel alarm", "取消闹钟"),
        "remember_fact" => t("Remember fact", "记录记忆"),
        "search_evicted_context" => t("Search old context", "搜索旧上下文"),
        "recall_past_events" => t("Recall past events", "回忆往事"),
        "recall_memory" | "recall_memories" => t("Recall memories", "召回记忆"),
        "forget_memory" | "forget_memories" => t("Forget memories", "删除记忆"),
        "list_memory" | "list_memories" => t("List memories", "列出记忆"),
        "query_deepseek_status" => t("Check DeepSeek status", "查询 DeepSeek 状态"),
        "online_man_search" | "man_search" => t("Search online manuals", "搜索在线手册"),
        "online_man_get_page" | "man_read" => t("Read online manual", "读取在线手册"),
        "moegirl_query" | "query_moegirl" => t("Query Moegirlpedia", "查询萌娘百科"),
        "calculate" | "calculator" | "scientific_calculator" => {
            t("Scientific calculation", "科学计算")
        }
        "calculate_hash" => t("Calculate hash", "计算哈希"),
        "decode_encoded_text" => t("Decode text", "解码文本"),
        "exchange_rate" | "get_exchange_rate" => t("Exchange rates", "汇率查询"),
        "weather" | "get_weather" => t("Weather", "天气查询"),
        "xuanxue_pick" => t("Divination choice", "玄学选择"),
        "xuanxue_divine" => t("Divination", "玄学占卜"),
        "draw_zhouyi_hexagram" => t("Draw I Ching hexagram", "周易起卦"),
        "draw_tarot_card" => t("Draw tarot card", "抽塔罗牌"),
        "draw_fortune_lot" => t("Draw fortune", "吉凶占"),
        "roll_dice" => t("Roll dice", "掷骰子"),
        "load_skill" => t("Load skill", "加载技能"),
        "load_tools" => t("Load", "加载"),
        "register_script" => t("Register script", "注册脚本"),
        "unregister_script" => t("Unregister script", "注销脚本"),
        "todowrite" => t("Todo list", "任务列表"),
        "todoupdate" => t("Update todos", "更新任务"),
        "register_deep_research_topic_title" => t("Register research title", "注册研究标题"),
        "register_deep_research_reference" => t("Register reference", "注册引用来源"),
        "remove_deep_research_reference" => t("Remove reference", "移除引用来源"),
        _ => return None,
    })
}

fn builtin_readable_group_name(group: &str) -> Option<&'static str> {
    Some(match group {
        "acg" => t("ACG tools", "ACG 工具组"),
        "agent" => t("Subagent tools", "子代理工具组"),
        "alarms" => t("Alarm tools", "闹钟工具组"),
        "dev" => t("Development tools", "开发修改工具组"),
        "dev-read" => t("Code search tools", "代码检索工具组"),
        "diagnostics" => t("Diagnostic tools", "诊断工具组"),
        "divination" => t("Divination tools", "玄学工具组"),
        "images" => t("Image tools", "图片工具组"),
        "knowledge" => t("Knowledge base tools", "知识库工具组"),
        "knowledge-admin" => t("Knowledge base management", "知识库管理工具组"),
        "linux-docs" => t("Linux documentation", "Linux 文档工具组"),
        "memory" => t("Memory tools", "记忆工具组"),
        "memes" => t("Meme tools", "表情包工具组"),
        "planning" => t("Planning tools", "任务规划工具组"),
        "research" => t("Research tools", "深度研究工具组"),
        "scripts" => t("Script tools", "脚本工具组"),
        "scripting" => t("Script management", "脚本管理工具组"),
        "shell" => t("Shell tools", "Shell 工具组"),
        "skills" => t("Skill tools", "技能工具组"),
        "systeminfo" => t("System information", "系统信息工具组"),
        "utility" => t("Utility tools", "实用工具组"),
        "web" => t("Web tools", "联网工具组"),
        _ => return None,
    })
}

pub fn builtin_registry(config: &AppConfig, paths: &GqyPaths) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    default_tools::register(&mut registry, true);
    apply_patch::register(&mut registry);
    write::register(&mut registry);
    edit_replace::register(&mut registry);
    todowrite::register(&mut registry);
    alarm::register(&mut registry, paths.clone());
    clipboard::register(&mut registry, paths.clone());
    web::register_fetch(&mut registry);
    weather::register(&mut registry);
    exchange_rate::register(&mut registry, config.plugins.exchange_rate.clone());
    xuanxue::register(&mut registry);
    if config.plugins.man.enabled {
        man::register(&mut registry);
    }
    moegirl::register(&mut registry);
    hash_codec::register(&mut registry);
    calculator::register(&mut registry);
    deepseek_status::register(&mut registry);
    vision::register_print(&mut registry, config.clone());
    if config.plugins.memes.enabled {
        memes::register(&mut registry, config.clone(), paths.clone());
    }
    if config.plugins.web.enabled {
        web::register(&mut registry, config.plugins.web.clone());
    }
    if config.plugins.web_images.enabled {
        web_images::register(&mut registry, config.clone(), paths.clone(), true);
    }
    if config.plugins.deep_research.enabled {
        let research_tools = registry.clone();
        deep_research::register(&mut registry, config.clone(), paths.clone(), research_tools);
    }
    if config.plugins.vision.enabled {
        vision::register(&mut registry, config.clone(), paths.clone(), true);
    }
    // 本地视觉（Apple Vision）不依赖插件开关：模型超额时兜底看图
    local_vision::register(&mut registry, paths.clone());
    speech::register(&mut registry, paths.clone());
    if config.plugins.image_generation.enabled {
        image_generation::register(&mut registry, config.clone());
    }
    if config.plugins.knowledge_base.enabled {
        knowledge_base::register(&mut registry, config.clone(), paths.clone());
    }
    if config.plugins.diagnostics.enabled {
        diagnostics::register(&mut registry, config.clone());
    }
    if config.memory_config().enabled {
        memory::register(&mut registry, config.clone(), paths.clone());
    }
    let task_tools = registry.clone();
    task::register(&mut registry, config.clone(), paths.clone(), task_tools);
    scripts::register(&mut registry, paths);
    // 自主 agent 集群（Kimi 式）：模型可自建/管理命名子代理
    crate::agents::register(&mut registry, paths.clone());
    if config.mcp.enabled {
        mcp::register(&mut registry, config.clone());
    }
    if is_hybrid_loading_mode(&config.tools.loading_mode) {
        load_tools::register(&mut registry);
    }
    registry
}

pub fn readonly_registry(config: &AppConfig, paths: &GqyPaths) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    default_tools::register_readonly(&mut registry);
    clipboard::register(&mut registry, paths.clone());
    web::register_fetch(&mut registry);
    if config.plugins.man.enabled {
        man::register(&mut registry);
    }
    if config.plugins.web.enabled {
        web::register(&mut registry, config.plugins.web.clone());
    }
    if config.plugins.web_images.enabled {
        web_images::register(&mut registry, config.clone(), paths.clone(), false);
    }
    if config.plugins.vision.enabled {
        vision::register(&mut registry, config.clone(), paths.clone(), true);
    }
    if config.plugins.knowledge_base.enabled {
        knowledge_base::register_readonly(&mut registry, config.clone(), paths.clone());
    }
    if config.plugins.diagnostics.enabled {
        diagnostics::register(&mut registry, config.clone());
    }
    if config.memory_config().enabled {
        memory::register_readonly(&mut registry, config.clone(), paths.clone());
    }
    if is_hybrid_loading_mode(&config.tools.loading_mode) {
        load_tools::register(&mut registry);
    }
    if config.mcp.enabled {
        mcp::register(&mut registry, config.clone());
    }
    registry
}

pub fn is_hybrid_loading_mode(mode: &str) -> bool {
    matches!(mode.trim(), "hybrid" | "lazy")
}

pub fn chat_registry(config: &AppConfig, paths: &GqyPaths) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    // 闲聊纯文本模式：不注册任何工具，杜绝本地模型工具循环/重复调用
    if config.prompt.chat_pure_text {
        return registry;
    }
    web::register_fetch(&mut registry);
    if config.plugins.web.enabled {
        web::register(&mut registry, config.plugins.web.clone());
    }
    if config.plugins.vision.enabled {
        vision::register(&mut registry, config.clone(), paths.clone(), true);
    }
    if config.plugins.memes.enabled {
        memes::register_chat(&mut registry, config.clone(), paths.clone());
    }
    // 角色扮演也要能「想起」别处的事（被动查询，不主动注入省 token）：
    // 记忆与普通/计划模式同库（同 persona），这里只暴露只读查询。
    if config.memory_config().enabled {
        memory::register_readonly(&mut registry, config.clone(), paths.clone());
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_paths(root: &std::path::Path) -> GqyPaths {
        GqyPaths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.jsonc"),
            skills_dir: root.join("config/skills"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            state_dir: root.join("state"),
            pictures_dir: root.join("pictures"),
            fish_hook_file: root.join("config/fish/conf.d/gqy.fish"),
            bash_hook_file: root.join("config/shell/bash-hook.sh"),
            zsh_hook_file: root.join("config/shell/zsh-hook.zsh"),
            scripts_dir: root.join("config/scripts"),
            system_scripts_dir: root.join("system-scripts"),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        }
    }

    #[test]
    fn readable_names_cover_all_built_in_tools_and_groups() {
        let mut missing_tools = tool_descriptions::all()
            .keys()
            .filter(|name| builtin_readable_tool_name(name).is_none())
            .cloned()
            .collect::<Vec<_>>();
        missing_tools.sort();
        assert!(
            missing_tools.is_empty(),
            "missing tool names: {missing_tools:?}"
        );

        let mut missing_groups = tool_descriptions::group_names()
            .into_iter()
            .filter(|group| builtin_readable_group_name(group).is_none())
            .collect::<Vec<_>>();
        missing_groups.sort();
        assert!(
            missing_groups.is_empty(),
            "missing tool group names: {missing_groups:?}"
        );
    }

    #[test]
    fn ui_language_does_not_change_agent_tool_definitions() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let mut english = AppConfig::default();
        english.display.language = "en".to_string();
        let mut chinese = english.clone();
        chinese.display.language = "zh".to_string();

        let english =
            serde_json::to_value(builtin_registry(&english, &paths).definitions()).unwrap();
        let chinese =
            serde_json::to_value(builtin_registry(&chinese, &paths).definitions()).unwrap();

        assert_eq!(english, chinese);
    }
}
