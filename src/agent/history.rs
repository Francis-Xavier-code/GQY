//! Agent conversation history management.
//!
//! This module handles conversation history construction,
//! including message formatting and context assembly.

use super::*;

impl Agent {
    pub(super) fn chat_messages(
        &self,
        current_turn_id: &str,
        current_input: &str,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages = vec![ChatMessage::system(self.system_prompt.clone())];
        if let Some(summary) = self.state.load_last_summary()? {
            messages.push(ChatMessage::system(format!(
                "<conversation-summary>\n{}\n</conversation-summary>",
                summary.assistant_content
            )));
        }
        let turns = self
            .state
            .load_visible_turns_for_mode_excluding(
                self.mode.key(),
                current_turn_id,
                self.chat_history_limit(),
            )?;
        for turn in &turns {
            if turn.is_summary {
                continue;
            }
            messages.push(ChatMessage::plain("user", &turn.user_content));
            for exchange in &turn.question_exchanges {
                messages.push(ChatMessage::plain(
                    "assistant",
                    crate::question::assistant_exchange_text(exchange),
                ));
                messages.push(ChatMessage::plain(
                    "user",
                    crate::question::user_exchange_text(exchange),
                ));
            }
            for followup in &turn.followups {
                if let Some(content) = followup_assistant_replay_content(followup) {
                    messages.push(ChatMessage::plain("assistant", content));
                }
                messages.push(self.followup_user_message(followup));
            }
            if let Some(content) = assistant_replay_content(turn) {
                messages.push(ChatMessage::plain("assistant", content));
            }
            if !turn.tool_reports.is_empty() {
                messages.push(ChatMessage::system(private_tool_memory(&turn.tool_reports)));
            }
        }
        messages.push(ChatMessage::system(runtime_context(self.mode)));
        // 挥发性变量（时间戳）放在 userPrompt 末尾，避免破坏前缀缓存
        let user_input_with_timestamp = format!("{}\n{}", current_input, user_prompt_timestamp());
        messages.push(ChatMessage::plain("user", user_input_with_timestamp));
        Ok(messages)
    }

}
