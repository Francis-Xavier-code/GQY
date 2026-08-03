use crate::agent::{Agent, AgentEvent, AgentMode, AgentTurnControl};
use crate::clipboard::{ClipboardImage, PastedImage};
use crate::cli::{build_tool_registry, WebArgs};
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::llm::{ChatResult, ChatStreamKind, LlmClient, Usage};
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use crate::question::{self, QuestionAnswers, QuestionRequest, QuestionResponse};
use crate::state::{
    ChannelSummary, ConversationSummary, ImageAsset, QueuedPrompt, QueuedPromptAttachment,
    StateStore, Turn, TurnFollowup, TurnStatus, UsageSnapshot,
};
use crate::tools::{self, CommandOutputStream, ToolRegistry};
use anyhow::{Context, Result};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, HOST, ORIGIN, REFERRER_POLICY,
    RETRY_AFTER, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::IntoFuture;
use std::io::{self, IsTerminal, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path as FilePath, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};

const JSON_BODY_LIMIT: usize = 4 * 1024 * 1024;
const MAX_CONTENT_CHARS: usize = 20_000;
const MAX_PROMPT_DOCUMENT_CHARS: usize = 200_000;
const MAX_PROMPT_DOCUMENTS: usize = 128;
const MAX_SECRET_CHARS: usize = 100_000;
const EVENT_CAPACITY: usize = 4096;
const AUTH_COOKIE: &str = "gqy_session";
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_ATTEMPT_LIMIT: u8 = 5;

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLES_CSS: &str = include_str!("../web/styles.css");
const APP_JS: &str = include_str!("../web/app.js");
const USAGE_VIZ_JS: &str = include_str!("../web/usage-viz.js");
const GQY_LOGO: &[u8] = include_bytes!("../pics/GQY-avatar.png");
const GQY_WALLPAPER: &[u8] = include_bytes!("../pics/GQY-image.png");
const PROVIDER_ICONS: &str = include_str!("../web/assets/provider-icons.svg");

#[derive(Clone)]
struct WebState {
    auth: WebAuth,
    boot_id: Arc<str>,
    paths: GqyPaths,
    manager: Arc<Mutex<ManagerState>>,
    state_store: StateStore,
    events: EventHub,
    questions: QuestionBroker,
    actor_tx: mpsc::UnboundedSender<ActorCommand>,
    /// 余额查询缓存（60s 防抖：每次对话后刷新一次即可，避免频繁请求公开接口）
    balance_cache: Arc<Mutex<Option<(std::time::Instant, serde_json::Value)>>>,
    /// pi 工具桥共用的工具注册表（Web 端 /api/tools/call 与桥共享）
    bridge_registry: Arc<std::sync::Mutex<ToolRegistry>>,
    /// 优雅退出信号（/api/shutdown 触发，菜单栏「退出」用它统一收尾）
    shutdown: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
struct WebAuth {
    password_digest: Option<[u8; 32]>,
    sessions: Arc<Mutex<HashSet<String>>>,
    attempts: Arc<Mutex<HashMap<IpAddr, LoginAttempt>>>,
}

#[derive(Clone, Copy)]
struct LoginAttempt {
    window_started: Instant,
    failures: u8,
}

#[derive(Debug, Clone, Copy)]
enum LoginFailure {
    Invalid,
    RateLimited,
}

impl WebAuth {
    fn new(password: Option<&str>) -> Self {
        let password_digest = password.map(|password| {
            let mut digest = Sha256::new();
            digest.update(password.as_bytes());
            digest.finalize().into()
        });
        Self {
            password_digest,
            sessions: Arc::new(Mutex::new(HashSet::new())),
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn required(&self) -> bool {
        self.password_digest.is_some()
    }

    fn is_authenticated(&self, supplied: Option<&str>) -> bool {
        if !self.required() {
            return true;
        }
        supplied.is_some_and(|token| self.sessions.lock().unwrap().contains(token))
    }

    fn login(&self, peer: IpAddr, password: &str) -> std::result::Result<String, LoginFailure> {
        let Some(expected) = self.password_digest else {
            return Ok(String::new());
        };
        let now = Instant::now();
        {
            let mut attempts = self.attempts.lock().unwrap();
            let entry = attempts.entry(peer).or_insert(LoginAttempt {
                window_started: now,
                failures: 0,
            });
            if now.duration_since(entry.window_started) >= LOGIN_WINDOW {
                entry.window_started = now;
                entry.failures = 0;
            }
            if entry.failures >= LOGIN_ATTEMPT_LIMIT {
                return Err(LoginFailure::RateLimited);
            }
        }

        let mut digest = Sha256::new();
        digest.update(password.as_bytes());
        let supplied: [u8; 32] = digest.finalize().into();
        if !constant_time_eq(&supplied, &expected) {
            let mut attempts = self.attempts.lock().unwrap();
            if let Some(entry) = attempts.get_mut(&peer) {
                entry.failures = entry.failures.saturating_add(1);
            }
            return Err(LoginFailure::Invalid);
        }

        let token = random_token(32);
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(token.clone());
        if sessions.len() > 64 {
            sessions.clear();
            sessions.insert(token.clone());
        }
        Ok(token)
    }
}

struct ManagerState {
    config: AppConfig,
    active_run_id: Option<String>,
    admin_busy: bool,
    context: ContextSnapshot,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct ContextSnapshot {
    tokens: u64,
    window: Option<usize>,
}

enum ActorCommand {
    StartTurn {
        run_id: String,
        content: String,
        images: Vec<Option<PastedImage>>,
        mode: AgentMode,
    },
    /// pi 控制命令（模型/思考级别）
    Pi {
        kind: PiCommandKind,
        value: String,
        reply: oneshot::Sender<Result<serde_json::Value>>,
    },
    Cancel {
        run_id: String,
    },
    SetModels {
        models: Vec<ActiveProviderModelConfig>,
        reply: oneshot::Sender<std::result::Result<(), AdminFailure>>,
    },
    ApplyConfig {
        config: AppConfig,
        prompts: PromptDocuments,
        reset_conversation: bool,
        reply: oneshot::Sender<std::result::Result<(), AdminFailure>>,
    },
    ResetConversation {
        reply: oneshot::Sender<std::result::Result<(), AdminFailure>>,
    },
    Shutdown,
}

#[derive(Debug)]
enum AdminFailure {
    Invalid(String),
    Internal(String),
}

#[derive(Clone, Debug)]
struct EventRecord {
    id: u64,
    kind: String,
    data: String,
}

#[derive(Clone)]
struct EventHub {
    inner: Arc<Mutex<EventHubInner>>,
    sender: broadcast::Sender<EventRecord>,
}

struct EventHubInner {
    next_id: u64,
    records: VecDeque<EventRecord>,
}

struct EventSubscription {
    pending: VecDeque<EventRecord>,
    receiver: broadcast::Receiver<EventRecord>,
}

impl EventHub {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(EventHubInner {
                next_id: 1,
                records: VecDeque::with_capacity(EVENT_CAPACITY),
            })),
            sender,
        }
    }

    fn publish(&self, kind: impl Into<String>, data: Value) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1);
        let record = EventRecord {
            id,
            kind: kind.into(),
            data: serde_json::to_string(&data)
                .unwrap_or_else(|_| "{\"error\":\"event serialization failed\"}".to_string()),
        };
        if inner.records.len() == EVENT_CAPACITY {
            inner.records.pop_front();
        }
        inner.records.push_back(record.clone());
        let _ = self.sender.send(record);
        id
    }

    fn latest_id(&self) -> u64 {
        self.inner.lock().unwrap().next_id.saturating_sub(1)
    }

    fn subscribe_after(&self, after: u64) -> EventSubscription {
        let mut inner = self.inner.lock().unwrap();
        let receiver = self.sender.subscribe();
        let pending = replay_records(&mut inner, after);
        EventSubscription { pending, receiver }
    }

    fn replay_after(&self, after: u64) -> VecDeque<EventRecord> {
        replay_records(&mut self.inner.lock().unwrap(), after)
    }
}

fn replay_records(inner: &mut EventHubInner, after: u64) -> VecDeque<EventRecord> {
    if after > inner.next_id.saturating_sub(1) {
        return resync_record(inner);
    }
    let Some(oldest) = inner.records.front().map(|record| record.id) else {
        return VecDeque::new();
    };
    if after < oldest.saturating_sub(1) {
        return resync_record(inner);
    }
    inner
        .records
        .iter()
        .filter(|record| record.id > after)
        .cloned()
        .collect()
}

fn resync_record(inner: &mut EventHubInner) -> VecDeque<EventRecord> {
    let id = inner.next_id;
    inner.next_id = inner.next_id.saturating_add(1);
    VecDeque::from([EventRecord {
        id,
        kind: "resync_required".to_string(),
        data: json!({ "latest_event_id": id }).to_string(),
    }])
}

#[derive(Clone)]
struct QuestionBroker {
    pending: Arc<Mutex<HashMap<String, PendingQuestion>>>,
}

struct PendingQuestion {
    run_id: String,
    request: QuestionRequest,
    responder: oneshot::Sender<QuestionResponse>,
}

#[derive(Debug)]
enum AnswerFailure {
    NotFound,
    Invalid(String),
    Gone,
}

impl QuestionBroker {
    fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn insert(
        &self,
        run_id: &str,
        request: QuestionRequest,
        responder: oneshot::Sender<QuestionResponse>,
    ) -> String {
        let mut pending = self.pending.lock().unwrap();
        loop {
            let question_id = random_id("question", 18);
            if !pending.contains_key(&question_id) {
                pending.insert(
                    question_id.clone(),
                    PendingQuestion {
                        run_id: run_id.to_string(),
                        request,
                        responder,
                    },
                );
                return question_id;
            }
        }
    }

    fn answer<F>(
        &self,
        question_id: &str,
        answers: QuestionAnswers,
        before_resume: F,
    ) -> std::result::Result<(), AnswerFailure>
    where
        F: FnOnce(&str, &QuestionAnswers),
    {
        let mut all_pending = self.pending.lock().unwrap();
        let request = all_pending
            .get(question_id)
            .map(|pending| pending.request.clone())
            .ok_or(AnswerFailure::NotFound)?;
        let answers = normalize_answers(&request, answers).map_err(AnswerFailure::Invalid)?;
        let pending = all_pending
            .remove(question_id)
            .ok_or(AnswerFailure::NotFound)?;
        let run_id = pending.run_id;
        pending
            .responder
            .send(QuestionResponse::Answered(answers.clone()))
            .map_err(|_| AnswerFailure::Gone)?;
        before_resume(&run_id, &answers);
        Ok(())
    }

    fn cancel_run(&self, run_id: &str) {
        let cancelled = {
            let mut pending = self.pending.lock().unwrap();
            let ids = pending
                .iter()
                .filter(|(_, question)| question.run_id == run_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| pending.remove(&id))
                .collect::<Vec<_>>()
        };
        for pending in cancelled {
            let _ = pending.responder.send(QuestionResponse::Cancelled);
        }
    }
}

struct RunEventMapper {
    run_id: String,
    events: EventHub,
    questions: QuestionBroker,
    state_store: StateStore,
    turn_id: Option<String>,
    tool_counter: u64,
    active_tool: Option<ActiveTool>,
}

struct ActiveTool {
    id: String,
    name: String,
    event_name: String,
}

impl RunEventMapper {
    fn new(
        run_id: String,
        events: EventHub,
        questions: QuestionBroker,
        state_store: StateStore,
    ) -> Self {
        Self {
            run_id,
            events,
            questions,
            state_store,
            turn_id: None,
            tool_counter: 0,
            active_tool: None,
        }
    }

    fn publish(&self, kind: &str, data: Value) {
        self.events.publish(kind, data);
    }

    fn next_tool(&mut self, event_name: String) -> ActiveTool {
        self.tool_counter = self.tool_counter.saturating_add(1);
        ActiveTool {
            id: format!("{}_tool_{}", self.run_id, self.tool_counter),
            name: real_tool_name(&event_name).to_string(),
            event_name,
        }
    }

