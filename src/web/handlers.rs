//! HTTP API handlers.
use crate::agent::AgentMode;
use crate::cli::WebArgs;
use crate::clipboard::{ClipboardImage, PastedImage};
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::llm::Usage;
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use crate::question::{self, QuestionAnswers, QuestionRequest, QuestionResponse};
use crate::state::{
    ImageAsset, QueuedPrompt, QueuedPromptAttachment, StateStore, TurnStatus,
};
use crate::tools::{self, CommandOutputStream, ToolRegistry};
use anyhow::{Context, Result};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, ORIGIN, RETRY_AFTER, SET_COOKIE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::io::{self, IsTerminal};
use std::net::SocketAddr;
use std::path::{Path as FilePath, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::actor::{
    finish_run, release_admin, require_no_running_turn, reserve_admin,
};
use super::auth::{
    cookie_value, origin_is_allowed, require_auth, require_mutation, LoginFailure, WebAuth,
};
use super::config_ops::*;
use super::error::ApiError;
use super::events::{AnswerFailure, EventHub, EventRecord, QuestionBroker};
use super::state::{ActorCommand, AdminFailure, ManagerState, WebState};
use super::types::*;
use super::util::{lock_mutex, random_id, random_token, safe_error_message, AUTH_COOKIE};

pub(crate) async fn auth_login(
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

pub(crate) fn resolve_web_password(args: &WebArgs) -> Result<Option<String>> {
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

pub(crate) async fn health() -> Json<Value> {
    Json(json!({
        "status": "ready",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// WebUI 语音回复：文本 → 本地 Qwen3-TTS 克隆音色（scripts/tts-server.py :8091）→ wav。
/// 返回 audio/wav 流；TTS 服务未启动时返回 503（前端静默降级）。
#[derive(Deserialize)]
pub(crate) struct TtsQuery {
    pub(crate) text: String,
}

pub(crate) async fn tts_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<TtsQuery>,
) -> Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    if query.text.trim().is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "text is required"));
    }
    // URL 编码
    let mut encoded = String::new();
    for c in query.text.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            encoded.push(c);
        } else if c == ' ' {
            encoded.push('+');
        } else {
            for b in c.to_string().as_bytes() {
                encoded.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let url = format!("http://127.0.0.1:8091/tts?text={encoded}");
    let client = &state.http_client;
    let resp = match client.get(&url).timeout(std::time::Duration::from_secs(90)).send().await {
        Ok(r) => r,
        Err(_) => {
            // TTS 服务未运行：自动拉起（按需启停，省内存）
            spawn_tts_server(&state.paths)?;
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            client
                .get(&url)
                .timeout(std::time::Duration::from_secs(90))
                .send()
                .await
                .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "TTS 服务启动失败"))?
        }
    };
    if !resp.status().is_success() {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "TTS 合成失败"));
    }
    let bytes = resp.bytes().await.map_err(ApiError::internal)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "audio/wav")
        .header("Content-Length", bytes.len())
        .body(axum::body::Body::from(bytes))
        .unwrap())
}

/// 按需拉起本地 TTS 服务（scripts/tts-server.py，venv python），空闲自动退出省内存。
pub(crate) fn spawn_tts_server(paths: &GqyPaths) -> Result<(), ApiError> {
    let script = paths.share_dir.join("scripts").join("tts-server.py");
    let venv_python = paths.share_dir.join("venv").join("bin").join("python");
    if !std::path::Path::new(&venv_python).exists() {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "未找到 venv python（TTS 依赖未安装）"));
    }
    std::process::Command::new(venv_python)
        .arg(script)
        .arg("8091")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("TTS 服务拉起失败: {e}")))?;
    Ok(())
}

/// 会话状态（供终端/面板同步轮询）：当前会话最大 seq + 是否有运行中的轮次。
/// 终端 `gqy` 与面板共享同一 conversation.db；前端轮询此接口，
/// 发现 seq 变化即重载历史，实现双端同步。
pub(crate) async fn session_state(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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
pub(crate) async fn backup_status_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, ApiError> {
    require_auth(&headers, &state.auth)?;
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
pub(crate) async fn usage_stats_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    let stats = state
        .state_store
        .usage_stats()
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "stats": stats })).into_response())
}

/// 最近调用明细（时间/模型/输入/输出/缓存命中/是否记忆辅助），供用量页列表与模型详情。
pub(crate) async fn usage_details_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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
pub(crate) async fn balance_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
    const ERROR_TTL: std::time::Duration = std::time::Duration::from_secs(10);
    let cache = state.balance_cache.clone();
    if let Some((at, value)) = lock_mutex(&cache).as_ref() {
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
        let config = lock_mutex(&manager).config.clone();
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
        let mut cache = lock_mutex(&cache);
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
pub(crate) async fn cancel_alarm_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(alarm_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    // 取消闹钟是写操作：登录 + 同源，避免跨站触发
    require_mutation(&headers, &state.auth)?;
    let cancelled = crate::alarm::cancel(&state.paths, &alarm_id).map_err(ApiError::internal)?;
    if !cancelled {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "alarm not found"));
    }
    Ok(Json(json!({ "ok": true, "id": alarm_id })).into_response())
}

