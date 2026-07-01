use super::{readable_tool_name, ToolProgress, ToolRegistry, ToolSpec};
use crate::config::AppConfig;
use crate::i18n::{is_zh, text as t};
use crate::llm::{ChatMessage, ChatResult, OpenAiCompatibleClient, Usage};
use crate::paths::MiyuPaths;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DIAGNOSER_SYSTEM_PROMPT: &str = r#"你是深度诊断系统中的“诊断者”。
你的任务是解决用户遇到的异常，不是写研究报告。

工作原则：
1. 主动调用 check_issue、知识库、官方文档、web 搜索和必要命令收集证据。
2. 可以调用 run_command，但默认只运行只读命令。任何安装、删除、写配置、重启服务、kill 进程、清缓存等会改变系统状态的操作，必须先让最终回复征求用户确认，不能在诊断循环中直接执行。
3. 先建立问题模型：现象、目标对象、环境、复现路径、影响范围、最近变化。
4. 根因判断必须区分“已证实事实”“推断”“仍缺证据”。证据不足时不要把猜测说成确定结论。
5. 关键证据应调用 register_diagnostic_evidence 注册，并在草稿中使用返回的标记引用：
   - [E1], [E2]... 本机运行时证据（check_issue、命令输出等）
   - [K1], [K2]... 知识库证据
   - [W1], [W2]... 网页/官方文档证据
   - [U1], [U2]... 用户陈述
6. 最终草稿要面向用户：结论、关键证据、下一步验证、推荐修复、风险和回滚。不要输出内部 JSON。
7. 不使用 emoji 或装饰性图标。

每轮输出必须包含以下章节（无证据的章节标注“暂无”）：
## 问题模型
现象 / 目标对象 / 环境 / 复现路径 / 影响范围 / 最近变化
## 已确认事实
引用 [E1]/[K1]/[W1]/[U1]，标注来源
## 候选根因
按可能性排序，标注证据状态（已证实 / 推断 / 缺证据）
## 下一步最小验证
## 推荐修复
可逆操作优先；破坏性操作标注风险和回滚，需用户确认
"#;

const REVIEWER_SYSTEM_PROMPT: &str = r#"你是深度诊断系统中的“审查者”。
你只审查诊断草稿，不替用户回答。请严格输出 JSON。

审查重点：
1. 是否解决用户遇到的具体异常，而不是泛泛研究。
2. 根因判断是否有证据支撑，是否把不确定推断说成确定。
3. 是否遗漏关键证据、复现路径、环境差异或最近变化。
4. 是否建议了危险或破坏性命令；若有，是否要求用户确认并提供回滚方式。
5. 修复步骤是否可执行、可验证、尽量可逆。
6. 是否区分已证实事实、推断、下一步验证。
7. 是否包含了要求的结构化章节（问题模型/已确认事实/候选根因/下一步验证/推荐修复）。

输出格式：
{
  "accepted": true/false,
  "challenge": "主要质疑或通过理由",
  "revision_instructions": ["需要修正的事项"],
  "risk_flags": ["危险命令/过度修复/证据不足等"],
  "missing_evidence": ["仍缺的关键证据"]
}
"#;

const INPUT_METHOD_DOMAIN_RULES: &str = r#"
输入法问题专项规则：
- 输入法路径是有限的，逐个排查以下路径是否走通：
  * Wayland 协议路径：compositor 支持 zwp_text_input_manager_v3 + fcitx5 加载了 libwaylandim.so
  * GTK 模块路径：GTK_IM_MODULE=fcitx 或 im-fcitx5.so 已加载/磁盘存在
  * Qt 模块路径：QT_IM_MODULE=fcitx 或 QT_IM_MODULES 含 fcitx，或 platforminputcontext fcitx 插件已加载/磁盘存在
  * SDL 模块路径：SDL_IM_MODULE=fcitx
  * XIM 路径：XMODIFIERS=@im=fcitx + locale 有效（非 C/POSIX，在 locale -a 中存在）+ im-xim.so 存在
