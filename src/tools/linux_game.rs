use super::{ToolProgress, ToolRegistry, ToolSpec};
use crate::config::AppConfig;
use crate::llm::{ChatMessage, ChatResult, OpenAiCompatibleClient};
use crate::paths::MiyuPaths;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

const GAME_COMPATIBILITY_PROMPT: &str = r#"你是Linux 游戏兼容性调查子代理。

你的任务是调查用户询问的游戏能否在 Linux 上运行、怎么玩、是否有反作弊阻断、需要什么 Proton 版本或启动方式，并输出一份可以直接交给主智能体回复用户的最终调查报告。

## 核心流程

你必须按以下流程工作：

1. 首先调用 Linux 游戏兼容性基础信号采集工具，查询：
   - Steam / AppID / 游戏名匹配情况
   - ProtonDB 概览
   - Can I Play on Linux
   - AreWeAntiCheatYet

2. 根据基础信号做第一轮判断后进行如下操作：
   - 如果 ProtonDB 有该游戏记录，就优先查看 ProtonDB 的玩家报告/评论，因为评论通常包含：
     - 能不能启动
     - 用什么 Proton / GE-Proton
     - 是否需要启动参数
     - 性能表现
     - 崩溃、黑屏、启动器、音频、视频、手柄等问题
     - Steam Deck 体验
   - 如果 ProtonDB 没有该游戏，或者 ProtonDB 信息明显不足，使用网络搜索、知识库等其他信息搜集工具补查。

3. 在以下情况必须进行额外网络搜索：
   - 三个兼容性来源缺失或冲突；
   - 反作弊状态不明确；
   - 用户明确问性能、崩溃、Mod、启动器、Steam Deck、多人/联机；
   - ProtonDB 没有该游戏；
   - 近期有重大更新，旧信息可能过期。

4. 搜索必须克制：
   - 优先官方页面、ProtonDB、AreWeAntiCheatYet、Can I Play on Linux、PCGamingWiki、GitHub issue、Steam 社区、玩家社区、各平台近期玩家讨论。
   - 不要为了补全所有细节反复搜索。
   - 查不到就明确说不确定，不要编造。

## 判断规则

最终必须给出红绿灯结论：

- 🟢 可玩
- 🟡 不一定能玩
- 🔴 不可玩

以下是可以参考的判断规则：

1. ProtonDB Gold / Platinum 且没有反作弊阻断，通常可以倾向 🟢。
2. Can I Play on Linux 标记 Works，且 ProtonDB/玩家报告一致，通常可以倾向 🟢。
3. AreWeAntiCheatYet 标记 Running，说明反作弊目前社区层面可运行，但不等于承诺 Linux 支持。
4. AreWeAntiCheatYet 标记 Broken / Denied，且通常应为 🔴。
5. 来源冲突、反作弊状态不明、近期变化多、玩家报告分裂时，用 🟡表示不确定。
6. 单机可玩但多人不可玩，必须拆开说，不要笼统说“可玩”。
7. Steam Deck Playable 不等于桌面 Linux 完全没问题。
8. Can I Play on Linux 的 recommended Proton 是该来源记录的历史验证版本，不要说成“当前最新推荐 Proton”。
9. 如果用户问“怎么玩”，必须给出实际可执行路线，而不是只回答能不能玩。

## 必须区分的维度

调查时尽量区分：

- Steam 版 / 非 Steam 版
- 桌面 Linux / Steam Deck
- 单机 / 多人 / 在线
- 反作弊是否阻断
- Proton/Wine 版本
- 启动器问题
- 性能表现
- 崩溃、黑屏、音频、视频、手柄、Mod 等常见问题
- 官方支持、社区经验、玩家临时绕过方案之间的区别

## 禁止事项

- 不要编造来源。
- 不要编造 Proton 版本。
- 不要编造 FPS。
- 不要编造官方声明。
- 不要编造封号案例。
- 不要把社区经验说成官方保证。
- 不要把“目前能玩”说成“永远稳定可玩”。
- 不要把“Steam Deck Playable”说成“Valve Verified”。
- 不要因为某个来源缺失就直接断言不可玩。

## 输出格式

最终只输出调查报告，不输出内部思考，不输出工具调用过程，不输出“以下是最终报告”这类元话语，不要在开头加分割线。

