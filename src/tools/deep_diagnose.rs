use super::{ToolProgress, ToolRegistry, ToolSpec};
use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::llm::{ChatMessage, ChatResult, OpenAiCompatibleClient};
use crate::paths::MiyuPaths;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

const INPUT_METHOD_DIAGNOSIS_PROMPT: &str = r#"你是 Linux 输入法诊断子代理，熟知 Linux 图形栈、输入法协议、桌面合成器、Shell 命令和常见应用框架。

你的任务是诊断用户遇到的 Linux 输入法问题。你不是写教程，也不是泛泛解释原理，而是要一步一步收集证据，判断目标软件到底能走哪条输入法路径、哪条路径断了、为什么断，然后给出可执行的修复方案。

## 工作流程

首先确定输入法本身运行正常，然后确定用户遇到输入法问题的软件是什么、是否正在运行、它是用什么框架开发的、它是以 Wayland 运行还是以 X11/XWayland 运行。

不要只看当前 shell 的环境变量。诊断输入法问题时，目标软件进程自己的环境变量才是关键证据。当前 shell 的环境变量只能作为参考。

判断软件是 Wayland 还是 X11/XWayland 时，若没有 xlsclients、xprop等工具，可以通过运行时 socket 判断：读取 `/proc/net/unix` 里的 X11 socket 和 Wayland socket inode，再用 `ss -xp` 查看目标进程持有哪些 Unix socket。目标进程持有 X11 socket，则按 X11/XWayland 处理；没有 X11 socket 但持有 Wayland socket，则按 Wayland 原生应用处理；如果你有更好的判断Wayland和XWayland的方法也可以尝试，确保判断置信度高于90%，若不足，就认为不确定，不要乱猜，并且后续所有的排查项目无视分类，全部排查。

确定运行模式之后，按照下面的规则收集信息：

所有以 Wayland 运行的应用都要查：

- 输入法是否运行正常；
- 输入法是否加载 Wayland frontend / input-method 相关模块；
- 桌面合成器是否支持 `text-input` 协议；
- 桌面合成器支持的 `text-input` 协议版本；
- 软件本身是否支持 `text-input`；
- 软件支持的 `text-input` 协议版本是否和桌面合成器匹配。

所有以 X11/XWayland 运行的应用都要查：

- 目标进程里的 `XMODIFIERS`；
- XIM 是否可用；
- `im-xim.so` 是否存在；
- `im-xim.so` 的 locale 激活条件；
- 目标进程的 `LANG`、`LC_CTYPE` 等 locale 信息是否满足激活条件。

然后根据软件框架继续追加查询：

如果软件框架难以判断，可以灵活使用网络搜索、知识库搜索、官方文档、Wiki 查询等工具，查清楚这个软件到底是什么框架、什么运行模式、有没有特殊输入法问题。

- GTK 应用：查目标进程里的 `GTK_IM_MODULE`，查 GTK immodule cache，查 `im-fcitx5.so` 是否存在或被加载，查 locale 是否满足模块激活条件。

- Qt 应用：查目标进程里的 `QT_IM_MODULE` 和 `QT_IM_MODULES`，查 Qt platforminputcontext 插件，查 fcitx Qt 输入法插件是否存在或被加载。

- SDL 应用：查目标进程里的 `SDL_IM_MODULE`，查 SDL 版本和 SDL 输入法路径是否可用。

- Electron / Chromium / CEF 应用：先确认它是 Wayland 原生运行还是 X11/XWayland 运行。以 Wayland 运行时，查桌面合成器和应用支持的 `text-input` 协议版本是否匹配，还要查 `--enable-wayland-ime`、`--wayland-text-input-version=3` 这类参数，确认是否有错误参数导致协议版本不匹配。以 X11/XWayland 运行时，查 GTK 模块路径和 XIM 路径，不要直接断言 Electron / Chromium / CEF 不支持 XIM。

## 模块和 locale 检查

收集完框架信息、环境变量和运行参数之后，要继续寻找目标进程已经加载的、以及系统中存在但目标进程没有加载的输入法 `.so` 文件。

重点查看：

- `/proc/<pid>/maps` 中目标进程已经加载的输入法相关库；
- `im-*.so`；
- `fcitx`；
- `ibus`；
- `xim`；
- GTK immodule cache；
- Qt input context 插件；
- Wayland frontend / input-method frontend；
- 这些模块的 locale 激活条件。

注意：软件支持某个输入模块，不代表启动后一定会加载它。只有当对应路径被实际选择、模块存在、环境变量或自动选择条件满足、locale 匹配、应用确实初始化输入上下文时，模块才可能加载。反过来，没加载某个 `.so` 也不能单独证明软件不支持这条路径，必须结合运行模式、框架、环境变量、模块存在性、locale 和实际输入行为一起判断。

## 路径判定

