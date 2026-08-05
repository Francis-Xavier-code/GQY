//! Agent streaming conversation logic.
//!
//! This module contains the main chat streaming functions that handle
//! the conversation flow, including turn management, tool calling,
//! and result processing.

use super::*;

impl Agent {
    pub async fn chat_stream<F>(&mut self, input: &str, on_event: F) -> Result<ChatResult>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        self.chat_stream_with_images(input, &[], on_event).await
    }

    pub async fn chat_stream_with_images<F>(
        &mut self,
        input: &str,
        images: &[Option<PastedImage>],
        on_event: F,
    ) -> Result<ChatResult>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        self.chat_stream_with_images_inner(input, images, None, on_event)
            .await
    }

    pub async fn chat_stream_with_control<F>(
        &mut self,
        input: &str,
        images: &[Option<PastedImage>],
        control: &AgentTurnControl,
        on_event: F,
    ) -> Result<ChatResult>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        self.chat_stream_with_images_inner(input, images, Some(control), on_event)
            .await
    }

    async fn chat_stream_with_images_inner<F>(
        &mut self,
        input: &str,
        images: &[Option<PastedImage>],
        control: Option<&AgentTurnControl>,
        on_event: F,
    ) -> Result<ChatResult>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        self.state.recover_stale_turns()?;
        // 定期归档：对话开始前检查（默认保留 7 天，节流静默）
        crate::cli::maybe_auto_archive(&self.paths, 7);
        self.trim_visible_context()?;
        let prepared = self.prepare_user_input(input, images).await?;
        let input = prepared.content.clone();
        let turn_id = format!(
            "turn_{}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            rand::random::<u64>()
        );
        self.state
            .start_turn_for_mode(&turn_id, &input, std::process::id(), self.mode.key())?;
        let guard = PendingTurnGuard::new(self.state.clone(), turn_id.clone());
        let mut on_event = on_event;
        on_event(AgentEvent::TurnStarted {
            turn_id: turn_id.clone(),
        })?;
        let mut messages = self.chat_messages(&turn_id, &input)?;
        if let Some(last) = messages.last_mut() {
            *last = prepared.message;
        }
        messages.extend(prepared.hints);
        if self.mode != AgentMode::Chat {
            if let Some(association) = self.memory.association(&input)? {
                messages.insert(
                    1,
                    ChatMessage::system(self.memory.format_association(&association)),
                );
            }
        }
        if self.mode != AgentMode::Plan {
            if let Some(reminder) = memes::auto_meme_reminder(&self.config, &input) {
                messages.push(ChatMessage::system(reminder));
            }
        }
        let mut used_tools = Vec::new();
        let mut persisted_tool_reports = Vec::new();
        let mut result = self
            .chat_with_tools(
                &turn_id,
                &mut messages,
                &mut used_tools,
                &mut persisted_tool_reports,
                control,
                &mut on_event,
            )
            .await?;
        for (_, report) in persisted_tool_reports {
            self.state.append_persisted_context(&turn_id, &report)?;
        }
        let token_total = result.usage.as_ref().map(Usage::effective_total_tokens);
        // 剥掉模型输出里的 <think> 思考块：Qwen3 系在 no-think 下也会输出空 think 标签，
        // 直接污染回复（本地 llama.cpp / Ollama 都可能出现）；全局处理，任何后端受益。
        result.content = strip_think_blocks(&result.content);
        // 兜底：模型只输出思考（reasoning）而没有正文时，补一句说明，
        // 保证面板/终端/历史都有可读的回复内容
        if result.content.trim().is_empty() {
            if let Some(reasoning) = result.reasoning.as_deref() {
                if !reasoning.trim().is_empty() {
                    result.content = crate::i18n::text(
                        "(Thinking completed, no additional text reply.)",
                        "（本轮思考完成，没有额外的文字回复。）",
                    )
                    .to_string();
                }
            }
        }
        guard.complete_with_model(
            &result.content,
            result.reasoning.as_deref(),
            result.provider_id.as_deref(),
            result.model.as_deref(),
            token_total,
            result.usage_estimated,
        )?;
        self.memory.process_after_turn(&input, &result.content)?;
        // 自我进化一期：每轮自动标注训练样本（开关 finetune.collect，默认关，
        // 只追加 JSONL 不训练；攒够阈值由外部 MLX 脚本批量微调）
        crate::finetune::record_turn(
            &self.paths,
            &self.config.finetune,
            self.mode,
            &input,
            &result.content,
            &used_tools,
        );
        // 自我成长：用户明确要求记住的方法 → 沉淀为技能（规则匹配，零模型开销）
        if let Some((skill_name, is_new)) =
            crate::learning::maybe_learn(&self.paths, &self.config, &input, &result.content)?
        {
            crate::activity::record(
                &self.paths,
                "learned_skill",
                &serde_json::json!({ "name": skill_name, "created": is_new }),
            );
        }
        if let Some(usage) = result.usage.clone() {
            self.state.add_usage(
                &usage,
                result.provider_id.as_deref().unwrap_or("unknown"),
                result.model.as_deref().unwrap_or("(未标注)"),
            )?;
        }
        // 自动备份移出对话热路径：后台执行，且内部有 30 分钟节流。
        // 一次性 CLI（gqy "问题"）退出前会通过 settle_pending_backup 等它完成。
        // 测试二进制不触发：并行测试会经由进程级 GQY_HOME 误操作其他测试的备份仓库。
        #[cfg(not(test))]
        {
            let backup_paths = self.paths.clone();
            let backup_task = tokio::task::spawn_blocking(move || {
                if let Err(error) = crate::backup::maybe_auto_backup(&backup_paths) {
                    tracing::error!("automatic backup failed: {error:#}");
                    eprintln!(
                        "{}: {error:#}",
                        crate::i18n::text("warning: automatic backup failed", "警告：自动备份失败")
                    );
                }
            });
            self.pending_backup = Some(backup_task);
        }
        Ok(result)
    }

    /// 等待后台自动备份任务结束（一次性 CLI 退出前调用，避免进程先退导致备份丢失）。
    pub async fn settle_pending_backup(&mut self) {
        if let Some(task) = self.pending_backup.take() {
            let _ = task.await;
        }
    }
}
