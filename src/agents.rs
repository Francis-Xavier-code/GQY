//! 自主 agent 集群（Kimi 式）。
//!
//! 模型在对话中可以自主创建命名 agent（角色/工具）、点名对话、并行派活、
//! 列表与销毁。每个 agent 是一个独立 LLM 会话：
//! - pi 模式下：独立 pi 进程（自定义系统提示词 → 独立进程，多轮记忆；
//!   工具由 pi 进程 + 过滤后的 bridge 清单提供）；
//! - 直连模式下：OpenAI 兼容客户端 + 实例内消息历史 + GQY 工具循环
//!  （多轮记忆，可调 web/知识库等，递归工具已剔除）。
//!
//! agent 定义持久化在 `GQY_HOME/data/agents/agents.json`，重启后定义仍在，
//! 进程按需懒启动。
//!
//! 递归防护：agent 自己的进程 / 直连工具集使用「子 agent 过滤清单」
//! （不含 spawn_agent / talk_to_agent / task / deep_research 等），
//! 防止 agent 再无限创建 agent。

use crate::config::AppConfig;
use crate::llm::{ChatMessage, ChatStreamChunk, ChatStreamKind, LlmClient};
use crate::paths::GqyPaths;
use crate::tools::{ToolProgress, ToolRegistry, ToolSpec};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::Duration;

const AGENTS_FILE: &str = "agents.json";
const MAX_AGENTS: usize = 16;
const MAX_HISTORY_TURNS: usize = 20;
/// 直连模式下每个 talk 允许的工具步数上限。
const MAX_TOOL_STEPS: usize = 40;
/// 直连模式下单次工具调用超时。
const TOOL_TIMEOUT_SECS: u64 = 120;

/// 子 agent / 集群 agent 不可再调用的递归性工具。
const RECURSIVE_TOOLS: &[&str] = &[
    "spawn_agent",
    "talk_to_agent",
    "list_agents",
    "kill_agent",
    "parallel_agents",
    "task",
    "task_agent",
    "deep_research",
];

/// 全局 agent 管理器（进程级单例，工具与桥共用）。
static AGENTS: OnceLock<ArcAgentManager> = OnceLock::new();

type ArcAgentManager = std::sync::Arc<AgentManager>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct AgentDef {
    name: String,
    role: String,
    created_at: String,
}

struct AgentInstance {
    def: AgentDef,
    client: LlmClient,
    history: Vec<ChatMessage>,
    /// 同一 agent 的 talk 串行化，避免并行写历史互相覆盖。
    talk_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

pub struct AgentManager {
    paths: GqyPaths,
    agents: RwLock<HashMap<String, AgentInstance>>,
    /// 直连模式 tool loop 用的工具集（已在注册时快照，不含集群工具本身）。
    tools: ToolRegistry,
}

fn manager(paths: &GqyPaths, tools: ToolRegistry) -> Result<&'static ArcAgentManager> {
    if let Some(existing) = AGENTS.get() {
        return Ok(existing);
    }
    let loaded = std::sync::Arc::new(AgentManager::load(paths, tools)?);
    match AGENTS.set(loaded) {
        Ok(()) => Ok(AGENTS.get().expect("just set")),
        Err(_) => Ok(AGENTS.get().expect("set by another thread")),
    }
}

fn global_manager() -> Result<&'static ArcAgentManager> {
    AGENTS.get().context("agent manager not initialized")
}

#[derive(Debug, PartialEq, Eq)]
enum EnsureOutcome {
    Created,
    Updated,
    Unchanged,
}

impl AgentManager {
    fn defs_path(paths: &GqyPaths) -> PathBuf {
        paths.data_dir.join("agents").join(AGENTS_FILE)
    }

    fn load(paths: &GqyPaths, tools: ToolRegistry) -> Result<Self> {
        let manager = Self {
            paths: paths.clone(),
            agents: RwLock::new(HashMap::new()),
            tools,
        };
        let defs_path = Self::defs_path(paths);
        if let Ok(raw) = std::fs::read_to_string(&defs_path) {
            if let Ok(defs) = serde_json::from_str::<Vec<AgentDef>>(&raw) {
                for def in defs {
                    let client = Self::make_client(paths);
                    manager.agents.write().unwrap().insert(
                        def.name.clone(),
                        AgentInstance {
                            def,
                            client,
                            history: Vec::new(),
                            talk_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                        },
                    );
                }
            }
        }
        Ok(manager)
    }