    fn handle(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TurnStarted { turn_id } => {
                self.turn_id = Some(turn_id.clone());
                self.publish(
                    "turn.started",
                    json!({ "run_id": self.run_id, "turn_id": turn_id }),
                );
            }
            AgentEvent::Chunk(chunk) => match chunk.kind {
                ChatStreamKind::Content => self.publish(
                    "assistant.delta",
                    json!({ "run_id": self.run_id, "delta": chunk.text }),
                ),
                ChatStreamKind::Reasoning => self.publish(
                    "reasoning.delta",
                    json!({ "run_id": self.run_id, "delta": chunk.text }),
                ),
                _ => {}
            },
            AgentEvent::ReasoningStart { .. } => {
                self.publish("reasoning.start", json!({ "run_id": self.run_id }))
            }
            AgentEvent::ReasoningReset { .. } => {
                self.publish("reasoning.reset", json!({ "run_id": self.run_id }))
            }
            AgentEvent::ReasoningPartStart { .. } => {
                self.publish("reasoning.part_start", json!({ "run_id": self.run_id }))
            }
            AgentEvent::ReasoningPartEnd { .. } => {
                self.publish("reasoning.part_end", json!({ "run_id": self.run_id }))
            }
            AgentEvent::ReasoningTitle(title) => self.publish(
                "reasoning.title",
                json!({ "run_id": self.run_id, "title": title }),
            ),
            AgentEvent::ToolCall { name, arguments } => {
                let tool = self.next_tool(name);
                self.publish(
                    "tool.started",
                    json!({
                        "run_id": self.run_id,
                        "tool_id": tool.id,
                        "name": tool.name,
                        "display_name": tools::readable_tool_name(&tool.event_name),
                        "arguments": arguments,
                    }),
                );
                self.active_tool = Some(tool);
            }
            AgentEvent::ToolProgress { name, message } => {
                let (tool_id, tool_name) = self.tool_identity(&name);
                self.publish(
                    "tool.progress",
                    json!({
                        "run_id": self.run_id,
                        "tool_id": tool_id,
                        "name": tool_name,
                        "message": message,
                    }),
                );
            }
            AgentEvent::CommandOutput {
                name,
                stream,
                chunk,
            } => {
                let (tool_id, tool_name) = self.tool_identity(&name);
                let stream = match stream {
                    CommandOutputStream::Stdout => "stdout",
                    CommandOutputStream::Stderr => "stderr",
                };
                self.publish(
                    "tool.output",
                    json!({
                        "run_id": self.run_id,
                        "tool_id": tool_id,
                        "name": tool_name,
                        "stream": stream,
                        "output": String::from_utf8_lossy(&chunk),
                    }),
                );
            }
            AgentEvent::ToolResult { name, ok, output } => {
                let tool = self
                    .active_tool
                    .take()
                    .unwrap_or_else(|| self.next_tool(name));
                self.publish(
                    "tool.finished",
                    json!({
                        "run_id": self.run_id,
                        "tool_id": tool.id,
                        "name": tool.name,
                        "ok": ok,
                        "output": output,
                    }),
                );
            }
            AgentEvent::PrepareForExternalOutput { ready } => {
                let _ = ready.send(false);
            }
            AgentEvent::Image { name, path, alt, emotion, action } => {
                let (tool_id, tool_name) = self.tool_identity(&name);
                let hide_caption = tool_name == "show_meme";
                let Some(turn_id) = self.turn_id.as_deref() else {
                    self.publish(
                        "tool.image",
                        json!({
                            "run_id": self.run_id,
                            "tool_id": tool_id,
                            "name": tool_name,
                            "error": "image could not be associated with the current turn",
                        }),
                    );
                    return;
                };
                match self
                    .state_store
                    .save_image_asset(turn_id, Some(&tool_id), &path, &alt)
                {
                    Ok(asset) => self.publish(
                        "tool.image",
                        json!({
                            "run_id": self.run_id,
                            "tool_id": tool_id,
                            "name": tool_name,
                            "emotion": emotion,
                            "action": action,
                            "asset": SafeImageAsset::from_asset(asset, hide_caption),
                        }),
                    ),
                    Err(error) => {
                        tracing::warn!(
                            run_id = %self.run_id,
                            tool = %tool_name,
                            error = %error,
                            "failed to persist a WebUI image"
                        );
                        self.publish(
                            "tool.image",
                            json!({
                                "run_id": self.run_id,
                                "tool_id": tool_id,
                                "name": tool_name,
                                "error": "image could not be added to the WebUI",
                            }),
                        );
                    }
                }
            }
            AgentEvent::AskQuestion { request, responder } => {
                let question_id = self
                    .questions
                    .insert(&self.run_id, request.clone(), responder);
                let (tool_id, tool_name) = self.tool_identity("ask_question");
                self.publish(
                    "question.requested",
                    json!({
                        "run_id": self.run_id,
                        "question_id": question_id,
                        "tool_id": tool_id,
                        "name": tool_name,
                        "questions": request.questions,
                    }),
                );
            }
            AgentEvent::QueuedPromptsConsumed {
                prompt_ids,
                mode,
                provider_id,
                model,
            } => self.publish(
                "queue.consumed",
                json!({
                    "run_id": self.run_id,
                    "prompt_ids": prompt_ids,
                    "mode": mode_name(mode),
                    "provider_id": provider_id,
                    "model": model,
                }),
            ),
            AgentEvent::SpinnerTick => {}
            AgentEvent::CompactStart => {
                self.publish("context.compact_start", json!({ "run_id": self.run_id }))
            }
            AgentEvent::CompactChunk(chunk) => self.publish(
                "context.compact_delta",
                json!({ "run_id": self.run_id, "delta": chunk.text }),
            ),
            AgentEvent::CompactEnd => {
                self.publish("context.compact_end", json!({ "run_id": self.run_id }))
            }
            AgentEvent::PopStart => {
                self.publish("context.pop_start", json!({ "run_id": self.run_id }))
            }
            AgentEvent::PopEnd => self.publish("context.pop_end", json!({ "run_id": self.run_id })),
        }
    }

    fn tool_identity(&self, fallback: &str) -> (String, String) {
        self.active_tool
            .as_ref()
            .map(|tool| (tool.id.clone(), tool.name.clone()))
            .unwrap_or_else(|| {
                (
                    format!(
                        "{}_tool_{}",
                        self.run_id,
                        self.tool_counter.saturating_add(1)
                    ),
                    real_tool_name(fallback).to_string(),
                )
            })
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "WebUI request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": { "message": self.message } })),
        )
            .into_response()
    }
}

#[derive(Default, Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: u64,
}

#[derive(Clone, Copy)]
enum PiCommandKind {
    GetState,
    GetModels,
    SetModel,
    SetThinking,
}

/// 解析 pi 控制命令（web /api/pi/* 用）
fn parse_pi_command(path: &str) -> Option<PiCommandKind> {
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
struct WebImageInput {
    mime: String,
    data_base64: String,
}

/// 把前端 base64 图片转成 agent 用的 PastedImage（解码失败则丢弃该项）。
fn pasted_images_from_input(images: &[WebImageInput]) -> Vec<Option<PastedImage>> {
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

fn queued_attachments_from_input(images: &[WebImageInput]) -> Vec<QueuedPromptAttachment> {
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
struct CreateTurnRequest {
    content: String,
    mode: String,
    #[serde(default)]
    images: Vec<WebImageInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueuePromptRequest {
    content: String,
    #[serde(default)]
    images: Vec<WebImageInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerQuestionRequest {
    answers: QuestionAnswers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetModelsRequest {
    models: Vec<ActiveProviderModelConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateConfigRequest {
    config: Value,
    #[serde(default)]
    secrets: HashMap<String, SecretMutation>,
    prompts: PromptDocuments,
    #[serde(default)]
    reset_conversation: bool,
}

#[derive(Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
enum SecretMutation {
    Set(String),
    Clear,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptDocuments {
    #[serde(default)]
    personas: Vec<PromptDocument>,
    #[serde(default)]
    identities: Vec<PromptDocument>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PromptDocument {
    name: String,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_name: Option<String>,
}

#[derive(Serialize)]
struct ConfigResponse {
    config: Value,
    secret_states: HashMap<String, bool>,
    prompts: PromptDocuments,
    models: Vec<SafeModel>,
    multimodal_models: Vec<SafeModel>,
    display: WebDisplayConfig,
    context: ContextSnapshot,
}

#[derive(Serialize)]
struct BootstrapResponse {
    version: &'static str,
    boot_id: String,
    latest_event_id: u64,
    active_run_id: Option<String>,
    running_turn_id: Option<String>,
    external_queue_available: bool,
    /// 本 WebUI 会话所属通道（默认 webui，GQY_CHANNEL 可覆盖）
    channel: String,
    /// 全部会话通道摘要（终端/网页/QQ/Telegram…），左侧通道列表
    channels: Vec<SafeChannelSummary>,
    /// 当前通道的会话列表（含归档历史对话），左侧历史对话列表
    conversations: Vec<SafeConversationSummary>,
    turns: Vec<SafeTurn>,
    queued_prompts: Vec<SafeQueuedPrompt>,
    models: Vec<SafeModel>,
    display: WebDisplayConfig,
    context: ContextSnapshot,
    usage: SafeUsageSnapshot,
    /// 引擎标识：`pi`（pi 底座）或 provider id
    engine: String,
    capabilities: Capabilities,
}

#[derive(Serialize)]
struct SafeChannelSummary {
    id: String,
    turn_count: u64,
    last_seq: i64,
    title: String,
    snippet: String,
    timestamp: Option<String>,
    running: bool,
}

#[derive(Serialize)]
struct SafeConversationSummary {
    id: String,
    title: String,
    snippet: String,
    timestamp: Option<String>,
    turn_count: u64,
    active: bool,
}

#[derive(Serialize)]
struct Capabilities {
    multi_conversation: bool,
    attachments: bool,
    queue: bool,
}

#[derive(Clone, Serialize)]
struct WebDisplayConfig {
    reasoning: String,
    tool_calls: String,
    readable_tool_names: bool,
    command_output_lines: usize,
    mixed_model_endpoint_display: String,
    show_mixed_model_endpoint: bool,
}

#[derive(Clone, Serialize)]
struct SafeQueuedPrompt {
    id: String,
    content: String,
    submitted_at: String,
}

#[derive(Serialize)]
struct SafeModel {
    provider_id: String,
    provider_name: String,
    model: String,
    active: bool,
}

#[derive(Serialize)]
struct SafeTurn {
    id: String,
    seq: i64,
    status: &'static str,
    active_context: bool,
    user_content: String,
    assistant_content: String,
    assistant_reasoning: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    user_timestamp: String,
    assistant_timestamp: Option<String>,
    token_total: u64,
    token_usage_estimated: bool,
    question_exchanges: Vec<crate::question::QuestionExchange>,
    followups: Vec<SafeFollowup>,
    assets: Vec<SafeImageAsset>,
}

#[derive(Serialize)]
struct SafeFollowup {
    id: String,
    content: String,
    submitted_at: String,
    preceding_assistant_content: Option<String>,
    preceding_assistant_reasoning: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
}

#[derive(Serialize)]
struct SafeImageAsset {
    id: String,
    url: String,
    mime: String,
    width: u32,
    height: u32,
    alt: String,
    hide_caption: bool,
    /// 资产归属：`user`（用户随消息发送的图片）/ `tool`（工具输出，表情包等）
    source: String,
}

#[derive(Serialize)]
struct SafeUsageSnapshot {
    requests: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    last_usage: Option<Usage>,
    last_conversation_usage: Option<Usage>,
}

#[derive(Serialize)]
struct ModelResponse {
    models: Vec<SafeModel>,
    display: WebDisplayConfig,
    context: ContextSnapshot,
}

pub async fn run(paths: GqyPaths, args: WebArgs) -> Result<()> {
    // WebUI 面板默认在独立的 webui 通道对话（与终端/QQ/Telegram 各自独立上下文）；
    // 终端显式设置 GQY_CHANNEL 可覆盖
    if std::env::var_os("GQY_CHANNEL").is_none() {
        unsafe { std::env::set_var("GQY_CHANNEL", "webui") };
    }
    let password = resolve_web_password(&args)?;
    let bind_ip: IpAddr = args
        .host
        .parse()
        .with_context(|| format!("invalid WebUI host: {}", args.host))?;
    if !bind_ip.is_loopback() && password.is_none() {
        anyhow::bail!(
            "绑定非回环地址（{}）必须设置访问密码：gqy web --host {} -p <password>",
            args.host,
            args.host
        );
    }
    AppConfig::init_files(&paths)?;
    let config = AppConfig::load_or_default(&paths)?;
    let state_store = StateStore::new(&paths)?;
    state_store.init_files()?;
    let client = LlmClient::from_config(&config, &paths)?;
    let registry = build_tool_registry(&config, &paths, AgentMode::Normal, true)?;
    let bridge_registry = Arc::new(std::sync::Mutex::new(registry.clone()));
    let agent = Agent::new(
        config.clone(),
        &paths,
        state_store.clone(),
        client,
        registry,
        AgentMode::Normal,
    )?;
    let context = ContextSnapshot {
        tokens: agent.effective_context_tokens()?,
        window: agent.context_window(),
    };

    let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind_ip, args.port))
        .await
        .with_context(|| format!("binding 顾清影 WebUI to {}:{}", args.host, args.port))?;
    let port = listener.local_addr()?.port();
    let boot_id: Arc<str> = random_id("boot", 18).into();
    let events = EventHub::new();
    let questions = QuestionBroker::new();
    let manager = Arc::new(Mutex::new(ManagerState {
        config: config.clone(),
        active_run_id: None,
        admin_busy: false,
        context,
    }));
    let (actor_tx, actor_join) = spawn_actor(
        agent,
        config,
        paths.clone(),
        state_store.clone(),
        manager.clone(),
        events.clone(),
        questions.clone(),
    )?;

    // pi 底座模式：启动工具桥；图片事件（表情包等）经 events/state_store 落到 WebUI 资产
    let bridge_sink: Option<Arc<dyn Fn(std::path::PathBuf, String) + Send + Sync>> = Some(Arc::new({
        let events = events.clone();
        let state_store = state_store.clone();
        let manager = manager.clone();
        move |path, alt| {
            publish_bridge_image(&events, &state_store, &manager, path, alt);
        }
    }));
    let bridge_progress_sink: Option<Arc<dyn Fn(String) + Send + Sync>> = Some(Arc::new({
        let events = events.clone();
        let manager = manager.clone();
        move |message| {
            publish_bridge_progress(&events, &manager, message);
        }
    }));
    crate::pi_bridge::ensure_pi_bridge(bridge_registry.clone(), &paths, bridge_sink, bridge_progress_sink)
        .await
        .with_context(|| "failed to start pi tool bridge")?;

    // 配置文件热重载：检测 GQY_HOME/config/config.jsonc 被外部修改
    // （CLI `gqy config set`、直接编辑等），自动重建 agent 并通知前端，
    // 让菜单栏 / CLI / 面板三端配置始终同步。
    spawn_config_watcher(paths.clone(), actor_tx.clone(), events.clone());

    let state = WebState {
        auth: WebAuth::new(password.as_deref()),
        boot_id,
        paths,
        manager,
        state_store,
        events,
        questions,
        actor_tx: actor_tx.clone(),
        balance_cache: Arc::new(Mutex::new(None)),
        bridge_registry,
        shutdown: Arc::new(tokio::sync::Notify::new()),
    };
    let shutdown_notify = state.shutdown.clone();
    let app = router(state);
    // 只有设置了密码才把局域网地址列出来（无密码时仅回环可达）
    let urls = web_access_urls(port, password.is_some());
    for url in &urls {
        println!("顾清影 WebUI: {url}");
    }
    std::io::stdout().flush().ok();
    if !args.no_open {
        open_browser(&format!("http://127.0.0.1:{port}"));
    }

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .into_future();
    tokio::pin!(server);
    let serve_result = tokio::select! {
        result = &mut server => result,
        _ = shutdown_signal() => Ok(()),
        _ = shutdown_notify.notified() => Ok(()),
    };
    let _ = actor_tx.send(ActorCommand::Shutdown);
    let actor_result = tokio::task::spawn_blocking(move || actor_join.join())
        .await
        .context("joining WebUI actor task")?
        .map_err(|_| anyhow::anyhow!("WebUI actor thread panicked"))?;
    serve_result.context("serving 顾清影 WebUI")?;
    actor_result
}

fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index_asset))
        .route("/styles.css", get(styles_asset))
        .route("/app.js", get(app_asset))
        .route("/usage-viz.js", get(usage_viz_asset))
        .route("/assets/gqy-logo.png", get(logo_asset))
        .route("/assets/gqy-wallpaper.png", get(wallpaper_asset))
        .route("/assets/provider-icons.svg", get(provider_icons_asset))
        .route("/api/health", get(health))
        .route("/api/auth/login", post(auth_login))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/config", get(get_config).put(update_config))
        .route("/api/events", get(events))
        .route("/api/assets/{asset_id}", get(image_asset))
        .route("/api/turns", post(create_turn))
        .route("/api/queue", post(queue_prompt))
        .route("/api/queue/{prompt_id}", delete(remove_queue_prompt))
        .route("/api/runs/{run_id}/cancel", post(cancel_run))
        .route("/api/questions/{question_id}/answer", post(answer_question))
        .route("/api/models/active", put(set_models))
        .route("/api/conversation/reset", post(reset_conversation))
        .route("/api/alarms", get(list_alarms_web))
        .route("/api/state", get(session_state))
        .route("/api/usage/stats", get(usage_stats_web))
        .route("/api/usage/details", get(usage_details_web))
        .route("/api/backup/status", get(backup_status_web))
        .route("/api/channels/{channel_id}/turns", get(channel_turns_web))
        .route("/api/search", get(search_web))
        .route("/api/tools/call", post(call_tool_web))
        .route("/api/pi/{kind}", get(pi_control_web).post(pi_control_web))
        .route("/api/export", get(export_conversation_web))
        .route("/api/shutdown", post(shutdown_web))
        .route(
            "/api/conversations/{conversation_id}/turns",
            get(conversation_turns_web),
        )
        .route("/api/balance", get(balance_web))
        .route("/api/alarms/{alarm_id}", delete(cancel_alarm_web))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .with_state(state)
}

async fn index_asset() -> Response {
    text_asset(INDEX_HTML, "text/html; charset=utf-8")
}

async fn styles_asset() -> Response {
    text_asset(STYLES_CSS, "text/css; charset=utf-8")
}

async fn app_asset() -> Response {
    text_asset(APP_JS, "application/javascript; charset=utf-8")
}

async fn usage_viz_asset() -> Response {
    text_asset(USAGE_VIZ_JS, "application/javascript; charset=utf-8")
}

async fn logo_asset() -> Response {
    binary_asset(GQY_LOGO, "image/png")
}

async fn wallpaper_asset() -> Response {
    binary_asset(GQY_WALLPAPER, "image/png")
}

async fn provider_icons_asset() -> Response {
    text_asset(PROVIDER_ICONS, "image/svg+xml; charset=utf-8")
}

fn text_asset(content: &'static str, content_type: &'static str) -> Response {
    asset_response(content.as_bytes(), content_type)
}

fn binary_asset(content: &'static [u8], content_type: &'static str) -> Response {
    asset_response(content, content_type)
}

fn asset_response(content: &'static [u8], content_type: &'static str) -> Response {
    let mut response = content.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

async fn auth_login(
    State(state): State<WebState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> std::result::Result<Response, ApiError> {
    if !origin_is_allowed(&headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "request origin is not allowed",
        ));
    }
    if !state.auth.required() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if request.password.chars().count() > 1_024 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "password is too long",
        ));
    }
    let session = match state.auth.login(peer.ip(), &request.password) {
        Ok(session) => session,
        Err(LoginFailure::Invalid) => {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid password"));
        }
        Err(LoginFailure::RateLimited) => {
            let mut response = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many login attempts; try again shortly",
            )
            .into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("60"));
            return Ok(response);
        }
    };
    let cookie =
        format!("{AUTH_COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400");
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(ApiError::internal)?,
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn resolve_web_password(args: &WebArgs) -> Result<Option<String>> {
    let password = if let Some(path) = &args.password_file {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("reading WebUI password file: {}", path.display()))?;
        Some(contents.trim_end_matches(['\r', '\n']).to_string())
    } else {
        match &args.password {
            Some(password) if !password.is_empty() => Some(password.clone()),
            Some(_) if io::stdin().is_terminal() => {
                Some(rpassword::prompt_password("WebUI password: ")?)
            }
            Some(_) => {
                anyhow::bail!("-p requires an interactive terminal or an explicit password value")
            }
            None => None,
        }
    };
    if let Some(password) = &password {
        if password.is_empty() {
            anyhow::bail!("WebUI password cannot be empty");
        }
        if password.chars().count() > 1_024 {
            anyhow::bail!("WebUI password cannot exceed 1,024 characters");
        }
    }
    Ok(password)
}

