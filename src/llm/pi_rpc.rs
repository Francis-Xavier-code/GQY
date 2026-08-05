//! pi（https://github.com/earendil-works/pi）RPC 客户端。
//!
//! GQY 通过 `pi --mode rpc` 把「大脑」交给 pi：
//! - pi 进程自己跑 agent 循环与内置工具（read/write/edit/bash/find/grep/ls）；
//! - GQY 把每轮用户消息通过 JSONL 发送给 pi，把 pi 的流式事件翻译成
//!   [`ChatStreamChunk`] 交给 GQY 的渲染层；
//! - 会话状态保存在 pi 进程内（`--no-session`，内存态），GQY 侧的记忆/备份照旧。
//!
//! 进程生命周期规则：
//! - 首次调用时按 messages[0] 的 system prompt（人格）spawn 一个长驻 pi 进程；
//! - 之后 system prompt 不变则复用同一进程（pi 自己维护对话历史）；
//! - system prompt 变化（切人格/子 agent/压缩等）则换新进程，天然隔离。

use super::{
    ChatContent, ChatContentPart, ChatMessage, ChatResult, ChatStreamChunk, ChatStreamKind,
    ToolDefinition,
};
use crate::config::{AppConfig, ProviderConfig};
use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, broadcast};

/// pi RPC 事件环形缓冲容量。工具执行（尤其 bash 大输出）会瞬时产生大量事件，
/// 容量不足时只会丢早于订阅窗口的事件，不影响当前轮（当前轮在订阅后发生）。
const PI_EVENT_BUFFER: usize = 8192;

/// 单轮等待上限（秒）：默认 30 分钟，覆盖 deep research 等长工具等待；
/// 可按需调小/调大：`GQY_PI_TURN_TIMEOUT`
const DEFAULT_TURN_TIMEOUT_SECS: u64 = 1800;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// pi RPC 客户端。内部是一个长驻的 `pi --mode rpc` 子进程 + 事件广播。
#[derive(Clone)]
pub struct PiRpcClient {
    inner: Arc<PiInner>,
}

struct PiInner {
    events: broadcast::Sender<Value>,
    /// 保活 receiver：broadcast 在没有任何 receiver 时 send 会失败
    _keep: broadcast::Receiver<Value>,
    state: Mutex<PiState>,
    paths: crate::paths::GqyPaths,
    /// 子 agent 模式：spawn 的 pi 进程使用过滤后的工具清单（剔除 task/deep_research，
    /// 防止子 agent 自我递归）
    subagent_mode: std::sync::atomic::AtomicBool,
}

struct PiState {
    proc: Option<PiProc>,
    persona: Option<String>,
}

struct PiProc {
    child: Child,
    stdin: ChildStdin,
    /// 人格临时文件：与进程同生命周期，随进程替换/退出自动删除
    _persona_file: tempfile::NamedTempFile,
}

impl Drop for PiProc {
    fn drop(&mut self) {
        // 杀整个进程组（含 pi 的 bash 孙进程），防止 GQY 退出后残留孤儿
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                let _ = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            }
        }
    }
}

impl PiRpcClient {
    pub fn from_config(_config: &AppConfig, paths: &GqyPaths) -> Result<Self> {
        let (tx, keep_rx) = broadcast::channel(PI_EVENT_BUFFER);
        Ok(Self {
            inner: Arc::new(PiInner {
                events: tx,
                _keep: keep_rx,
                state: Mutex::new(PiState {
                    proc: None,
                    persona: None,
                }),
                paths: paths.clone(),
                subagent_mode: std::sync::atomic::AtomicBool::new(false),
            }),
        })
    }