    fn persist(&self) -> Result<()> {
        let defs = self
            .agents
            .read()
            .unwrap()
            .values()
            .map(|instance| instance.def.clone())
            .collect::<Vec<_>>();
        let path = Self::defs_path(&self.paths);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(&defs)?)?;
        Ok(())
    }

    fn make_client(paths: &GqyPaths) -> LlmClient {
        // 直连模式直接构造；pi 模式用 from_config（进程按 persona 懒启动）。
        // for_subagent_output(true)：pi 用过滤工具清单；OpenAI 保留完整工具输出。
        match LlmClient::from_config(&AppConfig::load_or_default(paths).unwrap_or_default(), paths)
        {
            Ok(client) => client,
            Err(err) => {
                tracing::error!("agent client construction failed: {err:#}");
                // 回退到默认 client，tool call 会报错而非 panic
                LlmClient::OpenAi(
                    crate::llm::OpenAiCompatibleClient::from_config(&AppConfig::default(), paths)
                        .unwrap_or_else(|e| unreachable!("default config should always work: {e}")),
                )
            }
        }
        .for_subagent_output(true)
    }

    fn agent_system_prompt(role: &str) -> String {
        format!(
            "你是顾清影创建的专属子代理，负责完成交给你的具体任务。

你的角色设定：
{role}

工作守则（必须遵守）：
1. 只输出真实、可核查的信息；绝不编造事实、数据、引用或来源。
2. 涉及事实/数据/外部信息时，优先调用工具核实（如 web_search、web_fetch、search_knowledge_base），不要凭记忆猜测。
3. 无法核实或不确定的内容，明确标注「不确定」或「未能核实」，不要给出貌似权威的假答案。
4. 引用外部信息时说明来源；没有来源支撑的结论不要强加。
5. 输出简洁、直接、可用；任务完成即给出结论，不要过度寒暄。
6. 你不能创建或管理其他 agent，也不能启动 task/deep_research；专注完成本次派活。"
        )
    }

    /// 创建或更新命名 agent。同名且 role 变化时更新角色、重建 client、清空历史。
    fn ensure(&self, name: &str, role: &str) -> Result<EnsureOutcome> {
        {
            let agents = self.agents.read().unwrap();
            if let Some(existing) = agents.get(name) {
                if existing.def.role == role {
                    return Ok(EnsureOutcome::Unchanged);
                }
            } else if agents.len() >= MAX_AGENTS {
                bail!("agent 数量已达上限 {MAX_AGENTS}，先 kill 一些再创建");
            }
        }

        let mut agents = self.agents.write().unwrap();
        if let Some(existing) = agents.get_mut(name) {
            if existing.def.role == role {
                return Ok(EnsureOutcome::Unchanged);
            }
            existing.def.role = role.to_string();
            existing.client = Self::make_client(&self.paths);
            existing.history.clear();
            drop(agents);
            self.persist()?;
            return Ok(EnsureOutcome::Updated);
        }
        if agents.len() >= MAX_AGENTS {
            bail!("agent 数量已达上限 {MAX_AGENTS}，先 kill 一些再创建");
        }
        agents.insert(
            name.to_string(),
            AgentInstance {
                def: AgentDef {
                    name: name.to_string(),
                    role: role.to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
                client: Self::make_client(&self.paths),
                history: Vec::new(),
                talk_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            },
        );
        drop(agents);
        self.persist()?;
        Ok(EnsureOutcome::Created)
    }

    fn exists(&self, name: &str) -> bool {
        self.agents.read().unwrap().contains_key(name)
    }

    fn list(&self) -> Vec<Value> {
        self.agents
            .read()
            .unwrap()
            .values()
            .map(|instance| {
                json!({
                    "name": instance.def.name,
                    "role": instance.def.role,
                    "created_at": instance.def.created_at,
                    "turns": instance.history.len() / 2,
                })
            })
            .collect()
    }

    fn remove(&self, name: &str) -> Result<bool> {
        let removed = self.agents.write().unwrap().remove(name).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    async fn talk(
        &self,
        name: &str,
        message: &str,
        progress: &ToolProgress,
    ) -> Result<String> {
        let talk_lock = {
            let agents = self.agents.read().unwrap();
            let Some(instance) = agents.get(name) else {
                bail!("agent 不存在：{name}（先用 spawn_agent 创建）");
            };
            instance.talk_lock.clone()
        };
        let _guard = talk_lock.lock().await;

        let (client, role, mut history) = {
            let agents = self.agents.read().unwrap();
            let Some(instance) = agents.get(name) else {
                bail!("agent 不存在：{name}（先用 spawn_agent 创建）");
            };
            (
                instance.client.clone(),
                instance.def.role.clone(),
                instance.history.clone(),
            )
        };

        history.push(ChatMessage::plain("user", message));
        if history.len() > MAX_HISTORY_TURNS * 2 {
            let keep = MAX_HISTORY_TURNS * 2;
            history = history.split_off(history.len() - keep);
        }
        let mut request = vec![ChatMessage::system(Self::agent_system_prompt(&role))];
        request.extend(history.iter().cloned());

        let reply = if client.is_pi() {
            // pi：工具在子进程内执行，GQY 侧不传 tools；多轮记忆靠 pi 进程会话。
            self.stream_plain(&client, name, request, progress).await?
        } else {
            // 直连：带 tool loop，使 agent 能真正核实信息。
            self.chat_with_tools(&client, name, request, progress)
                .await?
        };

        if let Some(instance) = self.agents.write().unwrap().get_mut(name) {
            instance
                .history
                .push(ChatMessage::plain("user", message.to_string()));
            instance
                .history
                .push(ChatMessage::plain("assistant", reply.clone()));
            if instance.history.len() > MAX_HISTORY_TURNS * 2 {
                let keep = MAX_HISTORY_TURNS * 2;
                let remove = instance.history.len() - keep;
                instance.history.drain(..remove);
            }
        }
        Ok(reply)
    }

    async fn stream_plain(
        &self,
        client: &LlmClient,
        name: &str,
        request: Vec<ChatMessage>,
        progress: &ToolProgress,
    ) -> Result<String> {
        let agent_name = name.to_string();
        let progress_for_stream = progress.clone();
        let result = client
            .chat_stream(request, Vec::new(), move |chunk| {
                report_stream_chunk(&agent_name, &chunk, &progress_for_stream);
                Ok(())
            })
            .await?;
        Ok(result.content)
    }

    async fn chat_with_tools(
        &self,
        client: &LlmClient,
        name: &str,
        mut messages: Vec<ChatMessage>,
        progress: &ToolProgress,
    ) -> Result<String> {
        let definitions = self.tools.definitions_except(RECURSIVE_TOOLS);
        let mut steps = 0usize;

        loop {
            if steps >= MAX_TOOL_STEPS {
                messages.push(ChatMessage::plain(
                    "user",
                    crate::tools::subagent_runner_finalization_prompt(),
                ));
                let agent_name = name.to_string();
                let progress_for_stream = progress.clone();
                let result = client
                    .chat_stream(messages, Vec::new(), move |chunk| {
                        report_stream_chunk(&agent_name, &chunk, &progress_for_stream);
                        Ok(())
                    })
                    .await?;
                return Ok(result.content);
            }

            let agent_name = name.to_string();
            let progress_for_stream = progress.clone();
            let result = client
                .chat_stream(messages.clone(), definitions.clone(), move |chunk| {
                    report_stream_chunk(&agent_name, &chunk, &progress_for_stream);
                    Ok(())
                })
                .await?;

            if result.tool_calls.is_empty() {
                return Ok(result.content);
            }

            messages.push(ChatMessage::assistant(
                result.content.clone(),
                Some(result.tool_calls.clone()),
            ));

            for call in result.tool_calls {
                if steps >= MAX_TOOL_STEPS {
                    messages.push(ChatMessage::tool(
                        call.id,
                        "tool budget reached for this agent session",
                    ));
                    continue;
                }
                steps += 1;
                let tool_name = call.function.name.clone();
                if RECURSIVE_TOOLS.iter().any(|n| *n == tool_name) {
                    messages.push(ChatMessage::tool(
                        call.id,
                        format!("tool `{tool_name}` is not available to sub-agents"),
                    ));
                    continue;
                }

                progress.report(format!("🔧 {name} 调用 {tool_name}…"));
                let (output, ok) = match tokio::time::timeout(
                    Duration::from_secs(TOOL_TIMEOUT_SECS),
                    self.tools.call_with_progress(
                        &tool_name,
                        &call.function.arguments,
                        progress,
                    ),
                )
                .await
                {
                    Ok(Ok(output)) => (output, true),
                    Ok(Err(err)) => (format!("tool error: {err}"), false),
                    Err(_) => (
                        format!("tool error: {tool_name} timed out after {TOOL_TIMEOUT_SECS}s"),
                        false,
                    ),
                };
                progress.report(format!(
                    "🔧 {name} {tool_name} {}",
                    if ok { "ok" } else { "err" }
                ));
                messages.push(ChatMessage::tool(call.id, output));
            }
        }
    }
}

fn report_stream_chunk(agent_name: &str, chunk: &ChatStreamChunk, progress: &ToolProgress) {
    let prefix = match chunk.kind {
        ChatStreamKind::Reasoning => {
            if chunk.text.trim().is_empty() {
                return;
            }
            format!("🧠 {agent_name} 思考：{}", chunk.text)
        }
        ChatStreamKind::Content => {
            if chunk.text.trim().is_empty() {
                return;
            }
            format!("✍️ {agent_name}：{}", chunk.text)
        }
        _ => return,
    };
    progress.report(prefix);
}

async fn spawn_agent(args: Value) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let role = required_str(&args, "role")?.to_string();
    if !is_valid_agent_name(&name) {
        bail!("agent 名字只能包含字母数字下划线，且以字母开头：{name}");
    }
    let manager = global_manager()?;
    let outcome = manager.ensure(&name, &role)?;
    let message = match outcome {
        EnsureOutcome::Created => {
            format!("agent「{name}」已就绪（角色：{role}）。对它说 talk_to_agent(name=\"{name}\", message=...) 即可派活。")
        }
        EnsureOutcome::Updated => {
            format!("agent「{name}」角色已更新为：{role}（历史已清空）。对它说 talk_to_agent(name=\"{name}\", message=...) 即可派活。")
        }
        EnsureOutcome::Unchanged => {
            format!("agent「{name}」已存在（角色未变）。对它说 talk_to_agent(name=\"{name}\", message=...) 即可派活。")
        }
    };
    Ok(json!({
        "ok": true,
        "message": message,
        "name": name,
        "outcome": match outcome {
            EnsureOutcome::Created => "created",
            EnsureOutcome::Updated => "updated",
            EnsureOutcome::Unchanged => "unchanged",
        },
    })
    .to_string())
}