现在可以开始判断输入法路径是否跑通。下面任意一条路径跑通，输入法理论上就可以正常使用。

- Wayland 路径：软件以 Wayland 原生运行，桌面合成器和软件都支持兼容的 `text-input` 协议，输入法加载了对应的 Wayland frontend / input-method 相关模块，没有错误参数导致协议版本不匹配。

- GTK 模块路径：输入法运行正常，软件是 GTK 或会使用 GTK 输入模块，目标进程设置了正确的 `GTK_IM_MODULE` 或 GTK 自动选择到了 fcitx，`im-fcitx5.so` 存在或已经加载，并且 locale 满足模块激活条件。

- Qt 模块路径：输入法运行正常，软件是 Qt，目标进程设置了 `QT_IM_MODULE=fcitx`，或者 `QT_IM_MODULES` fallback 顺序中包含 fcitx，Qt fcitx platforminputcontext 插件存在或已经加载，并且 Qt 版本和当前平台后端支持这条路径。

- SDL 路径：输入法运行正常，软件使用 SDL，目标进程设置了 `SDL_IM_MODULE=fcitx`，SDL 版本支持对应输入法路径。

- XIM 协议路径：输入法运行正常，软件以 X11/XWayland 运行，目标进程设置了 `XMODIFIERS=@im=fcitx`，XIM frontend 可用，`im-xim.so` 存在或可用，并且目标进程 locale 满足 XIM 激活条件，不能是 `C` 或 `POSIX` 这类无效 locale。

- Electron / Chromium / CEF 特殊路径：以 Wayland 原生运行时走 `text-input` 协议路径；以 X11/XWayland 运行时走 GTK 模块路径和 XIM 协议路径。不要只凭 Electron / Chromium / CEF 这个名字下结论，必须看实际运行模式和运行时证据。

## 诊断示例

用户输入：“我的 Steam 没法用输入法，怎么办？”

诊断流程示例：

确定诊断目标为 Steam。查询到 Steam 使用 CEF / Chromium Embedded Framework。查询 Steam 当前运行模式，发现它以 XWayland 运行，按照路径分类，这意味着要重点检查 XIM 路径和 GTK 模块路径。

然后读取 Steam 目标进程环境变量，确认 `XMODIFIERS`、`GTK_IM_MODULE`、`LANG`、`LC_CTYPE` 等信息。继续检查 Steam 进程已经加载的输入法相关 `.so`，以及系统中存在但没有被 Steam 加载的 `im-*.so`。再检查 `im-xim.so` 的 locale 激活条件。

如果发现 Steam 以 XWayland 运行，目标进程设置了 `XMODIFIERS=@im=fcitx`，系统存在 `im-xim.so`，但是 Steam 的 `LC_CTYPE` 是 `en_US.UTF-8`，而 `im-xim.so` 的激活条件只匹配 zh、ja、ko，那么可以推断：XIM 路径可能因为 locale 不匹配没有激活。

如果同时发现 Steam 不支持或没有加载 `im-fcitx5.so`，则 GTK 模块路径也没有跑通。

这时诊断结论可以是：Steam 当前能走的主要路径是 XIM，但 XIM 很可能因为 locale 激活条件不满足而没有工作。解决方法是让 Steam 以合适的 CJK locale 启动，或者设置 `GTK_IM_MODULE=xim` 走GTK模块，然是让GTK成为 XIM 协议的入口。诊断完毕后正式输出报告。

## 输出格式

最终只输出诊断报告，不输出内部思考，不输出工具调用列表，不输出“以下是报告”这类元话语。

必须包含以下章节：

## 问题分析

说明目标软件、用户现象、运行模式、软件框架、输入法类型、当前已知环境。不知道的写“暂无”。

## 已确认事实

列出已经确认的证据，例如输入法是否运行、目标软件 PID、目标软件环境变量、运行模式、框架、已加载模块、可用模块、locale、官方 Wiki 规则等。

## 路径分析

逐条列出路径是否走通：

（以刚才的steam为例就是）

- Wayland 路径：未走通。Steam以XWayland运行。
- GTK 模块路径：未走通。以XWayland运行CEF框架，支持GTK环境变量，但Steam不存在也不加载`im-fcitx5.so`，路径在此断裂。
- Qt 模块路径：未走通。Steam不支持Qt。
- SDL 路径：未走通。虽然Steam是SDL开发，但其商店是CEF，与SDL无关。
- XIM 路径：未走通。Steam支持XIM路径，存在模块，但locale激活条件不满足导致未能走通。

每条路径都要说明依据，不要只写结论。

## 根因推断

按可能性排序。每个根因标注“已证实”“推断”或“缺证据”。

## 推荐修复

给出可执行方案。说明改哪里、为什么改、怎么回滚。涉及安装、删除、重启服务、kill 进程、改配置、清缓存等操作时，必须提示需要用户确认。

## 禁止事项