fn web_access_urls(port: u16, include_lan: bool) -> Vec<String> {
    let mut addresses = BTreeSet::new();
    addresses.insert(Ipv4Addr::LOCALHOST);
    if include_lan {
        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            for interface in interfaces {
                if let if_addrs::IfAddr::V4(address) = interface.addr {
                    if !address.ip.is_unspecified() {
                        addresses.insert(address.ip);
                    }
                }
            }
        }
    }
    addresses
        .into_iter()
        .map(|address| format!("http://{address}:{port}"))
        .collect()
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ready",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// 会话状态（供终端/面板同步轮询）：当前会话最大 seq + 是否有运行中的轮次。
/// 终端 `gqy` 与面板共享同一 conversation.db；前端轮询此接口，
/// 发现 seq 变化即重载历史，实现双端同步。
async fn session_state(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let visible = state
        .state_store
        .load_visible_turns()
        .map_err(ApiError::internal)?;
    let last_seq = visible
        .iter()
        .filter(|turn| !turn.is_summary)
        .count() as i64;
    let running = state
        .state_store
        .has_running_turns()
        .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "ok": true,
        "last_seq": last_seq,
        "running": running,
    }))
    .into_response())
}

/// 最近一次备份结果（WebUI 展示）：读 state/last_backup.json（由 backup.rs 写入）
async fn backup_status_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, ApiError> {
    require_auth(&headers, &state)?;
    let path = state.paths.state_dir.join("last_backup.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Json)
            .map_err(ApiError::internal),
        Err(_) => Ok(Json(json!({
            "ok": false,
            "error": "no backup record yet",
        }))),
    }
}

/// 用量统计（贡献图数据源）：每日 token + 按模型明细。
async fn usage_stats_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let stats = state
        .state_store
        .usage_stats()
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "stats": stats })).into_response())
}

/// 最近调用明细（时间/模型/输入/输出/缓存命中/是否记忆辅助），供用量页列表与模型详情。
async fn usage_details_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let details = state
        .state_store
        .usage_details(500)
        .map_err(ApiError::internal)?;
    let mut response = Json(json!({ "ok": true, "records": details })).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// 余额查询（DeepSeek 等公开接口）：60 秒缓存防抖，
/// 前端每次对话完成后刷新一次即可，不做轮询。
async fn balance_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
    const ERROR_TTL: std::time::Duration = std::time::Duration::from_secs(10);
    let cache = state.balance_cache.clone();
    if let Some((at, value)) = cache.lock().unwrap().as_ref() {
        let ttl = if value.get("error").is_some() {
            ERROR_TTL
        } else {
            CACHE_TTL
        };
        if at.elapsed() < ttl {
            return Ok(Json(json!({ "ok": true, "cached": true, "balance": value })).into_response());
        }
    }
    let paths = state.paths.clone();
    let manager = state.manager.clone();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<serde_json::Value, anyhow::Error> {
        let config = manager.lock().unwrap().config.clone();
        let provider_id = config
            .provider(None)
            .map(|provider| provider.id.clone())
            .unwrap_or_default();
        match crate::balance::fetch_balance(&config, &paths) {
            Ok(Some(infos)) => Ok(json!({
                "provider": provider_id,
                "balance": infos.iter().map(|info| json!({
                    "currency": info.currency,
                    "total": info.total_balance,
                    "granted": info.granted_balance,
                    "topped_up": info.topped_up_balance,
                })).collect::<Vec<_>>(),
            })),
            Ok(None) => Ok(json!({ "provider": provider_id, "unsupported": true })),
            Err(error) => Ok(json!({ "provider": provider_id, "error": format!("{error:#}") })),
        }
    })
    .await
    .map_err(ApiError::internal)?;
    let payload = result.map_err(ApiError::internal)?;
    {
        let mut cache = cache.lock().unwrap();
        *cache = Some((std::time::Instant::now(), payload.clone()));
    }
    let mut response =
        Json(json!({ "ok": true, "cached": false, "balance": payload })).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// 取消定时任务（面板「取消」按钮）。
async fn cancel_alarm_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(alarm_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let cancelled = crate::alarm::cancel(&state.paths, &alarm_id).map_err(ApiError::internal)?;
    if !cancelled {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "alarm not found"));
    }
    Ok(Json(json!({ "ok": true, "id": alarm_id })).into_response())
}

/// 定时任务（闹钟/番茄钟）列表，供面板可视化。
async fn list_alarms_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let records = crate::alarm::cleanup_dead(&state.paths).map_err(ApiError::internal)?;
    let alarms = records
        .into_iter()
        .map(|record| {
            json!({
                "id": record.id,
                "label": record.label,
                "time": record.time,
                "due_at": record.due_at,
                "due_at_local": crate::alarm::format_due_at(record.due_at),
                "repeat_seconds": record.repeat_seconds,
                "status": match record.status {
                    crate::alarm::AlarmStatus::Scheduled => "scheduled",
                    crate::alarm::AlarmStatus::Ringing => "ringing",
                },
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({ "ok": true, "alarms": alarms })).into_response())
}

/// 读取指定通道的历史对话（WebUI 左侧通道列表点击切换查看；
/// 非本进程通道只读展示，不影响各自上下文）
async fn channel_turns_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let mut assets_by_turn = HashMap::<String, Vec<ImageAsset>>::new();
    for asset in state
        .state_store
        .load_image_assets()
        .map_err(ApiError::internal)?
    {
        assets_by_turn
            .entry(asset.turn_id.clone())
            .or_default()
            .push(asset);
    }
    let turns: Vec<SafeTurn> = state
        .state_store
        .load_visible_turns_for_channel(&channel_id)
        .map_err(ApiError::internal)?
        .into_iter()
        .filter(|turn| !turn.is_summary)
        .map(|turn| {
            let assets = assets_by_turn.remove(&turn.turn_id).unwrap_or_default();
            SafeTurn::from_turn(turn, assets)
        })
        .collect();
    let running = state
        .state_store
        .load_visible_turns_for_channel(&channel_id)
        .map_err(ApiError::internal)?
        .iter()
        .any(|turn| turn.status == TurnStatus::Running);
    let conversations = state
        .state_store
        .conversation_summaries_for_channel(&channel_id)
        .map_err(ApiError::internal)?
        .into_iter()
        .map(safe_conversation_summary)
        .collect::<Vec<_>>();
    let mut response = Json(
        json!({ "ok": true, "channel": channel_id, "running": running, "turns": turns, "conversations": conversations }),
    )
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// pi 控制：state/models 查询 + model/thinking 设置。
/// 经 ActorCommand::Pi 转发到持有 agent 的线程执行（回合运行中也可切换）。
async fn pi_control_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    method: axum::http::Method,
    Path(kind): Path<String>,
    body: axum::body::Bytes,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let Some(command) = parse_pi_command(&kind) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown pi command"));
    };
    let value = if matches!(command, PiCommandKind::SetModel | PiCommandKind::SetThinking) {
        let payload: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        payload
            .get(if matches!(command, PiCommandKind::SetModel) { "modelId" } else { "level" })
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let (tx, rx) = oneshot::channel();
    state
        .actor_tx
        .send(ActorCommand::Pi {
            kind: command,
            value,
            reply: tx,
        })
        .map_err(ApiError::internal)?;
    let result = rx.await.map_err(ApiError::internal)?;
    let payload = result.map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "data": payload })).into_response())
}

/// 导出当前会话为 markdown 文档。
async fn export_conversation_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let turns = state
        .state_store
        .load_visible_turns()
        .map_err(ApiError::internal)?;
    let mut md = String::new();
    md.push_str("# GQY 对话导出\n\n");
    for turn in turns {
        if turn.is_summary {
            continue;
        }
        md.push_str(&format!("## {}\n\n", turn.user_timestamp));
        if !turn.user_content.trim().is_empty() {
            md.push_str(&format!("**你**：\n\n{}\n\n", turn.user_content));
        }
        if !turn.assistant_content.trim().is_empty() {
            md.push_str(&format!("**顾清影**：\n\n{}\n\n", turn.assistant_content));
        }
    }
    let filename = "gqy-export.md";
    let mut response = Response::new(axum::body::Body::from(md));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/markdown; charset=utf-8"));
    response.headers_mut().insert(
        "content-disposition",
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")).unwrap(),
    );
    Ok(response)
}

/// 优雅退出：菜单栏「退出」调用，停 serve → actor 结束（agent drop → pi 进程组被杀）→ 进程退出。
/// 本机信任端点：不要求 auth（本地进程可触发，外部不可达）。
async fn shutdown_web(State(state): State<WebState>) -> Json<Value> {
    state.shutdown.notify_one();
    Json(json!({ "ok": true, "message": "shutting down" }))
}

/// Web 端调用 GQY 工具（工具块「重跑」用）。
/// 与 pi 工具桥共用 registry；task/deep_research 等长任务给足超时。
async fn call_tool_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<WebToolCallRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state)?;
    let name = request.name.clone();
    let name_for_call = name.clone();
    let arguments = if request.arguments.is_null() {
        json!({})
    } else {
        request.arguments
    };
    let arguments_str = arguments.to_string();
    let registry = state
        .bridge_registry
        .lock()
        .unwrap()
        .clone();
    let timeout = if matches!(name.as_str(), "task" | "deep_research") {
        Duration::from_secs(1800)
    } else {
        Duration::from_secs(180)
    };
    let (progress_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = crate::tools::ToolProgress::new(progress_tx);
    let result = tokio::time::timeout(timeout, async move {
        registry.call_with_progress(&name_for_call, &arguments_str, &progress).await
    })
    .await;
    let body = match result {
        Ok(Ok(output)) => json!({ "ok": true, "output": output }),
        Ok(Err(err)) => json!({ "ok": false, "error": format!("{err:#}") }),
        Err(_) => json!({ "ok": false, "error": format!("tool {name} timed out after {}s", timeout.as_secs()) }),
    };
    Ok(Json(body).into_response())
}

