//! 自主 agent 集群（Kimi 式）。
//!
//! 模型在对话中可以自主创建命名 agent（角色/工具）、点名对话、并行派活、
//! 列表与销毁。每个 agent 是一个独立 LLM 会话：
//! - pi 模式下：独立 pi 进程（自定义系统提示词 → 独立进程，多轮记忆）；
//! - 直连模式下：OpenAI 兼容客户端 + 实例内消息历史（多轮记忆）。
//!
//! agent 定义持久化在 `GQY_HOME/data/agents/agents.json`，重启后定义仍在，
//! 进程按需懒启动。
//!
//! 递归防护：agent 自己的进程使用「子 agent 过滤清单」（不含 spawn_agent /
//! talk_to_agent 等），防止 agent 再无限创建 agent。

use crate::config::AppConfig;
use crate::llm::{ChatMessage, LlmClient};
use crate::paths::GqyPaths;
use crate::tools::{ToolRegistry, ToolSpec};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::RwLock;

const AGENTS_FILE: &str = "agents.json";
const MAX_AGENTS: usize = 16;
const MAX_HISTORY_TURNS: usize = 20;

/// 全局 agent 管理器（进程级单例，工具与桥共用）。
static AGENTS: OnceLock<ArcAgentManager> = OnceLock::new();

type ArcAgentManager = std::sync::Arc<AgentManager>;

#[derive(Serialize, Deserialize, Clone)]
struct AgentDef {
    name: String,
    role: String,
    created_at: String,
}

struct AgentInstance {
    def: AgentDef,
    client: LlmClient,
    history: Vec<ChatMessage>,
}

pub struct AgentManager {
    paths: GqyPaths,
    agents: RwLock<HashMap<String, AgentInstance>>,
}

fn manager(paths: &GqyPaths) -> Result<&'static ArcAgentManager> {
    if let Some(existing) = AGENTS.get() {
        return Ok(existing);
    }
    let loaded = std::sync::Arc::new(AgentManager::load(paths)?);
    match AGENTS.set(loaded) {
        Ok(()) => Ok(AGENTS.get().expect("just set")),
        Err(_) => Ok(AGENTS.get().expect("set by another thread")),
    }
}

impl AgentManager {
    fn defs_path(paths: &GqyPaths) -> PathBuf {
        paths.data_dir.join("agents").join(AGENTS_FILE)
    }

