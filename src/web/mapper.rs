//! Agent event → SSE payload mapper for live runs.
use crate::agent::AgentEvent;
use crate::llm::{ChatStreamKind, Usage};
use crate::state::{ImageAsset, StateStore};
use crate::tools::{self, CommandOutputStream};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::events::{EventHub, QuestionBroker};
use super::types::{mode_name, real_tool_name, SafeImageAsset};

pub(crate) struct RunEventMapper {
    pub(crate) run_id: String,
    pub(crate) events: EventHub,
    pub(crate) questions: QuestionBroker,
    pub(crate) state_store: StateStore,
    pub(crate) turn_id: Option<String>,
    pub(crate) tool_counter: u64,
    pub(crate) active_tool: Option<ActiveTool>,
}

pub(crate) struct ActiveTool {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) event_name: String,
}

impl RunEventMapper {
    pub(crate) fn new(
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

    pub(crate) fn publish(&self, kind: &str, data: Value) {
        self.events.publish(kind, data);
    }

    pub(crate) fn next_tool(&mut self, event_name: String) -> ActiveTool {
        self.tool_counter = self.tool_counter.saturating_add(1);
        ActiveTool {
            id: format!("{}_tool_{}", self.run_id, self.tool_counter),
            name: real_tool_name(&event_name).to_string(),
            event_name,
        }
    }

    pub(crate) fn handle(&mut self, event: AgentEvent) {
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

    pub(crate) fn tool_identity(&self, fallback: &str) -> (String, String) {
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