#[derive(Deserialize)]
struct WebToolCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// 全文搜索对话（跨通道），返回匹配轮次（新→旧）
async fn search_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let needle = query.q.trim();
    if needle.is_empty() {
        return Ok(Json(json!({ "ok": true, "query": "", "results": [] })).into_response());
    }
    let limit = query.limit.unwrap_or(30).clamp(1, 200);
    let turns = state
        .state_store
        .search_turns(needle, limit)
        .map_err(ApiError::internal)?;
    let mut assets_by_turn = HashMap::<String, Vec<ImageAsset>>::new();
    for asset in state
        .state_store
        .load_image_assets()
        .map_err(ApiError::internal)?
    {
        assets_by_turn
            .entry(asset.turn_id.clone())
            .or_default()
            .push(asset);
    }
    let results = turns
        .into_iter()
        .map(|turn| {
            let assets = assets_by_turn.remove(&turn.turn_id).unwrap_or_default();
            let turn_id = turn.turn_id.clone();
            let mut safe = SafeTurn::from_turn(turn, assets);
            safe.id = turn_id;
            safe
        })
        .collect::<Vec<_>>();
    let mut response = Json(json!({ "ok": true, "query": needle, "results": results })).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
}

/// 读取指定历史会话的 turns（当前通道内，含归档轮次；只读数据源）
async fn conversation_turns_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let channel = state.state_store.channel().to_string();
    let mut assets_by_turn = HashMap::<String, Vec<ImageAsset>>::new();
    for asset in state
        .state_store
        .load_image_assets()
        .map_err(ApiError::internal)?
    {
        assets_by_turn
            .entry(asset.turn_id.clone())
            .or_default()
            .push(asset);
    }
    let turns: Vec<SafeTurn> = state
        .state_store
        .load_turns_for_conversation(&channel, &conversation_id)
        .map_err(ApiError::internal)?
        .into_iter()
        .filter(|turn| !turn.is_summary)
        .map(|turn| {
            let assets = assets_by_turn.remove(&turn.turn_id).unwrap_or_default();
            SafeTurn::from_turn(turn, assets)
        })
        .collect();
    let mut response = Json(
        json!({ "ok": true, "channel": channel, "conversation_id": conversation_id, "turns": turns }),
    )
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn bootstrap(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    state
        .state_store
        .recover_stale_turns()
        .map_err(ApiError::internal)?;
    let (config, active_run_id, context) = {
        let manager = state.manager.lock().unwrap();
        (
            manager.config.clone(),
            manager.active_run_id.clone(),
            manager.context,
        )
    };
    let running_target = state
        .state_store
        .running_turn_queue_target()
        .map_err(ApiError::internal)?;
    let external_target = active_run_id
        .is_none()
        .then_some(running_target.as_ref())
        .flatten();
    let mut assets_by_turn = HashMap::<String, Vec<ImageAsset>>::new();
    for asset in state
        .state_store
        .load_image_assets()
        .map_err(ApiError::internal)?
    {
        assets_by_turn
            .entry(asset.turn_id.clone())
            .or_default()
            .push(asset);
    }
    let turns = state
        .state_store
        .load_visible_turns()
        .map_err(ApiError::internal)?
        .into_iter()
        .filter(|turn| !turn.is_summary)
        .map(|turn| {
            let assets = assets_by_turn.remove(&turn.turn_id).unwrap_or_default();
            SafeTurn::from_turn(turn, assets)
        })
        .collect();
    let usage = state
        .state_store
        .usage_snapshot()
        .map_err(ApiError::internal)?
        .into();
    let queued_prompts = match external_target {
        Some(target) => state
            .state_store
            .load_queued_prompts_for_target(target)
            .map_err(ApiError::internal)?,
        None => state
            .state_store
            .load_queued_prompts()
            .map_err(ApiError::internal)?,
    }
    .into_iter()
    .map(SafeQueuedPrompt::from)
    .collect();
    let running_turn_id = running_target.as_ref().map(|target| target.turn_id.clone());
    let external_queue_available = external_target
        .is_some_and(|target| target.queue_session_id.is_some() && target.owner_pid.is_some());
    let channel = state.state_store.channel().to_string();
    let channels = state
        .state_store
        .channel_summaries()
        .map_err(ApiError::internal)?
        .into_iter()
        .map(safe_channel_summary)
        .collect();
    let conversations = state
        .state_store
        .conversation_summaries()
        .map_err(ApiError::internal)?
        .into_iter()
        .map(safe_conversation_summary)
        .collect();
    let engine = match config.provider(None) {
        Ok(provider) if provider.is_pi() => "pi".to_string(),
        Ok(provider) => provider.id.clone(),
        Err(_) => "unknown".to_string(),
    };
    let mut response = Json(BootstrapResponse {
        version: env!("CARGO_PKG_VERSION"),
        boot_id: state.boot_id.to_string(),
        latest_event_id: state.events.latest_id(),
        engine,
        active_run_id,
        running_turn_id,
        external_queue_available,
        channel,
        channels,
        conversations,
        turns,
        queued_prompts,
        models: safe_models(&config),
        display: web_display_config(&config),
        context,
        usage,
        capabilities: Capabilities {
            multi_conversation: true,
            attachments: true,
            queue: true,
        },
    })
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn get_config(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    let (config, context) = {
        let manager = state.manager.lock().unwrap();
        (manager.config.clone(), manager.context)
    };
    let mut response = Json(config_response(&config, context, &state.paths)?).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn update_config(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<UpdateConfigRequest>,
) -> std::result::Result<Json<ConfigResponse>, ApiError> {
    require_mutation(&headers, &state)?;
    require_no_running_turn(&state.state_store)?;

    let current = state.manager.lock().unwrap().config.clone();
    let current_prompts =
        read_prompt_documents(&current, &state.paths).map_err(ApiError::internal)?;
    let mut candidate: AppConfig = serde_json::from_value(request.config).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid configuration: {}", safe_error_message(error)),
        )
    })?;
    restore_config_secrets(&mut candidate, &current, &request.secrets)?;
    validate_config_candidate(&candidate)?;
    validate_prompt_documents(&candidate, &request.prompts)?;
    let prompt_changed = prompt_configuration_changed(&current, &candidate)
        || prompt_documents_changed(&current_prompts, &request.prompts);
    if prompt_changed && !request.reset_conversation {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "prompt changes require explicit confirmation to reset the conversation",
        ));
    }

    reserve_admin(&state.manager)?;
    let (reply, receiver) = oneshot::channel();
    if state
        .actor_tx
        .send(ActorCommand::ApplyConfig {
            config: candidate,
            prompts: request.prompts,
            reset_conversation: prompt_changed,
            reply,
        })
        .is_err()
    {
        release_admin(&state.manager);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent worker is unavailable",
        ));
    }
    match receiver.await {
        Ok(Ok(())) => {}
        Ok(Err(AdminFailure::Invalid(message))) => {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, message));
        }
        Ok(Err(AdminFailure::Internal(message))) => {
            tracing::error!(error = %message, "WebUI configuration update failed");
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                safe_error_message(&message),
            ));
        }
        Err(_) => {
            release_admin(&state.manager);
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "agent worker stopped before updating the configuration",
            ));
        }
    }
    let manager = state.manager.lock().unwrap();
    Ok(Json(config_response(
        &manager.config,
        manager.context,
        &state.paths,
    )?))
}

async fn image_asset(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state)?;
    if asset_id.len() > 96
        || asset_id.is_empty()
        || !asset_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "image asset not found",
        ));
    }
    let Some(asset) = state
        .state_store
        .load_image_asset(&asset_id)
        .map_err(ApiError::internal)?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "image asset not found",
        ));
    };
    let mut response = asset.bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&asset.asset.mime).map_err(ApiError::internal)?,
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

async fn events(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    require_auth(&headers, &state)?;
    let header_after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let after = query.after.max(header_after);
    let subscription = state.events.subscribe_after(after);
    let stream_state = SseStreamState {
        pending: subscription.pending,
        receiver: subscription.receiver,
        events: state.events,
        last_id: after,
    };
    let events = stream::unfold(stream_state, |mut state| async move {
        loop {
            if let Some(record) = state.pending.pop_front() {
                if record.kind == "resync_required" {
                    state.last_id = record.id;
                    return Some((Ok(record_to_sse(record)), state));
                }
                if record.id <= state.last_id {
                    continue;
                }
                state.last_id = record.id;
                return Some((Ok(record_to_sse(record)), state));
            }
            match state.receiver.recv().await {
                Ok(record) if record.id > state.last_id => {
                    state.last_id = record.id;
                    return Some((Ok(record_to_sse(record)), state));
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    state.pending = state.events.replay_after(state.last_id);
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let ready =
        stream::once(async { Ok::<Event, Infallible>(Event::default().comment("connected")) });
    let stream = ready.chain(events);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

struct SseStreamState {
    pending: VecDeque<EventRecord>,
    receiver: broadcast::Receiver<EventRecord>,
    events: EventHub,
    last_id: u64,
}

fn record_to_sse(record: EventRecord) -> Event {
    Event::default()
        .id(record.id.to_string())
        .event(record.kind)
        .data(record.data)
}

fn enqueue_running_prompt(
    state: &WebState,
    content: &str,
    attachments: &[QueuedPromptAttachment],
) -> std::result::Result<(Option<String>, Option<String>, SafeQueuedPrompt), ApiError> {
    let active_run_id = {
        let manager = state.manager.lock().unwrap();
        if manager.admin_busy {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "GQY is busy with another operation",
            ));
        }
        manager.active_run_id.clone()
    };
    let prompt_id = random_id("queued", 18);
    if let Some(run_id) = active_run_id {
        let prompt = state
            .state_store
            .enqueue_prompt(&prompt_id, content, content, attachments)
            .map_err(ApiError::internal)?;
        return Ok((Some(run_id), None, SafeQueuedPrompt::from(prompt)));
    }

    state
        .state_store
        .recover_stale_turns()
        .map_err(ApiError::internal)?;
    let target = state
        .state_store
        .running_turn_queue_target()
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "there is no active reply to follow up",
            )
        })?;
    if target.queue_session_id.is_none() || target.owner_pid.is_none() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the running turn cannot accept messages from this WebUI",
        ));
    }
    let prompt = state
        .state_store
        .enqueue_prompt_for_target(&target, &prompt_id, content, content, &[])
        .map_err(ApiError::internal)?;
    Ok((None, Some(target.turn_id), SafeQueuedPrompt::from(prompt)))
}

fn publish_queued_prompt(
    state: &WebState,
    run_id: Option<&str>,
    turn_id: Option<&str>,
    prompt: &SafeQueuedPrompt,
) {
    state.events.publish(
        "queue.added",
        json!({ "run_id": run_id, "turn_id": turn_id, "prompt": prompt }),
    );
}

async fn create_turn(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<CreateTurnRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state)?;
    let content = validate_content(request.content)?;
    let mode = parse_mode(&request.mode)?;
    state
        .state_store
        .recover_stale_turns()
        .map_err(ApiError::internal)?;
    if state
        .state_store
        .has_running_turns()
        .map_err(ApiError::internal)?
    {
        let (run_id, turn_id, prompt) =
            enqueue_running_prompt(&state, &content, &[])?;
        publish_queued_prompt(&state, run_id.as_deref(), turn_id.as_deref(), &prompt);
        return Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "queued": true,
                "prompt": prompt,
                "run_id": run_id,
                "running_turn_id": turn_id,
            })),
        )
            .into_response());
    }
    let run_id = random_id("run", 18);
    {
        let mut manager = state.manager.lock().unwrap();
        if manager.active_run_id.is_some() || manager.admin_busy {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "GQY is busy with another operation",
            ));
        }
        manager.active_run_id = Some(run_id.clone());
    }
    let pasted = pasted_images_from_input(&request.images);
    if state
        .actor_tx
        .send(ActorCommand::StartTurn {
            run_id: run_id.clone(),
            content,
            images: pasted,
            mode,
        })
        .is_err()
    {
        finish_run(&state.manager, &run_id, None);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent worker is unavailable",
        ));
    }
    // 用户随消息发送的图片 → 持久化为回合资产（刷新/历史查看时仍能预览）
    if !request.images.is_empty() {
        let images = request.images.clone();
        let state_store = state.state_store.clone();
        let paths = state.paths.clone();
        tokio::spawn(async move {
            persist_user_images(&state_store, &paths, &images).await;
        });
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "run_id": run_id }))).into_response())
}

/// 等待当前回合启动（轮询 running_turn_queue_target），把用户图片写入
/// `cache/clipboard_images` 并保存为 `user_image_N` 资产。
async fn persist_user_images(
    state_store: &StateStore,
    paths: &GqyPaths,
    images: &[WebImageInput],
) {
    let turn_id = {
        let mut found = None;
        for _ in 0..100 {
            match state_store.running_turn_queue_target() {
                Ok(Some(target)) if !target.turn_id.is_empty() => {
                    found = Some(target.turn_id.clone());
                    break;
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            }
        }
        found
    };
    let Some(turn_id) = turn_id else {
        return;
    };
    let dir = paths.cache_dir.join("clipboard_images");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (index, image) in images.iter().enumerate() {
        let Ok(data) = base64::engine::general_purpose::STANDARD.decode(&image.data_base64)
        else {
            continue;
        };
        let ext = match image.mime.split('/').nth(1).unwrap_or("png") {
            "jpeg" | "jpg" => "jpg",
            "gif" => "gif",
            "webp" => "webp",
            _ => "png",
        };
        let path = dir.join(format!("user_image_{}_{}.{ext}", turn_id, index));
        if std::fs::write(&path, &data).is_err() {
            continue;
        }
        let _ = state_store.save_image_asset(
            &turn_id,
            Some(&format!("user_image_{index}")),
            &path,
            "用户发送的图片",
        );
    }
}

async fn queue_prompt(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<QueuePromptRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state)?;
    let content = validate_content(request.content)?;
    let attachments = queued_attachments_from_input(&request.images);
    let (run_id, turn_id, safe) = enqueue_running_prompt(&state, &content, &attachments)?;
    publish_queued_prompt(&state, run_id.as_deref(), turn_id.as_deref(), &safe);
    Ok((StatusCode::ACCEPTED, Json(safe)).into_response())
}

