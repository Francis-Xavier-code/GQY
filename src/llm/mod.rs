mod openai_compatible;
mod pi_rpc;

pub use openai_compatible::{OpenAiCompatibleClient, ThinkingVariantOptions};
pub use pi_rpc::PiRpcClient;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlContent },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlContent {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(ChatContent::Text(content.into())),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Option<Vec<ToolCall>>) -> Self {
        let text = content.into();
        let has_tool_calls = tool_calls.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        let content = if text.trim().is_empty() && has_tool_calls {
            None
        } else {
            Some(ChatContent::Text(text))
        };
        Self {
            role: "assistant".to_string(),
            content,
            tool_call_id: None,
            tool_calls,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(ChatContent::Text(content.into())),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
        }
    }

    pub fn plain(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(ChatContent::Text(content.into())),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn user_with_image(text: impl Into<String>, image_url: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(ChatContent::Parts(vec![
                ChatContentPart::Text { text: text.into() },
                ChatContentPart::ImageUrl {
                    image_url: ImageUrlContent {
                        url: image_url.into(),
                    },
                },
            ])),
            tool_call_id: None,
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    /// 命中的前缀缓存 token（Anthropic cache_read_input_tokens / DeepSeek prompt_cache_hit_tokens 等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    /// 本次新建的缓存 token（Anthropic cache_creation_input_tokens）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}

impl Usage {
    pub fn effective_total_tokens(&self) -> u64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.prompt_tokens.saturating_add(self.completion_tokens)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatResult {
    pub content: String,
    pub reasoning: Option<String>,
    pub usage: Option<Usage>,
    pub usage_estimated: bool,
    pub tool_calls: Vec<ToolCall>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatStreamKind {
    Content,
    Reasoning,
    ReasoningReset,
    ReasoningPartStart,
    ReasoningPartEnd,
    ToolCall,
    /// pi 模式工具调用进度（text = JSON {name, args} 或 {name, output}）
    ToolProgress,
    /// pi 模式工具执行结果（text = JSON {name, ok, output}）
    ToolResult,
}

#[derive(Debug, Clone)]
pub struct ChatStreamChunk {
    pub kind: ChatStreamKind,
    pub text: String,
}

/// 统一的 LLM 客户端：OpenAI 兼容直连，或 pi RPC（`provider.protocol == "pi"`）。
#[derive(Clone)]
pub enum LlmClient {
    OpenAi(OpenAiCompatibleClient),
    Pi(PiRpcClient),
}

impl LlmClient {
    pub fn from_config(config: &crate::config::AppConfig, paths: &crate::paths::GqyPaths) -> anyhow::Result<Self> {
        if std::env::var_os("GQY_PI_DEBUG").is_some() {
            eprintln!(
                "[llm-client] from_config: active_provider={}, protocol={}, is_pi={}",
                config.active_provider,
                config
                    .provider(None)
                    .map(|p| p.protocol.as_str())
                    .unwrap_or("?"),
                config.provider(None).map(|p| p.is_pi()).unwrap_or(false)
            );
        }
        if config
            .provider(None)
            .map(|p| p.is_pi())
            .unwrap_or(false)
        {
            Ok(Self::Pi(PiRpcClient::from_config(config, paths)?))
        } else {
            Ok(Self::OpenAi(OpenAiCompatibleClient::from_config(config, paths)?))
        }
    }

    pub fn is_pi(&self) -> bool {
        matches!(self, Self::Pi(_))
    }

    pub async fn chat_stream<F>(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        on_chunk: F,
    ) -> anyhow::Result<ChatResult>
    where
        F: FnMut(ChatStreamChunk) -> anyhow::Result<()>,
    {
        match self {
            Self::OpenAi(client) => client.chat_stream(messages, tools, on_chunk).await,
            Self::Pi(client) => client.chat_stream(messages, tools, on_chunk).await,
        }
    }

    pub fn context_window(&self, config: &crate::config::AppConfig) -> anyhow::Result<Option<usize>> {
        match self {
            Self::OpenAi(client) => client.context_window(config),
            // pi 自己管理上下文与压缩，GQY 侧不设窗口（从而禁用 GQY 的溢出/压缩）
            Self::Pi(_) => Ok(None),
        }
    }

    pub fn models_without_context_window(&self, config: &crate::config::AppConfig) -> Vec<String> {
        match self {
            Self::OpenAi(client) => client.models_without_context_window(config),
            Self::Pi(_) => Vec::new(),
        }
    }

    /// 子 agent 模式：OpenAi 调整输出完整度；Pi 标记子 agent 模式，
    /// 使其 spawn 的 pi 进程使用过滤工具清单（剔除 gqy_task/gqy_deep_research，防递归）。
    pub fn for_subagent_output(self, full: bool) -> Self {
        match self {
            Self::OpenAi(client) => Self::OpenAi(client.for_subagent_output(full)),
            Self::Pi(client) => Self::Pi(client.with_subagent_mode()),
        }
    }

    /// pi 状态（模型/思考级别）；非 pi 返回默认占位。
    #[allow(dead_code)]
    pub async fn pi_state(&self) -> anyhow::Result<serde_json::Value> {
        match self {
            Self::OpenAi(_) => Ok(serde_json::json!({ "model": null, "thinking_level": null })),
            Self::Pi(client) => client.pi_state().await,
        }
    }

    /// 设置 pi 模型；非 pi 无操作。
    #[allow(dead_code)]
    pub async fn pi_set_model(&self, model_id: &str) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(_) => Ok(()),
            Self::Pi(client) => client.set_model(model_id).await,
        }
    }

    /// 设置 pi 思考级别；非 pi 无操作。
    #[allow(dead_code)]
    pub async fn pi_set_thinking_level(&self, level: &str) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(_) => Ok(()),
            Self::Pi(client) => client.set_thinking_level(level).await,
        }
    }

    /// pi 可用模型列表；非 pi 返回空。
    #[allow(dead_code)]
    pub async fn pi_available_models(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        match self {
            Self::OpenAi(_) => Ok(Vec::new()),
            Self::Pi(client) => client.available_models().await,
        }
    }

    pub fn interrupt(&self) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(_) => Ok(()),
            Self::Pi(client) => {
                let client = client.clone();
                tokio::spawn(async move { let _ = client.interrupt().await; });
                Ok(())
            }
        }
    }

    // ---- thinking variant：仅 OpenAi 路径有意义，pi 模式返回空/无操作 ----

    pub fn thinking_variant_options(&self) -> Vec<ThinkingVariantOptions> {
        match self {
            Self::OpenAi(client) => client.thinking_variant_options(),
            Self::Pi(_) => Vec::new(),
        }
    }

    pub fn available_thinking_variants(&self) -> Vec<String> {
        match self {
            Self::OpenAi(client) => client.available_thinking_variants(),
            Self::Pi(_) => Vec::new(),
        }
    }

    pub fn set_thinking_variant(&mut self, variant: Option<String>) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(client) => client.set_thinking_variant(variant),
            Self::Pi(_) => Ok(()),
        }
    }

    pub fn set_thinking_variants(
        &mut self,
        selections: &[(String, String, Option<String>)],
    ) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(client) => client.set_thinking_variants(selections),
            Self::Pi(_) => Ok(()),
        }
    }

    pub fn save_thinking_variants(&self, paths: &crate::paths::GqyPaths) -> anyhow::Result<()> {
        match self {
            Self::OpenAi(client) => client.save_thinking_variants(paths),
            Self::Pi(_) => Ok(()),
        }
    }

    pub fn thinking_variant_summary(&self) -> Option<String> {
        match self {
            Self::OpenAi(client) => client.thinking_variant_summary(),
            Self::Pi(_) => None,
        }
    }

    pub fn thinking_variant_for(&self, provider_id: &str, model: &str) -> Option<String> {
        match self {
            Self::OpenAi(client) => client.thinking_variant_for(provider_id, model),
            Self::Pi(_) => None,
        }
    }
}