async fn talk_to_agent(args: Value, progress: &ToolProgress) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let message = required_str(&args, "message")?.to_string();
    let manager = global_manager()?;
    progress.report(format!("向 agent「{name}」派活：{message}"));
    let reply = manager.talk(&name, &message, progress).await?;
    Ok(json!({
        "ok": true,
        "name": name,
        "reply": reply,
    })
    .to_string())
}

async fn list_agents(_args: Value) -> Result<String> {
    let manager = global_manager()?;
    Ok(json!({
        "ok": true,
        "agents": manager.list(),
    })
    .to_string())
}

async fn kill_agent(args: Value) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let manager = global_manager()?;
    let removed = manager.remove(&name)?;
    Ok(json!({
        "ok": true,
        "removed": removed,
        "message": if removed {
            format!("agent「{name}」已销毁")
        } else {
            format!("没有找到 agent「{name}」")
        },
    })
    .to_string())
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("缺少必填参数：{key}"))
}

fn is_valid_agent_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 32
}

/// 并发执行多个 agent 任务（不同 agent 真正并行；同一 agent 内部仍串行）。
async fn parallel_agents(args: Value, progress: &ToolProgress) -> Result<String> {
    let tasks = args["tasks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'tasks' 数组是必填的"))?;

    if tasks.is_empty() {
        bail!("至少需要一个任务");
    }
    if tasks.len() > MAX_AGENTS {
        bail!("单次并行任务数不能超过 {MAX_AGENTS}");
    }

    let manager = global_manager()?;

    let mut handles = Vec::new();
    for task in tasks {
        let name = task["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("每个任务需要 'name' 字段"))?
            .to_string();
        let message = task["message"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("每个任务需要 'message' 字段"))?
            .to_string();

        if !manager.exists(&name) {
            bail!("agent '{name}' 不存在，请先用 spawn_agent 创建");
        }

        let progress_clone = progress.clone();
        let manager_ref = std::sync::Arc::clone(manager);
        handles.push(tokio::spawn(async move {
            let result = manager_ref.talk(&name, &message, &progress_clone).await;
            (name, result)
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((name, Ok(reply))) => {
                results.push(json!({
                    "name": name,
                    "success": true,
                    "reply": reply
                }));
            }
            Ok((name, Err(e))) => {
                results.push(json!({
                    "name": name,
                    "success": false,
                    "error": e.to_string()
                }));
            }
            Err(e) => {
                results.push(json!({
                    "name": "unknown",
                    "success": false,
                    "error": format!("Task panicked: {e}")
                }));
            }
        }
    }

    Ok(json!({
        "ok": true,
        "tasks_completed": results.len(),
        "results": results
    })
    .to_string())
}

/// 注册 agent 集群工具（spawn_agent / talk_to_agent / list_agents / kill_agent / parallel_agents）。
///
/// `agent_tools`：直连模式子 agent 可调用的工具快照（通常为注册集群工具之前的 registry clone）。
pub fn register(registry: &mut ToolRegistry, paths: GqyPaths, agent_tools: ToolRegistry) {
    let _ = manager(&paths, agent_tools);
    registry.register(
        ToolSpec::new(
            "spawn_agent",
            "创建或更新一个命名子代理（agent）。给它一个名字和角色设定，之后可以用 talk_to_agent 给它派活。同名再调用且 role 不同会更新角色并清空该 agent 历史。用于把复杂任务拆给多个专职 agent 并行处理。",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "agent 名字（字母开头，字母数字下划线，≤32 字符）。" },
                    "role": { "type": "string", "description": "角色设定：职责、擅长、输出风格，一两段话。" }
                },
                "required": ["name", "role"],
                "additionalProperties": false
            }),
            |args| async move { spawn_agent(args).await },
        )
        .writes(),
    );
    registry.register(ToolSpec::new_with_progress(
        "talk_to_agent",
        "给已创建的命名子代理发消息并返回它的回复。agent 有多轮记忆，可连续对话。多个 agent 并行派活时，同一轮里多次调用本工具，或使用 parallel_agents。agent 的思考与回复过程会实时展示。",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "目标 agent 名字（spawn_agent 创建）。" },
                "message": { "type": "string", "description": "交给 agent 的任务/消息。" }
            },
            "required": ["name", "message"],
            "additionalProperties": false
        }),
        move |args, progress| async move { talk_to_agent(args, &progress).await },
    ));
    registry.register(ToolSpec::new(
        "list_agents",
        "列出已创建的所有子代理（名字、角色、已对话轮数）。",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        |_| async move { list_agents(json!({})).await },
    ));
    registry.register(
        ToolSpec::new(
            "kill_agent",
            "销毁一个命名子代理（释放进程与记忆）。",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "要销毁的 agent 名字。" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            |args| async move { kill_agent(args).await },
        )
        .writes(),
    );
    registry.register(ToolSpec::new_with_progress(
        "parallel_agents",
        "并发执行多个 agent 任务。传入任务列表，系统会同时向多个 agent 发送消息并并行执行，最后汇总结果。适用于需要多个 agent 协作完成复杂任务的场景。",
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "目标 agent 名字" },
                            "message": { "type": "string", "description": "交给 agent 的任务消息" }
                        },
                        "required": ["name", "message"]
                    },
                    "description": "并发任务列表"
                }
            },
            "required": ["tasks"],
            "additionalProperties": false
        }),
        move |args, progress| async move { parallel_agents(args, &progress).await },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;

    fn isolated_manager() -> (tempfile::TempDir, AgentManager) {
        let temp = tempfile::tempdir().unwrap();
        let _guard = crate::paths::test_env::GQY_HOME_LOCK.lock().unwrap();
        let old = std::env::var_os("GQY_HOME");
        std::env::set_var("GQY_HOME", temp.path());
        let paths = GqyPaths::new().unwrap();
        if let Some(v) = old {
            std::env::set_var("GQY_HOME", v);
        } else {
            std::env::remove_var("GQY_HOME");
        }
        let manager = AgentManager::load(&paths, ToolRegistry::new()).unwrap();
        (temp, manager)
    }

    #[test]
    fn agent_name_validation() {
        assert!(is_valid_agent_name("architect"));
        assert!(is_valid_agent_name("Reviewer_1"));
        assert!(!is_valid_agent_name("1bad"));
        assert!(!is_valid_agent_name("has-dash"));
        assert!(!is_valid_agent_name(""));
        assert!(!is_valid_agent_name(&"a".repeat(33)));
    }

    #[test]
    fn ensure_creates_updates_and_enforces_limit() {
        let (_temp, manager) = isolated_manager();

        assert_eq!(
            manager.ensure("alpha", "角色A").unwrap(),
            EnsureOutcome::Created
        );
        assert_eq!(
            manager.ensure("alpha", "角色A").unwrap(),
            EnsureOutcome::Unchanged
        );
        assert_eq!(
            manager.ensure("alpha", "角色B").unwrap(),
            EnsureOutcome::Updated
        );

        let listed = manager.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["role"], "角色B");

        // 写入一轮历史后再更新 role，应清空
        {
            let mut agents = manager.agents.write().unwrap();
            agents
                .get_mut("alpha")
                .unwrap()
                .history
                .push(ChatMessage::plain("user", "hi"));
            agents
                .get_mut("alpha")
                .unwrap()
                .history
                .push(ChatMessage::plain("assistant", "yo"));
        }
        assert_eq!(
            manager.ensure("alpha", "角色C").unwrap(),
            EnsureOutcome::Updated
        );
        assert!(manager
            .agents
            .read()
            .unwrap()
            .get("alpha")
            .unwrap()
            .history
            .is_empty());

        for i in 0..(MAX_AGENTS - 1) {
            manager
                .ensure(&format!("a{i}"), "r")
                .unwrap();
        }
        let err = manager.ensure("overflow", "r").unwrap_err().to_string();
        assert!(err.contains("上限"), "{err}");
    }

    #[test]
    fn persist_and_reload_defs() {
        let (temp, manager) = isolated_manager();
        manager.ensure("keep", "持久角色").unwrap();
        manager.persist().unwrap();

        let _guard = crate::paths::test_env::GQY_HOME_LOCK.lock().unwrap();
        let old = std::env::var_os("GQY_HOME");
        std::env::set_var("GQY_HOME", temp.path());
        let paths = GqyPaths::new().unwrap();
        if let Some(v) = old {
            std::env::set_var("GQY_HOME", v);
        } else {
            std::env::remove_var("GQY_HOME");
        }

        let reloaded = AgentManager::load(&paths, ToolRegistry::new()).unwrap();
        let listed = reloaded.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], "keep");
        assert_eq!(listed[0]["role"], "持久角色");

        assert!(reloaded.remove("keep").unwrap());
        assert!(reloaded.list().is_empty());
    }

    #[test]
    fn recursive_tools_cover_swarm_and_task() {
        for name in [
            "spawn_agent",
            "talk_to_agent",
            "list_agents",
            "kill_agent",
            "parallel_agents",
            "task",
            "deep_research",
        ] {
            assert!(
                RECURSIVE_TOOLS.contains(&name),
                "{name} should be blocked for sub-agents"
            );
        }
    }
}
