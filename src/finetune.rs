//! 自我进化一期：每轮对话自动标注训练样本（JSONL 追加）。
//!
//! 设计约束（见 docs 方案 self-evolving-training.md）：
//! - **只收集、不训练**：微调是批量过程（攒够阈值如 500–1000 条 / 每周一次），
//!   绝不做「每轮即时微调」（灾难性遗忘 + 过拟合 + 性能灾难）；
//! - 数据格式与外部 MLX LoRA 训练脚本对齐：`{ts, mode, user, assistant, tools}`；
//! - 全程 best-effort：写入失败静默，绝不影响对话热路径。
use crate::agent::AgentMode;
use crate::config::FinetuneConfig;
use crate::paths::GqyPaths;
use chrono::Utc;
use serde_json::json;
use std::io::Write;

/// 追加一条训练样本。开关关闭 / 内容过短 / 写入失败时静默返回。
pub fn record_turn(
    paths: &GqyPaths,
    config: &FinetuneConfig,
    mode: AgentMode,
    user: &str,
    assistant: &str,
    tools: &[String],
) {
    if !config.collect {
        return;
    }
    if user.trim().len() < 2 || assistant.trim().len() < config.min_chars {
        return;
    }
    let dir = paths.data_dir.join("finetune");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mode_label = match mode {
        AgentMode::Normal => "normal",
        AgentMode::Plan => "plan",
        AgentMode::Chat => "chat",
    };
    let record = json!({
        "ts": Utc::now().timestamp(),
        "mode": mode_label,
        "user": user,
        "assistant": assistant,
        "tools": tools,
    });
    let path = dir.join("turns.jsonl");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{record}");
    }
}

/// 当前已收集的样本条数（供 `gqy finetune status` 之类命令展示）。
#[allow(dead_code)]
pub fn collected_count(paths: &GqyPaths) -> usize {
    let path = paths.data_dir.join("finetune").join("turns.jsonl");
    std::fs::read_to_string(path)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}
