//! Agent tool-calling loop: model stream → tool dispatch → queue consumption.
use super::*;
use crate::llm::{ChatMessage, ChatResult, ChatStreamChunk, ChatStreamKind};
use crate::question::{
    answered_tool_output, unavailable_tool_output, QuestionCancelled, QuestionExchange,
    QuestionRequest, QuestionResponse,
};
use crate::tools::{self, ToolPermission};
use anyhow::{bail, Result};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

impl Agent {
    pub(super) async fn chat_with_tools<F>(
        &mut self,
        current_turn_id: &str,
        messages: &mut Vec<ChatMessage>,
        used_tools: &mut Vec<String>,
        persisted_tool_reports: &mut Vec<(String, String)>,
        control: Option<&AgentTurnControl>,
        on_event: &mut F,
    ) -> Result<ChatResult>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        let mut tool_round = 0usize;
        let mut question_rounds = 0usize;
        let mut queue_consumed = 0usize;
        let mut loaded_tools = self.initial_loaded_tools(messages)?;
        let mut usage_accumulator = UsageAccumulator::default();
        loop {
            let tool_limit_reached = self.max_tool_rounds > 0 && tool_round >= self.max_tool_rounds;

            if self.mode == AgentMode::Normal {
                let mut tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
                tools::rescan_scripts(&mut tools, &self.paths);
                tools::register_script_display_names(&tools);
            }

            let definitions = if self.tools_enabled && !tool_limit_reached {
                let tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
                if tools::is_hybrid_loading_mode(&self.config.tools.loading_mode) {
                    tools.lazy_definitions(&loaded_tools)
                } else {
                    tools.definitions()
                }
            } else {
                Vec::new()
            };

            on_event(AgentEvent::ReasoningStart {
                received_at: Instant::now(),
            })?;
            let (chunk_tx, mut chunk_rx) =
                tokio::sync::mpsc::unbounded_channel::<(ChatStreamChunk, Instant)>();
            let request_messages = messages.clone();
            let mut reasoning_filter = ReasoningTitleFilter::default();
            let result = {
                let llm_future =
                    self.client
                        .chat_stream(request_messages.clone(), definitions, move |chunk| {
                            let _ = chunk_tx.send((chunk, Instant::now()));
                            Ok(())
                        });
                tokio::pin!(llm_future);
                let mut spinner_interval = new_spinner_interval();
                // 流式进度同步：累积正文，每 ~1s 写入 db 供面板轮询显示
                let mut progress_sync = tokio::time::interval(std::time::Duration::from_secs(1));
                progress_sync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut progress_content = String::new();
                loop {
                    tokio::select! {
                        result = &mut llm_future => {
                            break result?;
                        }
                        Some((chunk, received_at)) = chunk_rx.recv() => {
                            if chunk.kind == ChatStreamKind::Content {
                                progress_content.push_str(&chunk.text);
                            }
                            emit_filtered_chunk_at(chunk, received_at, &mut reasoning_filter, on_event)?;
                        }
                        _ = spinner_interval.tick() => {
                            on_event(AgentEvent::SpinnerTick)?;
                        }
                        _ = progress_sync.tick() => {
                            if !progress_content.is_empty() {
                                let _ = self
                                    .state
                                    .update_assistant_progress(current_turn_id, &progress_content);
                            }
                        }
                    }
                }
            };
            while let Ok((chunk, received_at)) = chunk_rx.try_recv() {
                emit_filtered_chunk_at(chunk, received_at, &mut reasoning_filter, on_event)?;
            }
            let (title, text) = reasoning_filter.finish();
            if let Some(title) = title {
                on_event(AgentEvent::ReasoningTitle(title))?;
            }
            if let Some(text) = text {
                on_event(AgentEvent::Chunk(ChatStreamChunk {
                    kind: ChatStreamKind::Reasoning,
                    text,
                }))?;
            }
            usage_accumulator.add_result(&result, &request_messages);
            if result.tool_calls.is_empty() || !self.tools_enabled {
                if let Some(control) = control {
                    let queued = self.state.load_queued_prompts()?;
                    if !queued.is_empty() && queue_consumed < MAX_QUEUE_CONSUMPTION_ROUNDS {
                        queue_consumed += 1;
                        if let Some(replay) = chat_result_replay_content(&result) {
                            messages.push(ChatMessage::plain("assistant", replay));
                        }
                        self.consume_queued_prompts(
                            current_turn_id,
                            messages,
                            queued,
                            (
                                Some(&result.content),
                                result.reasoning.as_deref(),
                                result.provider_id.as_deref(),
                                result.model.as_deref(),
                            ),
                            control,
                            on_event,
                        )
                        .await?;
                        continue;
                    }
                }
                let mut result = result;
                if let Some(usage) = usage_accumulator.usage() {
                    result.usage = Some(usage);
                    result.usage_estimated = usage_accumulator.estimated;
                }
                return Ok(result);
            }
            if tool_limit_reached {
                let mut result = result;
                let warning = format!(
                    "工具调用已达到上限 {} 轮，未执行后续工具调用。可将 `tools.max_rounds` 设为 0 以允许无限工具调用。",
                    self.max_tool_rounds
                );
                let warning_chunk = if result.content.trim().is_empty() {
                    warning.clone()
                } else {
                    format!("\n\n{warning}")
                };
                result.content.push_str(&warning_chunk);
                on_event(AgentEvent::Chunk(ChatStreamChunk {
                    kind: ChatStreamKind::Content,
                    text: warning_chunk,
                }))?;
                result.tool_calls.clear();
                if let Some(usage) = usage_accumulator.usage() {
                    result.usage = Some(usage);
                    result.usage_estimated = usage_accumulator.estimated;
                }
                return Ok(result);
            }
            tool_round += 1;
            messages.push(ChatMessage::assistant(
                result.content.clone(),
                Some(result.tool_calls.clone()),
            ));
            let ask_question_enabled = self
                .tools
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tool_names()
                .iter()
                .any(|name| name == "ask_question");
            let question_call_count = result
                .tool_calls
                .iter()
                .filter(|call| ask_question_enabled && call.function.name == "ask_question")
                .count();
            if question_call_count == 1 {
                question_rounds += 1;
            }
            let question_round_allowed =
                question_call_count == 1 && question_rounds <= MAX_QUESTION_ROUNDS_PER_TURN;
            let defer_sibling_tools = question_call_count == 1 && result.tool_calls.len() > 1;
            for call in result.tool_calls {
                let event_name = tool_event_name(&call.function.name, &call.function.arguments);
                on_event(AgentEvent::ToolCall {
                    name: event_name.clone(),
                    arguments: call.function.arguments.clone(),
                })?;
                if question_call_count > 1 {
                    let output = emit_tool_error(&event_name, "only one ask_question call is allowed per tool batch; combine all questions into one call", on_event)?;
                    messages.push(ChatMessage::tool(call.id, output));
                    continue;
                }
                if defer_sibling_tools && call.function.name != "ask_question" {
                    let output = emit_tool_error(&event_name, "deferred until the user answers ask_question; reissue this tool call after receiving the answer", on_event)?;
                    messages.push(ChatMessage::tool(call.id, output));
                    continue;
                }
                if ask_question_enabled && call.function.name == "ask_question" {
                    if !question_round_allowed {
                        let output = emit_tool_error(&event_name, format_args!("ask_question exceeded the per-turn limit of {MAX_QUESTION_ROUNDS_PER_TURN}"), on_event)?;
                        messages.push(ChatMessage::tool(call.id, output));
                        continue;
                    }
                    let request = match QuestionRequest::parse(&call.function.arguments) {
                        Ok(request) => request,
                        Err(err) => {
                            let output = emit_tool_error(&event_name, format_args!("invalid ask_question request: {err}"), on_event)?;
                            messages.push(ChatMessage::tool(call.id, output));
                            continue;
                        }
                    };
                    let (response_tx, response_rx) = oneshot::channel();
                    on_event(AgentEvent::AskQuestion {
                        request: request.clone(),
                        responder: response_tx,
                    })?;
                    let response = response_rx.await.unwrap_or_else(|_| QuestionResponse::Cancelled);
                    let output = match response {
                        QuestionResponse::Answered(answers) => {
                            let exchange = QuestionExchange::new(request, answers)?;
                            self.state
                                .append_question_exchange(current_turn_id, &exchange)?;
                            answered_tool_output(&exchange)
                        }
                        QuestionResponse::Cancelled => return Err(QuestionCancelled.into()),
                        QuestionResponse::Unavailable(reason) => unavailable_tool_output(&reason),
                    };
                    messages.push(ChatMessage::tool(call.id, output.clone()));
                    on_event(AgentEvent::ToolResult {
                        name: event_name,
                        ok: true,
                        output,
                    })?;
                    continue;
                }
                used_tools.push(call.function.name.clone());
                {
                    let tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
                    if matches!(self.mode, AgentMode::Plan | AgentMode::Chat)
                        && tools.permission(&call.function.name)? != ToolPermission::ReadOnly
                    {
                        bail!(
                            "{} mode blocked non-read-only tool: {}",
                            self.mode.label(),
                            call.function.name
                        );
                    }
                    if tools::is_hybrid_loading_mode(&self.config.tools.loading_mode)
                        && call.function.name != "load_tools"
                        && tools.requires_lazy_load(&call.function.name, &loaded_tools)
                    {
                        if tools.can_auto_load_direct_call(&call.function.name) {
                            loaded_tools.insert(call.function.name.clone());
                            if self.config.tools.persist_loaded_tools {
                                self.state.add_session_loaded_tools(
                                    &[call.function.name.clone()],
                                    Some(current_turn_id),
                                )?;
                            }
                        } else {
                            let msg = format!(
                                "工具 `{}` 尚未加载。请先调用 load_tools，参数为 {{\"names\":[\"{}\"]}}。",
                                call.function.name,
                                call.function.name,
                            );
                            let output = emit_tool_error(&event_name, msg, on_event)?;
                            messages.push(ChatMessage::tool(call.id, output));
                            continue;
                        }
                    }
                }
                if call.function.name == "install_aur_package"
                    && used_tools.iter().any(|name| name == "review_aur_package")
                {
                    let output = emit_tool_error(&event_name, "install_aur_package cannot run in the same turn as review_aur_package; ask the user to confirm installation first", on_event)?;
                    messages.push(ChatMessage::tool(call.id, output));
                    continue;
                }
                // Dry-run 模式：显示工具调用计划而不实际执行
                if self.dry_run {
                    let output = format!(
                        "[dry-run] 工具: {}\n参数: {}\n\n使用 --dry-run 时不会实际执行工具调用。",
                        call.function.name, call.function.arguments
                    );
                    emit_tool_result(&event_name, &output, on_event)?;
                    messages.push(ChatMessage::tool(call.id, output));
                    continue;
                }
                let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
                let tool_future = {
                    let tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
                    tools.call_with_progress_future(
                        &call.function.name,
                        &call.function.arguments,
                        progress_tx,
                    )
                };
                let tool_future = match tool_future {
                    Ok(f) => f,
                    Err(err) => {
                        let output = emit_tool_error(&event_name, err, on_event)?;
                        messages.push(ChatMessage::tool(call.id, output));
                        continue;
                    }
                };
                tokio::pin!(tool_future);
                let mut spinner_interval = new_spinner_interval();
                let (mut output, tool_succeeded) = loop {
                    tokio::select! {
                        result = &mut tool_future => {
                            break match result {
                                Ok(output) => {
                                    while let Ok(progress) = progress_rx.try_recv() {
                                        emit_tool_progress(on_event, &event_name, progress)?;
                                    }
                                    (output, true)
                                }
                                Err(err) => {
                                    while let Ok(progress) = progress_rx.try_recv() {
                                        emit_tool_progress(on_event, &event_name, progress)?;
                                    }
                                    let output = format!("tool error: {err}");
                                    on_event(AgentEvent::ToolResult {
                                        name: event_name.clone(),
                                        ok: false,
                                        output: output.clone(),
                                    })?;
                                    (output, false)
                                }
                            };
                        }
                        Some(progress) = progress_rx.recv() => {
                            emit_tool_progress(on_event, &event_name, progress)?;
                        }
                        _ = spinner_interval.tick() => {
                            on_event(AgentEvent::SpinnerTick)?;
                        }
                    }
                };
                let clipboard_image = if tool_succeeded {
                    clipboard_binary_image_from_tool_result(&call.function.name, &output)
                } else {
                    None
                };
                messages.push(ChatMessage::tool(call.id, output.clone()));
                // 活动日志：记录工具调用成败（默认不进上下文，gqy activity 可查）
                crate::activity::record_tool(&self.paths, &call.function.name, tool_succeeded);
                if tool_succeeded && call.function.name == "load_tools" {
                    let loaded = loaded_items_from_output(&output);
                    for name in &loaded.tools {
                        loaded_tools.insert(name.clone());
                    }
                    if self.config.tools.persist_loaded_tools {
                        self.state
                            .add_session_loaded_tools(&loaded.tools, Some(current_turn_id))?;
                        self.state
                            .add_session_loaded_targets(&loaded.targets, Some(current_turn_id))?;
                    }
                }
                if let Some(img) = clipboard_image {
                    let supports_vision = self.current_model_supports_vision();
                    let uses_vision_fallback =
                        !supports_vision && self.config.plugins.vision.enabled;
                    if !supports_vision {
                        let message = if self.config.plugins.vision.enabled {
                            if crate::i18n::is_zh() {
                                "视觉分析."
                            } else {
                                "Vision analysis."
                            }
                        } else if crate::i18n::is_zh() {
                            "当前模型不支持图片，且未启用视觉模型，无法分析剪贴板图片。"
                        } else {
                            "The current model does not support images and the vision plugin is disabled, so the clipboard image cannot be analyzed."
                        };
                        on_event(AgentEvent::ToolProgress {
                            name: event_name.clone(),
                            message: message.to_string(),
                        })?;
                    }
                    let image_message = if uses_vision_fallback {
                        let image_future = self.clipboard_image_message(img);
                        tokio::pin!(image_future);
                        let mut spinner_interval = new_spinner_interval();
                        let mut progress_interval =
                            tokio::time::interval(Duration::from_millis(900));
                        progress_interval
                            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        progress_interval.tick().await;
                        let mut progress_tick = 0usize;
                        loop {
                            tokio::select! {
                                result = &mut image_future => {
                                    break result?;
                                }
                                _ = progress_interval.tick() => {
                                    progress_tick = progress_tick.wrapping_add(1);
                                    on_event(AgentEvent::ToolProgress {
                                        name: event_name.clone(),
                                        message: vision_analysis_progress(progress_tick),
                                    })?;
                                }
                                _ = spinner_interval.tick() => {
                                    on_event(AgentEvent::SpinnerTick)?;
                                }
                            }
                        }
                    } else {
                        self.clipboard_image_message(img).await?
                    };
                    if let Some(message) = image_message {
                        messages.push(message);
                    }
                }
                if tool_succeeded {
                    let result_ok = if call.function.name == "run_command" {
                        let parsed: Option<serde_json::Value> = serde_json::from_str(&output).ok();
                        let success = parsed
                            .as_ref()
                            .and_then(|v| v.get("success").and_then(serde_json::Value::as_bool))
                            .unwrap_or(true);
                        // 误判回退：run_command 失败且 stderr 疑似 command not found 时，
                        // 提示 LLM 建议用户直接在 shell 执行
                        if !success && self.config.shell.fallback_to_shell {
                            if let Some((exit_code, stderr)) = parsed.as_ref().and_then(|v| {
                                let ec = v.get("exit_code")?.as_i64()?;
                                let err = v.get("stderr")?.as_str()?;
                                Some((ec, err))
                            }) {
                                let stderr_lower = stderr.to_lowercase();
                                if exit_code == 127
                                    || stderr_lower.contains("command not found")
                                    || stderr_lower.contains("未找到命令")
                                    || stderr_lower.contains("no such file or directory")
                                    || stderr_lower.contains("not found")
                                {
                                    output = format!(
                                        "{}\n\n[hint: The command appears to not exist. \
                                        If the user meant to run a shell command directly, \
                                        suggest they try it in their terminal.]",
                                        output
                                    );
                                }
                            }
                        }
                        success
                    } else {
                        true
                    };
                    on_event(AgentEvent::ToolResult {
                        name: event_name.clone(),
                        ok: result_ok,
                        output: output.clone(),
                    })?;
                    if let Some(report) =
                        extract_persistable_tool_report(&call.function.name, &output)
                    {
                        persisted_tool_reports.push((call.function.name.clone(), report));
                    }
                }
            }
            if question_round_allowed {
                tool_round = tool_round.saturating_sub(1);
            }
            if let Some(control) = control {
                let queued = self.state.load_queued_prompts()?;
                if !queued.is_empty() && queue_consumed < MAX_QUEUE_CONSUMPTION_ROUNDS {
                    queue_consumed += 1;
                    self.consume_queued_prompts(
                        current_turn_id,
                        messages,
                        queued,
                        (
                            Some(&result.content),
                            result.reasoning.as_deref(),
                            result.provider_id.as_deref(),
                            result.model.as_deref(),
                        ),
                        control,
                        on_event,
                    )
                    .await?;
                }
            }
        }
    }
}
