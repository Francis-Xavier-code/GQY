//! Runtime state shared by HTTP handlers and the agent actor.
use crate::agent::AgentMode;
use crate::clipboard::PastedImage;
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::paths::GqyPaths;
use crate::question::QuestionResponse;
use crate::state::StateStore;
use crate::tools::ToolRegistry;
use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, Notify};

use super::auth::WebAuth;
use super::events::{EventHub, QuestionBroker};
use super::types::{ContextSnapshot, PiCommandKind, PromptDocuments};

#[derive(Clone)]
pub(crate) struct WebState {
    pub(crate) auth: WebAuth,
    pub(crate) boot_id: Arc<str>,
    pub(crate) paths: GqyPaths,
    pub(crate) manager: Arc<Mutex<ManagerState>>,
    pub(crate) state_store: StateStore,
    pub(crate) events: EventHub,
    pub(crate) questions: QuestionBroker,
    pub(crate) actor_tx: mpsc::UnboundedSender<ActorCommand>,
    /// 余额查询缓存（60s 防抖：每次对话后刷新一次即可，避免频繁请求公开接口）
    pub(crate) balance_cache: Arc<Mutex<Option<(std::time::Instant, serde_json::Value)>>>,
    /// pi 工具桥共用的工具注册表（Web 端 /api/tools/call 与桥共享）
    pub(crate) bridge_registry: Arc<std::sync::Mutex<ToolRegistry>>,
    /// 优雅退出信号（/api/shutdown 触发，菜单栏「退出」用它统一收尾）
    pub(crate) shutdown: Arc<tokio::sync::Notify>,
    /// 共享 HTTP client（复用连接池，避免每次请求创建新 client）
    pub(crate) http_client: reqwest::Client,
}

pub(crate) struct ManagerState {
    pub(crate) config: AppConfig,
    pub(crate) active_run_id: Option<String>,
    pub(crate) admin_busy: bool,
    pub(crate) context: ContextSnapshot,
}

pub(crate) enum ActorCommand {
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
pub(crate) enum AdminFailure {
    Invalid(String),
    Internal(String),
}