- 应用类型决定查哪些路径（path_status.paths 中列出）：
  * GTK / Qt / SDL：Wayland协议 + 对应模块路径 + XIM
  * Electron(X11/XWayland)：GTK模块 + XIM
  * Electron(Wayland原生)：Wayland协议 + GTK模块
  * 未知类型：全查
- Electron 的运行模式由 --ozone-platform=wayland 和 socket 证据判定，不看 WAYLAND_DISPLAY（合成器是 Wayland 不代表应用走 Wayland 原生）
- .so 模块不仅看已加载，还要看磁盘是否存在（未加载本身就是诊断信息）
- .so 的激活条件受 locale 影响（查看 immodule_cache 中的 locale 映射，如 im-xim.so 仅 ko:ja:th:zh，im-fcitx5.so 有 ja:ko:zh:* 通配符）
- 只要有一条路径走通，输入法就可以用
- macOS 和 Windows 一般不会有输入法路径异常，不需要做路径诊断
- 必须先查询输入法官方 Wiki 规则，再采集本机证据
- 必须区分目标进程环境变量和当前 shell 环境变量
- 不允许仅凭进程名或工具包类型推断，必须有运行时证据

最终报告格式要求（输入法问题专用）：
- 最终报告必须包含「输入法路径分析」章节。
- 必须逐条列出 path_status.paths 中每条路径的分析，格式为：路径名称 → 走通 / 未走通 / 不确定，并引用 evidence 中的关键证据（环境变量值、已加载的 .so 文件名、immodule_cache 的 locale 映射等）。
- 必须引用 profile.toolkit 字段的值作为应用类型，不允许自行猜测应用类型。如果 toolkit 是 ElectronX11 或 ElectronWayland，必须引用 profile.display_mode 字段。
- 必须引用 profile.target_env 中的实际环境变量值，不允许编造。如果某个变量未设置，必须明确说"未设置"。
- 结论必须明确回答：哪条路径走通了（输入法可用），或所有路径都未走通（输入法不可用，并说明原因）。
- 不使用 confirmed / missing / configured 等英文状态术语，用"走通 / 未走通 / 不确定"描述。

输入法问题审查重点：
- 最终报告是否包含「输入法路径分析」章节。缺少则要求补充。
- 是否逐条分析了 path_status.paths 中每条路径。遗漏则要求补充。
- 是否引用了 profile.toolkit 字段。自行猜测应用类型则要求纠正。
- 是否引用了 profile.target_env 中的实际值。编造环境变量则要求纠正。
- 是否区分了目标进程环境变量和 shell 环境变量。混淆则要求纠正。
- Electron 结论必须有 display_mode 证据，不能仅凭进程名推断。
- XIM 路径结论必须检查 locale 有效性，locale 为 C/POSIX 时 XIM 路径未走通。
- Wayland 协议路径结论必须同时有 compositor 支持 text-input-v3 和 fcitx5 加载 libwaylandim.so 的证据。
- immodule_cache 中的 locale 映射必须与目标进程的 locale 对照检查。
"#;

fn domain_rules(issue: &str) -> &'static str {
    let lower = issue.to_ascii_lowercase();
    if issue.contains("输入法")
        || issue.contains("打不了中文")
        || issue.contains("候选框")
        || issue.contains("拼音")
        || lower.contains("fcitx")
        || lower.contains("ibus")
        || lower.contains("input method")
        || lower.contains("xim")
        || lower.contains("xmodifiers")
        || lower.contains("gtk_im_module")
        || lower.contains("qt_im_module")
    {
        INPUT_METHOD_DOMAIN_RULES
    } else {
        ""
    }
}

#[derive(Clone)]
struct DiagnosisContext {
    config: AppConfig,
    paths: MiyuPaths,
    tools: ToolRegistry,
}