/// 定时任务（闹钟/番茄钟）列表，供面板可视化。
pub(crate) async fn list_alarms_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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
pub(crate) async fn channel_turns_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Query(query): Query<ChannelTurnsQuery>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    let mode = query.mode.as_deref();
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
        .load_visible_turns_for_channel_mode(&channel_id, mode)
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
pub(crate) async fn pi_control_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    method: axum::http::Method,
    Path(kind): Path<String>,
    body: axum::body::Bytes,
) -> std::result::Result<Response, ApiError> {
    // 读操作用登录态即可；改模型/思考级别等写操作要求同源，防 CSRF
    if method == axum::http::Method::GET {
        require_auth(&headers, &state.auth)?;
    } else {
        require_mutation(&headers, &state.auth)?;
    }
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
pub(crate) async fn export_conversation_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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
/// 需要认证：防止非回环地址下的未授权远程关闭。
pub(crate) async fn shutdown_web(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, ApiError> {
    // 关机是高危写操作：需要登录 + 同源，避免 CSRF/跨站触发
    require_mutation(&headers, &state.auth)?;
    state.shutdown.notify_one();
    Ok(Json(json!({ "ok": true, "message": "shutting down" })))
}

/// Web 端调用 GQY 工具（工具块「重跑」用）。
/// 与 pi 工具桥共用 registry；task/deep_research 等长任务给足超时。
pub(crate) async fn call_tool_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<WebToolCallRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state.auth)?;
    let name = request.name.clone();
    let name_for_call = name.clone();
    let arguments = if request.arguments.is_null() {
        json!({})
    } else {
        request.arguments
    };
    let arguments_str = arguments.to_string();
    let registry = lock_mutex(&state.bridge_registry).clone();
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
pub(crate) struct WebToolCallRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

/// 全文搜索对话（跨通道），返回匹配轮次（新→旧）
pub(crate) async fn search_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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
pub(crate) struct SearchQuery {
    pub(crate) q: String,
    pub(crate) limit: Option<usize>,
}

/// 读取指定历史会话的 turns（当前通道内，含归档轮次；只读数据源）
pub(crate) async fn conversation_turns_web(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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

pub(crate) async fn bootstrap(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    state
        .state_store
        .recover_stale_turns()
        .map_err(ApiError::internal)?;
    let (config, active_run_id, context) = {
        let manager = lock_mutex(&state.manager);
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

pub(crate) async fn get_config(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
    let (config, context) = {
        let manager = lock_mutex(&state.manager);
        (manager.config.clone(), manager.context)
    };
    let mut response = Json(config_response(&config, context, &state.paths)?).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

pub(crate) async fn update_config(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<UpdateConfigRequest>,
) -> std::result::Result<Json<ConfigResponse>, ApiError> {
    require_mutation(&headers, &state.auth)?;
    require_no_running_turn(&state.state_store)?;

    let current = lock_mutex(&state.manager).config.clone();
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
    let manager = lock_mutex(&state.manager);
    Ok(Json(config_response(
        &manager.config,
        manager.context,
        &state.paths,
    )?))
}

pub(crate) async fn image_asset(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_auth(&headers, &state.auth)?;
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

pub(crate) async fn events(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    require_auth(&headers, &state.auth)?;
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

pub(crate) struct SseStreamState {
    pub(crate) pending: VecDeque<EventRecord>,
    pub(crate) receiver: broadcast::Receiver<EventRecord>,
    pub(crate) events: EventHub,
    pub(crate) last_id: u64,
}

pub(crate) fn record_to_sse(record: EventRecord) -> Event {
    Event::default()
        .id(record.id.to_string())
        .event(record.kind)
        .data(record.data)
}

pub(crate) fn enqueue_running_prompt(
    state: &WebState,
    content: &str,
    attachments: &[QueuedPromptAttachment],
) -> std::result::Result<(Option<String>, Option<String>, SafeQueuedPrompt), ApiError> {
    let active_run_id = {
        let manager = lock_mutex(&state.manager);
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

pub(crate) fn publish_queued_prompt(
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

pub(crate) async fn create_turn(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<CreateTurnRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state.auth)?;
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
        let mut manager = lock_mutex(&state.manager);
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
pub(crate) async fn persist_user_images(
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
    // 使用 spawn_blocking 避免阻塞 tokio 运行时
    let dir_clone = dir.clone();
    if tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir_clone))
        .await
        .is_err()
    {
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
        let path_clone = path.clone();
        if tokio::task::spawn_blocking(move || std::fs::write(&path_clone, &data))
            .await
            .is_err()
        {
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

pub(crate) async fn queue_prompt(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<QueuePromptRequest>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state.auth)?;
    let content = validate_content(request.content)?;
    let attachments = queued_attachments_from_input(&request.images);
    let (run_id, turn_id, safe) = enqueue_running_prompt(&state, &content, &attachments)?;
    publish_queued_prompt(&state, run_id.as_deref(), turn_id.as_deref(), &safe);
    Ok((StatusCode::ACCEPTED, Json(safe)).into_response())
}

pub(crate) async fn remove_queue_prompt(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(prompt_id): Path<String>,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state.auth)?;
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
    let run_id = lock_mutex(&state.manager).active_run_id.clone();
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

pub(crate) async fn cancel_run(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    require_mutation(&headers, &state.auth)?;
    let matches_active =
        lock_mutex(&state.manager).active_run_id.as_deref() == Some(run_id.as_str());
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

pub(crate) async fn answer_question(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(question_id): Path<String>,
    Json(request): Json<AnswerQuestionRequest>,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state.auth)?;
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

pub(crate) async fn set_models(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<SetModelsRequest>,
) -> std::result::Result<Json<ModelResponse>, ApiError> {
    require_mutation(&headers, &state.auth)?;
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
    let manager = lock_mutex(&state.manager);
    Ok(Json(ModelResponse {
        models: safe_models(&manager.config),
        display: web_display_config(&manager.config),
        context: manager.context,
    }))
}

pub(crate) async fn reset_conversation(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> std::result::Result<StatusCode, ApiError> {
    require_mutation(&headers, &state.auth)?;
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

