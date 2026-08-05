//! Request/response DTOs and Safe* view models.
use crate::agent::AgentMode;
use crate::clipboard::{ClipboardImage, PastedImage};
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::llm::Usage;
use crate::question::{self, QuestionAnswers, QuestionRequest};
use axum::http::StatusCode;
use crate::state::{
    ChannelSummary, ConversationSummary, ImageAsset, QueuedPrompt, QueuedPromptAttachment, Turn,
    TurnFollowup, TurnStatus, UsageSnapshot,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::error::ApiError;
use super::util::{
    safe_error_message, MAX_CONTENT_CHARS, MAX_PROMPT_DOCUMENT_CHARS, MAX_PROMPT_DOCUMENTS,
    MAX_SECRET_CHARS,
};

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct ContextSnapshot {
    pub(crate) tokens: u64,
    pub(crate) window: Option<usize>,
}
#[derive(Default, Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    pub(crate) after: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum PiCommandKind {
    GetState,
    GetModels,
    SetModel,
    SetThinking,
}

/// 解析 pi 控制命令（web /api/pi/* 用）
pub(crate) fn parse_pi_command(path: &str) -> Option<PiCommandKind> {
    match path {
        "state" => Some(PiCommandKind::GetState),
        "models" => Some(PiCommandKind::GetModels),
        "model" => Some(PiCommandKind::SetModel),
        "thinking" => Some(PiCommandKind::SetThinking),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone)]
pub(crate) struct WebImageInput {
    pub(crate) mime: String,
    pub(crate) data_base64: String,
}

/// 把前端 base64 图片转成 agent 用的 PastedImage（解码失败则丢弃该项）。
pub(crate) fn pasted_images_from_input(images: &[WebImageInput]) -> Vec<Option<PastedImage>> {
    images
        .iter()
        .map(|image| match base64::engine::general_purpose::STANDARD.decode(&image.data_base64) {
            Ok(data) => Some(PastedImage::Binary(ClipboardImage::new(
                image.mime.clone(),
                data,
            ))),
            Err(_) => None,
        })
        .collect()
}