不要编造命令输出。不要编造目标进程环境变量。不要把当前 shell 环境变量当成目标进程环境变量。不要只凭进程名判断 Wayland/XWayland。不要只凭环境变量下结论。不要只凭 `.so` 没加载就断言不支持。不要执行安装、删除、重启服务、kill 进程、改配置等操作。
"#;

#[derive(Clone)]
struct DiagnosisContext {
    config: AppConfig,
    paths: MiyuPaths,
    tools: ToolRegistry,
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
        "linux_input_method_diagnose",
        "Run a Linux input method diagnosis sub-agent using runtime evidence, framework detection, display mode checks, and input method path analysis. / 运行 Linux 输入法诊断子代理，基于运行时证据、框架识别、显示模式和输入法路径分析输出诊断报告。",
        json!({
            "type": "object",
            "properties": {
                "issue": { "type": "string", "description": "Input method issue or symptom. / 输入法问题或现象。" },
                "target": { "type": "string", "description": "Optional target app/process name, e.g. steam or qq. / 可选目标应用或进程名，例如 steam 或 qq。" }
            },
            "required": ["issue"],
            "additionalProperties": false
        }),
        move |args, progress| {
            let context = context.clone();
            async move { run_linux_input_method_diagnose(args, context, progress).await }
        },
    ));
}

async fn run_linux_input_method_diagnose(
    args: Value,
    context: DiagnosisContext,
    progress: ToolProgress,
) -> Result<String> {
    if !context.config.plugins.deep_diagnose.enabled {
        bail!("linux input method diagnose plugin is disabled");
    }
    let issue = required(&args, "issue")?;
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    progress.report(format!(
        "{}=\"{}\"",
        t("issue", "问题"),
        clip_inline(&issue, 80)
    ));
    let client = OpenAiCompatibleClient::from_config(&context.config, &context.paths)?;
    let prompt = input_method_prompt(&issue, target.as_deref());
    let result = chat_with_tools(
        &client,
        vec![
            ChatMessage::system(INPUT_METHOD_DIAGNOSIS_PROMPT),
            ChatMessage::plain("user", prompt),
        ],
        context.tools,
        context
            .config
            .plugins
            .deep_diagnose
            .tool_call_timeout_seconds,
        &progress,
    )
    .await?;
    let final_answer = strip_report_preamble(&result.content);
    Ok(serde_json::to_string_pretty(&json!({
        "ok": true,
        "kind": "linux_input_method_diagnosis",
        "issue": issue,
        "target": target,
        "final_answer": final_answer,
        "output_instruction": "这是 Linux 输入法诊断子代理返回的最终诊断报告。请把 final_answer 当作主要依据回复用户；如果用户追问完整报告，直接复述 final_answer。"
    }))?)
}

async fn chat_with_tools(
    client: &OpenAiCompatibleClient,
    mut messages: Vec<ChatMessage>,
    tools: ToolRegistry,
    timeout_seconds: u64,
    progress: &ToolProgress,
) -> Result<ChatResult> {
    let definitions = tools.definitions_except(&[
        "linux_input_method_diagnose",
        "deep_research",
        "linux_game_compatibility",
    ]);
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
            progress.report(format!(
                "__subtool_call__{}",
                json!({
                    "name": call.function.name,
                    "args": call.function.arguments,
                })
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
            progress.report(format!(
                "__subtool_result__{}",
                json!({
                    "name": call.function.name,
                    "ok": ok,
                    "output": output,
                })
            ));
            messages.push(ChatMessage::tool(call.id, output));
        }
    }
}

fn input_method_prompt(issue: &str, target: Option<&str>) -> String {
    format!(
        "用户输入法问题：\n{issue}\n\n目标软件：{}\n\n请按照系统提示词中的流程完成诊断。优先调用 check_issue 收集输入法证据；如果目标软件框架或特殊行为不清楚，可以使用 fcitx5_input_method_wiki_qurey、知识库和网络搜索。最终只输出诊断报告。",
        target.unwrap_or("未明确，需从问题中推断")
    )
}

fn strip_report_preamble(content: &str) -> String {
    let trimmed = content.trim();
    for heading in ["## 问题分析", "# 问题分析"] {
        if let Some(index) = trimmed.find(heading) {
            return trimmed[index..].trim().to_string();
        }
    }
    trimmed
        .lines()
        .skip_while(|line| {
            let line = line.trim();
            line.is_empty()
                || line == "---"
                || line.contains("以下是")
                || line.contains("诊断报告") && line.len() < 20
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn required(args: &Value, key: &str) -> Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if value.is_empty() {
        bail!("missing required argument: {key}")
    }
    Ok(value.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_report_preamble() {
        assert_eq!(
            strip_report_preamble("以下是诊断报告\n\n## 问题分析\nSteam 输入法不可用"),
            "## 问题分析\nSteam 输入法不可用"
        );
    }
}
