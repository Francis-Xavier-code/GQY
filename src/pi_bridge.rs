//! pi 底座模式下，向 pi extension 暴露 GQY 定制工具的本地 HTTP API。
//!
//! pi 内置工具只有 read/write/edit/bash/find/grep/ls，GQY 的「性格能力」
//! （记忆、表情包、闹钟、玄学、知识库、天气…）以 pi extension 的形式注入 pi：
//! GQY 主进程在本模块启动 `127.0.0.1:<随机端口>` 的 axum 服务，
//! 把端口通过 `GQY_PI_TOOL_API` 环境变量传给 pi 子进程；
//! `src/scripts/pi-bridge.ts` 扩展拉取工具清单并注册为 `gqy_*` 工具，
//! 调用时再回调本服务执行（走同一个 ToolRegistry）。

use crate::agent::Agent;
use crate::tools::{ToolProgress, ToolProgressEvent, ToolRegistry};
use anyhow::{Context, Result};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct BridgeState {
    tools: Arc<Mutex<ToolRegistry>>,
    /// 图片事件钩子：工具产生图片（表情包等）时调用（WebUI 存资产 + 推送 tool.image）
    image_sink: Option<Arc<dyn Fn(std::path::PathBuf, String) + Send + Sync>>,
    /// 进度消息钩子：工具执行中的进度文本（agent 思考/回复增量等）→ WebUI tool.progress
    progress_sink: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

#[derive(Serialize)]
struct ToolInfo {
    name: String,
    display_name: Option<String>,
    description: String,
    parameters: Value,
    /// 注入 pi system prompt 的 Available tools 段落的一句话简介（不提供则被省略）
    prompt_snippet: Option<String>,
    /// 注入 Guidelines 段落的触发规则
    prompt_guidelines: Vec<String>,
}

#[derive(Deserialize)]
struct ToolCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Serialize)]
struct ToolCallResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// 允许暴露给 pi 的 GQY 定制工具（白名单）。
/// 排除 pi 已有等价物的编码工具（run_command/read_file/write_file/edit_file/glob/grep…）、
/// REPL 交互工具（ask_question）、元工具（load_tools/load_skill/脚本注册）与子 agent 工具（task）。
const EXPOSED_TOOLS: &[&str] = &[
    // 记忆
    "remember_fact",
    "recall_memories",
    "recall_past_events",
    "search_evicted_context",
    "log_mood",
    "recall_mood",
    // 表情包
    "search_meme",
    "show_meme",
    "add_meme",
    "delete_meme",
    "update_meme",
    // 知识库
    "search_knowledge_base",
    "search_knowledge_base_by_name",
    "read_knowledge_base_file",
    "upload_text_to_knowledge_base",
    "edit_knowledge_base_file",
    "remove_knowledge_base_file",
    // 闹钟
    "set_alarm",
    "list_alarms",
    "cancel_alarm",
    "pomodoro",
    // 玄学
    "draw_zhouyi_hexagram",
    "draw_tarot_card",
    "draw_fortune_lot",
    "roll_dice",
    // 信息查询
    "get_weather",
    "get_exchange_rate",
    "online_man_search",
    "online_man_get_page",
    "query_moegirl",
    "query_deepseek_status",
    "protondb_query",
    "check_os_info",
    // 工具
    "calculate_hash",
    "decode_encoded_text",
    "scientific_calculator",
    "read_clipboard",
    "web_fetch",
    "speak",
    "listen_audio",
    // 本地视觉（Apple Vision，离线免费，不耗 API 额度）
    "analyze_image_local",
    // 子 agent 任务与深度研究（经独立 pi 子进程隔离执行，长耗时）
    "task",
    "deep_research",
    // 自主 agent 集群（Kimi 式）：模型可自建/管理命名子代理
    "spawn_agent",
    "talk_to_agent",
    "list_agents",
    "kill_agent",
];

/// 常规工具调用超时。
const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(180);
/// 长任务（task / deep_research，内部跑子 agent 流水线）超时：给足 30 分钟。
const TOOL_CALL_TIMEOUT_LONG: Duration = Duration::from_secs(1800);

fn tool_timeout(name: &str) -> Duration {
    if matches!(name, "task" | "deep_research") {
        TOOL_CALL_TIMEOUT_LONG
    } else {
        TOOL_CALL_TIMEOUT
    }
}