#[derive(Clone)]
struct LoopProgress {
    progress: ToolProgress,
    mode: ProgressMode,
    enabled: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProgressMode {
    Hidden,
    Summary,
    Full,
}

impl LoopProgress {
    fn new(config: &AppConfig, progress: ToolProgress) -> Self {
        let mode = match config
            .display
            .tool_calls
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "hidden" => ProgressMode::Hidden,
            "full" => ProgressMode::Full,
            _ => ProgressMode::Summary,
        };
        Self {
            progress,
            mode,
            enabled: config.plugins.deep_diagnose.show_progress,
        }
    }

    fn phase(&self, message: impl Into<String>) {
        if self.enabled && self.mode != ProgressMode::Hidden {
            self.progress.report(message.into());
        }
    }

    fn detail(&self, message: impl Into<String>) {
        if self.enabled && self.mode == ProgressMode::Full {
            self.progress.report(message.into());
        }
    }
}

#[derive(Default)]
struct DiagnosisState {
    evidence: Vec<Evidence>,
    counters: EvidenceCounters,
    stats: DiagnosisStats,
}

#[derive(Default)]
struct EvidenceCounters {
    local: usize,
    knowledge: usize,
    web: usize,
    user: usize,
}

#[derive(Clone)]
struct Evidence {
    marker: String,
    kind: String,
    title: String,
    source: String,
    snippet: String,
}

#[derive(Default)]
struct DiagnosisStats {
    tool_calls: usize,
    tool_ok: usize,
    tool_errors: usize,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    token_estimate: u64,
}

impl DiagnosisStats {
    fn add_usage_or_estimate(&mut self, usage: Option<&Usage>, texts: &[&str]) {
        if let Some(usage) = usage {
            if usage.total_tokens > 0 {
                self.prompt_tokens += usage.prompt_tokens;
                self.completion_tokens += usage.completion_tokens;
                self.total_tokens += usage.total_tokens;
                self.token_estimate += usage.total_tokens;
                return;
            }
        }
        self.token_estimate += estimate_tokens(texts);
    }
}

pub fn register(
    registry: &mut ToolRegistry,
    config: AppConfig,
    paths: MiyuPaths,
    tools: ToolRegistry,
) {
    let context = DiagnosisContext {
        config,
        paths,
        tools,
    };
    registry.register(ToolSpec::new_with_progress(
        "deep_diagnose",
        "Run a dual-role deep diagnosis loop to solve a concrete user issue. It may use check_issue, knowledge base, web tools, official docs, and commands. It returns the final answer directly and does not write a report file.",
        json!({
            "type": "object",
            "properties": {
                "issue": { "type": "string", "description": "Concrete user issue or failure symptom." },
                "thinking_depth": { "type": "string", "enum": ["minimal", "low", "medium", "high", "xhigh"], "description": "Optional depth override." }
            },
            "required": ["issue"],
            "additionalProperties": false
        }),
        move |args, progress| {
            let context = context.clone();
            async move { run_deep_diagnose(args, context, progress).await }
        },
    ));
}