    fn load(paths: &GqyPaths) -> Result<Self> {
        let agents = RwLock::new(HashMap::new());
        let manager = Self {
            paths: paths.clone(),
            agents,
        };
        let defs_path = Self::defs_path(paths);
        if let Ok(raw) = std::fs::read_to_string(&defs_path) {
            if let Ok(defs) = serde_json::from_str::<Vec<AgentDef>>(&raw) {
                for def in defs {
                    let client = Self::make_client(paths, &def.role);
                    manager.agents.write().unwrap().insert(
                        def.name.clone(),
                        AgentInstance {
                            def,
                            client,
                            history: Vec::new(),
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

    fn make_client(paths: &GqyPaths, role: &str) -> LlmClient {
        // 直连模式直接构造；pi 模式用 from_config（进程按 persona 懒启动）。
        // 角色系统提示词统一包装，保证 agent 行为可预期。
        match LlmClient::from_config(&AppConfig::load_or_default(paths).unwrap_or_default(), paths)
        {
            Ok(client) => client,
            Err(_) => LlmClient::OpenAi(
                crate::llm::OpenAiCompatibleClient::from_config(
                    &AppConfig::default(),
                    paths,
                )
                .unwrap_or_else(|_| {
                    // 极端兜底：构造一个不可能走到的占位（正常路径不会到这里）
                    unreachable!("agent client construction failed")
                }),
            ),
        }
        .for_subagent_output(true)
    }

    fn agent_system_prompt(role: &str) -> String {
        format!(
            "你是顾清影创建的专属子代理，负责完成交给你的具体任务。\n\n你的角色设定：\n{role}\n\n\
             工作守则（必须遵守）：\n\
             1. 只输出真实、可核查的信息；绝不编造事实、数据、引用或来源。\n\
             2. 涉及事实/数据/外部信息时，优先调用工具核实（如 gqy_web_search、gqy_web_fetch、gqy_search_knowledge_base），不要凭记忆猜测。\n\
             3. 无法核实或不确定的内容，明确标注「不确定」或「未能核实」，不要给出貌似权威的假答案。\n\
             4. 引用外部信息时说明来源；没有来源支撑的结论不要强加。\n\
             5. 输出简洁、直接、可用；任务完成即给出结论，不要过度寒暄。"
        )
    }

    fn ensure(&self, name: &str, role: &str) -> Result<()> {
        if !self.agents.read().unwrap().contains_key(name) {
            let mut agents = self.agents.write().unwrap();
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
                    client: Self::make_client(&self.paths, role),
                    history: Vec::new(),
                },
            );
            drop(agents);
            self.persist()?;
        }
        Ok(())
    }

    /// 取 agent 的 client 克隆 + 角色（不持有锁返回）。
    fn client_and_role(&self, name: &str) -> Option<(LlmClient, String, Vec<ChatMessage>)> {
        let agents = self.agents.read().unwrap();
        agents.get(name).map(|instance| {
            (
                instance.client.clone(),
                instance.def.role.clone(),
                instance.history.clone(),
            )
        })
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
        progress: &crate::tools::ToolProgress,
    ) -> Result<String> {
        let Some((client, role, mut history)) = self.client_and_role(name) else {
            bail!("agent 不存在：{name}（先用 spawn_agent 创建）");
        };
        history.push(ChatMessage::plain("user", message));
        if history.len() > MAX_HISTORY_TURNS * 2 {
            let keep = MAX_HISTORY_TURNS * 2;
            history = history.split_off(history.len() - keep);
        }
        let mut request = vec![ChatMessage::system(Self::agent_system_prompt(&role))];
        request.extend(history.iter().cloned());

        let agent_name = name.to_string();
        let progress_for_stream = progress.clone();
        let result = client
            .chat_stream(request, Vec::new(), move |chunk| {
                // agent 的思考与正文增量 → 进度消息，Web/终端实时可见
                let prefix = match chunk.kind {
                    crate::llm::ChatStreamKind::Reasoning => {
                        if chunk.text.trim().is_empty() {
                            return Ok(());
                        }
                        format!("🧠 {agent_name} 思考：{}", chunk.text)
                    }
                    crate::llm::ChatStreamKind::Content => {
                        if chunk.text.trim().is_empty() {
                            return Ok(());
                        }
                        format!("✍️ {agent_name}：{}", chunk.text)
                    }
                    _ => return Ok(()),
                };
                progress_for_stream.report(prefix);
                Ok(())
            })
            .await?;
        let reply = result.content;
        history.push(ChatMessage::plain("assistant", reply.clone()));

        if let Some(instance) = self.agents.write().unwrap().get_mut(name) {
            instance.history = history;
        }
        Ok(reply)
    }
}

async fn spawn_agent(args: Value) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let role = required_str(&args, "role")?.to_string();
    if !is_valid_agent_name(&name) {
        bail!("agent 名字只能包含字母数字下划线，且以字母开头：{name}");
    }
    let manager = AGENTS.get().context("agent manager not initialized")?;
    manager.ensure(&name, &role)?;
    Ok(json!({
        "ok": true,
        "message": format!("agent「{name}」已就绪（角色：{role}）。对它说 talk_to_agent(name=\"{name}\", message=...) 即可派活。"),
        "name": name,
    })
    .to_string())
}

async fn talk_to_agent(
    args: Value,
    progress: &crate::tools::ToolProgress,
) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let message = required_str(&args, "message")?.to_string();
    let manager = AGENTS.get().context("agent manager not initialized")?;
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
    let manager = AGENTS.get().context("agent manager not initialized")?;
    Ok(json!({
        "ok": true,
        "agents": manager.list(),
    })
    .to_string())
}

async fn kill_agent(args: Value) -> Result<String> {
    let name = required_str(&args, "name")?.to_string();
    let manager = AGENTS.get().context("agent manager not initialized")?;
    let removed = manager.remove(&name)?;
    Ok(json!({
        "ok": true,
        "removed": removed,
        "message": if removed { format!("agent「{name}」已销毁") } else { format!("没有找到 agent「{name}」") },
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

/// 注册 agent 集群工具（spawn_agent / talk_to_agent / list_agents / kill_agent）。
pub fn register(registry: &mut ToolRegistry, paths: GqyPaths) {
    // 初始化全局管理器（幂等）
    let _ = manager(&paths);
    registry.register(ToolSpec::new(
        "spawn_agent",
        "创建或更新一个命名子代理（agent）。给它一个名字和角色设定，之后可以用 talk_to_agent 给它派活。用于把复杂任务拆给多个专职 agent 并行处理。",
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
    ).writes());
    registry.register(ToolSpec::new_with_progress(
        "talk_to_agent",
        "给已创建的命名子代理发消息并返回它的回复。agent 有多轮记忆，可连续对话。多个 agent 并行派活时，同一轮里多次调用本工具即可并行执行。agent 的思考与回复过程会实时展示。",
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
    registry.register(ToolSpec::new(
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
    ).writes());
}