async fn remove_queue_prompt(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(prompt_id): Path<String>,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state)?;
    if prompt_id.len() > 96
        || prompt_id.is_empty()
        || !prompt_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "queued prompt not found",
        ));
    }
    let run_id = state.manager.lock().unwrap().active_run_id.clone();
    let target = if run_id.is_none() {
        state
            .state_store
            .running_turn_queue_target()
            .map_err(ApiError::internal)?
    } else {
        None
    };
    let removed = match target.as_ref() {
        Some(target) => state
            .state_store
            .remove_queued_prompt_for_target(target, &prompt_id)
            .map_err(ApiError::internal)?,
        None => state
            .state_store
            .remove_queued_prompt(&prompt_id)
            .map_err(ApiError::internal)?,
    };
    if !removed {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "queued prompt not found",
        ));
    }
    state.events.publish(
        "queue.removed",
        json!({
            "run_id": run_id,
            "turn_id": target.as_ref().map(|target| target.turn_id.as_str()),
            "prompt_id": prompt_id,
        }),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_run(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state)?;
    let matches_active =
        state.manager.lock().unwrap().active_run_id.as_deref() == Some(run_id.as_str());
    if !matches_active {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "active run not found"));
    }
    state
        .actor_tx
        .send(ActorCommand::Cancel {
            run_id: run_id.clone(),
        })
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "agent worker is unavailable",
            )
        })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "run_id": run_id,
            "cancellation_requested": true,
        })),
    )
        .into_response())
}

async fn answer_question(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(question_id): Path<String>,
    Json(request): Json<AnswerQuestionRequest>,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state)?;
    match state
        .questions
        .answer(&question_id, request.answers, |run_id, answers| {
            state.events.publish(
                "question.answered",
                json!({
                    "run_id": run_id,
                    "question_id": question_id,
                    "answers": answers,
                }),
            );
        }) {
        Ok(()) => {}
        Err(AnswerFailure::NotFound) => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "pending question not found",
            ));
        }
        Err(AnswerFailure::Invalid(message)) => {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, message));
        }
        Err(AnswerFailure::Gone) => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "the question is no longer awaiting an answer",
            ));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_models(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<SetModelsRequest>,
) -> std::result::Result<Json<ModelResponse>, ApiError> {
    require_mutation(&headers, &state)?;
    let models = validate_model_selection(request.models)?;
    require_no_running_turn(&state.state_store)?;
    reserve_admin(&state.manager)?;
    let (reply, receiver) = oneshot::channel();
    if state
        .actor_tx
        .send(ActorCommand::SetModels { models, reply })
        .is_err()
    {
        release_admin(&state.manager);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent worker is unavailable",
        ));
    }
    match receiver.await {
        Ok(Ok(())) => {}
        Ok(Err(AdminFailure::Invalid(message))) => {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, message));
        }
        Ok(Err(AdminFailure::Internal(message))) => {
            tracing::error!(error = %message, "WebUI model update failed");
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                safe_error_message(&message),
            ));
        }
        Err(_) => {
            release_admin(&state.manager);
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "agent worker stopped before updating the model",
            ));
        }
    }
    let manager = state.manager.lock().unwrap();
    Ok(Json(ModelResponse {
        models: safe_models(&manager.config),
        display: web_display_config(&manager.config),
        context: manager.context,
    }))
}

async fn reset_conversation(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state)?;
    require_no_running_turn(&state.state_store)?;
    reserve_admin(&state.manager)?;
    let (reply, receiver) = oneshot::channel();
    if state
        .actor_tx
        .send(ActorCommand::ResetConversation { reply })
        .is_err()
    {
        release_admin(&state.manager);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent worker is unavailable",
        ));
    }
    match receiver.await {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT),
        Ok(Err(AdminFailure::Invalid(message))) => {
            Err(ApiError::new(StatusCode::CONFLICT, message))
        }
        Ok(Err(AdminFailure::Internal(message))) => {
            tracing::error!(error = %message, "WebUI conversation reset failed");
            Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                safe_error_message(&message),
            ))
        }
        Err(_) => {
            release_admin(&state.manager);
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "agent worker stopped before resetting the conversation",
            ))
        }
    }
}

/// 配置文件热重载：每 2 秒检查 config.jsonc 的修改时间，
/// 变化时重新加载配置并重建 agent（不重置会话），同时发布事件让前端刷新。
/// 注意：ApplyConfig 内部会把规范化后的配置写回文件（mtime 再次变化），
/// 因此必须在应用完成后把写入后的 mtime 作为新基线，否则会自我触发无限循环。
fn spawn_config_watcher(paths: GqyPaths, actor_tx: mpsc::UnboundedSender<ActorCommand>, events: EventHub) {
    let mut last_mtime = std::fs::metadata(&paths.config_file)
        .and_then(|meta| meta.modified())
        .ok();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let mtime = std::fs::metadata(&paths.config_file)
                .and_then(|meta| meta.modified())
                .ok();
            if mtime.is_none() {
                continue;
            }
            if mtime == last_mtime {
                continue;
            }
            let Some(mtime) = mtime else { continue };
            // 防抖静置 300ms，等待写文件彻底落盘
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let Ok(config) = AppConfig::load(&paths) else {
                tracing::warn!("config watcher: failed to reload configuration");
                continue;
            };
            let Ok(prompts) = read_prompt_documents(&config, &paths) else {
                continue;
            };
            let (reply, receiver) = tokio::sync::oneshot::channel();
            if actor_tx
                .send(ActorCommand::ApplyConfig {
                    config,
                    prompts,
                    reset_conversation: false,
                    reply,
                })
                .is_ok()
            {
                // 等应用完成（含配置写回），把写入后的 mtime 设为基线，
                // 吸收自我写入，避免「重载→写回→再重载」死循环
                let _ = receiver.await;
                last_mtime = std::fs::metadata(&paths.config_file)
                    .and_then(|meta| meta.modified())
                    .ok();
                tracing::info!("config watcher: configuration reloaded from file");
                events.publish("config.reloaded", serde_json::json!({}));
            }
        }
    });
}

fn spawn_actor(
    agent: Agent,
    config: AppConfig,
    paths: GqyPaths,
    state_store: StateStore,
    manager: Arc<Mutex<ManagerState>>,
    events: EventHub,
    questions: QuestionBroker,
) -> Result<(mpsc::UnboundedSender<ActorCommand>, JoinHandle<Result<()>>)> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let join = std::thread::Builder::new()
        .name("gqy-web-agent".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("building WebUI agent runtime")?;
            runtime.block_on(actor_loop(
                agent,
                config,
                paths,
                state_store,
                manager,
                events,
                questions,
                receiver,
            ));
            Ok(())
        })
        .context("starting WebUI agent thread")?;
    Ok((sender, join))
}

#[allow(clippy::too_many_arguments)]
async fn actor_loop(
    mut agent: Agent,
    mut config: AppConfig,
    paths: GqyPaths,
    state_store: StateStore,
    manager: Arc<Mutex<ManagerState>>,
    events: EventHub,
    questions: QuestionBroker,
    mut receiver: mpsc::UnboundedReceiver<ActorCommand>,
) {
    while let Some(command) = receiver.recv().await {
        match command {
            ActorCommand::StartTurn {
                run_id,
                content,
                images,
                mode,
            } => {
                let keep_running = run_agent_turn(
                    &mut agent,
                    &config,
                    &paths,
                    &state_store,
                    &manager,
                    &events,
                    &questions,
                    &mut receiver,
                    run_id,
                    content,
                    images,
                    mode,
                )
                .await;
                if !keep_running {
                    break;
                }
            }
            ActorCommand::Pi { kind, value, reply } => {
                let result = run_pi_command(&agent.llm_client(), kind, value).await;
                let _ = reply.send(result);
            }
            ActorCommand::Cancel { .. } => {}
            ActorCommand::SetModels { models, reply } => {
                // 供应商变更前先压缩上下文：旧供应商缓存命中、压缩成本极低；
                // 新供应商首请求不再全价重发全量历史（曾出现单次切换损失 34 元）。
                let compact_first = provider_will_change(&config, &models);
                let result = async {
                    if compact_first {
                        match agent.compact_now(|_| Ok(())).await {
                            Ok(Some(_)) => {
                                tracing::info!("provider switch: context compacted before switching")
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    error = %error,
                                    "pre-switch compact failed; switching anyway"
                                );
                            }
                        }
                    }
                    rebuild_for_models(
                        &mut agent,
                        &mut config,
                        &paths,
                        &state_store,
                        &manager,
                        &models,
                    )
                }
                .await;
                release_admin(&manager);
                let _ = reply.send(result);
            }
            ActorCommand::ApplyConfig {
                config: next_config,
                prompts,
                reset_conversation,
                reply,
            } => {
                let result = rebuild_for_config(
                    &mut agent,
                    &mut config,
                    &paths,
                    &state_store,
                    &manager,
                    &events,
                    next_config,
                    &prompts,
                    reset_conversation,
                );
                release_admin(&manager);
                let _ = reply.send(result);
            }
            ActorCommand::ResetConversation { reply } => {
                let result = reset_actor_conversation(
                    &mut agent,
                    &config,
                    &paths,
                    &state_store,
                    &manager,
                    &events,
                );
                release_admin(&manager);
                let _ = reply.send(result);
            }
            ActorCommand::Shutdown => break,
        }
    }
}

/// pi 工具桥产生的图片（表情包等）：保存为 WebUI 资产并推送 tool.image 事件。
/// 图片事件发生在 axum 工具调用线程，与 agent 回合并发，这里通过
/// state_store 找到当前运行中的回合来归属资产。
fn publish_bridge_image(
    events: &EventHub,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    path: std::path::PathBuf,
    alt: String,
) {
    let run_id = manager
        .lock()
        .unwrap()
        .active_run_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let turn_id = match state_store.running_turn_queue_target() {
        Ok(Some(target)) => target.turn_id.clone(),
        _ => String::new(),
    };
    if turn_id.is_empty() {
        return;
    }
    let tool_id = format!("{run_id}_bridge_image");
    let name = if alt.contains("表情") || alt.contains("meme") {
        "show_meme"
    } else {
        "image"
    };
    match state_store.save_image_asset(&turn_id, Some(&tool_id), &path, &alt) {
        Ok(asset) => {
            let hide_caption = name == "show_meme";
            events.publish(
                "tool.image",
                json!({
                    "run_id": run_id,
                    "tool_id": tool_id,
                    "name": name,
                    "asset": SafeImageAsset::from_asset(asset, hide_caption),
                }),
            );
        }
        Err(error) => {
            tracing::warn!(run_id, error = %error, "pi bridge: failed to persist image asset");
        }
    }
}

/// pi 工具桥产生的进度消息（agent 思考/回复增量等）→ tool.progress SSE，
/// 挂到独立的「agent 集群活动」工具卡（实时滚动）。
fn publish_bridge_progress(events: &EventHub, manager: &Arc<Mutex<ManagerState>>, message: String) {
    let run_id = manager
        .lock()
        .unwrap()
        .active_run_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if run_id == "unknown" {
        return;
    }
    events.publish(
        "tool.progress",
        json!({
            "run_id": run_id,
            "tool_id": format!("{run_id}_agent_activity"),
            "name": "agent 集群活动",
            "message": message,
        }),
    );
}

#[allow(clippy::too_many_arguments)]
async fn run_agent_turn(
    agent: &mut Agent,
    config: &AppConfig,
    paths: &GqyPaths,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    questions: &QuestionBroker,
    receiver: &mut mpsc::UnboundedReceiver<ActorCommand>,
    run_id: String,
    content: String,
    images: Vec<Option<PastedImage>>,
    mode: AgentMode,
) -> bool {
    events.publish(
        "run.started",
        json!({ "run_id": run_id, "mode": mode_name(mode) }),
    );
    let setup = (|| -> Result<AgentTurnControl> {
        let normal_tools = build_tool_registry(config, paths, AgentMode::Normal, true)?;
        let plan_tools = build_tool_registry(config, paths, AgentMode::Plan, true)?;
        let chat_tools = build_tool_registry(config, paths, AgentMode::Chat, true)?;
        let active_tools = match mode {
            AgentMode::Normal => normal_tools.clone(),
            AgentMode::Plan => plan_tools.clone(),
            AgentMode::Chat => chat_tools.clone(),
        };
        agent.switch_mode(mode, active_tools);
        agent.prepare_for_turn()?;
        Ok(AgentTurnControl::new(
            mode,
            normal_tools,
            plan_tools,
            chat_tools,
        ))
    })();
    let control = match setup {
        Ok(control) => control,
        Err(error) => {
            finish_failed_run(manager, events, questions, agent, &run_id, &error);
            return true;
        }
    };

    let mapper = Arc::new(Mutex::new(RunEventMapper::new(
        run_id.clone(),
        events.clone(),
        questions.clone(),
        state_store.clone(),
    )));
    // pi 控制命令在回合运行中也能执行：用 client 克隆（chat future 借用了 agent）
    let pi_client = agent.llm_client();
    let chat_outcome = {
        let callback_mapper = mapper.clone();
        let chat = agent.chat_stream_with_control(&content, &images, &control, move |event| {
            callback_mapper.lock().unwrap().handle(event);
            Ok(())
        });
        tokio::pin!(chat);
        loop {
            tokio::select! {
                biased;
                result = &mut chat => break TurnOutcome::Finished(result),
                command = receiver.recv() => {
                    if let Some(ActorCommand::Pi { kind, value, reply }) = command {
                        let result = run_pi_command(&pi_client, kind, value).await;
                        let _ = reply.send(result);
                        continue;
                    }
                    match active_directive(command, &run_id, manager) {
                        ActiveDirective::Continue => {}
                        ActiveDirective::Cancel => {
                            questions.cancel_run(&run_id);
                            break TurnOutcome::Cancelled;
                        }
                        ActiveDirective::Shutdown => {
                            questions.cancel_run(&run_id);
                            break TurnOutcome::Shutdown;
                        }
                    }
                }
            }
        }
    };

    let result = match chat_outcome {
        TurnOutcome::Cancelled => {
            finish_cancelled_run(manager, events, agent, &run_id);
            return true;
        }
        TurnOutcome::Shutdown => {
            finish_cancelled_run(manager, events, agent, &run_id);
            return false;
        }
        TurnOutcome::Finished(Err(error)) if question::is_question_cancelled(&error) => {
            questions.cancel_run(&run_id);
            finish_cancelled_run(manager, events, agent, &run_id);
            return true;
        }
        TurnOutcome::Finished(Err(error)) => {
            finish_failed_run(manager, events, questions, agent, &run_id, &error);
            return true;
        }
        TurnOutcome::Finished(Ok(result)) => result,
    };

    questions.cancel_run(&run_id);
    let context_tokens = match agent.effective_context_tokens() {
        Ok(tokens) => tokens,
        Err(error) => {
            finish_completed_with_context_error(manager, events, agent, &run_id, &result, &error);
            return true;
        }
    };
    let overflow_outcome = {
        let callback_mapper = mapper;
        let overflow = agent.handle_overflow_after_turn(context_tokens, move |event| {
            callback_mapper.lock().unwrap().handle(event);
            Ok(())
        });
        tokio::pin!(overflow);
        loop {
            tokio::select! {
                biased;
                result = &mut overflow => break OverflowOutcome::Finished(result),
                command = receiver.recv() => {
                    if let Some(ActorCommand::Pi { kind, value, reply }) = command {
                        let result = run_pi_command(&pi_client, kind, value).await;
                        let _ = reply.send(result);
                        continue;
                    }
                    match active_directive(command, &run_id, manager) {
                        ActiveDirective::Continue => {}
                        ActiveDirective::Cancel => break OverflowOutcome::Cancelled,
                        ActiveDirective::Shutdown => break OverflowOutcome::Shutdown,
                    }
                }
            }
        }
    };
    match overflow_outcome {
        OverflowOutcome::Cancelled => {
            let context =
                current_context(agent).unwrap_or_else(|_| manager.lock().unwrap().context);
            finish_run(manager, &run_id, Some(context));
            publish_completed(events, &run_id, &result, context);
            return true;
        }
        OverflowOutcome::Shutdown => {
            let context =
                current_context(agent).unwrap_or_else(|_| manager.lock().unwrap().context);
            finish_run(manager, &run_id, Some(context));
            publish_completed(events, &run_id, &result, context);
            return false;
        }
        OverflowOutcome::Finished(Err(error)) => {
            finish_completed_with_context_error(manager, events, agent, &run_id, &result, &error);
            return true;
        }
        OverflowOutcome::Finished(Ok(_)) => {}
    }
    let context = match current_context(agent) {
        Ok(context) => context,
        Err(error) => {
            finish_completed_with_context_error(manager, events, agent, &run_id, &result, &error);
            return true;
        }
    };
    finish_run(manager, &run_id, Some(context));
    publish_completed(events, &run_id, &result, context);
    true
}