async fn run_deep_diagnose(
    args: Value,
    context: DiagnosisContext,
    progress: ToolProgress,
) -> Result<String> {
    if !context.config.plugins.deep_diagnose.enabled {
        bail!("deep_diagnose plugin is disabled");
    }
    let issue = args
        .get("issue")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if issue.is_empty() {
        bail!("issue is required");
    }
    let plugin = &context.config.plugins.deep_diagnose;
    let depth = args
        .get("thinking_depth")
        .and_then(Value::as_str)
        .unwrap_or(&plugin.thinking_depth);
    let max_revisions = if plugin.max_review_revisions == 0 {
        depth_default_revisions(depth)
    } else {
        plugin.max_review_revisions
    };
    let max_tool_steps = if plugin.max_tool_steps_per_round == 0 {
        depth_default_tool_steps(depth)
    } else {
        plugin.max_tool_steps_per_round
    };
    let progress = LoopProgress::new(&context.config, progress);
    let client = OpenAiCompatibleClient::from_config(&context.config, &context.paths)?;
    let state = Arc::new(Mutex::new(DiagnosisState::default()));
    let mut draft = String::new();
    let mut review =
        json!({"accepted": false, "challenge": "首轮暂无审查意见", "revision_instructions": []});
    let mut iterations = 0usize;
    let mut stop_reason = "max_review_revisions_reached".to_string();
    progress.phase(format!(
        "{}=\"{}\"",
        t("issue", "问题"),
        clip_inline(&issue, 80)
    ));

    loop {
        let iteration = iterations + 1;
        if max_revisions != usize::MAX && iteration > max_revisions.saturating_add(1) {
            break;
        }
        iterations = iteration;
        progress.phase(if is_zh() {
            format!("第 {iteration} 轮：诊断中")
        } else {
            format!("round {iteration}: diagnosing")
        });
        let tools = diagnosis_tool_registry(&context, Arc::clone(&state));
        let prompt = diagnoser_prompt(&issue, iteration, &draft, &review, &state)?;
        let result = chat_with_tools(
            &client,
            vec![
                ChatMessage::system(DIAGNOSER_SYSTEM_PROMPT),
                ChatMessage::plain("user", prompt.clone()),
            ],
            tools,
            max_tool_steps,
            plugin.tool_call_timeout_seconds,
            &progress,
            Arc::clone(&state),
        )
        .await?;
        state
            .lock()
            .expect("deep diagnose state lock")
            .stats
            .add_usage_or_estimate(
                result.usage.as_ref(),
                &[DIAGNOSER_SYSTEM_PROMPT, &prompt, &result.content],
            );
        if !result.content.trim().is_empty() {
            draft = result.content.trim().to_string();
        }
        if draft.is_empty() {
            stop_reason = "diagnoser_failed".to_string();
            break;
        }
        progress.phase(if is_zh() {
            format!("第 {iteration} 轮：审查中")
        } else {
            format!("round {iteration}: reviewer checking")
        });
        let review_prompt = reviewer_prompt(&issue, iteration, &draft, &state)?;
        let review_result = client
            .chat_stream(
                vec![
                    ChatMessage::system(REVIEWER_SYSTEM_PROMPT),
                    ChatMessage::plain("user", review_prompt.clone()),
                ],
                Vec::new(),
                |_| Ok(()),
            )
            .await?;
        state
            .lock()
            .expect("deep diagnose state lock")
            .stats
            .add_usage_or_estimate(
                review_result.usage.as_ref(),
                &[
                    REVIEWER_SYSTEM_PROMPT,
                    &review_prompt,
                    &review_result.content,
                ],
            );
        review = parse_review(&review_result.content);
        if review
            .get("accepted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            stop_reason = "accepted".to_string();
            progress.phase(if is_zh() {
                format!("第 {iteration} 轮：通过")
            } else {
                format!("round {iteration}: accepted")
            });
            break;
        }
        progress.phase(if is_zh() {
            format!(
                "第 {iteration} 轮：需修订 — {}",
                clip_inline(
                    review
                        .get("challenge")
                        .and_then(Value::as_str)
                        .unwrap_or("审查者要求修改"),
                    100
                )
            )
        } else {
            format!(
                "round {iteration}: revision requested — {}",
                clip_inline(
                    review
                        .get("challenge")
                        .and_then(Value::as_str)
                        .unwrap_or("reviewer requested changes"),
                    100
                )
            )
        });
    }

    let mut final_answer = normalize_final_answer(&draft, &state);
    if plugin.max_final_answer_chars > 0
        && final_answer.chars().count() > plugin.max_final_answer_chars
    {
        final_answer = format!(
            "{}\n\n...[truncated to {} chars]",
            final_answer
                .chars()
                .take(plugin.max_final_answer_chars)
                .collect::<String>(),
            plugin.max_final_answer_chars
        );
    }
    let stats = public_stats(&state);
    progress.phase(format!(
        "{} {} {} {} {}",
        t("tool calls", "工具调用"),
        stats["tool_calls"].as_u64().unwrap_or(0),
        t("times", "次"),
        t("token cost", "消耗 Token"),
        format_token_count(stats["token_estimate"].as_u64().unwrap_or(0))
    ));
    Ok(serde_json::to_string_pretty(&json!({
        "ok": true,
        "kind": "deep_diagnosis",
        "issue": issue,
        "iterations_used": iterations,
        "stop_reason": stop_reason,
        "final_answer": final_answer,
        "stats": stats,
        "evidence": public_evidence(&state),
    }))?)
}