    /// 标记为子 agent 模式：后续 spawn 的 pi 进程不暴露递归性工具
    /// （gqy_task / gqy_deep_research）。
    pub fn with_subagent_mode(self) -> Self {
        self.inner
            .subagent_mode
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// 向当前 pi 进程发送 abort（打断本轮生成）。调用方不需要持有 state 锁。
    pub async fn interrupt(&self) -> Result<()> {
        self.rpc_command("abort", Value::Null, None).await.map(|_| ())
    }

    /// 发送任意 RPC 命令并等待对应 `response` 事件。
    /// `timeout` 为 None 时用 `turn_timeout_secs()`；进程未启动时返回 Ok(None)。
    pub async fn rpc_command(
        &self,
        command: &str,
        extra: Value,
        timeout: Option<std::time::Duration>,
    ) -> Result<Option<Value>> {
        let mut state = self.inner.state.lock().await;
        let Some(proc) = state.proc.as_mut() else {
            return Ok(None);
        };
        let request_id = format!(
            "gqycmd-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let mut payload = match extra {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        payload.insert("type".to_string(), json!(command));
        payload.insert("id".to_string(), json!(request_id));
        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, &Value::Object(payload))?;
        buf.push(b'\n');
        proc.stdin.write_all(&buf).await?;
        proc.stdin.flush().await?;

        let mut rx = self.inner.events.subscribe();
        let deadline = timeout.unwrap_or_else(|| {
            std::time::Duration::from_secs(turn_timeout_secs())
        });
        drop(state);
        let outcome = tokio::time::timeout(deadline, async {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Ok(None),
                };
                if ev.get("type").and_then(Value::as_str) == Some("response")
                    && ev.get("id").and_then(Value::as_str) == Some(request_id.as_str())
                {
                    return Ok(Some(ev));
                }
            }
        })
        .await;
        match outcome {
            Ok(result) => result,
            Err(_) => Ok(None),
        }
    }

    /// 获取当前模型 / 思考级别（进程未启动时返回默认占位）。
    pub async fn pi_state(&self) -> Result<serde_json::Value> {
        let Some(response) = self.rpc_command("get_state", Value::Null, None).await? else {
            return Ok(json!({ "model": null, "thinking_level": null }));
        };
        let state = response.get("data").cloned().unwrap_or(serde_json::json!({}));
        Ok(json!({
            "model": state.get("model").and_then(|m| m.get("id")).cloned().or_else(|| state.get("model").and_then(|m| m.get("name")).cloned()),
            "model_provider": state.get("model").and_then(|m| m.get("provider")).cloned().or_else(|| state.get("model").and_then(|m| m.get("providerId")).cloned()),
            "thinking_level": state.get("thinkingLevel").cloned(),
        }))
    }

    /// 设置模型（仅对已启动的进程生效；未启动则下次对话用 pi 默认模型）。
    pub async fn set_model(&self, model_id: &str) -> Result<()> {
        let _ = self
            .rpc_command("set_model", json!({ "modelId": model_id }), None)
            .await?;
        Ok(())
    }

    /// 设置思考级别：off/minimal/low/medium/high/xhigh/max
    pub async fn set_thinking_level(&self, level: &str) -> Result<()> {
        let _ = self
            .rpc_command("set_thinking_level", json!({ "level": level }), None)
            .await?;
        Ok(())
    }

    /// 可用模型列表（来自 pi 的 get_available_models）。
    pub async fn available_models(&self) -> Result<Vec<serde_json::Value>> {
        let Some(response) = self
            .rpc_command("get_available_models", Value::Null, None)
            .await?
        else {
            return Ok(Vec::new());
        };
        Ok(response
            .get("data")
            .and_then(|data| data.get("models"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// 与 [`super::OpenAiCompatibleClient::chat_stream`] 同签名。
    ///
    /// - `messages[0]`（system）作为人格注入 pi（变化则换进程）；
    /// - 取最后一条 user 消息作为本轮 prompt（含图片）；
    /// - `tools` 参数被忽略：pi 用自己注册的工具（GQY 定制工具经 extension 注入，
    ///   见后续步骤）；
    /// - 返回的 `tool_calls` 恒为空：工具执行发生在 pi 进程内部。
    pub async fn chat_stream<F>(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
        mut on_chunk: F,
    ) -> Result<ChatResult>
    where
        F: FnMut(ChatStreamChunk) -> Result<()>,
    {
        if std::env::var_os("GQY_PI_DEBUG").is_some() {
            eprintln!("[pi-rpc] chat_stream called, messages={}", messages.len());
        }
        let (persona, prompt, images) = extract_turn(&messages)?;
        if prompt.trim().is_empty() && images.is_empty() {
            bail!("pi rpc: no user message found in request");
        }

        let request_id = format!(
            "gqy-{:x}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );

        // 1) 确保进程存在且人格匹配
        {
            let mut state = self.inner.state.lock().await;
            let respawn = match &state.proc {
                None => true,
                Some(_) => state.persona.as_deref() != Some(persona.as_str()),
            };
            if respawn {
                if let Some(mut old) = state.proc.take() {
                    let _ = old.child.kill().await;
                    let _ = old.child.wait().await;
                }
                state.proc = Some(self.spawn_proc(&persona).await?);
                state.persona = Some(persona);
            }

            // 2) 先订阅，再写 prompt，避免漏事件
            let mut rx = self.inner.events.subscribe();
            let proc = state.proc.as_mut().expect("pi proc must exist");
            let cmd = json!({
                "id": request_id,
                "type": "prompt",
                "message": prompt,
                "images": images,
            });
            let mut buf = Vec::new();
            serde_json::to_writer(&mut buf, &cmd)?;
            buf.push(b'\n');
            proc.stdin.write_all(&buf).await?;
            proc.stdin.flush().await?;
            drop(state);

            // 3) 读取事件直到本轮结束
            let mut content = String::new();
            let mut reasoning = String::new();
            let mut saw_response = false;
            let mut provider_id: Option<String> = None;
            let mut model: Option<String> = None;
            // pi 每个内部 LLM 调用都会在 assistant/toolResult 消息上带 usage，逐条累加
            let mut usage = super::Usage::default();
            let mut usage_seen = false;
            // 聚合思考：跨内部 LLM 调用的 thinking 合并进同一个前端思考块
            let mut thinking_open = false;
            let mut thinking_emitted = false;
            let timeout = std::time::Duration::from_secs(turn_timeout_secs());
            let outcome = tokio::time::timeout(timeout, async {
                loop {
                    let ev = match rx.recv().await {
                        Ok(ev) => ev,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => {
                            bail!("pi rpc: event stream closed (pi process exited?)")
                        }
                    };
                    let Some(kind) = ev.get("type").and_then(Value::as_str) else {
                        continue;
                    };
                    if std::env::var_os("GQY_PI_DEBUG").is_some() {
                        eprintln!("[pi-rpc] event: {ev}");
                    }
                    if !saw_response {
                        if kind == "response"
                            && ev.get("command").and_then(Value::as_str) == Some("prompt")
                            && ev.get("id").and_then(Value::as_str) == Some(request_id.as_str())
                        {
                            saw_response = true;
                            if ev.get("success").and_then(Value::as_bool) != Some(true) {
                                let err = ev
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown error");
                                bail!("pi rejected prompt: {err}");
                            }
                        }
                        continue; // 丢弃上一轮的残留事件
                    }
                    match kind {
                        "message_update" => {
                            // 记录 pi 实际使用的模型/供应商（用于用量归因与界面显示）
                            if provider_id.is_none() {
                                provider_id = ev
                                    .get("message")
                                    .and_then(|m| m.get("provider"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                            if model.is_none() {
                                model = ev
                                    .get("message")
                                    .and_then(|m| m.get("model"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                            if let Some(aem) = ev.get("assistantMessageEvent") {
                                match aem.get("type").and_then(Value::as_str) {
                                    Some("text_delta") => {
                                        let delta = aem
                                            .get("delta")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default();
                                        if !delta.is_empty() {
                                            content.push_str(delta);
                                            on_chunk(ChatStreamChunk {
                                                kind: ChatStreamKind::Content,
                                                text: delta.to_string(),
                                            })?;
                                        }
                                    }
                                    Some("thinking_delta") => {
                                        let delta = aem
                                            .get("delta")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default();
                                        if !delta.is_empty() {
                                            if !thinking_open {
                                                thinking_open = true;
                                                thinking_emitted = true;
                                                on_chunk(ChatStreamChunk {
                                                    kind: ChatStreamKind::ReasoningPartStart,
                                                    text: String::new(),
                                                })?;
                                            }
                                            reasoning.push_str(delta);
                                            on_chunk(ChatStreamChunk {
                                                kind: ChatStreamKind::Reasoning,
                                                text: delta.to_string(),
                                            })?;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "tool_execution_start" => {
                            let name = ev
                                .get("toolName")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let args = ev.get("args").cloned().unwrap_or(serde_json::json!({}));
                            on_chunk(ChatStreamChunk {
                                kind: ChatStreamKind::ToolProgress,
                                text: serde_json::json!({
                                    "name": name,
                                    "args": args,
                                })
                                .to_string(),
                            })?;
                        }
                        "tool_execution_update" => {
                            let name = ev
                                .get("toolName")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let output = ev
                                .get("partialResult")
                                .and_then(|r| r.get("content"))
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("")
                                })
                                .unwrap_or_default();
                            if !output.is_empty() {
                                on_chunk(ChatStreamChunk {
                                    kind: ChatStreamKind::ToolProgress,
                                    text: serde_json::json!({
                                        "name": name,
                                        "output": output,
                                    })
                                    .to_string(),
                                })?;
                            }
                        }
                        "tool_execution_end" => {
                            let name = ev
                                .get("toolName")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let ok = ev
                                .get("isError")
                                .and_then(Value::as_bool)
                                .map(|is_error| !is_error)
                                .unwrap_or(true);
                            let output = ev
                                .get("result")
                                .and_then(|r| r.get("content"))
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("")
                                })
                                .unwrap_or_default();
                            on_chunk(ChatStreamChunk {
                                kind: ChatStreamKind::ToolResult,
                                text: serde_json::json!({
                                    "name": name,
                                    "ok": ok,
                                    "output": output,
                                })
                                .to_string(),
                            })?;
                        }
                        "message_end" => {
                            // 汇总每条消息上报的 token 用量（assistant 与嵌套工具 LLM 调用）
                            if let Some(message) = ev.get("message") {
                                if let Some(usage_obj) = message.get("usage") {
                                    let input = usage_obj
                                        .get("input")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0);
                                    let output = usage_obj
                                        .get("output")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0);
                                    let cache_read = usage_obj
                                        .get("cacheRead")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0);
                                    let cache_write = usage_obj
                                        .get("cacheWrite")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0);
                                    if input > 0 || output > 0 || cache_read > 0 {
                                        usage_seen = true;
                                        usage.prompt_tokens = usage.prompt_tokens.saturating_add(input);
                                        usage.completion_tokens =
                                            usage.completion_tokens.saturating_add(output);
                                        usage.total_tokens = usage.total_tokens.saturating_add(
                                            input.saturating_add(output),
                                        );
                                        usage.cache_read_input_tokens = usage
                                            .cache_read_input_tokens
                                            .unwrap_or(0)
                                            .saturating_add(cache_read)
                                            .into();
                                        usage.cache_creation_input_tokens = usage
                                            .cache_creation_input_tokens
                                            .unwrap_or(0)
                                            .saturating_add(cache_write)
                                            .into();
                                    }
                                }
                            }
                        }
                        "agent_settled" => {
                            if thinking_open {
                                thinking_open = false;
                                on_chunk(ChatStreamChunk {
                                    kind: ChatStreamKind::ReasoningPartEnd,
                                    text: String::new(),
                                })?;
                            }
                            break;
                        }
                        _ => {}
                    }
                }
                Ok::<(), anyhow::Error>(())
            })
            .await;

            match outcome {
                Ok(result) => result?,
                Err(_elapsed) => {
                    let _ = self.interrupt().await;
                    bail!(
                        "pi rpc: turn timed out after {timeout:?} (set GQY_PI_TURN_TIMEOUT to adjust)"
                    )
                }
            }

            Ok(ChatResult {
                content,
                reasoning: if reasoning.trim().is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
                usage: if usage_seen { Some(usage) } else { None },
                usage_estimated: !usage_seen,
                tool_calls: Vec::new(),
                provider_id: provider_id.or_else(|| Some("pi".to_string())),
                model,
            })
        }
    }

    async fn spawn_proc(&self, persona: &str) -> Result<PiProc> {
        let pi_bin = std::env::var("GQY_PI_BIN").unwrap_or_else(|_| "pi".to_string());
        let persona_file = write_persona_temp(persona)?;
        let persona_path = persona_file.path().to_path_buf();
        let cwd = std::env::current_dir().context("failed to get current dir")?;
        let extension_path = bridge_extension_path(&self.inner.paths);
        let subagent = self
            .inner
            .subagent_mode
            .load(std::sync::atomic::Ordering::Relaxed);
        if std::env::var_os("GQY_PI_DEBUG").is_some() {
            eprintln!(
                "[pi-rpc] spawn: tool_api={:?}, extension={:?}, subagent={subagent}, persona_len={}",
                std::env::var_os("GQY_PI_TOOL_API"),
                extension_path,
                persona.len()
            );
        }

        let mut cmd = Command::new(&pi_bin);
        // pi 及其 bash 孙进程同属一个进程组，退出时统一清理
        cmd.process_group(0);
        if let Some(extension) = extension_path {
            // pi 底座模式：注入 GQY 工具桥接扩展（GQY_PI_TOOL_API 已由主进程设置）
            cmd.arg("--extension").arg(&extension);
        }
        if subagent {
            // 子 agent：仅对该子进程覆盖工具清单（剔除递归性工具）
            if let Some(filtered) = subagent_tool_list_path(&self.inner.paths)? {
                cmd.env("GQY_PI_TOOL_LIST", &filtered);
            }
        }
        cmd.arg("--mode")
            .arg("rpc")
            .arg("--no-session")
            .arg("--no-context-files")
            .arg("--no-extensions")
            .arg("--no-skills")
            .arg("--no-prompt-templates")
            .arg("--no-themes")
            .arg("--name")
            .arg("gqy")
            .arg("--append-system-prompt")
            .arg(&persona_path)            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `{pi_bin} --mode rpc` (is pi installed?)"))?;
        let stdin = child.stdin.take().context("pi process stdin unavailable")?;
        let stdout = child.stdout.take().context("pi process stdout unavailable")?;

        let reader_tx = self.inner.events.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                            let _ = reader_tx.send(value);
                        }
                    }
                }
            }
        });

        Ok(PiProc {
            child,
            stdin,
            _persona_file: persona_file,
        })
    }
}

fn turn_timeout_secs() -> u64 {
    std::env::var("GQY_PI_TURN_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_TURN_TIMEOUT_SECS)
}

/// 解析 pi 工具桥接扩展路径：
/// 环境变量 `GQY_PI_EXTENSION` 显式指定 → 随包脚本 `src/scripts/pi-bridge.ts`。
/// 仅当 `GQY_PI_TOOL_API` 已设置（pi 底座模式）才加载扩展。
fn bridge_extension_path(paths: &crate::paths::GqyPaths) -> Option<std::path::PathBuf> {
    if std::env::var_os("GQY_PI_TOOL_API").is_none() {
        return None;
    }
    if let Some(override_path) = std::env::var_os("GQY_PI_EXTENSION") {
        let path = std::path::PathBuf::from(override_path);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidate = paths.system_scripts_dir.join("pi-bridge.ts");
    candidate.is_file().then_some(candidate)
}

/// 提取 (人格, 本轮用户输入, 图片列表)
fn extract_turn(messages: &[ChatMessage]) -> Result<(String, String, Vec<Value>)> {
    let persona = messages
        .iter()
        .find(|m| m.role == "system")
        .and_then(|m| match &m.content {
            Some(ChatContent::Text(text)) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();

    // pi 进程看不到 GQY agent 循环注入的联想记忆/摘要系统消息，
    // 这里把 persona 之外的系统消息拼进本轮 prompt，让 pi 也能利用 GQY 的记忆。
    let extra_context = messages
        .iter()
        .filter(|m| m.role == "system")
        .skip(1)
        .filter_map(|m| match &m.content {
            Some(ChatContent::Text(text)) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let Some(user) = messages.iter().rev().find(|m| m.role == "user") else {
        bail!("pi rpc: no user message in request");
    };

    let mut prompt = String::new();
    let mut images = Vec::new();
    match &user.content {
        Some(ChatContent::Text(text)) => prompt.push_str(text),
        Some(ChatContent::Parts(parts)) => {
            for part in parts {
                match part {
                    ChatContentPart::Text { text } => prompt.push_str(text),
                    ChatContentPart::ImageUrl { image_url } => {
                        if let Some(image) = image_to_rpc(&image_url.url)? {
                            images.push(image);
                        }
                    }
                }
            }
        }
        None => {}
    }
    if !extra_context.is_empty() {
        prompt = format!(
            "<gqy-context>\n{extra_context}\n</gqy-context>\n\n用户输入：{prompt}"
        );
    }
    Ok((persona, prompt, images))
}

/// 把 GQY 的图片引用（data URI 或本地文件路径）转成 pi RPC 的 images 项。
fn image_to_rpc(url: &str) -> Result<Option<Value>> {
    if let Some(rest) = url.strip_prefix("data:") {
        // data:<mime>;base64,<data>
        if let Some((header, data)) = rest.split_once(',') {
            let mime = header.split(';').next().unwrap_or("image/png").to_string();
            return Ok(Some(json!({
                "type": "image",
                "data": data,
                "mimeType": mime,
            })));
        }
        return Ok(None);
    }
    if let Some(path) = url.strip_prefix("file://") {
        let path = Path::new(path);
        if let Some(image) = encode_image_file(path)? {
            return Ok(Some(image));
        }
        return Ok(None);
    }
    let path = Path::new(url);
    if path.is_file() {
        if let Some(image) = encode_image_file(path)? {
            return Ok(Some(image));
        }
        return Ok(None);
    }
    // http(s) 或未知形式：跳过图片，只保留文本
    Ok(None)
}

fn encode_image_file(path: &Path) -> Result<Option<Value>> {
    let data = std::fs::read(path).with_context(|| {
        format!("pi rpc: failed to read image {}", path.display())
    })?;
    let mime = image_mime(path);
    Ok(Some(json!({
        "type": "image",
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
    })))
}

fn image_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "bmp" => "image/bmp".to_string(),
        _ => "image/png".to_string(),
    }
}

/// 生成子 agent 用的过滤工具清单：读当前 `GQY_PI_TOOL_LIST`，剔除
/// 递归性工具（task / deep_research），写入 cache/pi-bridge-tools-subagent.json。
fn subagent_tool_list_path(paths: &crate::paths::GqyPaths) -> Result<Option<std::path::PathBuf>> {
    let Some(list_file) = std::env::var_os("GQY_PI_TOOL_LIST") else {
        return Ok(None);
    };
    let list_path = std::path::PathBuf::from(list_file);
    let Ok(raw) = std::fs::read_to_string(&list_path) else {
        return Ok(None);
    };
    let Ok(mut tools) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) else {
        return Ok(None);
    };
    let before = tools.len();
    tools.retain(|tool| {
        !matches!(
            tool.get("name").and_then(serde_json::Value::as_str),
            Some(
                "task"
                    | "deep_research"
                    | "spawn_agent"
                    | "talk_to_agent"
                    | "list_agents"
                    | "kill_agent"
                    | "parallel_agents"
            )
        )
    });
    if tools.len() == before {
        return Ok(None);
    }
    let cache = paths.cache_dir.join("pi-bridge-tools-subagent.json");
    std::fs::create_dir_all(
        cache
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )?;
    std::fs::write(&cache, serde_json::to_string_pretty(&tools)?)?;
    Ok(Some(cache))
}

/// 把人格写到临时文件，避免超长 argv（pi 的 --append-system-prompt 支持文件路径）。
fn write_persona_temp(persona: &str) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("gqy-pi-persona-")
        .suffix(".md")
        .tempfile()
        .context("failed to create persona temp file")?;
    file.write_all(persona.as_bytes())?;
    file.flush()?;
    Ok(file)
}

impl ProviderConfig {
    /// 是否为 pi 底座 provider（protocol == "pi"，或 id == "pi"）。
    pub fn is_pi(&self) -> bool {
        self.protocol.trim().eq_ignore_ascii_case("pi")
            || self.id.trim().eq_ignore_ascii_case("pi")
    }
}