enum TurnOutcome {
    Finished(Result<ChatResult>),
    Cancelled,
    Shutdown,
}

enum OverflowOutcome {
    Finished(Result<Option<ChatResult>>),
    Cancelled,
    Shutdown,
}

enum ActiveDirective {
    Continue,
    Cancel,
    Shutdown,
}

async fn run_pi_command(
    client: &LlmClient,
    kind: PiCommandKind,
    value: String,
) -> Result<serde_json::Value> {
    match kind {
        PiCommandKind::GetState => client.pi_state().await,
        PiCommandKind::GetModels => client
            .pi_available_models()
            .await
            .map(serde_json::Value::Array),
        PiCommandKind::SetModel => client
            .pi_set_model(&value)
            .await
            .map(|_| serde_json::json!({ "ok": true })),
        PiCommandKind::SetThinking => client
            .pi_set_thinking_level(&value)
            .await
            .map(|_| serde_json::json!({ "ok": true })),
    }
}

fn active_directive(
    command: Option<ActorCommand>,
    run_id: &str,
    manager: &Arc<Mutex<ManagerState>>,
) -> ActiveDirective {
    match command {
        Some(ActorCommand::Cancel { run_id: requested }) if requested == run_id => {
            ActiveDirective::Cancel
        }
        Some(ActorCommand::Cancel { .. }) => ActiveDirective::Continue,
        Some(ActorCommand::Shutdown) | None => ActiveDirective::Shutdown,
        Some(ActorCommand::Pi { .. }) => ActiveDirective::Continue,
        Some(ActorCommand::SetModels { reply, .. }) => {
            release_admin(manager);
            let _ = reply.send(Err(AdminFailure::Invalid(
                "the model cannot be changed while a turn is running".to_string(),
            )));
            ActiveDirective::Continue
        }
        Some(ActorCommand::ApplyConfig { reply, .. }) => {
            release_admin(manager);
            let _ = reply.send(Err(AdminFailure::Invalid(
                "the configuration cannot be changed while a turn is running".to_string(),
            )));
            ActiveDirective::Continue
        }
        Some(ActorCommand::ResetConversation { reply }) => {
            release_admin(manager);
            let _ = reply.send(Err(AdminFailure::Invalid(
                "the conversation cannot be reset while a turn is running".to_string(),
            )));
            ActiveDirective::Continue
        }
        Some(ActorCommand::StartTurn {
            run_id: rejected, ..
        }) => {
            finish_run(manager, &rejected, None);
            ActiveDirective::Continue
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// 模型选择变更是否涉及供应商切换（供应商不变则不动上下文）。
///
/// 注意：`set_active_provider_models` 不会更新 `active_provider` 字段
/// （单数版 `set_active_provider_model` 才会），因此这里比较的是
/// 「当前活动模型列表的 provider_id 集合」与「新选择的 provider_id 集合」。
fn provider_will_change(config: &AppConfig, models: &[ActiveProviderModelConfig]) -> bool {
    let next: Vec<&str> = models.iter().map(|m| m.provider_id.as_str()).collect();
    let current: Vec<&str> = match config.active_provider_models.as_deref() {
        Some(list) => list.iter().map(|m| m.provider_id.as_str()).collect(),
        None => vec![config.active_provider.as_str()],
    };
    next != current
}

fn rebuild_for_models(
    agent: &mut Agent,
    config: &mut AppConfig,
    paths: &GqyPaths,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    models: &[ActiveProviderModelConfig],
) -> std::result::Result<(), AdminFailure> {
    let mut next_config = config.clone();
    next_config
        .set_active_provider_models(models)
        .map_err(|error| AdminFailure::Invalid(safe_error_message(&error)))?;
    if next_config.active_provider_models == config.active_provider_models {
        return Ok(());
    }
    let client = LlmClient::from_config(&next_config, paths)
        .map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    let registry = build_tool_registry(&next_config, paths, AgentMode::Normal, true)
        .map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    let next_agent = Agent::new(
        next_config.clone(),
        paths,
        state_store.clone(),
        client,
        registry,
        AgentMode::Normal,
    )
    .map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    let context = current_context(&next_agent)
        .map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    next_config
        .save(paths)
        .map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    *agent = next_agent;
    *config = next_config.clone();
    let mut manager = manager.lock().unwrap();
    manager.config = next_config;
    manager.context = context;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn rebuild_for_config(
    agent: &mut Agent,
    config: &mut AppConfig,
    paths: &GqyPaths,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    next_config: AppConfig,
    prompts: &PromptDocuments,
    reset_conversation: bool,
) -> std::result::Result<(), AdminFailure> {
    let previous_prompts = read_prompt_documents(config, paths)
        .map_err(|error| AdminFailure::Internal(safe_error_message(error)))?;
    let prompt_backups =
        apply_prompt_documents(config, &next_config, &previous_prompts, prompts, paths)
            .map_err(|error| AdminFailure::Internal(safe_error_message(error)))?;
    let scope_backups = match apply_persona_scope_changes(
        config,
        &next_config,
        &previous_prompts,
        prompts,
        paths,
    ) {
        Ok(backups) => backups,
        Err(error) => {
            restore_file_backups(&prompt_backups);
            return Err(AdminFailure::Internal(safe_error_message(error)));
        }
    };
    let config_backup = FileBackup {
        path: paths.config_file.clone(),
        content: std::fs::read(&paths.config_file).ok(),
    };
    let system_prompt_backup = next_config.system_prompt.as_ref().map(|_| FileBackup {
        path: next_config.system_prompt_path(paths),
        content: std::fs::read(next_config.system_prompt_path(paths)).ok(),
    });

    let build_agent = || -> Result<Agent> {
        let client = LlmClient::from_config(&next_config, paths)?;
        let registry = build_tool_registry(&next_config, paths, AgentMode::Normal, true)?;
        Agent::new(
            next_config.clone(),
            paths,
            state_store.clone(),
            client,
            registry,
            AgentMode::Normal,
        )
    };
    let mut next_agent = match build_agent() {
        Ok(agent) => agent,
        Err(error) => {
            restore_file_backups(&prompt_backups);
            restore_persona_scope_backups(&scope_backups);
            return Err(AdminFailure::Invalid(safe_error_message(error)));
        }
    };
    let mut context = match current_context(&next_agent) {
        Ok(context) => context,
        Err(error) => {
            restore_file_backups(&prompt_backups);
            restore_persona_scope_backups(&scope_backups);
            return Err(AdminFailure::Invalid(safe_error_message(error)));
        }
    };
    if let Err(error) = next_config.save(paths) {
        restore_file_backups(&prompt_backups);
        restore_persona_scope_backups(&scope_backups);
        restore_file_backups(std::slice::from_ref(&config_backup));
        if let Some(backup) = &system_prompt_backup {
            restore_file_backups(std::slice::from_ref(backup));
        }
        return Err(AdminFailure::Internal(safe_error_message(error)));
    }

    if reset_conversation {
        let reset = (|| -> Result<()> {
            state_store.reset_conversation()?;
            let memory = MemoryStore::new(&next_config, paths);
            memory.clear_evicted_context()?;
            memory.clear_pending_events()?;
            next_agent.reset_memory()?;
            next_agent.prepare_for_turn()?;
            context = current_context(&next_agent)?;
            Ok(())
        })();
        if let Err(error) = reset {
            restore_file_backups(&prompt_backups);
            restore_persona_scope_backups(&scope_backups);
            restore_file_backups(std::slice::from_ref(&config_backup));
            if let Some(backup) = &system_prompt_backup {
                restore_file_backups(std::slice::from_ref(backup));
            }
            return Err(AdminFailure::Internal(safe_error_message(error)));
        }
    }

    *agent = next_agent;
    *config = next_config.clone();
    let mut manager = manager.lock().unwrap();
    manager.config = next_config;
    manager.context = context;
    drop(manager);
    if reset_conversation {
        events.publish("conversation.reset", json!({}));
    }
    finalize_persona_scope_backups(&scope_backups);
    Ok(())
}

fn reset_actor_conversation(
    agent: &mut Agent,
    config: &AppConfig,
    paths: &GqyPaths,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
) -> std::result::Result<(), AdminFailure> {
    let mut reset = || -> Result<ContextSnapshot> {
        state_store.reset_conversation()?;
        // 清空后回收空闲页（软删对 VACUUM 无收益，incremental 回收历史硬删留下的空洞）
        state_store.incremental_vacuum().ok();
        let memory = MemoryStore::new(config, paths);
        memory.clear_evicted_context()?;
        memory.clear_pending_events()?;
        agent.reset_memory()?;
        agent.prepare_for_turn()?;
        current_context(agent)
    };
    let context = reset().map_err(|error| AdminFailure::Internal(safe_error_message(&error)))?;
    manager.lock().unwrap().context = context;
    events.publish("conversation.reset", json!({}));
    Ok(())
}

fn finish_cancelled_run(
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    agent: &Agent,
    run_id: &str,
) {
    let context = current_context(agent).ok();
    finish_run(manager, run_id, context);
    events.publish("run.cancelled", json!({ "run_id": run_id }));
}

fn finish_failed_run(
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    questions: &QuestionBroker,
    agent: &Agent,
    run_id: &str,
    error: &anyhow::Error,
) {
    questions.cancel_run(run_id);
    let context = current_context(agent).ok();
    finish_run(manager, run_id, context);
    let message = safe_error_message(error);
    tracing::error!(run_id, error = %error, "WebUI agent run failed");
    events.publish(
        "run.failed",
        json!({ "run_id": run_id, "message": message }),
    );
}

fn finish_completed_with_context_error(
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    agent: &Agent,
    run_id: &str,
    result: &ChatResult,
    error: &anyhow::Error,
) {
    let message = safe_error_message(error);
    tracing::error!(run_id, error = %error, "WebUI post-turn context maintenance failed");
    events.publish(
        "context.error",
        json!({ "run_id": run_id, "message": message }),
    );
    let context = current_context(agent).unwrap_or_else(|_| manager.lock().unwrap().context);
    finish_run(manager, run_id, Some(context));
    publish_completed(events, run_id, result, context);
}

fn finish_run(manager: &Arc<Mutex<ManagerState>>, run_id: &str, context: Option<ContextSnapshot>) {
    let mut manager = manager.lock().unwrap();
    if let Some(context) = context {
        manager.context = context;
    }
    if manager.active_run_id.as_deref() == Some(run_id) {
        manager.active_run_id = None;
    }
}

fn publish_completed(
    events: &EventHub,
    run_id: &str,
    result: &ChatResult,
    context: ContextSnapshot,
) {
    events.publish(
        "run.completed",
        json!({
            "run_id": run_id,
            "usage": result.usage,
            "usage_estimated": result.usage_estimated,
            "provider_id": result.provider_id,
            "model": result.model,
            "context_tokens": context.tokens,
            "context_window": context.window,
        }),
    );
}

fn current_context(agent: &Agent) -> Result<ContextSnapshot> {
    Ok(ContextSnapshot {
        tokens: agent.effective_context_tokens()?,
        window: agent.context_window(),
    })
}

fn reserve_admin(manager: &Arc<Mutex<ManagerState>>) -> std::result::Result<(), ApiError> {
    let mut manager = manager.lock().unwrap();
    if manager.active_run_id.is_some() || manager.admin_busy {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "GQY is busy with another operation",
        ));
    }
    manager.admin_busy = true;
    Ok(())
}

fn require_no_running_turn(state_store: &StateStore) -> std::result::Result<(), ApiError> {
    if state_store
        .has_running_turns()
        .map_err(ApiError::internal)?
    {
        Err(ApiError::new(
            StatusCode::CONFLICT,
            "a conversation turn is already running",
        ))
    } else {
        Ok(())
    }
}

fn release_admin(manager: &Arc<Mutex<ManagerState>>) {
    manager.lock().unwrap().admin_busy = false;
}

fn config_response(
    config: &AppConfig,
    context: ContextSnapshot,
    paths: &GqyPaths,
) -> std::result::Result<ConfigResponse, ApiError> {
    let mut redacted = config.clone();
    let mut secret_states = HashMap::new();
    for (index, provider) in redacted.providers.iter_mut().enumerate() {
        secret_states.insert(
            format!("providers.{index}.api_key"),
            provider
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        );
        provider.api_key = None;
    }
    redact_secret_list(
        &mut secret_states,
        "plugins.web.tavily_api_keys",
        &mut redacted.plugins.web.tavily_api_keys,
    );
    redact_secret_list(
        &mut secret_states,
        "plugins.web.firecrawl_api_keys",
        &mut redacted.plugins.web.firecrawl_api_keys,
    );
    redact_secret_list(
        &mut secret_states,
        "plugins.web.anysearch_api_keys",
        &mut redacted.plugins.web.anysearch_api_keys,
    );
    secret_states.insert(
        "plugins.exchange_rate.api_key".to_string(),
        !redacted.plugins.exchange_rate.api_key.trim().is_empty(),
    );
    redacted.plugins.exchange_rate.api_key.clear();
    redact_secret_list(
        &mut secret_states,
        "plugins.image_generation.api_keys",
        &mut redacted.plugins.image_generation.api_keys,
    );
    let mut config_value = serde_json::to_value(&redacted).map_err(ApiError::internal)?;
    if let Value::Object(config_object) = &mut config_value {
        config_object.insert(
            "memory".to_string(),
            serde_json::to_value(redacted.memory_config()).map_err(ApiError::internal)?,
        );
    }
    let prompts = read_prompt_documents(config, paths).map_err(ApiError::internal)?;
    Ok(ConfigResponse {
        config: config_value,
        secret_states,
        prompts,
        models: safe_models(config),
        multimodal_models: safe_multimodal_models(config),
        display: web_display_config(config),
        context,
    })
}

fn redact_secret_list(states: &mut HashMap<String, bool>, key: &str, values: &mut Vec<String>) {
    states.insert(
        key.to_string(),
        values.iter().any(|value| !value.trim().is_empty()),
    );
    values.clear();
}

fn restore_config_secrets(
    candidate: &mut AppConfig,
    current: &AppConfig,
    mutations: &HashMap<String, SecretMutation>,
) -> std::result::Result<(), ApiError> {
    let mut recognized = HashSet::new();
    for (index, provider) in candidate.providers.iter_mut().enumerate() {
        let key = format!("providers.{index}.api_key");
        recognized.insert(key.clone());
        let existing = current
            .providers
            .iter()
            .find(|item| item.id == provider.id)
            .and_then(|item| item.api_key.clone());
        provider.api_key = match mutations.get(&key) {
            Some(SecretMutation::Set(value)) => normalize_single_secret(value, &key)?,
            Some(SecretMutation::Clear) => None,
            None => existing,
        };
    }

    restore_secret_list(
        candidate,
        current,
        mutations,
        &mut recognized,
        "plugins.web.tavily_api_keys",
        |config| &mut config.plugins.web.tavily_api_keys,
        |config| &config.plugins.web.tavily_api_keys,
    )?;
    restore_secret_list(
        candidate,
        current,
        mutations,
        &mut recognized,
        "plugins.web.firecrawl_api_keys",
        |config| &mut config.plugins.web.firecrawl_api_keys,
        |config| &config.plugins.web.firecrawl_api_keys,
    )?;
    restore_secret_list(
        candidate,
        current,
        mutations,
        &mut recognized,
        "plugins.web.anysearch_api_keys",
        |config| &mut config.plugins.web.anysearch_api_keys,
        |config| &config.plugins.web.anysearch_api_keys,
    )?;
    restore_secret_list(
        candidate,
        current,
        mutations,
        &mut recognized,
        "plugins.image_generation.api_keys",
        |config| &mut config.plugins.image_generation.api_keys,
        |config| &config.plugins.image_generation.api_keys,
    )?;

    let exchange_key = "plugins.exchange_rate.api_key";
    recognized.insert(exchange_key.to_string());
    candidate.plugins.exchange_rate.api_key = match mutations.get(exchange_key) {
        Some(SecretMutation::Set(value)) => {
            normalize_single_secret(value, exchange_key)?.unwrap_or_default()
        }
        Some(SecretMutation::Clear) => String::new(),
        None => current.plugins.exchange_rate.api_key.clone(),
    };

    if let Some(key) = mutations.keys().find(|key| !recognized.contains(*key)) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("unknown secret field: {key}"),
        ));
    }
    Ok(())
}