报告必须包含以下章节：

## 调查结果

第一行必须是红绿灯结论，例如：

🟢 Wuthering Waves 可玩

或：

🟡 Apex Legends 不一定能玩

然后用 1-3 句话说明总体判断。

## 依据

列出关键证据。可以使用项目符号或表格。

每条证据要说明：
- 来源
- 关键信息
- 支撑了什么判断
- 如果能确认时间或时效性，也要写出来

如果来源冲突，必须单独说明冲突点和你的取舍。

## 怎么玩

必须给出可执行路线。

根据实际情况可能包含：

- Steam 安装方式
- Proton/Wine 版本选择
- 是否需要启动参数
- 是否需要第三方启动器
- 是否需要 Flatpak / AUR / Heroic / Lutris
- 第一次启动要注意什么

## 注意事项

必须说明风险：

- 反作弊更新风险
- 官方未承诺 Linux 支持
- 账号/ToS 风险
- Steam Deck 与桌面 Linux 差异
- 非 Steam 版本差异
- 性能不确定性
- 来源过期风险

只有在有明确证据时，才额外添加：

## 性能表现

不要编造 FPS。没有 FPS、硬件、画质、Steam Deck 或 Windows 对比数据时，不要写这个章节。
"#;

const OUTPUT_INSTRUCTION: &str = r#"这是 Linux 游戏兼容性调查子代理返回的最终报告。

请把 final_report 当作主要依据回复用户。不要重新编造兼容性结论。

回复时保留以下核心信息：
- 红绿灯结论，能不能玩
- 怎么玩
- 注意事项

如果用户问“怎么玩”，必须给出可执行步骤。
如果用户追问“刚才完整报告”，直接复述 final_report。"#;

#[derive(Clone)]
struct GameCompatibilityContext {
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
    let context = GameCompatibilityContext {
        config,
        paths,
        tools,
    };
    registry.register(ToolSpec::new_with_progress(
        "linux_game_compatibility",
        "Run the Linux game compatibility investigation sub-agent and return its final report. / 运行 Linux 游戏兼容性调查子代理并返回最终报告。",
        json!({"type":"object","properties":{"game":{"type":"string","description":"Game title. / 游戏名称。"},"issue":{"type":"string","description":"Optional issue such as crash, multiplayer, anti-cheat, performance, mods. / 可选关注点，例如崩溃、多人、反作弊、性能、Mod。"}},"required":["game"],"additionalProperties":false}),
        move |args, progress| {
            let context = context.clone();
            async move { linux_game_compatibility(args, context, progress).await }
        },
    ));
}

async fn linux_game_compatibility(
    args: Value,
    context: GameCompatibilityContext,
    progress: ToolProgress,
) -> Result<String> {
    let game = required(&args, "game")?;
    let issue = args
        .get("issue")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    progress.report(format!("{}: {}", "Linux 游戏兼容性", game));
    let client = OpenAiCompatibleClient::from_config(&context.config, &context.paths)?;
    let prompt = format!(
        "用户问题：\n游戏：{game}\n关注点：{}\n\n请按系统提示词流程完成调查。第一步必须调用 gather_linux_game_compatibility_signals。最终只输出调查报告。",
        if issue.trim().is_empty() { "未明确" } else { &issue }
    );
    let report = chat_with_tools(
        &client,
        vec![
            ChatMessage::system(GAME_COMPATIBILITY_PROMPT),
            ChatMessage::plain("user", prompt),
        ],
        game_tool_registry(&context),
        &progress,
    )
    .await?
    .content;
    let report = strip_report_preamble(&report);
    Ok(serde_json::to_string_pretty(&json!({
        "ok": true,
        "kind": "linux_game_compatibility",
        "game_query": game,
        "final_report": report,
        "output_instruction": OUTPUT_INSTRUCTION,
    }))?)
}