fn diagnosis_tool_registry(
    context: &DiagnosisContext,
    state: Arc<Mutex<DiagnosisState>>,
) -> ToolRegistry {
    let mut registry = context.tools.clone();
    register_evidence_tools(&mut registry, state);
    registry
}

fn register_evidence_tools(registry: &mut ToolRegistry, state: Arc<Mutex<DiagnosisState>>) {
    registry.register(ToolSpec::new(
        "register_diagnostic_evidence",
        "Register diagnostic evidence and receive a stable marker such as [E1], [K1], [W1], or [U1].",
        json!({"type":"object","properties":{"evidence_type":{"type":"string","enum":["E","K","W","U","local","knowledge","web","user"]},"title":{"type":"string"},"source":{"type":"string"},"snippet":{"type":"string"}},"required":["evidence_type","title"],"additionalProperties":false}),
        move |args| {
            let state = Arc::clone(&state);
            async move {
                let kind = normalized_evidence_kind(args.get("evidence_type").and_then(Value::as_str).unwrap_or("E"));
                let title = args.get("title").and_then(Value::as_str).unwrap_or("Untitled").trim().to_string();
                let source = args.get("source").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                let snippet = args.get("snippet").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                let mut state = state.lock().expect("deep diagnose state lock");
                let number = match kind.as_str() {
                    "K" => { state.counters.knowledge += 1; state.counters.knowledge }
                    "W" => { state.counters.web += 1; state.counters.web }
                    "U" => { state.counters.user += 1; state.counters.user }
                    _ => { state.counters.local += 1; state.counters.local }
                };
                let marker = format!("{kind}{number}");
                state.evidence.push(Evidence { marker: marker.clone(), kind, title, source, snippet });
                Ok(json!({"ok": true, "evidence": marker, "marker": format!("[{marker}]")}).to_string())
            }
        },
    ));
}