fn restore_secret_list<Mut, Ref>(
    candidate: &mut AppConfig,
    current: &AppConfig,
    mutations: &HashMap<String, SecretMutation>,
    recognized: &mut HashSet<String>,
    key: &str,
    candidate_values: Mut,
    current_values: Ref,
) -> std::result::Result<(), ApiError>
where
    Mut: FnOnce(&mut AppConfig) -> &mut Vec<String>,
    Ref: FnOnce(&AppConfig) -> &Vec<String>,
{
    recognized.insert(key.to_string());
    *candidate_values(candidate) = match mutations.get(key) {
        Some(SecretMutation::Set(value)) => parse_secret_list(value, key)?,
        Some(SecretMutation::Clear) => Vec::new(),
        None => current_values(current).clone(),
    };
    Ok(())
}

fn normalize_single_secret(
    value: &str,
    field: &str,
) -> std::result::Result<Option<String>, ApiError> {
    validate_secret_text(value, field)?;
    Ok(Some(value.trim().to_string()).filter(|value| !value.is_empty()))
}

fn parse_secret_list(value: &str, field: &str) -> std::result::Result<Vec<String>, ApiError> {
    validate_secret_text(value, field)?;
    Ok(value
        .split(|character| matches!(character, ',' | '\n' | '\r'))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect())
}

fn validate_secret_text(value: &str, field: &str) -> std::result::Result<(), ApiError> {
    if value.chars().count() > MAX_SECRET_CHARS
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{field} is invalid"),
        ));
    }
    Ok(())
}

fn validate_config_candidate(config: &AppConfig) -> std::result::Result<(), ApiError> {
    config.validate().map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid configuration: {}", safe_error_message(error)),
        )
    })?;
    let mut provider_ids = HashSet::with_capacity(config.providers.len());
    for provider in &config.providers {
        if !provider_ids.insert(provider.id.trim()) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("duplicate provider id: {}", provider.id),
            ));
        }
    }
    if let Some(active) = &config.active_provider_models {
        let mut checked = config.clone();
        checked
            .set_active_provider_models(active)
            .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, safe_error_message(error)))?;
    }
    if let Some(active) = &config.active_multimodal_provider_models {
        let choices = config.provider_model_choices();
        let mut seen = HashSet::with_capacity(active.len());
        for model in active {
            if !seen.insert((&model.provider_id, &model.model))
                || !choices.iter().any(|choice| {
                    choice.provider_id == model.provider_id && choice.model == model.model
                })
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "invalid multimodal provider/model: {} / {}",
                        model.provider_id, model.model
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_prompt_documents(
    config: &AppConfig,
    prompts: &PromptDocuments,
) -> std::result::Result<(), ApiError> {
    validate_prompt_document_list("persona", &prompts.personas)?;
    validate_prompt_document_list("identity", &prompts.identities)?;
    if !config.prompt.active_persona.trim().is_empty()
        && !prompts
            .personas
            .iter()
            .any(|document| document.name == config.prompt.active_persona)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "the active persona does not exist",
        ));
    }
    if !config.prompt.active_identity.trim().is_empty()
        && !prompts
            .identities
            .iter()
            .any(|document| document.name == config.prompt.active_identity)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "the active identity does not exist",
        ));
    }
    Ok(())
}

fn validate_prompt_document_list(
    kind: &str,
    documents: &[PromptDocument],
) -> std::result::Result<(), ApiError> {
    if documents.len() > MAX_PROMPT_DOCUMENTS {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("at most {MAX_PROMPT_DOCUMENTS} {kind} documents are allowed"),
        ));
    }
    let mut names = HashSet::with_capacity(documents.len());
    let mut original_names = HashSet::with_capacity(documents.len());
    for document in documents {
        validate_prompt_document_name(&document.name, kind)?;
        if !names.insert(document.name.as_str()) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("duplicate {kind} document: {}", document.name),
            ));
        }
        if document.content.chars().count() > MAX_PROMPT_DOCUMENT_CHARS
            || document.content.contains('\0')
        {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("{kind} document is too large: {}", document.name),
            ));
        }
        if let Some(original) = document.original_name.as_deref() {
            validate_prompt_document_name(original, kind)?;
            if !original_names.insert(original) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("duplicate original {kind} document: {original}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_prompt_document_name(name: &str, kind: &str) -> std::result::Result<(), ApiError> {
    let valid = name == name.trim()
        && name.ends_with(".md")
        && name.len() <= 240
        && name.len() > 3
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
        && FilePath::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            == Some(name);
    if !valid {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid {kind} document name: {name}"),
        ));
    }
    Ok(())
}

fn read_prompt_documents(config: &AppConfig, paths: &GqyPaths) -> Result<PromptDocuments> {
    Ok(PromptDocuments {
        personas: read_prompt_document_dir(&config.prompts_dir_path(paths))?,
        identities: read_prompt_document_dir(&config.identities_dir_path(paths))?,
    })
}

fn read_prompt_document_dir(dir: &FilePath) -> Result<Vec<PromptDocument>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut documents = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        let content = std::fs::read_to_string(entry.path())?;
        documents.push(PromptDocument {
            original_name: Some(name.clone()),
            name,
            content,
        });
    }
    documents.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(documents)
}

fn prompt_configuration_changed(current: &AppConfig, candidate: &AppConfig) -> bool {
    serde_json::to_value(&current.prompt).ok() != serde_json::to_value(&candidate.prompt).ok()
        || current.system_prompt_file != candidate.system_prompt_file
        || current.system_prompt != candidate.system_prompt
}

fn prompt_documents_changed(current: &PromptDocuments, candidate: &PromptDocuments) -> bool {
    canonical_prompt_documents(&current.personas) != canonical_prompt_documents(&candidate.personas)
        || canonical_prompt_documents(&current.identities)
            != canonical_prompt_documents(&candidate.identities)
}

fn canonical_prompt_documents(documents: &[PromptDocument]) -> Vec<(String, String)> {
    let mut values = documents
        .iter()
        .map(|document| (document.name.clone(), document.content.clone()))
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values
}

struct FileBackup {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

struct PersonaScopeBackup {
    original: PathBuf,
    staged: PathBuf,
    destination: Option<PathBuf>,
}

fn apply_prompt_documents(
    current_config: &AppConfig,
    next_config: &AppConfig,
    current: &PromptDocuments,
    next: &PromptDocuments,
    paths: &GqyPaths,
) -> Result<Vec<FileBackup>> {
    let mut mutations = HashMap::<PathBuf, Option<Vec<u8>>>::new();
    collect_prompt_file_mutations(
        &current.personas,
        &next.personas,
        &current_config.prompts_dir_path(paths),
        &next_config.prompts_dir_path(paths),
        &mut mutations,
    );
    collect_prompt_file_mutations(
        &current.identities,
        &next.identities,
        &current_config.identities_dir_path(paths),
        &next_config.identities_dir_path(paths),
        &mut mutations,
    );
    let backups = mutations
        .keys()
        .map(|path| FileBackup {
            path: path.clone(),
            content: std::fs::read(path).ok(),
        })
        .collect::<Vec<_>>();
    for (path, content) in mutations {
        let result = if let Some(content) = content {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)
        } else if path.exists() {
            std::fs::remove_file(&path)
        } else {
            Ok(())
        };
        if let Err(error) = result {
            restore_file_backups(&backups);
            return Err(error.into());
        }
    }
    Ok(backups)
}

