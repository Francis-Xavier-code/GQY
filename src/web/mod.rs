//! 顾清影 WebUI（Axum + SSE + agent actor）。
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`handlers`] | HTTP API |
//! | [`actor`] | 后台回合 / 管理命令 / 热重载 |
//! | [`auth`] | 登录会话与 CSRF |
//! | [`events`] | SSE 事件中心 |
//! | [`config_ops`] | 配置脱敏与提示词文档 |
//! | [`types`] | DTO / Safe* 视图 |
//! | [`assets`] | 内嵌前端静态资源 |

mod actor;
mod assets;
mod auth;
mod config_ops;
mod error;
mod events;
mod handlers;
mod mapper;
mod state;
mod types;
mod util;

#[cfg(test)]
mod tests;

use crate::agent::{Agent, AgentMode};
use crate::cli::{build_tool_registry, WebArgs};
use crate::config::AppConfig;
use crate::llm::LlmClient;
use crate::paths::GqyPaths;
use crate::state::StateStore;
use anyhow::{Context, Result};
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::Router;
use std::future::IntoFuture;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use actor::{publish_bridge_image, publish_bridge_progress, spawn_actor, spawn_config_watcher};
use assets::{
    app_asset, index_asset, logo_asset, provider_icons_asset, styles_asset, usage_viz_asset,
    wallpaper_asset,
};
use auth::WebAuth;
use events::{EventHub, QuestionBroker};
use handlers::{
    answer_question, auth_login, backup_status_web, balance_web, bootstrap, call_tool_web,
    cancel_alarm_web, cancel_run, channel_turns_web, conversation_turns_web, create_turn, events,
    export_conversation_web, get_config, health, image_asset, list_alarms_web, pi_control_web,
    queue_prompt, remove_queue_prompt, reset_conversation, resolve_web_password, search_web,
    session_state, set_models, shutdown_web, tts_web, update_config, usage_details_web,
    usage_stats_web,
};
use state::{ActorCommand, ManagerState, WebState};
use types::ContextSnapshot;
use util::{
    open_browser, random_id, shutdown_signal, web_access_urls, JSON_BODY_LIMIT,
};

pub async fn run(paths: GqyPaths, args: WebArgs) -> Result<()> {
    // WebUI 面板默认在独立的 webui 通道对话（与终端/QQ/Telegram 各自独立上下文）；
    // 终端显式设置 GQY_CHANNEL 可覆盖
    if std::env::var_os("GQY_CHANNEL").is_none() {
        // SAFETY: 此处在 run() 入口、服务器启动前调用，尚无并发读写 env 的其他线程。
        // 2024 edition 将 set_var 标记为 unsafe 是因为多线程并发时有 data race 风险，
        // 但此处的调用时序保证了安全。
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
        http_client: reqwest::Client::new(),
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
        .route("/api/tts", get(tts_web))
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