fn game_tool_registry(context: &GameCompatibilityContext) -> ToolRegistry {
    let mut registry = context.tools.clone();
    registry.register(ToolSpec::new(
        "gather_linux_game_compatibility_signals",
        "Gather Steam, ProtonDB, Can I Play on Linux, and AreWeAntiCheatYet compatibility signals for one game. / 收集单个游戏在 Steam、ProtonDB、Can I Play on Linux、AreWeAntiCheatYet 上的兼容性信号。",
        json!({"type":"object","properties":{"game":{"type":"string","description":"Game title. / 游戏名称。"},"issue":{"type":"string","description":"Optional issue such as crash, multiplayer, anti-cheat, performance, mods. / 可选关注点，例如崩溃、多人、反作弊、性能、Mod。"}},"required":["game"],"additionalProperties":false}),
        |args| async move { gather_linux_game_compatibility_signals(args).await },
    ));
    registry
}

async fn chat_with_tools(
    client: &OpenAiCompatibleClient,
    mut messages: Vec<ChatMessage>,
    tools: ToolRegistry,
    progress: &ToolProgress,
) -> Result<ChatResult> {
    let definitions = tools.definitions_except(&["linux_game_compatibility", "deep_research"]);
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
            let (output, ok) = match tools.call(&call.function.name, &call.function.arguments).await {
                Ok(output) => (output, true),
                Err(err) => (format!("tool error: {err}"), false),
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

fn strip_report_preamble(content: &str) -> String {
    let trimmed = content.trim();
    for heading in ["## 调查结果", "# 调查结果"] {
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
                || line.contains("最终报告") && line.len() < 30
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn gather_linux_game_compatibility_signals(args: Value) -> Result<String> {
    let game = required(&args, "game")?;
    let candidates = game_candidates(&game);
    let search_game = candidates.first().cloned().unwrap_or_else(|| game.clone());
    let issue = args
        .get("issue")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("miyu-linux-game-compatibility/0.1")
        .build()?;
    let (steam, steam_attempts) = steam_search_candidates(&client, &candidates).await;
    let appid = steam["appid"].as_u64();
    let steam_name = steam["name"].as_str().unwrap_or(&game).to_string();
    let mut slug_candidates = slug_candidates(&candidates);
    if appid.is_some() {
        slug_candidates.insert(0, slugify(&steam_name));
    }
    slug_candidates.sort();
    slug_candidates.dedup();
    let protondb = if let Some(appid) = appid {
        fetch_json(
            &client,
            &format!("https://www.protondb.com/api/v1/reports/summaries/{appid}.json"),
        )
        .await
        .ok()
    } else {
        None
    };
    let can_i_play_result = fetch_first_text(&client, &slug_candidates, |slug| {
        format!("https://caniplayonlinux.com/games/{slug}/")
    })
    .await;
    let anticheat_result = fetch_first_text(&client, &slug_candidates, |slug| {
        format!("https://areweanticheatyet.com/game/{slug}")
    })
    .await;
    let can_i_play = can_i_play_result.text.as_deref();
    let anticheat = anticheat_result.text.as_deref();
    let verdict = verdict(&protondb, can_i_play, anticheat, &issue);
    let confidence = compatibility_confidence(appid, &protondb, can_i_play, anticheat, &verdict);
    let needs_followup = confidence["needs_followup"].as_bool().unwrap_or(true);
    Ok(serde_json::to_string_pretty(&json!({
        "ok": true,
        "game_query": game,
        "search_query": search_game,
        "query_candidates": candidates,
        "matched_name": steam_name,
        "steam": steam,
        "source_attempts": {
            "steam": steam_attempts,
            "can_i_play_on_linux": can_i_play_result.attempts,
            "are_we_anticheat_yet": anticheat_result.attempts,
        },
        "verdict": verdict,
        "confidence": confidence,
        "needs_followup": needs_followup,
        "protondb": protondb,
        "can_i_play_on_linux": can_i_play.map(extract_can_i_play_summary),
        "are_we_anticheat_yet": anticheat.map(extract_anticheat_summary),
        "sources": {
            "steam": appid.map(|id| format!("https://store.steampowered.com/app/{id}/")),
            "protondb": appid.map(|id| format!("https://www.protondb.com/app/{id}")),
            "can_i_play_on_linux": can_i_play_result.url,
            "are_we_anticheat_yet": anticheat_result.url,
        },
        "methodology": "If ProtonDB exists, use ProtonDB reports/comments as the primary practical playability signal. If ProtonDB is missing or insufficient, continue with web_search/web_fetch outside this tool. Keep final answer concise and include 调查结果, 依据, 怎么玩, 注意事项.",
    }))?)
}

#[derive(Default)]
struct TextFetchResult {
    text: Option<String>,
    url: Option<String>,
    attempts: Vec<Value>,
}

fn game_candidates(game: &str) -> Vec<String> {
    let normalized = normalize_game_query(game);
    let mut candidates = vec![normalized];
    candidates.retain(|candidate| !candidate.trim().is_empty());
    candidates.sort();
    candidates.dedup();
    candidates
}

fn slug_candidates(candidates: &[String]) -> Vec<String> {
    let mut slugs = candidates
        .iter()
        .map(|candidate| slugify(candidate))
        .filter(|slug| !slug.is_empty())
        .collect::<Vec<_>>();
    slugs.sort();
    slugs.dedup();
    slugs
}

fn normalize_game_query(game: &str) -> String {
    let compact = game
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.contains("赛博朋克2077")
        || compact.contains("电驭叛客2077")
        || compact.contains("cyberpunk2077")
    {
        return "Cyberpunk 2077".to_string();
    }
    if compact.contains("原神") || compact.contains("genshinimpact") {
        return "Genshin Impact".to_string();
    }
    game.trim().to_string()
}

async fn steam_search_candidates(
    client: &reqwest::Client,
    candidates: &[String],
) -> (Value, Vec<Value>) {
    let mut attempts = Vec::new();
    for candidate in candidates {
        match steam_search(client, candidate).await {
            Ok(value) => {
                attempts.push(json!({"query": candidate, "ok": true, "appid": value["appid"], "name": value["name"]}));
                return (value, attempts);
            }
            Err(err) => {
                attempts.push(json!({"query": candidate, "ok": false, "error": err.to_string()}))
            }
        }
    }
    (Value::Null, attempts)
}

async fn fetch_first_text<F>(
    client: &reqwest::Client,
    slugs: &[String],
    url_for_slug: F,
) -> TextFetchResult
where
    F: Fn(&str) -> String,
{
    let mut result = TextFetchResult::default();
    for slug in slugs {
        let url = url_for_slug(slug);
        match fetch_text(client, &url).await {
            Ok(text) => {
                result
                    .attempts
                    .push(json!({"slug": slug, "url": url, "ok": true}));
                result.url = Some(url);
                result.text = Some(text);
                return result;
            }
            Err(err) => result
                .attempts
                .push(json!({"slug": slug, "url": url, "ok": false, "error": err.to_string()})),
        }
    }
    result
}

async fn steam_search(client: &reqwest::Client, game: &str) -> Result<Value> {
    let value: Value = client
        .get("https://store.steampowered.com/api/storesearch/")
        .query(&[("term", game), ("l", "english"), ("cc", "US")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let item = value["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Steam app not found for {game}"))?;
    Ok(json!({"appid": item["id"], "name": item["name"], "url": item["tiny_image"]}))
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

fn verdict(
    protondb: &Option<Value>,
    can_i_play: Option<&str>,
    anticheat: Option<&str>,
    issue: &str,
) -> Value {
    let issue_lower = issue.to_ascii_lowercase();
    let multiplayer_sensitive = issue_lower.contains("multi")
        || issue_lower.contains("online")
        || issue.contains("联机")
        || issue.contains("多人")
        || issue.contains("反作弊");
    let anticheat_denied = anticheat
        .map(|text| text.contains("Denied") || text.contains("Broken"))
        .unwrap_or(false);
    if multiplayer_sensitive && anticheat_denied {
        return json!({"traffic_light":"🔴", "label":"不可玩", "reason":"anti-cheat denied or broken for multiplayer/online use"});
    }
    if can_i_play
        .map(|text| text.contains("Broken"))
        .unwrap_or(false)
    {
        return json!({"traffic_light":"🔴", "label":"不可玩", "reason":"Can I Play on Linux marks it broken"});
    }
    let tier = protondb
        .as_ref()
        .and_then(|value| value["tier"].as_str())
        .unwrap_or_default();
    if matches!(tier, "platinum" | "gold")
        || can_i_play
            .map(|text| text.contains("Works"))
            .unwrap_or(false)
    {
        return json!({"traffic_light":"🟢", "label":"可玩", "reason":"ProtonDB/Can I Play on Linux indicate it works"});
    }
    if matches!(tier, "silver" | "bronze")
        || can_i_play
            .map(|text| text.contains("Partial"))
            .unwrap_or(false)
    {
        return json!({"traffic_light":"🟡", "label":"不一定能玩", "reason":"partial or lower confidence compatibility"});
    }
    json!({"traffic_light":"🟡", "label":"不一定能玩", "reason":"insufficient compatibility data"})
}

fn compatibility_confidence(
    appid: Option<u64>,
    protondb: &Option<Value>,
    can_i_play: Option<&str>,
    anticheat: Option<&str>,
    verdict: &Value,
) -> Value {
    let tier = protondb
        .as_ref()
        .and_then(|value| value["tier"].as_str())
        .unwrap_or_default();
    let has_protondb = protondb.is_some();
    let has_can_i_play = can_i_play.is_some();
    let has_anticheat = anticheat.is_some();
    let can_i_play_works = can_i_play
        .map(|text| text.contains("Works"))
        .unwrap_or(false);
    let can_i_play_partial = can_i_play
        .map(|text| text.contains("Partial"))
        .unwrap_or(false);
    let reason = verdict["reason"].as_str().unwrap_or_default();
    let mut reasons = Vec::new();
    if appid.is_none() {
        reasons.push("Steam app id was not found");
    }
    if !has_protondb {
        reasons.push("ProtonDB data is missing");
    }
    if !has_can_i_play {
        reasons.push("Can I Play on Linux data is missing");
    }
    if !has_anticheat {
        reasons.push("AreWeAntiCheatYet data is missing");
    }
    if reason.contains("insufficient") {
        reasons.push("compatibility data is insufficient");
    }

    let confidence = if appid.is_some()
        && matches!(tier, "platinum" | "gold")
        && can_i_play_works
        && has_anticheat
    {
        "high"
    } else if matches!(tier, "platinum" | "gold" | "silver" | "bronze")
        || can_i_play_partial
        || can_i_play_works
    {
        "medium"
    } else {
        "low"
    };
    let needs_followup =
        confidence == "low" || reason.contains("insufficient") || !reasons.is_empty();
    json!({
        "level": confidence,
        "needs_followup": needs_followup,
        "followup_reason": if reasons.is_empty() { Value::Null } else { json!(reasons.join("; ")) },
        "source_coverage": {
            "steam_appid": appid.is_some(),
            "protondb": has_protondb,
            "can_i_play_on_linux": has_can_i_play,
            "are_we_anticheat_yet": has_anticheat
        },
        "suggested_followup_queries": [
            "ProtonDB game compatibility latest reports",
            "PCGamingWiki Linux Proton known issues",
            "Steam Community Linux Proton performance issues"
        ]
    })
}

fn extract_can_i_play_summary(html: &str) -> Value {
    let text = html2text::from_read(html.as_bytes(), 120);
    json!({
        "works": text.contains("Works"),
        "partial": text.contains("Partial"),
        "broken": text.contains("Broken"),
        "source_recommended_proton": value_after_label(&text, "Recommended Proton"),
        "steam_deck_verified": text.contains("Steam Deck Verified"),
        "known_issues": section_excerpt(&text, "Known issues", "Fixes", 1200),
        "fixes": section_excerpt(&text, "Fixes", "Verdict", 1200),
        "text_excerpt": excerpt(&text, 2000),
    })
}

fn extract_anticheat_summary(html: &str) -> Value {
    let text = html2text::from_read(html.as_bytes(), 120);
    let status = ["Supported", "Running", "Planned", "Broken", "Denied"]
        .into_iter()
        .find(|status| text.contains(status));
    json!({
        "status": status,
        "mentions_eac": text.contains("Easy Anti-Cheat"),
        "mentions_battleye": text.contains("BattlEye"),
        "text_excerpt": excerpt(&text, 1600),
    })
}

fn value_after_label(text: &str, label: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    while let Some(line) = lines.next() {
        if line == label {
            return lines.next().map(|value| value.chars().take(120).collect());
        }
    }
    None
}

fn section_excerpt(text: &str, start: &str, end: &str, max_chars: usize) -> Option<String> {
    let after = text.split(start).nth(1)?;
    let section = after.split(end).next().unwrap_or(after);
    Some(excerpt(section, max_chars))
}

fn excerpt(text: &str, max_chars: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugifies_game_names() {
        assert_eq!(slugify("Cyberpunk 2077"), "cyberpunk-2077");
        assert_eq!(
            slugify("Tom Clancy's Rainbow Six® Siege"),
            "tom-clancy-s-rainbow-six-siege"
        );
    }

    #[test]
    fn normalizes_chinese_cyberpunk_query() {
        assert_eq!(normalize_game_query("赛博朋克2077"), "Cyberpunk 2077");
        assert_eq!(
            normalize_game_query("Linux能玩赛博朋克2077吗"),
            "Cyberpunk 2077"
        );
    }

    #[test]
    fn normalizes_chinese_genshin_query() {
        assert_eq!(normalize_game_query("原神"), "Genshin Impact");
        assert!(game_candidates("linux能玩原神吗")
            .iter()
            .any(|candidate| candidate == "Genshin Impact"));
        assert_eq!(slugify("Genshin Impact"), "genshin-impact");
        assert_eq!(
            slug_candidates(&game_candidates("linux能玩原神吗")),
            vec!["genshin-impact"]
        );
    }

    #[test]
    fn output_instruction_mentions_final_report() {
        assert!(OUTPUT_INSTRUCTION.contains("final_report"));
        assert!(OUTPUT_INSTRUCTION.contains("红绿灯"));
        assert!(OUTPUT_INSTRUCTION.contains("怎么"));
    }

    #[test]
    fn insufficient_data_requires_followup() {
        let result = verdict(&None, None, None, "");
        assert_eq!(result["label"], "不一定能玩");
        let confidence = compatibility_confidence(None, &None, None, None, &result);
        assert_eq!(confidence["level"], "low");
        assert_eq!(confidence["needs_followup"], true);
    }

    #[test]
    fn strong_cross_source_signal_is_high_confidence() {
        let protondb = Some(json!({"tier":"gold"}));
        let result = verdict(&protondb, Some("Works"), None, "");
        let confidence = compatibility_confidence(
            Some(1091500),
            &protondb,
            Some("Works"),
            Some("Running"),
            &result,
        );
        assert_eq!(result["label"], "可玩");
        assert_eq!(confidence["level"], "high");
        assert_eq!(confidence["needs_followup"], false);
    }

    #[test]
    fn genshin_can_i_play_and_anticheat_indicate_playable() {
        let result = verdict(
            &None,
            Some("Genshin Impact Works Yes — runs via Proton"),
            Some("Genshin Impact Running AntiCheat"),
            "",
        );
        assert_eq!(result["label"], "可玩");
        let confidence =
            compatibility_confidence(None, &None, Some("Works"), Some("Running"), &result);
        assert_eq!(confidence["level"], "medium");
        assert_eq!(confidence["needs_followup"], true);
    }

    #[test]
    fn single_source_signal_still_suggests_followup() {
        let protondb = Some(json!({"tier":"gold"}));
        let result = verdict(&protondb, None, None, "");
        let confidence = compatibility_confidence(Some(1091500), &protondb, None, None, &result);
        assert_eq!(confidence["level"], "medium");
        assert_eq!(confidence["needs_followup"], true);
    }

    #[test]
    fn anticheat_denied_blocks_multiplayer_verdict() {
        let result = verdict(
            &None,
            None,
            Some("Apex Legends Denied Easy Anti-Cheat"),
            "多人",
        );
        assert_eq!(result["traffic_light"], "🔴");
    }

    #[test]
    fn gold_protondb_is_playable() {
        let result = verdict(&Some(json!({"tier":"gold"})), None, None, "");
        assert_eq!(result["traffic_light"], "🟢");
    }

    #[test]
    fn can_i_play_marks_recommended_proton_as_source_value() {
        let summary = extract_can_i_play_summary(
            "<p>Works</p><p>Recommended Proton</p><p>Proton 9.0-3</p><p>Steam Deck Verified</p>",
        );
        assert_eq!(summary["source_recommended_proton"], "Proton 9.0-3");
        assert!(summary.get("recommended_proton").is_none());
    }
}