async fn chat_with_tools(
    client: &OpenAiCompatibleClient,
    mut messages: Vec<ChatMessage>,
    tools: ToolRegistry,
    max_steps: usize,
    timeout_seconds: u64,
    progress: &LoopProgress,
    state: Arc<Mutex<DiagnosisState>>,
) -> Result<ChatResult> {
    let definitions = tools.definitions_except(&["deep_diagnose", "deep_research"]);
    let mut steps = 0usize;
    loop {
        let result = client
            .chat_stream(messages.clone(), definitions.clone(), |_| Ok(()))
            .await?;
        if result.tool_calls.is_empty() {
            return Ok(result);
        }
        messages.push(ChatMessage::assistant(
            result.content.clone(),
            Some(result.tool_calls.clone()),
        ));
        for call in result.tool_calls {
            if max_steps > 0 && steps >= max_steps {
                messages.push(ChatMessage::tool(
                    call.id,
                    "tool budget reached for this deep diagnosis round",
                ));
                continue;
            }
            steps += 1;
            state
                .lock()
                .expect("deep diagnose state lock")
                .stats
                .tool_calls += 1;
            progress.phase(if is_zh() {
                format!(
                    "工具 #{steps}：{} 运行中",
                    readable_tool_name(&call.function.name)
                )
            } else {
                format!("tool #{steps}: {} running", call.function.name)
            });
            progress.detail(format!(
                "→{} {}",
                call.function.name,
                compact_arguments(&call.function.arguments)
            ));
            let (output, ok) = match tokio::time::timeout(
                Duration::from_secs(timeout_seconds.max(5)),
                tools.call(&call.function.name, &call.function.arguments),
            )
            .await
            {
                Ok(Ok(output)) => (output, true),
                Ok(Err(err)) => (format!("tool error: {err}"), false),
                Err(_) => (
                    format!(
                        "tool error: {} timed out after {timeout_seconds}s",
                        call.function.name
                    ),
                    false,
                ),
            };
            {
                let mut state = state.lock().expect("deep diagnose state lock");
                if ok {
                    state.stats.tool_ok += 1;
                } else {
                    state.stats.tool_errors += 1;
                }
            }
            progress.phase(if is_zh() {
                format!(
                    "工具 #{steps}：{} {}",
                    readable_tool_name(&call.function.name),
                    if ok { "完成" } else { "出错" }
                )
            } else {
                format!(
                    "tool #{steps}: {} {}",
                    call.function.name,
                    if ok { "ok" } else { "error" }
                )
            });
            messages.push(ChatMessage::tool(call.id, output));
        }
    }
}

fn diagnoser_prompt(
    issue: &str,
    iteration: usize,
    draft: &str,
    review: &Value,
    state: &Arc<Mutex<DiagnosisState>>,
) -> Result<String> {
    let rules = domain_rules(issue);
    let rules_section = if rules.is_empty() {
        String::new()
    } else {
        format!("\n\n领域专项规则：{rules}")
    };
    let evidence = evidence_registry_json(state)?;
    Ok(if iteration == 1 {
        format!(
            "这是第 1 轮。重点：调用 check_issue 和相关官方文档工具收集证据，建立问题模型。不要急于给修复建议。\n\n用户问题：\n{issue}\n\n当前证据注册表：\n{evidence}{rules_section}\n\n要求：主动收集证据；可以运行必要命令，但默认只读，破坏性操作只能作为需用户确认的建议；输出可直接给用户的诊断回复。"
        )
    } else {
        let draft_display = if draft.trim().is_empty() {
            "（无）"
        } else {
            draft
        };
        format!(
            "这是第 {iteration} 轮。上一轮草稿：\n{draft_display}\n\n上一轮审查意见：\n{}\n\n当前证据注册表：\n{evidence}{rules_section}\n\n要求：针对审查意见的 missing_evidence 和 revision_instructions 做最小验证，更新候选根因，给出可执行修复建议。输出可直接给用户的诊断回复。",
            serde_json::to_string_pretty(review)?,
        )
    })
}

fn reviewer_prompt(
    issue: &str,
    iteration: usize,
    draft: &str,
    state: &Arc<Mutex<DiagnosisState>>,
) -> Result<String> {
    let rules = domain_rules(issue);
    let rules_section = if rules.is_empty() {
        String::new()
    } else {
        format!("\n\n领域专项审查规则：{rules}")
    };
    Ok(format!(
        "请审查第 {iteration} 轮诊断草案。\n\n用户问题：\n{issue}\n\n草案：\n{draft}\n\n证据注册表：\n{}{rules_section}\n\n若可以发送，accepted=true；否则列出具体 revision_instructions。",
        evidence_registry_json(state)?
    ))
}

fn evidence_registry_json(state: &Arc<Mutex<DiagnosisState>>) -> Result<String> {
    let state = state.lock().expect("deep diagnose state lock");
    let evidence = state
        .evidence
        .iter()
        .map(|item| json!({"marker": item.marker, "type": item.kind, "title": item.title, "source": item.source, "snippet": item.snippet}))
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&evidence)?)
}

