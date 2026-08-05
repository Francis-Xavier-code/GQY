//! Background agent actor thread and turn execution.
use crate::agent::{Agent, AgentEvent, AgentMode, AgentTurnControl};
use crate::cli::build_tool_registry;
use crate::clipboard::PastedImage;
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::llm::{ChatResult, ChatStreamKind, LlmClient, Usage};
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use crate::question::{self, QuestionAnswers, QuestionRequest, QuestionResponse};
use crate::state::{QueuedPromptAttachment, StateStore, TurnStatus};
use crate::tools::ToolRegistry;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::config_ops::*;
use super::error::ApiError;
use super::events::{EventHub, QuestionBroker};
use super::mapper::RunEventMapper;
use super::state::{ActorCommand, AdminFailure, ManagerState};
use axum::http::StatusCode;
use super::types::{
    mode_name, normalize_answers, ContextSnapshot, PiCommandKind, PromptDocuments, SafeImageAsset,
    SafeQueuedPrompt, SafeTurn, safe_models, web_display_config,
};
use super::util::{lock_mutex, random_id, safe_error_message};

pub(crate) fn spawn_config_watcher(paths: GqyPaths, actor_tx: mpsc::UnboundedSender<ActorCommand>, events: EventHub) {
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
            let Some(_mtime) = mtime else { continue };
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

pub(crate) fn spawn_actor(
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
pub(crate) async fn actor_loop(
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
pub(crate) fn publish_bridge_image(
    events: &EventHub,
    state_store: &StateStore,
    manager: &Arc<Mutex<ManagerState>>,
    path: std::path::PathBuf,
    alt: String,
) {
    let run_id = lock_mutex(manager)
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
pub(crate) fn publish_bridge_progress(events: &EventHub, manager: &Arc<Mutex<ManagerState>>, message: String) {
    let run_id = lock_mutex(manager)
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
pub(crate) async fn run_agent_turn(
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
            lock_mutex(&callback_mapper).handle(event);
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
            lock_mutex(&callback_mapper).handle(event);
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
                current_context(agent).unwrap_or_else(|_| lock_mutex(&manager).context);
            finish_run(manager, &run_id, Some(context));
            publish_completed(events, &run_id, &result, context);
            return true;
        }
        OverflowOutcome::Shutdown => {
            let context =
                current_context(agent).unwrap_or_else(|_| lock_mutex(&manager).context);
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

pub(crate) enum TurnOutcome {
    Finished(Result<ChatResult>),
    Cancelled,
    Shutdown,
}

pub(crate) enum OverflowOutcome {
    Finished(Result<Option<ChatResult>>),
    Cancelled,
    Shutdown,
}

pub(crate) enum ActiveDirective {
    Continue,
    Cancel,
    Shutdown,
}

pub(crate) async fn run_pi_command(
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

pub(crate) fn active_directive(
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
pub(crate) fn provider_will_change(config: &AppConfig, models: &[ActiveProviderModelConfig]) -> bool {
    let next: Vec<&str> = models.iter().map(|m| m.provider_id.as_str()).collect();
    let current: Vec<&str> = match config.active_provider_models.as_deref() {
        Some(list) => list.iter().map(|m| m.provider_id.as_str()).collect(),
        None => vec![config.active_provider.as_str()],
    };
    next != current
}

pub(crate) fn rebuild_for_models(
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
    let mut manager = lock_mutex(&manager);
    manager.config = next_config;
    manager.context = context;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_for_config(
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
    let mut manager = lock_mutex(&manager);
    manager.config = next_config;
    manager.context = context;
    drop(manager);
    if reset_conversation {
        events.publish("conversation.reset", json!({}));
    }
    finalize_persona_scope_backups(&scope_backups);
    Ok(())
}

pub(crate) fn reset_actor_conversation(
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
    lock_mutex(&manager).context = context;
    events.publish("conversation.reset", json!({}));
    Ok(())
}

pub(crate) fn finish_cancelled_run(
    manager: &Arc<Mutex<ManagerState>>,
    events: &EventHub,
    agent: &Agent,
    run_id: &str,
) {
    let context = current_context(agent).ok();
    finish_run(manager, run_id, context);
    events.publish("run.cancelled", json!({ "run_id": run_id }));
}

pub(crate) fn finish_failed_run(
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

pub(crate) fn finish_completed_with_context_error(
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
    let context = current_context(agent).unwrap_or_else(|_| lock_mutex(&manager).context);
    finish_run(manager, run_id, Some(context));
    publish_completed(events, run_id, result, context);
}

pub(crate) fn finish_run(manager: &Arc<Mutex<ManagerState>>, run_id: &str, context: Option<ContextSnapshot>) {
    let mut manager = lock_mutex(&manager);
    if let Some(context) = context {
        manager.context = context;
    }
    if manager.active_run_id.as_deref() == Some(run_id) {
        manager.active_run_id = None;
    }
}

pub(crate) fn publish_completed(
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

pub(crate) fn current_context(agent: &Agent) -> Result<ContextSnapshot> {
    Ok(ContextSnapshot {
        tokens: agent.effective_context_tokens()?,
        window: agent.context_window(),
    })
}

pub(crate) fn reserve_admin(manager: &Arc<Mutex<ManagerState>>) -> std::result::Result<(), ApiError> {
    let mut manager = lock_mutex(&manager);
    if manager.active_run_id.is_some() || manager.admin_busy {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "GQY is busy with another operation",
        ));
    }
    manager.admin_busy = true;
    Ok(())
}

pub(crate) fn require_no_running_turn(state_store: &StateStore) -> std::result::Result<(), ApiError> {
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

pub(crate) fn release_admin(manager: &Arc<Mutex<ManagerState>>) {
    lock_mutex(&manager).admin_busy = false;
}