/// 若当前 client 是 pi，启动工具桥并设置 `GQY_PI_TOOL_API` / `GQY_PI_TOOL_LIST` 环境变量
/// （pi 子进程继承，spawn 是懒加载的，确保在首次对话前设置完成）。
///
/// 工具清单同时写一份 JSON 到 `GQY_HOME/cache/pi-bridge-tools.json`：
/// pi-bridge 扩展在**加载时同步读取**（避免 session_start 里异步 fetch 与首轮
/// prompt 的竞态，导致首轮 system prompt 里没有 gqy_* 工具）。
pub async fn ensure_for_agent(
    agent: &Agent,
    image_sink: Option<Arc<dyn Fn(std::path::PathBuf, String) + Send + Sync>>,
    progress_sink: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<Option<u16>> {
    if !agent.llm_client().is_pi() {
        return Ok(None);
    }
    ensure_pi_bridge(agent.tools_registry(), agent.paths(), image_sink, progress_sink).await
}

/// 启动 pi 工具桥并设置环境变量（不依赖 Agent 实例，Web 侧 agent 在 actor 线程里时用）。
pub async fn ensure_pi_bridge(
    tools: Arc<Mutex<ToolRegistry>>,
    paths: &crate::paths::GqyPaths,
    image_sink: Option<Arc<dyn Fn(std::path::PathBuf, String) + Send + Sync>>,
    progress_sink: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<Option<u16>> {
    let port = start_with_sink(tools.clone(), image_sink, progress_sink).await?;
    std::env::set_var("GQY_PI_TOOL_API", format!("http://127.0.0.1:{port}"));

    // 同步工具清单文件（扩展加载时读取）
    let tool_infos = {
        let guard = tools.lock().unwrap();
        collect_tool_infos(&guard)
    };
    let cache = paths.cache_dir.join("pi-bridge-tools.json");
    std::fs::create_dir_all(
        cache
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )?;
    std::fs::write(&cache, serde_json::to_string_pretty(&tool_infos)?)?;
    std::env::set_var("GQY_PI_TOOL_LIST", &cache);

    tracing::info!(port, tool_list = %cache.display(), "pi bridge started for pi mode");
    Ok(Some(port))
}

/// 启动本地 HTTP 服务（无图片/进度回调）。
pub async fn start(tools: Arc<Mutex<ToolRegistry>>) -> Result<u16> {
    start_with_sink(tools, None, None).await
}

/// 启动本地 HTTP 服务，返回监听端口。
/// `image_sink`：可选，工具产生图片（表情包等）时回调（WebUI 用）。
pub async fn start_with_sink(
    tools: Arc<Mutex<ToolRegistry>>,
    image_sink: Option<Arc<dyn Fn(std::path::PathBuf, String) + Send + Sync>>,
    progress_sink: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<u16> {
    let state = BridgeState {
        tools,
        image_sink,
        progress_sink,
    };
    let app = Router::new()
        .route("/ping", get(ping))
        .route("/tools", get(list_tools))
        .route("/tool", post(call_tool))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("pi bridge: failed to bind 127.0.0.1")?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!("pi bridge server error: {err:#}");
        }
    });
    Ok(port)
}

async fn ping() -> &'static str {
    "pong"
}

async fn list_tools(State(state): State<BridgeState>) -> Json<Vec<ToolInfo>> {
    let registry = state.tools.lock().unwrap();
    Json(collect_tool_infos(&registry))
}