pub(crate) fn queued_attachments_from_input(images: &[WebImageInput]) -> Vec<QueuedPromptAttachment> {
    images
        .iter()
        .filter(|image| base64::engine::general_purpose::STANDARD.decode(&image.data_base64).is_ok())
        .map(|image| QueuedPromptAttachment::Binary {
            mime: image.mime.clone(),
            data_base64: image.data_base64.clone(),
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTurnRequest {
    pub(crate) content: String,
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) images: Vec<WebImageInput>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct ChannelTurnsQuery {
    pub(crate) mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueuePromptRequest {
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) images: Vec<WebImageInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnswerQuestionRequest {
    pub(crate) answers: QuestionAnswers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetModelsRequest {
    pub(crate) models: Vec<ActiveProviderModelConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRequest {
    pub(crate) password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateConfigRequest {
    pub(crate) config: Value,
    #[serde(default)]
    pub(crate) secrets: HashMap<String, SecretMutation>,
    pub(crate) prompts: PromptDocuments,
    #[serde(default)]
    pub(crate) reset_conversation: bool,
}

#[derive(Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub(crate) enum SecretMutation {
    Set(String),
    Clear,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptDocuments {
    #[serde(default)]
    pub(crate) personas: Vec<PromptDocument>,
    #[serde(default)]
    pub(crate) identities: Vec<PromptDocument>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptDocument {
    pub(crate) name: String,
    pub(crate) content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) original_name: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ConfigResponse {
    pub(crate) config: Value,
    pub(crate) secret_states: HashMap<String, bool>,
    pub(crate) prompts: PromptDocuments,
    pub(crate) models: Vec<SafeModel>,
    pub(crate) multimodal_models: Vec<SafeModel>,
    pub(crate) display: WebDisplayConfig,
    pub(crate) context: ContextSnapshot,
}

#[derive(Serialize)]
pub(crate) struct BootstrapResponse {
    pub(crate) version: &'static str,
    pub(crate) boot_id: String,
    pub(crate) latest_event_id: u64,
    pub(crate) active_run_id: Option<String>,
    pub(crate) running_turn_id: Option<String>,
    pub(crate) external_queue_available: bool,
    /// 本 WebUI 会话所属通道（默认 webui，GQY_CHANNEL 可覆盖）
    pub(crate) channel: String,
    /// 全部会话通道摘要（终端/网页/QQ/Telegram…），左侧通道列表
    pub(crate) channels: Vec<SafeChannelSummary>,
    /// 当前通道的会话列表（含归档历史对话），左侧历史对话列表
    pub(crate) conversations: Vec<SafeConversationSummary>,
    pub(crate) turns: Vec<SafeTurn>,
    pub(crate) queued_prompts: Vec<SafeQueuedPrompt>,
    pub(crate) models: Vec<SafeModel>,
    pub(crate) display: WebDisplayConfig,
    pub(crate) context: ContextSnapshot,
    pub(crate) usage: SafeUsageSnapshot,
    /// 引擎标识：`pi`（pi 底座）或 provider id
    pub(crate) engine: String,
    pub(crate) capabilities: Capabilities,
}

#[derive(Serialize)]
pub(crate) struct SafeChannelSummary {
    pub(crate) id: String,
    pub(crate) turn_count: u64,
    pub(crate) last_seq: i64,
    pub(crate) title: String,
    pub(crate) snippet: String,
    pub(crate) timestamp: Option<String>,
    pub(crate) running: bool,
}

#[derive(Serialize)]
pub(crate) struct SafeConversationSummary {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) snippet: String,
    pub(crate) timestamp: Option<String>,
    pub(crate) turn_count: u64,
    pub(crate) active: bool,
}

#[derive(Serialize)]
pub(crate) struct Capabilities {
    pub(crate) multi_conversation: bool,
    pub(crate) attachments: bool,
    pub(crate) queue: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct WebDisplayConfig {
    pub(crate) reasoning: String,
    pub(crate) tool_calls: String,
    pub(crate) readable_tool_names: bool,
    pub(crate) command_output_lines: usize,
    pub(crate) mixed_model_endpoint_display: String,
    pub(crate) show_mixed_model_endpoint: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct SafeQueuedPrompt {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) submitted_at: String,
}

#[derive(Serialize)]
pub(crate) struct SafeModel {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) model: String,
    pub(crate) active: bool,
}

#[derive(Serialize)]
pub(crate) struct SafeTurn {
    pub(crate) id: String,
    pub(crate) seq: i64,
    pub(crate) status: &'static str,
    pub(crate) active_context: bool,
    pub(crate) user_content: String,
    pub(crate) assistant_content: String,
    pub(crate) assistant_reasoning: Option<String>,
    pub(crate) provider_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) user_timestamp: String,
    pub(crate) assistant_timestamp: Option<String>,
    pub(crate) token_total: u64,
    pub(crate) token_usage_estimated: bool,
    pub(crate) question_exchanges: Vec<crate::question::QuestionExchange>,
    pub(crate) followups: Vec<SafeFollowup>,
    pub(crate) assets: Vec<SafeImageAsset>,
}

#[derive(Serialize)]
pub(crate) struct SafeFollowup {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) submitted_at: String,
    pub(crate) preceding_assistant_content: Option<String>,
    pub(crate) preceding_assistant_reasoning: Option<String>,
    pub(crate) provider_id: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct SafeImageAsset {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) mime: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) alt: String,
    pub(crate) hide_caption: bool,
    /// 资产归属：`user`（用户随消息发送的图片）/ `tool`（工具输出，表情包等）
    pub(crate) source: String,
}

#[derive(Serialize)]
pub(crate) struct SafeUsageSnapshot {
    pub(crate) requests: u64,
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) last_usage: Option<Usage>,
    pub(crate) last_conversation_usage: Option<Usage>,
}

#[derive(Serialize)]
pub(crate) struct ModelResponse {
    pub(crate) models: Vec<SafeModel>,
    pub(crate) display: WebDisplayConfig,
    pub(crate) context: ContextSnapshot,
}

pub(crate) fn safe_models(config: &AppConfig) -> Vec<SafeModel> {
    config
        .provider_model_choices()
        .into_iter()
        .map(|choice| SafeModel {
            active: config.is_active_provider_model(&choice.provider_id, &choice.model),
            provider_id: choice.provider_id,
            provider_name: choice.provider_name,
            model: choice.model,
        })
        .collect()
}

pub(crate) fn web_display_config(config: &AppConfig) -> WebDisplayConfig {
    let mixed_model_endpoint_display = config.display.mixed_model_endpoint_display.clone();
    WebDisplayConfig {
        reasoning: config.display.reasoning.clone(),
        tool_calls: config.display.tool_calls.clone(),
        readable_tool_names: config.display.readable_tool_names,
        command_output_lines: config.display.command_output_lines,
        show_mixed_model_endpoint: config.active_provider_model_choices().len() > 1
            && matches!(mixed_model_endpoint_display.as_str(), "interactive" | "all"),
        mixed_model_endpoint_display,
    }
}

pub(crate) fn safe_multimodal_models(config: &AppConfig) -> Vec<SafeModel> {
    config
        .multimodal_provider_model_choices()
        .into_iter()
        .map(|choice| SafeModel {
            active: config.is_active_multimodal_provider_model(&choice.provider_id, &choice.model),
            provider_id: choice.provider_id,
            provider_name: choice.provider_name,
            model: choice.model,
        })
        .collect()
}

impl SafeTurn {
    pub(crate) fn from_turn(turn: Turn, assets: Vec<ImageAsset>) -> Self {
        let assets = assets
            .into_iter()
            .map(|asset| {
                let hide_caption = meme_asset_caption_hidden(&asset, &turn.tool_reports);
                SafeImageAsset::from_asset(asset, hide_caption)
            })
            .collect();
        Self {
            id: turn.turn_id,
            seq: turn.seq,
            status: match turn.status {
                TurnStatus::Running => "running",
                TurnStatus::Completed => "completed",
                TurnStatus::Interrupted => "interrupted",
            },
            active_context: !turn.hidden,
            user_content: turn.user_content,
            assistant_content: redact_internal_assistant_text(&turn.assistant_content),
            assistant_reasoning: turn
                .assistant_reasoning
                .map(|reasoning| redact_internal_assistant_text(&reasoning)),
            provider_id: turn.assistant_provider_id,
            model: turn.assistant_model,
            user_timestamp: turn.user_timestamp,
            assistant_timestamp: turn.assistant_timestamp,
            token_total: turn.token_total,
            token_usage_estimated: turn.token_usage_estimated,
            question_exchanges: turn.question_exchanges,
            followups: turn.followups.into_iter().map(SafeFollowup::from).collect(),
            assets,
        }
    }
}

/// 通道摘要 → 前端列表项（标题取首条用户消息首行，摘要取最新可见消息）
pub(crate) fn safe_channel_summary(summary: ChannelSummary) -> SafeChannelSummary {
    let (title, snippet, timestamp, running) = match &summary.recent {
        Some((user_content, assistant_content, user_timestamp, assistant_timestamp, status, _)) => {
            let title = first_line(user_content)
                .filter(|line| !line.trim().is_empty())
                .unwrap_or_else(|| "（无标题）".to_string());
            let snippet = if *status == "running" {
                "正在回复…".to_string()
            } else {
                first_line(assistant_content)
                    .filter(|line| !line.trim().is_empty())
                    .unwrap_or_else(|| first_line(user_content).unwrap_or_default())
            };
            (
                title,
                snippet,
                assistant_timestamp
                    .clone()
                    .or_else(|| Some(user_timestamp.clone())),
                *status == "running",
            )
        }
        None => ("（空对话）".to_string(), String::new(), None, false),
    };
    SafeChannelSummary {
        id: summary.channel,
        turn_count: summary.turn_count,
        last_seq: summary.last_seq,
        title,
        snippet,
        timestamp,
        running,
    }
}

pub(crate) fn first_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

pub(crate) fn safe_conversation_summary(summary: ConversationSummary) -> SafeConversationSummary {
    SafeConversationSummary {
        id: summary.conversation_id,
        title: summary.title,
        snippet: summary.snippet,
        timestamp: summary.timestamp,
        turn_count: summary.turn_count,
        active: summary.active,
    }
}

impl SafeImageAsset {
    pub(crate) fn from_asset(asset: ImageAsset, hide_caption: bool) -> Self {
        let source = match asset.tool_id.as_deref() {
            Some(id) if id.starts_with("user_image") => "user",
            _ => "tool",
        };
        Self {
            url: format!("/api/assets/{}", asset.asset_id),
            id: asset.asset_id,
            mime: asset.mime,
            width: asset.width,
            height: asset.height,
            alt: asset.alt,
            hide_caption,
            source: source.to_string(),
        }
    }
}

impl From<ImageAsset> for SafeImageAsset {
    fn from(asset: ImageAsset) -> Self {
        Self::from_asset(asset, false)
    }
}

pub(crate) fn meme_asset_caption_hidden(asset: &ImageAsset, reports: &[String]) -> bool {
    const MAX_DESCRIPTION_CHARS: usize = 120;

    let description = asset.alt.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return false;
    }
    let mut characters = description.chars();
    let mut compact = characters
        .by_ref()
        .take(MAX_DESCRIPTION_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        compact.push('…');
    }
    let escaped = compact
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let marker = format!("description={escaped}</sent_meme>");
    reports
        .iter()
        .any(|report| report.starts_with("<sent_meme>") && report.contains(&marker))
}

impl From<TurnFollowup> for SafeFollowup {
    fn from(followup: TurnFollowup) -> Self {
        Self {
            id: followup.prompt_id,
            content: followup.display_content,
            submitted_at: followup.submitted_at,
            preceding_assistant_content: followup
                .preceding_assistant_content
                .map(|content| redact_internal_assistant_text(&content)),
            preceding_assistant_reasoning: followup
                .preceding_assistant_reasoning
                .map(|reasoning| redact_internal_assistant_text(&reasoning)),
            provider_id: followup.preceding_assistant_provider_id,
            model: followup.preceding_assistant_model,
        }
    }
}

impl From<QueuedPrompt> for SafeQueuedPrompt {
    fn from(prompt: QueuedPrompt) -> Self {
        Self {
            id: prompt.prompt_id,
            content: prompt.display_content,
            submitted_at: prompt.submitted_at,
        }
    }
}

impl From<UsageSnapshot> for SafeUsageSnapshot {
    fn from(usage: UsageSnapshot) -> Self {
        Self {
            requests: usage.requests,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            last_usage: usage.last_usage,
            last_conversation_usage: usage.last_conversation_usage,
        }
    }
}

pub(crate) fn redact_internal_assistant_text(value: &str) -> String {
    value
        .replace(crate::state::pending_placeholder(), "")
        .replace(crate::state::interrupted_text(), "")
}

pub(crate) fn normalize_answers(
    request: &QuestionRequest,
    mut answers: QuestionAnswers,
) -> std::result::Result<QuestionAnswers, String> {
    for answer in &mut answers {
        for value in answer {
            *value = value.trim().to_string();
            if value.chars().any(char::is_control) {
                return Err("answers cannot contain control characters".to_string());
            }
        }
    }
    question::validate_answers(request, &answers).map_err(|error| safe_error_message(&error))?;
    Ok(answers)
}

pub(crate) fn validate_content(content: String) -> std::result::Result<String, ApiError> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "content cannot be empty",
        ));
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("content cannot exceed {MAX_CONTENT_CHARS} characters"),
        ));
    }
    if content
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "content contains unsupported control characters",
        ));
    }
    Ok(content)
}