fn parse_review(content: &str) -> Value {
    serde_json::from_str(content.trim()).unwrap_or_else(|_| {
        json!({"accepted": false, "challenge": "reviewer returned non-JSON feedback", "revision_instructions": [content.trim()]})
    })
}

fn normalize_final_answer(draft: &str, state: &Arc<Mutex<DiagnosisState>>) -> String {
    let mut answer = draft.trim().to_string();
    let warnings = evidence_warnings(&answer, state);
    if !warnings.is_empty() {
        answer.push_str("\n\n## 证据校验提示\n");
        for warning in warnings {
            answer.push_str(&format!("- {warning}\n"));
        }
    }
    answer
}

fn evidence_warnings(draft: &str, state: &Arc<Mutex<DiagnosisState>>) -> Vec<String> {
    let state = state.lock().expect("deep diagnose state lock");
    let known = state
        .evidence
        .iter()
        .map(|item| item.marker.as_str())
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    for marker in extract_markers(draft) {
        if !known.iter().any(|item| *item == marker) {
            warnings.push(format!("正文引用了未注册证据 [{marker}]。"));
        }
    }
    warnings
}

fn extract_markers(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in value.split('[').skip(1) {
        let Some(end) = part.find(']') else {
            continue;
        };
        let marker = &part[..end];
        if marker.len() >= 2
            && matches!(marker.as_bytes()[0], b'E' | b'K' | b'W' | b'U')
            && marker[1..].chars().all(|ch| ch.is_ascii_digit())
        {
            out.push(marker.to_string());
        }
    }
    out
}

fn normalized_evidence_kind(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "k" | "knowledge" => "K".to_string(),
        "w" | "web" => "W".to_string(),
        "u" | "user" => "U".to_string(),
        _ => "E".to_string(),
    }
}

fn public_evidence(state: &Arc<Mutex<DiagnosisState>>) -> Vec<Value> {
    let state = state.lock().expect("deep diagnose state lock");
    state
        .evidence
        .iter()
        .map(|item| json!({"marker": item.marker, "type": item.kind, "title": item.title, "source": item.source}))
        .collect()
}

fn public_stats(state: &Arc<Mutex<DiagnosisState>>) -> Value {
    let state = state.lock().expect("deep diagnose state lock");
    json!({
        "tool_calls": state.stats.tool_calls,
        "tool_ok": state.stats.tool_ok,
        "tool_errors": state.stats.tool_errors,
        "prompt_tokens": state.stats.prompt_tokens,
        "completion_tokens": state.stats.completion_tokens,
        "total_tokens": state.stats.total_tokens,
        "token_estimate": state.stats.token_estimate,
        "evidence": state.evidence.len(),
    })
}

fn depth_default_revisions(depth: &str) -> usize {
    match depth {
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "xhigh" => usize::MAX,
        _ => 3,
    }
}

fn depth_default_tool_steps(depth: &str) -> usize {
    match depth {
        "minimal" => 8,
        "low" => 14,
        "medium" => 24,
        "xhigh" => 0,
        _ => 40,
    }
}

fn estimate_tokens(texts: &[&str]) -> u64 {
    let chars = texts
        .iter()
        .map(|text| text.chars().count() as u64)
        .sum::<u64>();
    if chars == 0 {
        0
    } else {
        (chars / 4).max(1)
    }
}

fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn clip_inline(value: &str, max_chars: usize) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() <= max_chars {
        value
    } else {
        format!(
            "{}...",
            value
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn compact_arguments(arguments: &str) -> String {
    clip_inline(arguments, 160)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_markers_are_extracted() {
        assert_eq!(extract_markers("a [E1] b [K2] c"), vec!["E1", "K2"]);
    }

    #[test]
    fn depth_defaults_are_bounded() {
        assert!(depth_default_tool_steps("high") >= depth_default_tool_steps("low"));
    }
}