/// 收集暴露工具清单（HTTP /tools 与同步文件共用）。
/// 白名单工具 + 用户导入的脚本工具（`gqy tools import` 产物）都放行。
fn collect_tool_infos(registry: &ToolRegistry) -> Vec<ToolInfo> {
    if std::env::var_os("GQY_PI_DEBUG").is_some() {
        let scripts = registry.tool_names().into_iter().filter(|n| registry.is_script_tool(n)).collect::<Vec<_>>();
        eprintln!("[pi-bridge] script tools: {scripts:?}");
    }
    registry
        .definitions()
        .into_iter()
        .filter(|definition| {
            EXPOSED_TOOLS.contains(&definition.function.name.as_str())
                || registry.is_script_tool(&definition.function.name)
        })
        .map(|definition| {
            let guidance = tool_guidance(&definition.function.name);
            let tool_name = definition.function.name.clone();
            ToolInfo {
                display_name: registry.display_name(&tool_name),
                name: tool_name.clone(),
                description: definition.function.description,
                parameters: definition.function.parameters,
                prompt_snippet: Some(
                    guidance
                        .map(|g| g.0.to_string())
                        .unwrap_or_else(|| format!("用户脚本工具：{tool_name}")),
                ),
                prompt_guidelines: guidance
                    .map(|g| g.1.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_else(|| {
                        vec![format!(
                            "用户明确要求用「{tool_name}」这个工具/脚本时，调用 gqy_{tool_name}"
                        )]
                    }),
            }
        })
        .collect()
}

/// 每个暴露工具的「使用时机」引导，注入 pi 的 system prompt，
/// 帮助模型在自然语言请求时选择 gqy_* 工具而不是用 bash 自行实现。
fn tool_guidance(name: &str) -> Option<(&'static str, &'static [&'static str])> {
    let pair = match name {
        "remember_fact" => (
            "记住事实/偏好/知识点",
            &[
                "用户说「记住：xxx」或要求你记住某个事实/偏好/方法时，使用 gqy_remember_fact",
                "不要用普通聊天代替记忆写入；需要长期可召回的信息一律走 gqy_remember_fact",
            ][..],
        ),
        "recall_memories" => (
            "按关键词召回记忆",
            &[
                "用户问「你记得…吗」「回忆一下/想想之前」时，调用 gqy_recall_memories 并附上与内容重叠的关键词",
                "回答涉及跨会话信息时，先用 gqy_recall_memories / gqy_recall_past_events 检索再作答，不要凭空编造",
            ][..],
        ),
        "recall_past_events" => (
            "召回过往事件",
            &["用户问「我们之前聊过什么/发生过什么」时，使用 gqy_recall_past_events"][..],
        ),
        "search_evicted_context" => (
            "检索被压缩的上下文",
            &["用户提到很早以前的对话且当前上下文里没有时，使用 gqy_search_evicted_context"][..],
        ),
        "log_mood" | "recall_mood" => (
            "心情记录/回忆",
            &["用户表达情绪或询问过去的心情时，使用 gqy_log_mood / gqy_recall_mood"][..],
        ),
        "search_meme" => (
            "搜索表情包",
            &[
                "用户要求发/找/使用表情包时，使用 gqy_search_meme 搜索表情包库",
                "聊天中气氛合适时也可以主动用 gqy_search_meme 找表情包",
            ][..],
        ),
        "show_meme" => (
            "发送/展示表情包",
            &[
                "用户要求发/展示某个表情包，或聊天氛围合适时，先用 gqy_search_meme 找到 id，再用 gqy_show_meme 发送（图片会直接展示）",
                "gqy_show_meme 需要 id 参数：从 gqy_search_meme 的结果里取",
            ][..],
        ),
        "add_meme" => (
            "把图片加入表情包库",
            &["用户提供图片并希望存入表情包库时，使用 gqy_add_meme"][..],
        ),
        "delete_meme" | "update_meme" => (
            "管理表情包库",
            &["用户要求删除/更新表情包时，使用 gqy_delete_meme / gqy_update_meme"][..],
        ),
        "search_knowledge_base" => (
            "检索知识库",
            &[
                "回答涉及 macOS/Homebrew/系统操作等实操问题时，优先用 gqy_search_knowledge_base 检索知识库再回答",
                "知识库检索不到再考虑网络搜索或 bash",
            ][..],
        ),
        "search_knowledge_base_by_name" => (
            "按名称检索知识库条目",
            &["用户给出明确主题/标题时，用 gqy_search_knowledge_base_by_name 查知识库"][..],
        ),
        "read_knowledge_base_file" => (
            "读取知识库条目全文",
            &["需要知识库条目完整内容时，用 gqy_read_knowledge_base_file"][..],
        ),
        "upload_text_to_knowledge_base" => (
            "把内容写入知识库",
            &["用户要求把某段内容存入知识库时，用 gqy_upload_text_to_knowledge_base"][..],
        ),
        "edit_knowledge_base_file" | "remove_knowledge_base_file" => (
            "编辑/删除知识库条目",
            &["用户要求修改/删除知识库条目时，用 gqy_edit_knowledge_base_file / gqy_remove_knowledge_base_file"][..],
        ),
        "set_alarm" | "pomodoro" => (
            "设置闹钟/番茄钟",
            &[
                "用户要求设闹钟、倒计时、定时提醒或番茄钟学习时，使用 gqy_set_alarm / gqy_pomodoro",
                "不要用 bash sleep 模拟闹钟，用 gqy_set_alarm 才能真正到时提醒",
            ][..],
        ),
        "list_alarms" | "cancel_alarm" => (
            "查看/取消闹钟",
            &["用户询问闹钟状态或要取消闹钟时，使用 gqy_list_alarms / gqy_cancel_alarm"][..],
        ),
        "draw_zhouyi_hexagram" | "draw_tarot_card" | "draw_fortune_lot" | "roll_dice" => (
            "玄学算命/掷骰子",
            &[
                "用户要算卦、抽塔罗、求签、看运势或掷骰子时，使用对应的 gqy_draw_* / gqy_roll_dice 工具",
                "不要用 bash 自己生成卦象或随机数代替专业工具",
            ][..],
        ),
        "get_weather" => (
            "查询天气",
            &["用户查询天气时，使用 gqy_get_weather（不要用 bash 猜）"][..],
        ),
        "get_exchange_rate" => (
            "查询汇率",
            &["用户查询汇率时，使用 gqy_get_exchange_rate"][..],
        ),
        "online_man_search" | "online_man_get_page" => (
            "查询 man 手册",
            &["用户要查 man 手册/命令用法时，使用 gqy_online_man_search / gqy_online_man_get_page"][..],
        ),
        "query_moegirl" => (
            "查询萌娘百科",
            &["用户查询 ACG/萌娘百科词条时，使用 gqy_query_moegirl"][..],
        ),
        "calculate_hash" | "decode_encoded_text" => (
            "哈希计算/编解码",
            &["用户要计算哈希或 base64/hex/url 等编解码时，使用 gqy_calculate_hash / gqy_decode_encoded_text"][..],
        ),
        "scientific_calculator" => (
            "科学计算器",
            &["需要精确的数学/科学计算时，使用 gqy_scientific_calculator 而不是手算"][..],
        ),
        "read_clipboard" => (
            "读取剪贴板",
            &["用户询问剪贴板内容或要基于剪贴板操作时，使用 gqy_read_clipboard"][..],
        ),
        "web_fetch" => (
            "抓取网页内容",
            &["用户要求读取某个网页的内容时，使用 gqy_web_fetch"][..],
        ),
        "check_os_info" => (
            "查看系统信息",
            &["需要系统/桌面环境信息时，使用 gqy_check_os_info"][..],
        ),
        "query_deepseek_status" => (
            "查询 DeepSeek 余额",
            &["用户询问 API 余额/额度时，使用 gqy_query_deepseek_status"][..],
        ),
        "protondb_query" => (
            "查询 ProtonDB 游戏兼容性",
            &["用户询问 Linux 游戏兼容性/ProtonDB 时，使用 gqy_protondb_query"][..],
        ),
        "speak" => (
            "文字转语音",
            &["用户要求读出/播放某段文字时，使用 gqy_speak"][..],
        ),
        "listen_audio" => (
            "语音转文字",
            &["用户要求听一段音频时，使用 gqy_listen_audio"][..],
        ),
        "spawn_agent" => (
            "创建命名子代理（agent）",
            &[
                "用户要求「建一个某某 agent/角色代理/团队成员」或任务需要多角色协作时，用 gqy_spawn_agent 创建",
                "创建后对每个 agent 用 gqy_talk_to_agent 派活，多个 agent 可同一轮并行派发",
            ][..],
        ),
        "talk_to_agent" => (
            "给子代理派活",
            &[
                "创建 agent 后，把具体任务用 gqy_talk_to_agent 交给它；多 agent 并行时同一轮多次调用",
                "agent 有独立记忆，可连续对话",
            ][..],
        ),
        "list_agents" | "kill_agent" => (
            "管理子代理",
            &["用户询问有哪些 agent 或用完要销毁时，用 gqy_list_agents / gqy_kill_agent"][..],
        ),
        "task" => (
            "子任务（子 agent）",
            &[
                "用户要求并行调研多个子主题、分步骤执行复杂任务时，使用 gqy_task 拆成子 agent 并行处理",
                "gqy_task 内部会启动独立的子 agent，耗时可能较长，耐心等结果",
            ][..],
        ),
        "deep_research" => (
            "深度研究（多阶段报告）",
            &[
                "用户要求「深度研究/写研究报告/查证某个命题」时，使用 gqy_deep_research 生成带引用的结构化报告",
                "gqy_deep_research 会分阶段（规划→多路调研→审查→撰写）执行，可能耗时数分钟到十几分钟",
            ][..],
        ),
        "analyze_image_local" => (
            "本地离线看图（OCR/分类）",
            &[
                "用户提供本地图片路径、要求识别图中文字/物体/验证码/截图内容时，使用 gqy_analyze_image_local（Apple Vision，免费离线，不耗 API 额度）",
                "多模态模型不可用或限流时，用 gqy_analyze_image_local 兜底看图",
            ][..],
        ),
        _ => return None,
    };
    Some(pair)
}

async fn call_tool(
    State(state): State<BridgeState>,
    Json(request): Json<ToolCallRequest>,
) -> Json<ToolCallResponse> {
    let arguments = if request.arguments.is_null() {
        json!({})
    } else {
        request.arguments
    };
    let arguments_str = arguments.to_string();
    let tool_name = request.name.clone();
    let tool_name_for_call = tool_name.clone();
    // 克隆 registry（Arc 工具表，代价小），避免在 async 中持有 std Mutex guard
    let registry = state.tools.lock().unwrap().clone();
    let image_sink = state.image_sink.clone();
    let progress_sink = state.progress_sink.clone();
    // 工具执行 + 进度事件并发：图片事件上报（WebUI 存资产），
    // PrepareForExternalOutput 自动应答 true（保留终端 chafa 打印行为）。
    // 工具在独立任务中执行，progress（含 sender）随任务完成被 drop，
    // 事件循环收到 None 自然退出，join 必然收敛。
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = ToolProgress::new(progress_tx);
    let tool_task = tokio::spawn(async move {
        registry
            .call_with_progress(&tool_name_for_call, &arguments_str, &progress)
            .await
    });
    let call_timeout = tool_timeout(&tool_name);
    let output = tokio::time::timeout(call_timeout, async move {
        tokio::join!(tool_task, async {
            while let Some(event) = progress_rx.recv().await {
                match event {
                    ToolProgressEvent::Image { path, alt, .. } => {
                        if let Some(sink) = &image_sink {
                            sink(path, alt);
                        }
                    }
                    ToolProgressEvent::PrepareForExternalOutput { ready } => {
                        let _ = ready.send(true);
                    }
                    ToolProgressEvent::Message(message) => {
                        if let Some(sink) = &progress_sink {
                            sink(message);
                        }
                    }
                    _ => {}
                }
            }
        })
        .0
    })
    .await;
    match output {
        Ok(Ok(Ok(output))) => Json(ToolCallResponse {
            ok: true,
            output: Some(output),
            error: None,
        }),
        Ok(Ok(Err(err))) => Json(ToolCallResponse {
            ok: false,
            output: None,
            error: Some(format!("{err:#}")),
        }),
        Ok(Err(join_err)) => Json(ToolCallResponse {
            ok: false,
            output: None,
            error: Some(format!("pi bridge tool task failed: {join_err}")),
        }),
        Err(_elapsed) => Json(ToolCallResponse {
            ok: false,
            output: None,
            error: Some(format!(
                "GQY tool {} timed out after {}s",
                tool_name,
                call_timeout.as_secs()
            )),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::paths::GqyPaths;
    use crate::tools;
    use std::path::PathBuf;

    fn test_paths(root: &std::path::Path) -> GqyPaths {
        GqyPaths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.jsonc"),
            skills_dir: root.join("config/skills"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            state_dir: root.join("state"),
            pictures_dir: root.join("pictures"),
            fish_hook_file: root.join("config/fish/conf.d/gqy.fish"),
            bash_hook_file: root.join("config/shell/bash-hook.sh"),
            zsh_hook_file: root.join("config/shell/zsh-hook.zsh"),
            share_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            system_scripts_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/scripts"),
            kb_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kb"),
            scripts_dir: root.join("config/scripts"),
        }
    }

    #[tokio::test]
    async fn bridge_lists_and_calls_exposed_tools() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let config = AppConfig::load_or_default(&paths).unwrap();
        let registry = Arc::new(std::sync::Mutex::new(tools::builtin_registry(
            &config, &paths,
        )));
        let port = start(registry).await.unwrap();
        let base = format!("http://127.0.0.1:{port}");

        let client = reqwest::Client::new();
        let tools_json: Value = client
            .get(format!("{base}/tools"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let names = tools_json
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(
            names.contains(&"remember_fact".to_string()),
            "exposed tools should include memory tools, got: {names:?}"
        );
        assert!(
            names.contains(&"get_weather".to_string()),
            "exposed tools should include weather"
        );
        assert!(
            !names.contains(&"run_command".to_string()),
            "coding tools should not be exposed, got: {names:?}"
        );
        assert!(
            !names.contains(&"grep".to_string()),
            "colliding tools should not be exposed"
        );

        // 调用一个确定性的工具：calculate_hash
        let resp: Value = client
            .post(format!("{base}/tool"))
            .json(&json!({
                "name": "calculate_hash",
                "arguments": {"input_text": "abc", "algorithms": "md5"}
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["ok"], true, "resp: {resp}");
        let output = resp["output"].as_str().unwrap();
        assert!(output.contains("900150983cd24fb0d6963f7d28e17f72"), "{output}");

        // 未知工具返回失败
        let resp: Value = client
            .post(format!("{base}/tool"))
            .json(&json!({"name": "no_such_tool", "arguments": {}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["ok"], false);
    }

    /// 工具矩阵：本地/确定性工具逐个经桥路径（call_with_progress + 事件通道）调用。
    /// 每个工具独立计时、动态小超时（5s），网络/音频/剪贴板/副作用类工具不在此测试。
    #[tokio::test]
    async fn exposed_tools_matrix_runs_ok() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let config = AppConfig::load_or_default(&paths).unwrap();
        let registry = tools::builtin_registry(&config, &paths);

        // (工具名, 参数, 输出需包含的子串（None 表示只断言 ok）)
        let cases: &[(&str, Value, Option<&str>)] = &[
            ("calculate_hash", json!({"input_text": "abc", "algorithms": "md5"}), Some("900150983cd24fb0d6963f7d28e17f72")),
            ("decode_encoded_text", json!({"input_text": "aGVsbG8=", "input_format": "base64"}), Some("hello")),
            ("scientific_calculator", json!({"expression": "1+2*3"}), Some("7")),
            ("check_os_info", json!({}), None),
            ("roll_dice", json!({}), None),
            ("draw_zhouyi_hexagram", json!({}), None),
            ("draw_tarot_card", json!({}), None),
            ("draw_fortune_lot", json!({}), None),
            ("log_mood", json!({"mood": "开心"}), Some("ok")),
            ("remember_fact", json!({"content": "矩阵测试记忆"}), Some("ok")),
            ("recall_memories", json!({"query": "矩阵"}), Some("ok")),
            ("recall_past_events", json!({"query": "矩阵"}), Some("ok")),
            ("search_evicted_context", json!({"query": "矩阵"}), Some("ok")),
            ("search_meme", json!({"query": "猫"}), Some("library")),
            ("list_alarms", json!({}), Some("ok")),
            ("search_knowledge_base", json!({"query": "brew"}), Some("ok")),
        ];

        let mut failures = Vec::new();
        let mut timings = Vec::new();
        for (name, args, needle) in cases {
            // 与 pi_bridge::call_tool 相同的并发模型：工具在独立任务中执行，
            // progress（含 sender）随任务完成 drop，事件循环收敛。
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let progress = ToolProgress::new(progress_tx);
            let arguments_str = args.to_string();
            let tool_task = {
                let registry = registry.clone();
                let name = name.to_string();
                tokio::spawn(async move {
                    registry
                        .call_with_progress(&name, &arguments_str, &progress)
                        .await
                })
            };
            let started = std::time::Instant::now();
            let run = async {
                tokio::join!(tool_task, async {
                    while let Some(event) = progress_rx.recv().await {
                        if let ToolProgressEvent::PrepareForExternalOutput { ready } = event {
                            let _ = ready.send(true);
                        }
                    }
                })
                .0
            };
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), run).await;
            timings.push(format!("{name}:{:.0}ms", started.elapsed().as_millis()));
            match outcome {
                Ok(Ok(Ok(output))) => {
                    if let Some(needle) = needle {
                        if !output.contains(needle) {
                            failures.push(format!(
                                "{name}: 输出缺少 {needle:?}: {}",
                                output.chars().take(80).collect::<String>()
                            ));
                        }
                    }
                }
                Ok(Ok(Err(err))) => failures.push(format!("{name}: Err({err:#})")),
                Ok(Err(join_err)) => failures.push(format!("{name}: task join Err({join_err})")),
                Err(_) => failures.push(format!("{name}: 超时")),
            }
        }
        eprintln!("工具矩阵耗时: {}", timings.join(" "));

        if !failures.is_empty() {
            panic!("工具矩阵失败:\n{}", failures.join("\n"));
        }
    }
}