pub(crate) fn validate_short_field(
    value: String,
    field: &str,
    max_chars: usize,
) -> std::result::Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{field} cannot be empty"),
        ));
    }
    if value.chars().count() > max_chars || value.chars().any(char::is_control) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{field} is invalid"),
        ));
    }
    Ok(value)
}

pub(crate) fn validate_model_selection(
    models: Vec<ActiveProviderModelConfig>,
) -> std::result::Result<Vec<ActiveProviderModelConfig>, ApiError> {
    if models.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "at least one model endpoint must remain active",
        ));
    }
    if models.len() > 64 {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "at most 64 model endpoints can be active",
        ));
    }
    let mut seen = HashSet::with_capacity(models.len());
    let mut validated = Vec::with_capacity(models.len());
    for model in models {
        let provider_id = validate_short_field(model.provider_id, "provider_id", 200)?;
        let model = validate_short_field(model.model, "model", 500)?;
        if !seen.insert((provider_id.clone(), model.clone())) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "duplicate provider/model selection",
            ));
        }
        validated.push(ActiveProviderModelConfig { provider_id, model });
    }
    Ok(validated)
}

pub(crate) fn parse_mode(mode: &str) -> std::result::Result<AgentMode, ApiError> {
    match mode {
        "normal" => Ok(AgentMode::Normal),
        "plan" => Ok(AgentMode::Plan),
        "chat" => Ok(AgentMode::Chat),
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "mode must be normal, plan, or chat",
        )),
    }
}

pub(crate) fn mode_name(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Normal => "normal",
        AgentMode::Plan => "plan",
        AgentMode::Chat => "chat",
    }
}

pub(crate) fn real_tool_name(event_name: &str) -> &str {
    if event_name.starts_with("load_skill:") {
        "load_skill"
    } else if event_name.starts_with("load_tools:") {
        "load_tools"
    } else {
        event_name
    }
}

