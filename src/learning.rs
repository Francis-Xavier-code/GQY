//! 自我成长（知识库反哺）：对话结束后，若用户明确要求记住方法/任务，
//! 把结论沉淀为可加载的技能（SKILL.md）并记录到 skill_records。
//!
//! 触发条件（尽量省 token，只做规则匹配，不额外调用模型）：
//! - 用户消息包含「记住这个/记下来/以后就这么做/总结成方法」等明确信号
//! - 且本轮有足够长的助手回答（有实质内容可沉淀）
//!
//! 生成的文件带 `generated_by: gqy` 标记，清理逻辑（memory::prune 里已有）
//! 会在技能文件缺失时自动清理 skill_records。

use crate::config::AppConfig;
use crate::paths::GqyPaths;
use anyhow::Result;
use chrono::Local;

/// 判断用户消息是否明确要求沉淀方法/知识（规则匹配，零模型开销）。
pub fn wants_to_learn(user_message: &str) -> bool {
    const SIGNALS: &[&str] = &[
        "记住这个",
        "记下来",
        "记住这个方法",
        "记住这条",
        "以后就这么做",
        "总结成方法",
        "写成技能",
        "保存这个方法",
        "这个方法不错",
        "以后遇到",
        "记到记忆里",
        "写进知识库",
        "存到知识库",
    ];
    SIGNALS.iter().any(|signal| user_message.contains(signal))
}

/// 从助手回答里提取可沉淀的方法标题（取首个实质行，截断）。
fn method_title(assistant_message: &str) -> String {
    let line = assistant_message
        .lines()
        .map(str::trim)
        .find(|line| line.chars().count() >= 6 && !line.starts_with(['#', '-', '*', '>']))
        .unwrap_or("对话中总结的方法");
    let title: String = line.chars().take(40).collect();
    title
}

/// 把本轮对话沉淀为自动学习技能。
/// 返回 (技能名, 是否新建)。
pub fn maybe_learn(
    paths: &GqyPaths,
    config: &AppConfig,
    user_message: &str,
    assistant_message: &str,
) -> Result<Option<(String, bool)>> {
    let memory = config.memory_config();
    if !memory.enabled || !memory.auto_skill_enabled {
        return Ok(None);
    }
    let min_method = memory.learning_min_method_chars.max(16);
    if !wants_to_learn(user_message) {
        return Ok(None);
    }
    let content = assistant_message.trim();
    if content.chars().count() < min_method {
        return Ok(None);
    }

    let title = method_title(content);
    let skill_name = slugify(&title);
    if skill_name.is_empty() {
        return Ok(None);
    }
    let skill_dir = paths.skills_dir.join(&skill_name);
    let skill_file = skill_dir.join("SKILL.md");
    let is_new = !skill_file.exists();

    std::fs::create_dir_all(&skill_dir)?;
    let body = format!(
        "# {title}\n\n> generated_by: gqy · 自动学习于 {}\n> 来源：对话中用户要求记住的方法\n\n{}\n",
        Local::now().format("%Y-%m-%d %H:%M"),
        content
    );
    std::fs::write(&skill_file, body)?;

    // 记录到 skill_records（memory 的清理逻辑据此管理）
    let store = crate::memory::MemoryStore::new(config, paths);
    store.record_skill(&skill_name, &skill_file.display().to_string(), &title)?;

    Ok(Some((skill_name, is_new)))
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars().take(30) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            slug.push(ch);
        } else if ch.is_whitespace() || ch == '-' || ch == '_' || ch == '/' || ch == '：' || ch == ':' {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_learning_signals() {
        assert!(wants_to_learn("记住这个方法，以后编译用这个命令"));
        assert!(wants_to_learn("把这个记下来"));
        assert!(!wants_to_learn("今天天气不错"));
        assert!(!wants_to_learn(""));
    }

    #[test]
    fn slugifies_titles() {
        assert_eq!(slugify("编译 Rust 项目"), "编译-rust-项目");
        assert_eq!(slugify("  Hello World  "), "hello-world");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn skips_short_answers() {
        let root = tempfile::tempdir().unwrap().into_path();
        let _env_guard = crate::paths::test_env::GQY_HOME_LOCK.lock().unwrap();
        let old = std::env::var_os("GQY_HOME");
        std::env::set_var("GQY_HOME", &root);
        let paths = crate::paths::GqyPaths::new().unwrap();
        let config = AppConfig::default();
        // 用户要求记住但回答太短 → 不沉淀
        let result = maybe_learn(&paths, &config, "记住这个方法", "好").unwrap();
        assert!(result.is_none());
        if let Some(v) = old {
            std::env::set_var("GQY_HOME", v);
        } else {
            std::env::remove_var("GQY_HOME");
        }
    }
}