fn apply_persona_scope_changes(
    current_config: &AppConfig,
    next_config: &AppConfig,
    current: &PromptDocuments,
    next: &PromptDocuments,
    paths: &GqyPaths,
) -> Result<Vec<PersonaScopeBackup>> {
    let mut changes = Vec::<(String, Option<String>)>::new();
    for document in &current.personas {
        let represented = next.personas.iter().find(|next_document| {
            next_document.original_name.as_deref() == Some(document.name.as_str())
                || next_document.original_name.is_none() && next_document.name == document.name
        });
        match represented {
            Some(next_document) if next_document.name != document.name => {
                changes.push((document.name.clone(), Some(next_document.name.clone())));
            }
            None => changes.push((document.name.clone(), None)),
            _ => {}
        }
    }

    let mut backups = Vec::new();
    let stage_result = (|| -> Result<()> {
        for (change_index, (old_name, new_name)) in changes.iter().enumerate() {
            let old_paths = [
                current_config.persona_memory_data_dir(paths, old_name),
                current_config.persona_memory_state_dir(paths, old_name),
                current_config.persona_skills_dir(paths, old_name),
            ];
            let new_paths = new_name.as_ref().map(|name| {
                [
                    next_config.persona_memory_data_dir(paths, name),
                    next_config.persona_memory_state_dir(paths, name),
                    next_config.persona_skills_dir(paths, name),
                ]
            });
            for (scope_index, original) in old_paths.into_iter().enumerate() {
                if !original.exists() {
                    continue;
                }
                let parent = original
                    .parent()
                    .context("persona scope path has no parent")?;
                let staged = parent.join(format!(
                    ".gqy-web-scope-{}-{change_index}-{scope_index}",
                    random_token(10)
                ));
                std::fs::rename(&original, &staged)?;
                backups.push(PersonaScopeBackup {
                    original,
                    staged,
                    destination: new_paths.as_ref().map(|paths| paths[scope_index].clone()),
                });
            }
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        restore_persona_scope_backups(&backups);
        return Err(error);
    }

    let result = (|| -> Result<()> {
        for backup in &backups {
            let Some(destination) = &backup.destination else {
                continue;
            };
            if destination.exists() {
                anyhow::bail!(
                    "persona scope destination already exists: {}",
                    destination.display()
                );
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&backup.staged, destination)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        restore_persona_scope_backups(&backups);
        return Err(error);
    }
    Ok(backups)
}

fn restore_persona_scope_backups(backups: &[PersonaScopeBackup]) {
    for backup in backups.iter().rev() {
        if let Some(destination) = &backup.destination {
            if destination.exists() && !backup.staged.exists() {
                let _ = std::fs::rename(destination, &backup.staged);
            }
        }
        if backup.staged.exists() {
            if let Some(parent) = backup.original.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&backup.staged, &backup.original);
        }
    }
}

fn finalize_persona_scope_backups(backups: &[PersonaScopeBackup]) {
    for backup in backups {
        if backup.destination.is_none() && backup.staged.exists() {
            let _ = std::fs::remove_dir_all(&backup.staged);
        }
    }
}

fn collect_prompt_file_mutations(
    current: &[PromptDocument],
    next: &[PromptDocument],
    current_dir: &FilePath,
    next_dir: &FilePath,
    mutations: &mut HashMap<PathBuf, Option<Vec<u8>>>,
) {
    for document in next {
        let content = document.content.trim_end();
        let content = if content.is_empty() {
            Vec::new()
        } else {
            format!("{content}\n").into_bytes()
        };
        mutations.insert(next_dir.join(&document.name), Some(content));
    }
    for document in current {
        let represented = next.iter().find(|next_document| {
            next_document.original_name.as_deref() == Some(document.name.as_str())
                || next_document.original_name.is_none() && next_document.name == document.name
        });
        let old_path = current_dir.join(&document.name);
        let retained_at_same_path = represented
            .map(|next_document| next_dir.join(&next_document.name) == old_path)
            .unwrap_or(false);
        if !retained_at_same_path {
            mutations.entry(old_path).or_insert(None);
        }
    }
}

fn restore_file_backups(backups: &[FileBackup]) {
    for backup in backups {
        restore_optional_file(&backup.path, backup.content.as_deref());
    }
}

fn restore_optional_file(path: &FilePath, content: Option<&[u8]>) {
    if let Some(content) = content {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, content);
    } else if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

fn safe_models(config: &AppConfig) -> Vec<SafeModel> {
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

fn web_display_config(config: &AppConfig) -> WebDisplayConfig {
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

fn safe_multimodal_models(config: &AppConfig) -> Vec<SafeModel> {
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
    fn from_turn(turn: Turn, assets: Vec<ImageAsset>) -> Self {
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
fn safe_channel_summary(summary: ChannelSummary) -> SafeChannelSummary {
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

fn first_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn safe_conversation_summary(summary: ConversationSummary) -> SafeConversationSummary {
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
    fn from_asset(asset: ImageAsset, hide_caption: bool) -> Self {
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

fn meme_asset_caption_hidden(asset: &ImageAsset, reports: &[String]) -> bool {
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

fn redact_internal_assistant_text(value: &str) -> String {
    value
        .replace(crate::state::pending_placeholder(), "")
        .replace(crate::state::interrupted_text(), "")
}

fn normalize_answers(
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

fn validate_content(content: String) -> std::result::Result<String, ApiError> {
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

fn validate_short_field(
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

fn validate_model_selection(
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

fn parse_mode(mode: &str) -> std::result::Result<AgentMode, ApiError> {
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

fn mode_name(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Normal => "normal",
        AgentMode::Plan => "plan",
        AgentMode::Chat => "chat",
    }
}

fn real_tool_name(event_name: &str) -> &str {
    if event_name.starts_with("load_skill:") {
        "load_skill"
    } else if event_name.starts_with("load_tools:") {
        "load_tools"
    } else {
        event_name
    }
}

fn require_auth(headers: &HeaderMap, state: &WebState) -> std::result::Result<(), ApiError> {
    if state
        .auth
        .is_authenticated(cookie_value(headers, AUTH_COOKIE))
    {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authentication required",
        ))
    }
}

fn require_mutation(headers: &HeaderMap, state: &WebState) -> std::result::Result<(), ApiError> {
    require_auth(headers, state)?;
    if origin_is_allowed(headers) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "request origin is not allowed",
        ))
    }
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for header in headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for pair in header.split(';') {
            let Some((key, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if key.trim() == name {
                return Some(value.trim());
            }
        }
    }
    None
}

fn origin_is_allowed(headers: &HeaderMap) -> bool {
    let mut origins = headers.get_all(ORIGIN).iter();
    let Some(origin) = origins.next() else {
        return true;
    };
    if origins.next().is_some() {
        return false;
    }
    let Some(host) = headers.get(HOST).and_then(|host| host.to_str().ok()) else {
        return false;
    };
    let expected = format!("http://{host}");
    origin.to_str().is_ok_and(|origin| origin == expected)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

fn random_id(prefix: &str, bytes: usize) -> String {
    format!("{prefix}_{}", random_token(bytes))
}

fn safe_error_message(error: impl std::fmt::Display) -> String {
    let message = error
        .to_string()
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .take(1000)
        .collect::<String>();
    if message.trim().is_empty() {
        "operation failed".to_string()
    } else {
        message
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    if let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn open_browser(_url: &str) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::{QuestionOption, QuestionPrompt};

    #[test]
    fn assistant_sentinels_are_never_exposed() {
        assert_eq!(
            redact_internal_assistant_text(crate::state::pending_placeholder()),
            ""
        );
        assert_eq!(
            redact_internal_assistant_text(crate::state::interrupted_text()),
            ""
        );
        let combined = format!("before {} after", crate::state::interrupted_text());
        let redacted = redact_internal_assistant_text(&combined);
        assert_eq!(redacted, "before  after");
        assert!(!redacted.contains("system-reminder"));
    }

    #[test]
    fn persisted_meme_assets_hide_their_descriptive_caption() {
        let asset = ImageAsset {
            asset_id: "img_test".to_string(),
            turn_id: "turn_test".to_string(),
            tool_id: Some("tool_test".to_string()),
            mime: "image/png".to_string(),
            width: 64,
            height: 64,
            alt: "猫猫 开心 & <得意>".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let reports = vec![
            "<sent_meme>发送了一个表情包：id=sha256:test；description=猫猫 开心 &amp; &lt;得意&gt;</sent_meme>"
                .to_string(),
        ];

        assert!(meme_asset_caption_hidden(&asset, &reports));
        assert!(!meme_asset_caption_hidden(
            &asset,
            &["normal tool output".to_string()]
        ));
    }

    #[test]
    fn cookie_parser_matches_an_exact_cookie_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("other=1; gqy_session=secret-token; suffix=2"),
        );
        assert_eq!(cookie_value(&headers, AUTH_COOKIE), Some("secret-token"));
        assert_eq!(cookie_value(&headers, "session"), None);
    }

    #[test]
    fn origin_check_accepts_absent_or_current_host_origin() {
        let mut headers = HeaderMap::new();
        assert!(origin_is_allowed(&headers));
        headers.insert(HOST, HeaderValue::from_static("192.168.1.20:4096"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:4096"));
        assert!(!origin_is_allowed(&headers));
        headers.insert(ORIGIN, HeaderValue::from_static("http://192.168.1.20:4096"));
        assert!(origin_is_allowed(&headers));
        headers.append(ORIGIN, HeaderValue::from_static("http://192.168.1.20:4096"));
        assert!(!origin_is_allowed(&headers));
    }

    #[test]
    fn optional_password_auth_issues_server_side_sessions_and_limits_failures() {
        let disabled = WebAuth::new(None);
        assert!(disabled.is_authenticated(None));

        let auth = WebAuth::new(Some("correct horse"));
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!auth.is_authenticated(None));
        assert!(matches!(
            auth.login(peer, "wrong"),
            Err(LoginFailure::Invalid)
        ));
        let token = auth.login(peer, "correct horse").unwrap();
        assert!(auth.is_authenticated(Some(&token)));

        let limited = WebAuth::new(Some("secret"));
        for _ in 0..LOGIN_ATTEMPT_LIMIT {
            assert!(matches!(
                limited.login(peer, "wrong"),
                Err(LoginFailure::Invalid)
            ));
        }
        assert!(matches!(
            limited.login(peer, "secret"),
            Err(LoginFailure::RateLimited)
        ));
    }

    #[test]
    fn model_selection_rejects_empty_and_duplicate_pools() {
        assert!(validate_model_selection(Vec::new()).is_err());
        let model = ActiveProviderModelConfig {
            provider_id: "provider".to_string(),
            model: "model".to_string(),
        };
        assert!(validate_model_selection(vec![model.clone()]).is_ok());
        assert!(validate_model_selection(vec![model.clone(), model]).is_err());
    }

    #[test]
    fn config_response_never_serializes_secret_values() {
        let mut config = AppConfig::default();
        config.providers[0].api_key = Some("provider-secret".to_string());
        config.plugins.web.tavily_api_keys = vec!["tavily-secret".to_string()];
        config.plugins.exchange_rate.api_key = "exchange-secret".to_string();
        config.plugins.image_generation.api_keys = vec!["image-secret".to_string()];
        let paths = tempfile::tempdir().unwrap();
        let paths = GqyPaths {
            config_dir: paths.path().join("config"),
            config_file: paths.path().join("config/config.jsonc"),
            skills_dir: paths.path().join("config/skills"),
            data_dir: paths.path().join("data"),
            cache_dir: paths.path().join("cache"),
            state_dir: paths.path().join("state"),
            pictures_dir: paths.path().join("pictures"),
            fish_hook_file: paths.path().join("fish"),
            bash_hook_file: paths.path().join("bash"),
            zsh_hook_file: paths.path().join("zsh"),
            scripts_dir: paths.path().join("scripts"),
            system_scripts_dir: paths.path().join("system-scripts"),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        };
        let response = config_response(
            &config,
            ContextSnapshot {
                tokens: 0,
                window: None,
            },
            &paths,
        )
        .unwrap();
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("provider-secret"));
        assert!(!serialized.contains("tavily-secret"));
        assert!(!serialized.contains("exchange-secret"));
        assert!(!serialized.contains("image-secret"));
        assert_eq!(response.secret_states["providers.0.api_key"], true);
        assert_eq!(response.secret_states["plugins.web.tavily_api_keys"], true);
        assert!(response.config.get("memory").is_some());
    }

    #[test]
    fn omitted_provider_secret_does_not_follow_array_position_after_rename() {
        let mut current = AppConfig::default();
        current.providers[0].id = "first".to_string();
        current.providers[0].api_key = Some("first-secret".to_string());
        let mut candidate = current.clone();
        candidate.providers[0].id = "renamed".to_string();
        candidate.providers[0].api_key = None;
        restore_config_secrets(&mut candidate, &current, &HashMap::new()).unwrap();
        assert_eq!(candidate.providers[0].api_key, None);
    }

    #[test]
    fn explicit_secret_clear_removes_a_provider_key() {
        let mut current = AppConfig::default();
        current.providers[0].api_key = Some("secret".to_string());
        let mut candidate = current.clone();
        candidate.providers[0].api_key = None;
        let mutations = HashMap::from([("providers.0.api_key".to_string(), SecretMutation::Clear)]);
        restore_config_secrets(&mut candidate, &current, &mutations).unwrap();
        assert_eq!(candidate.providers[0].api_key, None);
    }

    #[test]
    fn stale_event_cursor_receives_resync_marker() {
        let events = EventHub::new();
        for index in 0..=EVENT_CAPACITY {
            events.publish("test", json!({ "index": index }));
        }
        let replay = events.replay_after(0);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].kind, "resync_required");
        assert_eq!(replay[0].id, events.latest_id());
        let next = events.publish("after-resync", json!({}));
        assert!(next > replay[0].id);
    }

    #[test]
    fn replay_after_cursor_is_ordered_and_exclusive() {
        let events = EventHub::new();
        events.publish("one", json!({}));
        events.publish("two", json!({}));
        events.publish("three", json!({}));
        let replay = events.replay_after(1);
        assert_eq!(
            replay.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn future_event_cursor_requests_resync_after_server_restart() {
        let events = EventHub::new();
        let replay = events.replay_after(42);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].kind, "resync_required");
    }

    #[test]
    fn answer_validation_trims_values_and_rejects_control_characters() {
        let request = sample_question();
        assert_eq!(
            normalize_answers(&request, vec![vec!["  All  ".to_string()]]).unwrap(),
            vec![vec!["All".to_string()]]
        );
        assert!(normalize_answers(&request, vec![vec!["bad\nanswer".to_string()]]).is_err());
    }

    #[test]
    fn invalid_answer_keeps_question_pending() {
        let broker = QuestionBroker::new();
        let (responder, mut response) = oneshot::channel();
        let question_id = broker.insert("run_test", sample_question(), responder);
        let invalid = broker.answer(&question_id, vec![Vec::new()], |_, _| {
            panic!("invalid answer must not be published")
        });
        assert!(matches!(invalid, Err(AnswerFailure::Invalid(_))));
        assert!(broker.pending.lock().unwrap().contains_key(&question_id));

        broker
            .answer(
                &question_id,
                vec![vec![" All ".to_string()]],
                |run_id, answers| {
                    assert_eq!(run_id, "run_test");
                    assert_eq!(answers, &vec![vec!["All".to_string()]]);
                },
            )
            .unwrap();
        assert!(matches!(
            response.try_recv().unwrap(),
            QuestionResponse::Answered(answers) if answers == vec![vec!["All".to_string()]]
        ));
    }

    #[test]
    fn closed_question_responder_does_not_publish_an_answer() {
        let broker = QuestionBroker::new();
        let (responder, response) = oneshot::channel();
        drop(response);
        let question_id = broker.insert("run_test", sample_question(), responder);
        let mut published = false;
        let result = broker.answer(&question_id, vec![vec!["All".to_string()]], |_, _| {
            published = true
        });
        assert!(matches!(result, Err(AnswerFailure::Gone)));
        assert!(!published);
    }

    fn sample_question() -> QuestionRequest {
        QuestionRequest {
            questions: vec![QuestionPrompt {
                header: "Scope".to_string(),
                question: "Which scope?".to_string(),
                options: vec![QuestionOption {
                    label: "All".to_string(),
                    description: String::new(),
                }],
                multiple: false,
                custom: true,
            }],
        }
    }

    #[test]
    fn content_limit_counts_characters() {
        assert!(validate_content("x".repeat(MAX_CONTENT_CHARS)).is_ok());
        let error = validate_content("界".repeat(MAX_CONTENT_CHARS + 1)).unwrap_err();
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
