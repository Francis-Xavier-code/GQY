//! Interactive REPL: input editing, live streaming display, and slash commands.
use super::*;
use crate::agent::{Agent, AgentEvent, AgentMode, AgentTurnControl};
use crate::config::AppConfig;
use crate::llm::LlmClient;
use crate::paths::GqyPaths;
use crate::render;
use crate::state::StateStore;
use anyhow::Result;
use crossterm::cursor::{self, Hide, Show};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use crossterm::{execute, queue};
use std::io::{self, Write};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use vte::{Params as VteParams, Parser as VteParser, Perform as VtePerform};

const REPL_MAX_VISIBLE_INPUT_ROWS: u16 = 12;
const REPL_PASTE_PLACEHOLDER_MIN_LINES: usize = 3;
const REPL_PASTE_PLACEHOLDER_MIN_CHARS: usize = 150;
#[derive(Clone, Debug)]
pub(crate) struct PastedText {
    text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ReplFooterStatus {
    provider: String,
    model: String,
    mixed_models: bool,
    thinking: Option<String>,
    token_usage: ReplTokenUsage,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplTokenUsage {
    turn_tokens: u64,
    session_tokens: u64,
    context_window: Option<usize>,
    cumulative_tokens: Option<u64>,
}

impl ReplFooterStatus {
    fn from_config(
        config: &AppConfig,
        session_tokens: u64,
        cumulative_tokens: Option<u64>,
    ) -> Self {
        let active = config.active_provider_model_choices();
        let mixed_models = active.len() > 1;
        let (provider_id, model) = match active.as_slice() {
            [] => ("-".to_string(), t("None", "无").to_string()),
            [choice] => (
                choice.provider_id.clone(),
                short_model_name(&choice.model, &choice.provider_id),
            ),
            _ => ("mixed".to_string(), t("Mixed", "混合").to_string()),
        };

        Self {
            model,
            provider: provider_id,
            mixed_models,
            thinking: None,
            token_usage: ReplTokenUsage {
                turn_tokens: 0,
                session_tokens,
                context_window: config.active_context_window().ok().flatten(),
                cumulative_tokens,
            },
        }
    }

    fn update_token_usage(
        &mut self,
        result: &crate::llm::ChatResult,
        session_tokens: u64,
        context_window: Option<usize>,
        cumulative_tokens: Option<u64>,
    ) {
        if let Some(usage) = &result.usage {
            self.set_token_usage(
                render::usage_total(usage),
                session_tokens,
                context_window,
                cumulative_tokens,
            );
        }
    }

    fn set_token_usage(
        &mut self,
        turn_tokens: u64,
        session_tokens: u64,
        context_window: Option<usize>,
        cumulative_tokens: Option<u64>,
    ) {
        self.token_usage = ReplTokenUsage {
            turn_tokens,
            session_tokens,
            context_window,
            cumulative_tokens,
        };
    }

    fn update_session_tokens(&mut self, session_tokens: u64) {
        self.token_usage.session_tokens = session_tokens;
    }

    fn update_context_window(&mut self, context_window: Option<usize>) {
        self.token_usage.context_window = context_window;
    }

    fn reset_token_usage(&mut self, session_tokens: u64, context_window: Option<usize>) {
        self.token_usage = ReplTokenUsage {
            turn_tokens: 0,
            session_tokens,
            context_window,
            cumulative_tokens: None,
        };
    }

    fn update_thinking_variant(&mut self, variant: Option<&str>) {
        self.thinking = if self.mixed_models {
            None
        } else {
            variant.map(str::to_string)
        };
    }
}

pub(crate) fn short_model_name(model: &str, provider: &str) -> String {
    model
        .strip_prefix(&format!("{provider}/"))
        .unwrap_or(model)
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_string()
}

pub(crate) fn repl_footer_line(mode: AgentMode, footer: &ReplFooterStatus, cols: usize) -> String {
    let cols = cols.max(1);
    let bar = input_prompt_bar(mode);
    let bar_width = visible_width(&bar);

    // 左侧：模式标签 + 模型名
    let mode_label = colored_footer_mode_label(mode);
    let model = if footer.model.is_empty() {
        String::new()
    } else {
        format!(" \x1b[2m{}\x1b[0m", footer.model)
    };
    let left = format!("{mode_label}{model}");
    let left_width = visible_width(&left);

    // 右侧：token 用量
    let usage = footer.token_usage;
    let right_plain = render::format_token_usage_inline(
        usage.turn_tokens,
        usage.session_tokens,
        usage.context_window,
        usage.cumulative_tokens,
    );
    let right = format!("\x1b[2m{right_plain}\x1b[0m");
    let right_width = visible_width(&right);

    let gap = cols
        .saturating_sub(bar_width.saturating_add(left_width).saturating_add(right_width))
        .max(1);
    format!("{bar}{left}{}{right}", " ".repeat(gap))
}

#[allow(dead_code)]
fn repl_footer_left(mode: AgentMode, footer: &ReplFooterStatus, width: usize) -> String {
    let thinking = footer.thinking.as_deref().unwrap_or_default();
    let colored_thinking = (!thinking.is_empty()).then(|| primary_footer_text(thinking));
    let colored_thinking = colored_thinking.as_deref().unwrap_or_default();
    // 默认 opencode 公共服务不显示，自定义供应商才显示（更清爽）
    let provider = if footer.provider.is_empty() || footer.provider == "opencode" {
        String::new()
    } else {
        format!("\x1b[2m{}\x1b[0m", footer.provider)
    };
    let mode = colored_footer_mode_label(mode);
    let full = repl_footer_left_parts(&mode, &footer.model, Some(&provider), colored_thinking);
    if visible_width(&full) <= width {
        return full;
    }

    let compact = repl_footer_left_parts(&mode, &footer.model, None, colored_thinking);
    if visible_width(&compact) <= width {
        return compact;
    }

    let fixed_width =
        visible_width(&mode)
            .saturating_add(3)
            .saturating_add(if thinking.is_empty() {
                0
            } else {
                3 + visible_width(colored_thinking)
            });
    let model_budget = width.saturating_sub(fixed_width).max(1);
    let model = truncate_display(&footer.model, model_budget);
    repl_footer_left_parts(&mode, &model, None, colored_thinking)
}

#[allow(dead_code)]
fn repl_footer_left_parts(
    mode: &str,
    model: &str,
    provider: Option<&str>,
    thinking: &str,
) -> String {
    let mut endpoint = model.to_string();
    if let Some(provider) = provider.filter(|provider| !provider.is_empty()) {
        if !endpoint.is_empty() {
            endpoint.push(' ');
        }
        endpoint.push_str(provider);
    }
    let mut parts = vec![mode.to_string(), endpoint];
    if !thinking.is_empty() {
        parts.push(thinking.to_string());
    }
    parts.join(" · ")
}

pub(crate) fn print_mixed_model_endpoint(show: bool, result: &crate::llm::ChatResult, variant: Option<&str>) {
    if !show {
        return;
    }
    let provider = result.provider_id.as_deref().unwrap_or("-");
    let model = result.model.as_deref().unwrap_or("-");
    println!(
        "\x1b[2m{}\x1b[0m\n",
        mixed_model_endpoint_label(provider, model, variant)
    );
}

pub(crate) fn mixed_model_endpoint_label(provider: &str, model: &str, variant: Option<&str>) -> String {
    let variant = variant
        .filter(|variant| !variant.is_empty())
        .map(|variant| format!(" · {variant}"))
        .unwrap_or_default();
    format!("{provider} / {model}{variant}")
}

pub(crate) fn show_mixed_model_endpoint(config: &AppConfig, interactive: bool) -> bool {
    config.active_provider_model_choices().len() > 1
        && match config.display.mixed_model_endpoint_display.as_str() {
            "off" => false,
            "all" => true,
            _ => interactive,
        }
}

fn colored_footer_mode_label(mode: AgentMode) -> String {
    let label = mode.label();
    // 月夜主题：带背景色的模式标签
    match mode {
        AgentMode::Normal => format!("\x1b[48;2;30;58;138m\x1b[38;2;203;213;225m {label} \x1b[0m"),
        AgentMode::Plan => format!("\x1b[48;2;124;58;237m\x1b[38;2;237;233;254m {label} \x1b[0m"),
        AgentMode::Chat => format!("\x1b[48;2;167;139;250m\x1b[38;2;15;23;42m {label} \x1b[0m"),
    }
}

fn primary_footer_text(text: &str) -> String {
    format!("\x1b[1m\x1b[34m{text}\x1b[0m")
}

pub async fn run_repl(paths: &GqyPaths, initial_mode: AgentMode) -> Result<()> {
    let _cursor_restore = ReplCursorRestore;
    AppConfig::init_files(paths)?;
    let mut config = AppConfig::load_or_default(paths)?;
    let state = StateStore::new(paths)?;
    state.init_files()?;
    let mut client = LlmClient::from_config(&config, paths)?;
    let mut mode = initial_mode;
    let mut input_history = load_repl_input_history(&state)?;
    let mut prefill = None::<String>;
    let mut live_repl = None::<LiveReplTail>;

    // 启动 banner：居中图片 + 状态信息
    {
        let active = config.active_provider_model_choices();
        let (provider, model) = match active.as_slice() {
            [] => ("-".to_string(), "-".to_string()),
            [choice] => (choice.provider_id.clone(), choice.model.clone()),
            _ => ("mixed".to_string(), "mixed".to_string()),
        };
        let mode_str = match mode {
            AgentMode::Normal => t("normal", "普通"),
            AgentMode::Plan => t("plan", "计划"),
            AgentMode::Chat => t("chat", "闲聊"),
        };
        crate::repl_avatar::print_startup_banner(
            &mut std::io::stdout(),
            &provider,
            &model,
            mode_str,
            0, // memory count — agent 尚未初始化
            0, // kb count
        );
    }

    let mut cumulative_tokens = 0u64;
    let mut show_shortcut_hint = true;
    let initial_registry =
        build_tool_registry(&config, paths, mode, crate::question_tui::available(false))?;
    let mut agent = Agent::new(
        config.clone(),
        paths,
        state.clone(),
        client.clone(),
        initial_registry,
        mode,
    )?;
    crate::pi_bridge::ensure_for_agent(&agent, None, None).await?;
    let mut footer =
        ReplFooterStatus::from_config(&config, agent.effective_context_tokens()?, None);
    let thinking_summary = client.thinking_variant_summary();
    footer.update_thinking_variant(thinking_summary.as_deref());
    footer.update_context_window(agent.context_window());
    loop {
        let thinking_summary = client.thinking_variant_summary();
        footer.update_thinking_variant(thinking_summary.as_deref());
        let next_input = if let Some(live) = live_repl.as_mut() {
            live.set_footer(footer.clone());
            read_live_repl_input(live, paths)?
        } else {
            read_repl_input(
                paths,
                mode,
                prefill.take(),
                &input_history,
                &footer,
                show_shortcut_hint,
            )?
        };
        let (input, pasted_images) = match next_input {
            Some((new_mode, input, pasted_images)) => {
                mode = new_mode;
                (input, pasted_images)
            }
            None => break,
        };
        let input = input.trim();
        let (command_input, command_args) = split_repl_command(input);
        let command = resolve_repl_command(command_input);
        let command_args_empty = command_args.trim().is_empty();
        if input.eq_ignore_ascii_case("exit")
            || input.eq_ignore_ascii_case("quit")
            || (command.eq_ignore_ascii_case("/exit") && command_args_empty)
        {
            break;
        }
        if command.eq_ignore_ascii_case("/help") && command_args_empty {
            print_repl_help();
            continue;
        }
        if command.eq_ignore_ascii_case("/models") && command_args_empty {
            run_models(paths, ModelsArgs { index: None })?;
            reload_repl_config(paths, &mut config, &mut client)?;
            footer = ReplFooterStatus::from_config(
                &config,
                agent.effective_context_tokens()?,
                (cumulative_tokens > 0).then_some(cumulative_tokens),
            );
            let thinking_summary = client.thinking_variant_summary();
            footer.update_thinking_variant(thinking_summary.as_deref());
            let registry =
                build_tool_registry(&config, paths, mode, crate::question_tui::available(false))?;
            agent.reload_config(config.clone(), client.clone())?;
            agent.switch_mode(mode, registry);
            footer.update_context_window(agent.context_window());
            println!("{}", t("configuration reloaded", "配置已重新加载"));
            println!();
            continue;
        }
        if command.eq_ignore_ascii_case("/config") && command_args_empty {
            crate::config_tui::run(paths)?;
            reload_repl_config(paths, &mut config, &mut client)?;
            footer = ReplFooterStatus::from_config(
                &config,
                agent.effective_context_tokens()?,
                (cumulative_tokens > 0).then_some(cumulative_tokens),
            );
            let thinking_summary = client.thinking_variant_summary();
            footer.update_thinking_variant(thinking_summary.as_deref());
            let registry =
                build_tool_registry(&config, paths, mode, crate::question_tui::available(false))?;
            agent.reload_config(config.clone(), client.clone())?;
            agent.switch_mode(mode, registry);
            footer.update_context_window(agent.context_window());
            println!("{}", t("configuration reloaded", "配置已重新加载"));
            println!();
            continue;
        }
        if command.eq_ignore_ascii_case("/variant") {
            if !crate::models_cache::is_loaded() {
                println!(
                    "{}\n",
                    t(
                        "model metadata is still loading; try /variant again shortly",
                        "模型元数据仍在加载，请稍后重试 /variant"
                    )
                );
                continue;
            }
            let selected = command_args.trim();
            match execute_variant(
                paths,
                &mut client,
                (!selected.is_empty()).then_some(selected),
                "/variant",
            )? {
                VariantOutcome::Updated => {
                    let thinking_summary = client.thinking_variant_summary();
                    footer.update_thinking_variant(thinking_summary.as_deref());
                    agent.replace_client(client.clone());
                    print_variant_updated();
                }
                VariantOutcome::Cancelled => {}
                VariantOutcome::Rejected(message) => {
                    eprintln!("\x1b[31m{message}\x1b[0m");
                }
            }
            continue;
        }
        if command.eq_ignore_ascii_case("/undo") && command_args_empty {
            let (removed, prompt) = state.undo_last_turn()?;
            footer.update_session_tokens(agent.effective_context_tokens()?);
            if removed > 0 && prompt.is_none() {
                println!("{}", t("context compaction undone", "已撤销上下文压缩"));
            } else {
                println!("{}: {removed}", t("undone messages", "已撤销消息数"));
            }
            if let Some(prompt) = prompt {
                if let Some(live) = live_repl.as_mut() {
                    live.editor.input = prompt;
                    live.editor.cursor = live.editor.input.chars().count();
                    live.editor.history_clean_index = None;
                } else {
                    prefill = Some(prompt);
                }
            }
            continue;
        }
        if command.eq_ignore_ascii_case("/pop") {
            let count = match parse_repl_pop_count(command_args) {
                Ok(count) => count,
                Err(err) => {
                    eprintln!("\x1b[31m{}: {err}\x1b[0m", t("error", "错误"));
                    continue;
                }
            };
            state.recover_stale_turns()?;
            match execute_pop(paths, &config, &state, count) {
                Ok(Some(outcome)) => {
                    print_pop_outcome(outcome);
                    footer.update_session_tokens(agent.effective_context_tokens()?);
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("\x1b[31m{}: {err}\x1b[0m", t("error", "错误"));
                }
            }
            continue;
        }
        if command.eq_ignore_ascii_case("/compact") && command_args_empty {
            let reasoning_mode =
                render::ReasoningDisplayMode::from_config(&config.display.reasoning);
            let tool_call_mode =
                render::ToolCallDisplayMode::from_config(&config.display.tool_calls);
            let mut renderer = render::StreamRenderer::new(
                reasoning_mode,
                tool_call_mode,
                false,
                config.display.readable_tool_names,
                config.display.command_output_lines,
            );
            match agent
                .compact_now(|event| handle_agent_event(&mut renderer, event))
                .await
            {
                Ok(Some(result)) => {
                    renderer.finish()?;
                    if let Some(usage) = result.usage.as_ref() {
                        cumulative_tokens =
                            cumulative_tokens.saturating_add(render::usage_total(usage));
                    }
                    footer.update_token_usage(
                        &result,
                        agent.effective_context_tokens()?,
                        agent.context_window(),
                        (cumulative_tokens > 0).then_some(cumulative_tokens),
                    );
                    if config.display.show_token_usage {
                        print_chat_token_usage(
                            &result,
                            true,
                            agent.effective_context_tokens()?,
                            agent.context_window(),
                            (cumulative_tokens > 0).then_some(cumulative_tokens),
                        )?;
                    }
                }
                Ok(None) => {
                    renderer.finish()?;
                    println!(
                        "\x1b[2m{}\x1b[0m",
                        t("nothing to compact", "没有可压缩的上下文")
                    );
                    footer.update_session_tokens(agent.effective_context_tokens()?);
                }
                Err(err) => {
                    renderer.finish()?;
                    eprintln!("\x1b[31m{}: {err}\x1b[0m", t("error", "错误"));
                }
            }
            continue;
        }
        if command.eq_ignore_ascii_case("/reset") && command_args.trim().is_empty() {
            commands::reset::run_reset(paths, None)?;
            input_history.clear();
            if let Some(live) = live_repl.as_mut() {
                live.editor.history.clear();
                live.editor.history_index = 0;
                live.queued.clear();
            }
            cumulative_tokens = 0;
            footer.reset_token_usage(agent.effective_context_tokens()?, agent.context_window());
            continue;
        }
        if command.eq_ignore_ascii_case("/reset") && command_args.trim().eq_ignore_ascii_case("all")
        {
            commands::reset::run_reset(paths, Some("all"))?;
            input_history.clear();
            if let Some(live) = live_repl.as_mut() {
                live.editor.history.clear();
                live.editor.history_index = 0;
                live.queued.clear();
            }
            agent.reset_memory()?;
            cumulative_tokens = 0;
            footer.reset_token_usage(agent.effective_context_tokens()?, agent.context_window());
            continue;
        }
        if input.is_empty() {
            continue;
        }
        input_history.push(input.to_string());
        if let Some(live) = live_repl.as_mut() {
            live.editor.record_history(input);
        }
        if agent.mode() != mode {
            let registry =
                build_tool_registry(&config, paths, mode, crate::question_tui::available(false))?;
            agent.switch_mode(mode, registry);
        }
        agent.prepare_for_turn()?;
        let reasoning_mode = render::ReasoningDisplayMode::from_config(&config.display.reasoning);
        let tool_call_mode = render::ToolCallDisplayMode::from_config(&config.display.tool_calls);
        let mut renderer = render::StreamRenderer::new(
            reasoning_mode,
            tool_call_mode,
            false,
            config.display.readable_tool_names,
            config.display.command_output_lines,
        );
        let control = AgentTurnControl::new(
            mode,
            build_tool_registry(
                &config,
                paths,
                AgentMode::Normal,
                crate::question_tui::available(false),
            )?,
            build_tool_registry(
                &config,
                paths,
                AgentMode::Plan,
                crate::question_tui::available(false),
            )?,
            build_tool_registry(
                &config,
                paths,
                AgentMode::Chat,
                crate::question_tui::available(false),
            )?,
        );
        if live_repl.is_none() {
            live_repl = Some(LiveReplTail::new(
                mode,
                input_history.clone(),
                state.load_queued_prompts()?,
                footer.clone(),
            )?);
        }
        let live = live_repl.as_mut().expect("live REPL was initialized");
        let chat_result = run_live_agent_turn(
            live,
            paths,
            &state,
            &mut agent,
            LiveAgentInput {
                content: input,
                images: &pasted_images,
            },
            &control,
            &mut renderer,
        )
        .await;
        mode = live.mode();
        match chat_result {
            Ok(Some(result)) => {
                let context_window =
                    result_context_window(&config, &result).or(agent.context_window());
                let mut turn_tokens = result.usage.as_ref().map(render::usage_total).unwrap_or(0);
                if let Some(usage) = result.usage.as_ref() {
                    cumulative_tokens =
                        cumulative_tokens.saturating_add(render::usage_total(usage));
                }
                let context_tokens = agent.effective_context_tokens()?;
                footer.update_token_usage(
                    &result,
                    context_tokens,
                    context_window,
                    (cumulative_tokens > 0).then_some(cumulative_tokens),
                );
                let endpoint_variant = result.provider_id.as_deref().and_then(|provider_id| {
                    result
                        .model
                        .as_deref()
                        .and_then(|model| client.thinking_variant_for(provider_id, model))
                });
                if show_mixed_model_endpoint(&config, true) {
                    let provider = result.provider_id.as_deref().unwrap_or("-");
                    let model = result.model.as_deref().unwrap_or("-");
                    let frame = format!(
                        "\x1b[2m{}\x1b[0m\n",
                        mixed_model_endpoint_label(provider, model, endpoint_variant.as_deref())
                    );
                    live.apply_output_frame(frame.as_bytes())?;
                }
                match handle_live_post_turn_overflow(
                    live,
                    &agent,
                    &mut renderer,
                    context_tokens,
                    config.display.show_token_usage,
                    Some(&mut cumulative_tokens),
                )
                .await
                {
                    Ok(Some(compact_result)) => {
                        if let Some(usage) = compact_result.usage.as_ref() {
                            turn_tokens = turn_tokens.saturating_add(render::usage_total(usage));
                        }
                        footer.set_token_usage(
                            turn_tokens,
                            agent.effective_context_tokens()?,
                            agent.context_window(),
                            (cumulative_tokens > 0).then_some(cumulative_tokens),
                        );
                    }
                    Ok(None) => {
                        footer.update_session_tokens(agent.effective_context_tokens()?);
                    }
                    Err(err) => {
                        let frame = format!("\x1b[31m{}: {err}\x1b[0m\n", t("error", "错误"));
                        live.apply_output_frame(frame.as_bytes())?;
                        continue;
                    }
                }
                show_shortcut_hint = false;
            }
            Ok(None) => {
                if let Some(live) = live_repl.as_mut() {
                    synchronized_terminal_update(CursorAfterUpdate::Shown, || {
                        live.reload_queue(&state)
                    })?;
                }
            }
            Err(err) if crate::question::is_question_cancelled(&err) => {
                if let Some(live) = live_repl.as_mut() {
                    synchronized_terminal_update(CursorAfterUpdate::Shown, || {
                        live.reload_queue(&state)
                    })?;
                }
                footer.update_session_tokens(agent.effective_context_tokens()?);
                continue;
            }
            Err(err) => {
                if let Some(live) = live_repl.as_mut() {
                    let frame = format!("\x1b[31m{}: {err}\x1b[0m\n", t("error", "错误"));
                    live.apply_output_frame(frame.as_bytes())?;
                    synchronized_terminal_update(CursorAfterUpdate::Shown, || {
                        live.reload_queue(&state)
                    })?;
                }
                continue;
            }
        }
    }
    state.discard_queued_prompts()?;
    Ok(())
}

fn reload_repl_config(
    paths: &GqyPaths,
    config: &mut AppConfig,
    client: &mut LlmClient,
) -> Result<()> {
    *config = AppConfig::load(paths)?;
    *client = LlmClient::from_config(config, paths)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariantMenuItem {
    provider_id: String,
    model: String,
    options: Vec<VariantMenuOption>,
    selected: usize,
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariantMenuOption {
    label: String,
    value: Option<String>,
}

impl VariantMenuItem {
    fn from_options(options: &ThinkingVariantOptions) -> Self {
        let mut variants = vec![VariantMenuOption {
            label: "default".to_string(),
            value: None,
        }];
        variants.extend(options.variants.iter().map(|variant| VariantMenuOption {
            label: if variant == "default" {
                "default (variant)".to_string()
            } else {
                variant.clone()
            },
            value: Some(variant.clone()),
        }));
        let selected = options
            .selected
            .as_ref()
            .and_then(|selected| {
                variants
                    .iter()
                    .position(|variant| variant.value.as_ref() == Some(selected))
            })
            .unwrap_or(0);
        Self {
            provider_id: options.provider_id.clone(),
            model: options.model.clone(),
            options: variants,
            selected,
            cursor: selected,
        }
    }

    fn selection(&self) -> (String, String, Option<String>) {
        (
            self.provider_id.clone(),
            self.model.clone(),
            self.options[self.selected].value.clone(),
        )
    }

    fn check_cursor(&mut self) {
        self.selected = self.cursor;
    }
}

pub(crate) fn inline_variant_select(
    options: &[ThinkingVariantOptions],
) -> Result<Option<Vec<(String, String, Option<String>)>>> {
    let mut items = options
        .iter()
        .map(VariantMenuItem::from_options)
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() == 1 {
        return inline_single_variant_select(items.remove(0));
    }
    let max_options = items
        .iter()
        .map(|item| item.options.len())
        .max()
        .unwrap_or(1);
    let menu_lines = inline_fuzzy_lines(items.len().max(max_options));
    reserve_inline_fuzzy_space(menu_lines)?;
    let mut session = InlineRawMode::start()?;
    let mut active_column = 0usize;
    let mut model_index = 0usize;
    let mut model_scroll = 0usize;
    let mut variant_scroll = 0usize;
    let (_, cursor_y) = cursor::position().unwrap_or((0, menu_lines.saturating_sub(1)));
    let anchor_y = cursor_y.saturating_sub(menu_lines.saturating_sub(1));
    loop {
        let visible = menu_lines.saturating_sub(2) as usize;
        model_scroll = inline_fuzzy_scroll(model_index, model_scroll, visible.min(items.len()));
        let item = &items[model_index];
        variant_scroll =
            inline_fuzzy_scroll(item.cursor, variant_scroll, visible.min(item.options.len()));
        draw_inline_variant(
            &mut session.stdout,
            anchor_y,
            menu_lines,
            &items,
            active_column,
            model_index,
            model_scroll,
            variant_scroll,
        )?;
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        {
            match code {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Enter => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(items.iter().map(VariantMenuItem::selection).collect()));
                }
                KeyCode::Left | KeyCode::Char('h') => active_column = 0,
                KeyCode::Right | KeyCode::Char('l') => active_column = 1,
                KeyCode::Up | KeyCode::Char('k') if active_column == 0 => {
                    model_index = model_index.saturating_sub(1);
                    variant_scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') if active_column == 0 => {
                    model_index = (model_index + 1).min(items.len() - 1);
                    variant_scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    items[model_index].cursor = items[model_index].cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let last = items[model_index].options.len() - 1;
                    items[model_index].cursor = (items[model_index].cursor + 1).min(last);
                }
                KeyCode::Tab if active_column == 1 => {
                    items[model_index].check_cursor();
                }
                _ => {}
            }
        }
    }
}

fn inline_single_variant_select(
    mut item: VariantMenuItem,
) -> Result<Option<Vec<(String, String, Option<String>)>>> {
    let menu_lines = inline_fuzzy_lines(item.options.len());
    reserve_inline_fuzzy_space(menu_lines)?;
    let mut session = InlineRawMode::start()?;
    let mut scroll = 0usize;
    let (_, cursor_y) = cursor::position().unwrap_or((0, menu_lines.saturating_sub(1)));
    let anchor_y = cursor_y.saturating_sub(menu_lines.saturating_sub(1));
    loop {
        let visible = menu_lines.saturating_sub(2) as usize;
        scroll = inline_fuzzy_scroll(item.cursor, scroll, visible.min(item.options.len()));
        draw_inline_single_variant(&mut session.stdout, anchor_y, menu_lines, &item, scroll)?;
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        {
            match code {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Enter => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(vec![item.selection()]));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    item.cursor = item.cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    item.cursor = (item.cursor + 1).min(item.options.len() - 1);
                }
                KeyCode::Tab => item.check_cursor(),
                _ => {}
            }
        }
    }
}

fn draw_inline_single_variant(
    stdout: &mut io::Stdout,
    anchor_y: u16,
    menu_lines: u16,
    item: &VariantMenuItem,
    scroll: usize,
) -> Result<()> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let bar = inline_fuzzy_bar();
    let available = (cols as usize).saturating_sub(visible_width(&bar)).max(1);
    let width = single_variant_content_width(item).min(available);
    let visible = menu_lines.saturating_sub(2) as usize;
    queue!(stdout, Hide)?;
    for row in 0..menu_lines {
        queue!(
            stdout,
            MoveTo(0, anchor_y + row),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y),
        Print(&bar),
        Print(variant_menu_header(
            t("Thinking variant", "思考档位"),
            true,
            width,
        )),
    )?;
    for row in 0..visible {
        let index = scroll + row;
        let line = item.options.get(index).map_or_else(
            || " ".repeat(width),
            |variant| {
                variant_menu_cell(
                    &variant.label,
                    index == item.cursor,
                    index == item.cursor,
                    Some(index == item.selected),
                    width,
                )
            },
        );
        queue!(
            stdout,
            MoveTo(0, anchor_y + row as u16 + 1),
            Print(&bar),
            Print(line),
        )?;
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y + menu_lines.saturating_sub(1)),
        Print(&bar),
        Print(format!(
            "\x1b[2m{}\x1b[0m",
            truncate_visible_width(
                t(
                    "j/k move · Tab select · Enter confirm · Esc/q cancel",
                    "j/k 移动 · Tab 勾选 · Enter 确认 · Esc/q 取消"
                ),
                available,
            )
        ))
    )?;
    stdout.flush()?;
    Ok(())
}

fn single_variant_content_width(item: &VariantMenuItem) -> usize {
    item.options
        .iter()
        .map(|option| visible_width(&option.label).saturating_add(6))
        .chain(std::iter::once(visible_width(t(
            "Thinking variant",
            "思考档位",
        ))))
        .max()
        .unwrap_or(1)
}

fn draw_inline_variant(
    stdout: &mut io::Stdout,
    anchor_y: u16,
    menu_lines: u16,
    items: &[VariantMenuItem],
    active_column: usize,
    model_index: usize,
    model_scroll: usize,
    variant_scroll: usize,
) -> Result<()> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let bar = inline_fuzzy_bar();
    let width = (cols as usize).saturating_sub(visible_width(&bar)).max(1);
    let separator = if width >= 3 { " │ " } else { "" };
    let available = width.saturating_sub(visible_width(separator));
    let (left_width, right_width) = variant_menu_column_widths(items, available);
    let visible = menu_lines.saturating_sub(2) as usize;
    queue!(stdout, Hide)?;
    for row in 0..menu_lines {
        queue!(
            stdout,
            MoveTo(0, anchor_y + row),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y),
        Print(&bar),
        Print(variant_menu_header(
            t("Provider / Model", "Provider / 模型"),
            active_column == 0,
            left_width,
        )),
        Print(format!("\x1b[2m{separator}\x1b[0m")),
        Print(variant_menu_header(
            t("Thinking variant", "思考档位"),
            active_column == 1,
            right_width,
        )),
    )?;
    let variants = &items[model_index];
    for row in 0..visible {
        let left_index = model_scroll + row;
        let right_index = variant_scroll + row;
        let left = items.get(left_index).map_or_else(
            || " ".repeat(left_width),
            |item| {
                variant_menu_cell(
                    &format!("{} / {}", item.provider_id, item.model),
                    active_column == 0 && left_index == model_index,
                    left_index == model_index,
                    None,
                    left_width,
                )
            },
        );
        let right = variants.options.get(right_index).map_or_else(
            || " ".repeat(right_width),
            |variant| {
                variant_menu_cell(
                    &variant.label,
                    active_column == 1 && right_index == variants.cursor,
                    right_index == variants.cursor,
                    Some(right_index == variants.selected),
                    right_width,
                )
            },
        );
        queue!(
            stdout,
            MoveTo(0, anchor_y + row as u16 + 1),
            Print(&bar),
            Print(left),
            Print(format!("\x1b[2m{separator}\x1b[0m")),
            Print(right),
        )?;
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y + menu_lines.saturating_sub(1)),
        Print(&bar),
        Print(format!(
            "\x1b[2m{}\x1b[0m",
            truncate_visible_width(
                t(
                    "h/l switch · j/k move · Tab select · Enter confirm · Esc/q cancel",
                    "h/l 切栏 · j/k 移动 · Tab 勾选 · Enter 确认 · Esc/q 取消"
                ),
                width,
            )
        ))
    )?;
    stdout.flush()?;
    Ok(())
}

fn variant_menu_column_widths(items: &[VariantMenuItem], available: usize) -> (usize, usize) {
    if available == 0 {
        return (0, 0);
    }
    if available == 1 {
        return (1, 0);
    }
    let left_needed = items
        .iter()
        .map(|item| {
            visible_width(&format!("{} / {}", item.provider_id, item.model)).saturating_add(2)
        })
        .chain(std::iter::once(visible_width(t(
            "Provider / Model",
            "Provider / 模型",
        ))))
        .max()
        .unwrap_or(1);
    let right_needed = items
        .iter()
        .flat_map(|item| item.options.iter())
        .map(|option| visible_width(&option.label).saturating_add(6))
        .chain(std::iter::once(visible_width(t(
            "Thinking variant",
            "思考档位",
        ))))
        .max()
        .unwrap_or(1);
    if left_needed.saturating_add(right_needed) <= available {
        return (left_needed, right_needed);
    }
    let total_needed = left_needed.saturating_add(right_needed).max(1);
    let left = available
        .saturating_mul(left_needed)
        .saturating_div(total_needed)
        .clamp(1, available - 1);
    (left, available - left)
}

fn variant_menu_header(label: &str, active: bool, width: usize) -> String {
    let label = pad_visible_width(&truncate_visible_width(label, width), width);
    if active {
        format!("\x1b[1m\x1b[35m{label}\x1b[0m")
    } else {
        format!("\x1b[1m{label}\x1b[0m")
    }
}

fn variant_menu_cell(
    label: &str,
    focused: bool,
    highlighted: bool,
    checked: Option<bool>,
    width: usize,
) -> String {
    let marker = if highlighted { "›" } else { " " };
    let check = match checked {
        Some(true) => "[*] ",
        Some(false) => "[ ] ",
        None => "",
    };
    let line = pad_visible_width(
        &truncate_visible_width(&format!("{marker} {check}{label}"), width),
        width,
    );
    if focused {
        format!("\x1b[1m\x1b[35m{line}\x1b[0m")
    } else if checked == Some(true) {
        format!("\x1b[1m\x1b[32m{line}\x1b[0m")
    } else if highlighted {
        format!("\x1b[1m{line}\x1b[0m")
    } else {
        format!("\x1b[2m{line}\x1b[0m")
    }
}

fn pad_visible_width(value: &str, width: usize) -> String {
    format!(
        "{value}{}",
        " ".repeat(width.saturating_sub(visible_width(value)))
    )
}

fn split_repl_command(input: &str) -> (&str, &str) {
    let Some((command, args)) = input.split_once(char::is_whitespace) else {
        return (input, "");
    };
    (command, args)
}

fn load_repl_input_history(state: &StateStore) -> Result<Vec<String>> {
    Ok(state
        .load_conversation()?
        .into_iter()
        .filter(|entry| entry.role == "user" && !entry.content.trim().is_empty())
        .map(|entry| strip_terminal_control_sequences(&entry.content))
        .filter(|content| !content.trim().is_empty())
        .collect())
}

fn print_repl_help() {
    println!("{}", t("commands:", "命令:"));
    println!(
        "  /models     {}",
        t("quickly switch model", "快速切换模型")
    );
    println!(
        "  /config     {}",
        t("open configuration UI", "打开配置界面")
    );
    println!(
        "  /variant [name] {}",
        t("view or switch thinking level", "查看或切换思考档位")
    );
    println!(
        "  /undo       {}",
        t(
            "undo the last turn or context compaction",
            "撤销上一轮或上下文压缩"
        )
    );
    println!(
        "  /pop [count] {}",
        t(
            "pop selected turns or the oldest count from active context",
            "从当前上下文弹出所选轮次或最旧的指定轮数"
        )
    );
    println!(
        "  /compact   {}",
        t(
            "compact current conversation context now",
            "立即压缩当前会话上下文"
        )
    );
    println!(
        "  /reset [all] {}",
        t(
            "clear current conversation history; all also clears memory",
            "清空当前会话历史；all 同时清空记忆"
        )
    );
    println!("  /help       {}", t("show this help", "显示此帮助"));
    println!("  /exit       {}", t("leave REPL", "退出 REPL"));
    println!("{}", t("keys:", "快捷键:"));
    println!(
        "  Tab         {}",
        t(
            "cycle NORMAL/PLAN/CHAT, or complete slash commands",
            "循环切换 普通/计划/闲聊，或补全斜杠菜单"
        )
    );
    println!("  Enter       {}", t("send message", "发送消息"));
    println!("  Ctrl+J      {}", t("insert newline", "插入换行"));
    println!(
        "  Ctrl+V      {}",
        t(
            "paste image or text from clipboard",
            "从剪贴板粘贴图片或文本"
        )
    );
    println!("  Ctrl+L      {}", t("clear screen", "清屏"));
    println!(
        "  Up/Down     {}",
        t("browse input history", "切换输入历史")
    );
    println!(
        "  Esc Esc     {}",
        t("interrupt running reply", "中断当前回复")
    );
}

pub(crate) struct LiveReplEditor {
    mode: AgentMode,
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: usize,
    history_clean_index: Option<usize>,
    is_pasted: bool,
    pasted_images: Vec<Option<crate::clipboard::PastedImage>>,
    pasted_texts: Vec<Option<PastedText>>,
    escape_armed_until: Option<Instant>,
}

pub(crate) struct LiveSubmission {
    content: String,
    display_content: String,
    images: Vec<Option<crate::clipboard::PastedImage>>,
}

pub(crate) struct LiveAgentInput<'a> {
    content: &'a str,
    images: &'a [Option<crate::clipboard::PastedImage>],
}

enum LiveEditorAction {
    None,
    Redraw,
    ClearScreen,
    EmptySubmit,
    Submit(LiveSubmission),
    Interrupt,
    Exit,
    ToggleReasoning,
}

impl LiveReplEditor {
    fn new(mode: AgentMode, history: Vec<String>) -> Self {
        let history_index = history.len();
        Self {
            mode,
            input: String::new(),
            cursor: 0,
            history,
            history_index,
            history_clean_index: None,
            is_pasted: false,
            pasted_images: Vec::new(),
            pasted_texts: Vec::new(),
            escape_armed_until: None,
        }
    }

    fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.history_clean_index = None;
        self.is_pasted = false;
        self.pasted_images.clear();
        self.pasted_texts.clear();
        self.escape_armed_until = None;
    }

    fn submit(&mut self) -> Option<LiveSubmission> {
        let display_content = strip_terminal_control_sequences(&self.input);
        let content = expand_pasted_text_placeholders(&display_content, &self.pasted_texts);
        let content = content.trim().to_string();
        if content.is_empty() {
            return None;
        }
        let display_content = display_content.trim().to_string();
        let images = std::mem::take(&mut self.pasted_images);
        self.input.clear();
        self.cursor = 0;
        self.history_clean_index = None;
        self.is_pasted = false;
        self.pasted_texts.clear();
        Some(LiveSubmission {
            content,
            display_content,
            images,
        })
    }

    fn record_history(&mut self, content: &str) {
        self.history.push(content.to_string());
        self.history_index = self.history.len();
    }

    fn handle_event(
        &mut self,
        event: Event,
        paths: &GqyPaths,
        allow_interrupt: bool,
    ) -> Result<LiveEditorAction> {
        let is_escape = matches!(
            &event,
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                ..
            })
        );
        if !is_escape {
            self.escape_armed_until = None;
        }
        match event {
            Event::Key(KeyEvent {
                kind: KeyEventKind::Release,
                ..
            }) => return Ok(LiveEditorAction::None),
            Event::Resize(_, _) => return Ok(LiveEditorAction::Redraw),
            Event::Paste(text) => {
                insert_pasted_text_at_cursor(
                    &mut self.input,
                    &mut self.cursor,
                    text,
                    &mut self.pasted_texts,
                );
                self.history_clean_index = None;
                self.is_pasted = true;
                return Ok(LiveEditorAction::Redraw);
            }
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => match code {
                KeyCode::Tab => {
                    if self.input.starts_with('/') {
                        if let Some(completed) = complete_repl_command(&self.input) {
                            self.input = completed.to_string();
                            self.cursor = self.input.chars().count();
                            self.history_clean_index = None;
                        }
                    } else {
                        self.mode = match self.mode {
                            AgentMode::Normal => AgentMode::Plan,
                            AgentMode::Plan => AgentMode::Chat,
                            AgentMode::Chat => AgentMode::Normal,
                        };
                    }
                }
                KeyCode::Esc => {
                    if allow_interrupt
                        && self
                            .escape_armed_until
                            .is_some_and(|deadline| Instant::now() < deadline)
                    {
                        self.escape_armed_until = None;
                        return Ok(LiveEditorAction::Interrupt);
                    }
                    self.clear();
                    if allow_interrupt {
                        self.escape_armed_until = Some(Instant::now() + Duration::from_secs(2));
                    }
                }
                KeyCode::Left => {
                    if let Some((start, _)) = placeholder_at_cursor(&self.input, self.cursor) {
                        self.cursor = start;
                    } else {
                        self.cursor = self.cursor.saturating_sub(1);
                    }
                }
                KeyCode::Right => {
                    if let Some((_, end)) = placeholder_at_cursor(&self.input, self.cursor) {
                        self.cursor = end;
                    } else {
                        self.cursor = (self.cursor + 1).min(self.input.chars().count());
                    }
                }
                KeyCode::Home => self.cursor = 0,
                KeyCode::End => self.cursor = self.input.chars().count(),
                KeyCode::Up => {
                    if !self.history.is_empty()
                        && repl_should_browse_history(
                            &self.input,
                            &self.history,
                            self.history_clean_index,
                        )
                    {
                        if self.input.is_empty() {
                            self.history_index = self.history.len();
                        }
                        self.history_index = self.history_index.saturating_sub(1);
                        self.input = self
                            .history
                            .get(self.history_index)
                            .cloned()
                            .unwrap_or_default();
                        self.cursor = self.input.chars().count();
                        self.history_clean_index = Some(self.history_index);
                        self.is_pasted = false;
                        self.pasted_images.clear();
                        self.pasted_texts.clear();
                    } else {
                        self.cursor = repl_move_cursor_vertical("  ", &self.input, self.cursor, -1);
                    }
                }
                KeyCode::Down => {
                    if repl_history_is_clean(&self.input, &self.history, self.history_clean_index) {
                        if self.history_index + 1 < self.history.len() {
                            self.history_index += 1;
                            self.input = self
                                .history
                                .get(self.history_index)
                                .cloned()
                                .unwrap_or_default();
                            self.cursor = self.input.chars().count();
                            self.history_clean_index = Some(self.history_index);
                        } else {
                            self.history_index = self.history.len();
                            self.input.clear();
                            self.cursor = 0;
                            self.history_clean_index = None;
                        }
                        self.is_pasted = false;
                        self.pasted_images.clear();
                        self.pasted_texts.clear();
                    } else {
                        self.cursor = repl_move_cursor_vertical("  ", &self.input, self.cursor, 1);
                    }
                }
                KeyCode::Enter => {
                    return Ok(self
                        .submit()
                        .map(LiveEditorAction::Submit)
                        .unwrap_or(LiveEditorAction::EmptySubmit));
                }
                KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                    insert_newline_at_cursor(&mut self.input, &mut self.cursor);
                    self.history_clean_index = None;
                    self.is_pasted = false;
                }
                KeyCode::Char('o') if modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl+O：展开/收起思考详情
                    return Ok(LiveEditorAction::ToggleReasoning);
                }
                KeyCode::Char('c')
                    if modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    if self.input.is_empty() {
                        return Ok(LiveEditorAction::Interrupt);
                    }
                    self.clear();
                }
                KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) && self.input.is_empty() =>
                {
                    return Ok(LiveEditorAction::Exit);
                }
                KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some((start, end)) =
                        placeholder_before_or_at_cursor(&self.input, self.cursor)
                    {
                        clear_placeholder_payload(
                            &self.input,
                            start,
                            end,
                            &mut self.pasted_images,
                            &mut self.pasted_texts,
                        );
                        remove_range_chars(&mut self.input, start, end);
                        self.cursor = start;
                    } else {
                        remove_word_before_cursor(&mut self.input, &mut self.cursor);
                    }
                    self.history_clean_index = None;
                    self.is_pasted = false;
                }
                KeyCode::Backspace => {
                    if self.cursor > 0 {
                        if let Some((start, end)) =
                            placeholder_before_or_at_cursor(&self.input, self.cursor)
                        {
                            clear_placeholder_payload(
                                &self.input,
                                start,
                                end,
                                &mut self.pasted_images,
                                &mut self.pasted_texts,
                            );
                            remove_range_chars(&mut self.input, start, end);
                            self.cursor = start;
                        } else {
                            remove_char_before_cursor(&mut self.input, &mut self.cursor);
                        }
                        self.history_clean_index = None;
                    }
                    self.is_pasted = false;
                }
                KeyCode::Delete => {
                    if let Some((start, end)) =
                        placeholder_after_or_at_cursor(&self.input, self.cursor)
                    {
                        clear_placeholder_payload(
                            &self.input,
                            start,
                            end,
                            &mut self.pasted_images,
                            &mut self.pasted_texts,
                        );
                        remove_range_chars(&mut self.input, start, end);
                    } else {
                        remove_char_at_cursor(&mut self.input, self.cursor);
                    }
                    self.history_clean_index = None;
                    self.is_pasted = false;
                }
                KeyCode::Char('c' | 'C')
                    if modifiers.contains(KeyModifiers::CONTROL)
                        && modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    if let Some(selected) =
                        placeholder_text_near_cursor(&self.input, self.cursor, &self.pasted_texts)
                    {
                        let _ = crate::clipboard::write_clipboard_text(&selected)?;
                    }
                }
                KeyCode::Char('v') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.paste_clipboard(paths)?;
                }
                KeyCode::Char('l') if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(LiveEditorAction::ClearScreen);
                }
                KeyCode::Char(ch) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    if !is_disallowed_control_char(ch) {
                        if let Some((_, end)) = placeholder_at_cursor(&self.input, self.cursor) {
                            self.cursor = end;
                        }
                        insert_char_at_cursor(&mut self.input, &mut self.cursor, ch);
                        self.history_clean_index = None;
                    }
                    self.is_pasted = false;
                }
                _ => return Ok(LiveEditorAction::None),
            },
            _ => return Ok(LiveEditorAction::None),
        }
        Ok(LiveEditorAction::Redraw)
    }

    fn paste_clipboard(&mut self, paths: &GqyPaths) -> Result<()> {
        match crate::clipboard::read_clipboard() {
            Ok(crate::clipboard::ClipboardContent::Image(image)) => {
                let index = self.pasted_images.len() + 1;
                let placeholder = match image.write_temp_file(&paths.cache_dir, index) {
                    Ok(path) => {
                        let filename = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("image");
                        format!("[Image {index}: {filename}]")
                    }
                    Err(_) => format!("[Image {index}]"),
                };
                insert_str_at_cursor(&mut self.input, &mut self.cursor, &placeholder);
                self.pasted_images
                    .push(Some(crate::clipboard::PastedImage::Binary(image)));
                self.is_pasted = false;
            }
            Ok(crate::clipboard::ClipboardContent::ImagePath(path)) => {
                let index = self.pasted_images.len() + 1;
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image");
                insert_str_at_cursor(
                    &mut self.input,
                    &mut self.cursor,
                    &format!("[Image {index}: {filename}]"),
                );
                self.pasted_images
                    .push(Some(crate::clipboard::PastedImage::Path(path)));
                self.is_pasted = false;
            }
            Ok(crate::clipboard::ClipboardContent::TextPath(path)) => {
                insert_str_at_cursor(&mut self.input, &mut self.cursor, &path);
                self.is_pasted = false;
            }
            _ => {
                if let Ok(Some(text)) = crate::clipboard::read_clipboard_text() {
                    insert_pasted_text_at_cursor(
                        &mut self.input,
                        &mut self.cursor,
                        text,
                        &mut self.pasted_texts,
                    );
                    self.is_pasted = true;
                }
            }
        }
        self.history_clean_index = None;
        Ok(())
    }
}

pub(crate) struct LiveReplTail {
    editor: LiveReplEditor,
    queued: Vec<QueuedPrompt>,
    pending_chunks: Vec<ChatStreamChunk>,
    footer: ReplFooterStatus,
    output_cursor: (u16, u16),
    tail_start: u16,
    tail_rows: u16,
    input_cursor: (u16, u16),
    rendered: bool,
    external_output_active: bool,
    raw_mode_handoff: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiveTailPlacement {
    output_row: u16,
    tail_start: u16,
    overflow: u16,
    anchored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalFrameLayout {
    cursor: (u16, u16),
    occupied_bottom: Option<u16>,
}

struct TerminalFrameTracker {
    columns: usize,
    bottom_margin: Option<usize>,
    cursor_col: usize,
    cursor_row: usize,
    saved_cursor: (usize, usize, bool),
    pending_wrap: bool,
    pending_text: String,
    occupied_bottom: Option<usize>,
}

impl TerminalFrameTracker {
    fn new(start: (u16, u16), columns: u16, bottom_margin: Option<u16>) -> Self {
        let columns = usize::from(columns.max(1));
        let cursor_col = usize::from(start.0).min(columns.saturating_sub(1));
        let cursor_row = usize::from(start.1);
        Self {
            columns,
            bottom_margin: bottom_margin.map(usize::from),
            cursor_col,
            cursor_row,
            saved_cursor: (cursor_col, cursor_row, false),
            pending_wrap: false,
            pending_text: String::new(),
            occupied_bottom: None,
        }
    }

    fn finish(mut self) -> TerminalFrameLayout {
        self.flush_text();
        TerminalFrameLayout {
            cursor: (
                self.cursor_col.min(u16::MAX as usize) as u16,
                self.cursor_row.min(u16::MAX as usize) as u16,
            ),
            occupied_bottom: self
                .occupied_bottom
                .map(|row| row.min(u16::MAX as usize) as u16),
        }
    }

    fn flush_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending_text);
        for grapheme in text.graphemes(true) {
            self.print_width(UnicodeWidthStr::width(grapheme));
        }
    }

    fn print_width(&mut self, width: usize) {
        if width == 0 {
            return;
        }
        if self.pending_wrap || self.cursor_col.saturating_add(width) > self.columns {
            self.cursor_col = 0;
            self.index();
            self.pending_wrap = false;
        }
        self.occupied_bottom = Some(
            self.occupied_bottom
                .map_or(self.cursor_row, |row| row.max(self.cursor_row)),
        );
        let next_col = self.cursor_col.saturating_add(width);
        if next_col >= self.columns {
            self.cursor_col = self.columns.saturating_sub(1);
            self.pending_wrap = true;
        } else {
            self.cursor_col = next_col;
        }
    }

    fn index(&mut self) {
        if self
            .bottom_margin
            .is_some_and(|bottom| self.cursor_row >= bottom)
        {
            return;
        }
        self.cursor_row = self.cursor_row.saturating_add(1);
    }

    fn move_down(&mut self, count: usize) {
        self.pending_wrap = false;
        self.cursor_row = self.cursor_row.saturating_add(count);
        if let Some(bottom) = self.bottom_margin {
            self.cursor_row = self.cursor_row.min(bottom);
        }
    }

    fn move_up(&mut self, count: usize) {
        self.pending_wrap = false;
        self.cursor_row = self.cursor_row.saturating_sub(count);
    }

    fn move_right(&mut self, count: usize) {
        self.pending_wrap = false;
        self.cursor_col = self
            .cursor_col
            .saturating_add(count)
            .min(self.columns.saturating_sub(1));
    }

    fn move_left(&mut self, count: usize) {
        self.pending_wrap = false;
        self.cursor_col = self.cursor_col.saturating_sub(count);
    }

    fn set_row(&mut self, row: usize) {
        self.pending_wrap = false;
        self.cursor_row = row;
        if let Some(bottom) = self.bottom_margin {
            self.cursor_row = self.cursor_row.min(bottom);
        }
    }

    fn set_col(&mut self, col: usize) {
        self.pending_wrap = false;
        self.cursor_col = col.min(self.columns.saturating_sub(1));
    }

    fn param(params: &VteParams, index: usize, default: usize) -> usize {
        params
            .iter()
            .nth(index)
            .and_then(|param| param.first())
            .copied()
            .map(usize::from)
            .filter(|value| *value != 0)
            .unwrap_or(default)
    }
}

impl VtePerform for TerminalFrameTracker {
    fn print(&mut self, character: char) {
        self.pending_text.push(character);
    }

    fn execute(&mut self, byte: u8) {
        self.flush_text();
        match byte {
            b'\n' => {
                self.cursor_col = 0;
                self.pending_wrap = false;
                self.index();
            }
            b'\r' => self.set_col(0),
            0x08 => self.move_left(1),
            b'\t' => {
                let next = (self.cursor_col / 8 + 1) * 8;
                self.set_col(next);
            }
            0x0b | 0x0c => {
                self.pending_wrap = false;
                self.index();
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &VteParams,
        _intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.flush_text();
        if ignore {
            return;
        }
        let count = Self::param(params, 0, 1);
        match action {
            'A' => self.move_up(count),
            'B' | 'e' => self.move_down(count),
            'C' | 'a' => self.move_right(count),
            'D' => self.move_left(count),
            'E' => {
                self.move_down(count);
                self.set_col(0);
            }
            'F' => {
                self.move_up(count);
                self.set_col(0);
            }
            'G' | '`' => self.set_col(count.saturating_sub(1)),
            'H' | 'f' => {
                self.set_row(Self::param(params, 0, 1).saturating_sub(1));
                self.set_col(Self::param(params, 1, 1).saturating_sub(1));
            }
            'd' => self.set_row(count.saturating_sub(1)),
            's' => {
                self.saved_cursor = (self.cursor_col, self.cursor_row, self.pending_wrap);
            }
            'u' => {
                (self.cursor_col, self.cursor_row, self.pending_wrap) = self.saved_cursor;
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], ignore: bool, byte: u8) {
        self.flush_text();
        if ignore {
            return;
        }
        match byte {
            b'7' => self.saved_cursor = (self.cursor_col, self.cursor_row, self.pending_wrap),
            b'8' => {
                (self.cursor_col, self.cursor_row, self.pending_wrap) = self.saved_cursor;
            }
            b'D' => {
                self.pending_wrap = false;
                self.index();
            }
            b'E' => {
                self.cursor_col = 0;
                self.pending_wrap = false;
                self.index();
            }
            b'M' => self.move_up(1),
            _ => {}
        }
    }
}

fn terminal_frame_layout(
    frame: &[u8],
    start: (u16, u16),
    columns: u16,
    bottom_margin: Option<u16>,
) -> TerminalFrameLayout {
    let mut parser = VteParser::new();
    let mut tracker = TerminalFrameTracker::new(start, columns, bottom_margin);
    parser.advance(&mut tracker, frame);
    tracker.finish()
}

fn live_frame_output_bottom(frame_margin: u16, layout: TerminalFrameLayout) -> Option<u16> {
    let ends_on_free_line = layout.cursor.0 == 0
        && layout
            .occupied_bottom
            .is_none_or(|bottom| layout.cursor.1 > bottom);
    if ends_on_free_line {
        Some(frame_margin)
    } else {
        frame_margin.checked_sub(1)
    }
}

#[derive(Clone, Copy)]
enum CursorAfterUpdate {
    Preserve,
    Shown,
    Hidden,
}

fn synchronized_terminal_update<T>(
    cursor_after: CursorAfterUpdate,
    update: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut stdout = io::stdout();
    match cursor_after {
        CursorAfterUpdate::Preserve => execute!(stdout, BeginSynchronizedUpdate)?,
        CursorAfterUpdate::Shown | CursorAfterUpdate::Hidden => {
            execute!(stdout, Hide, BeginSynchronizedUpdate)?
        }
    }
    let result = update();
    let end = match cursor_after {
        CursorAfterUpdate::Shown => execute!(stdout, EndSynchronizedUpdate, Show),
        CursorAfterUpdate::Preserve | CursorAfterUpdate::Hidden => {
            execute!(stdout, EndSynchronizedUpdate)
        }
    };
    match result {
        Ok(value) => {
            end?;
            Ok(value)
        }
        Err(error) => {
            let _ = end;
            Err(error)
        }
    }
}

fn live_tail_placement(
    output_col: u16,
    output_row: u16,
    total_rows: u16,
    terminal_rows: u16,
) -> LiveTailPlacement {
    let terminal_rows = terminal_rows.max(1);
    let last_row = terminal_rows.saturating_sub(2);
    let natural_start = output_row.saturating_add(u16::from(output_col > 0));
    let natural_end = natural_start.saturating_add(total_rows.saturating_sub(1));
    let overflow = natural_end.saturating_sub(last_row);
    let output_row = output_row.saturating_sub(overflow);
    let natural_start = output_row.saturating_add(u16::from(output_col > 0));
    let anchored = overflow > 0 || natural_end == last_row;
    let anchored_start = last_row.saturating_add(1).saturating_sub(total_rows);
    let tail_start = if anchored {
        natural_start.max(anchored_start)
    } else {
        natural_start
    };
    LiveTailPlacement {
        output_row,
        tail_start,
        overflow,
        anchored,
    }
}

fn max_live_tail_start(terminal_rows: u16, tail_rows: u16) -> u16 {
    terminal_rows
        .max(1)
        .saturating_sub(1)
        .saturating_sub(tail_rows)
}

impl LiveReplTail {
    fn new(
        mode: AgentMode,
        history: Vec<String>,
        queued: Vec<QueuedPrompt>,
        footer: ReplFooterStatus,
    ) -> Result<Self> {
        Ok(Self {
            editor: LiveReplEditor::new(mode, history),
            queued,
            pending_chunks: Vec::new(),
            footer,
            output_cursor: cursor::position()?,
            tail_start: 0,
            tail_rows: 0,
            input_cursor: (0, 0),
            rendered: false,
            external_output_active: false,
            raw_mode_handoff: false,
        })
    }

    fn mode(&self) -> AgentMode {
        self.editor.mode
    }

    fn set_footer(&mut self, footer: ReplFooterStatus) {
        self.footer = footer;
    }

    fn suspend(&mut self) -> Result<()> {
        if !self.rendered {
            return Ok(());
        }
        let mut stdout = io::stdout();
        let (_, terminal_rows) = terminal::size().unwrap_or((80, 24));
        for offset in 0..self.tail_rows {
            let row = self.tail_start.saturating_add(offset);
            if row >= terminal_rows {
                break;
            }
            queue!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        }
        queue!(stdout, MoveTo(self.output_cursor.0, self.output_cursor.1))?;
        stdout.flush()?;
        self.rendered = false;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.resume_at(cursor::position()?)
    }

    fn resume_at(&mut self, (output_col, output_row): (u16, u16)) -> Result<()> {
        let (cols, terminal_rows) = terminal::size().unwrap_or((80, 24));
        let terminal_rows = terminal_rows.max(1);
        let editor_rows = repl_input_rendered_rows(
            &self.editor.input,
            self.editor.is_pasted,
            false,
            usize::from(cols),
        );
        let mut queue_lines =
            queued_prompt_lines(&self.queued, self.editor.mode, usize::from(cols));
        let queue_gap = u16::from(!queue_lines.is_empty());
        let max_queue_rows = terminal_rows.saturating_sub(editor_rows).saturating_sub(3) as usize;
        if queue_lines.len() > max_queue_rows {
            let omitted = queue_lines.len() - max_queue_rows.saturating_sub(1);
            let mut clipped = vec![format!(
                "\x1b[2m… {}\x1b[0m",
                if is_zh() {
                    format!("已隐藏 {omitted} 行排队内容")
                } else {
                    format!("{omitted} queued lines hidden")
                }
            )];
            let keep = max_queue_rows.saturating_sub(1);
            clipped.extend(queue_lines.split_off(queue_lines.len().saturating_sub(keep)));
            queue_lines = clipped;
        }
        let total_rows = 1u16
            .saturating_add(queue_lines.len().min(u16::MAX as usize) as u16)
            .saturating_add(queue_gap)
            .saturating_add(editor_rows);
        let placement = live_tail_placement(output_col, output_row, total_rows, terminal_rows);
        if placement.overflow > 0 {
            let mut stdout = io::stdout();
            queue!(stdout, MoveTo(0, terminal_rows.saturating_sub(1)))?;
            for _ in 0..placement.overflow {
                queue!(stdout, Print("\n"))?;
            }
            stdout.flush()?;
        }
        let output_row = placement.output_row;
        let tail_start = placement.tail_start;

        let mut stdout = io::stdout();
        queue!(stdout, MoveTo(0, tail_start), Clear(ClearType::CurrentLine))?;
        let mut row = tail_start.saturating_add(1);
        for line in &queue_lines {
            queue!(
                stdout,
                MoveTo(0, row),
                Clear(ClearType::CurrentLine),
                Print(line)
            )?;
            row = row.saturating_add(1);
        }
        if !queue_lines.is_empty() {
            queue!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
            row = row.saturating_add(1);
        }
        stdout.flush()?;

        let mut input_row = row;
        let mut rendered_rows = 0u16;
        render_repl_input_with_footer(
            &mut stdout,
            &mut input_row,
            &mut rendered_rows,
            self.editor.mode,
            &self.editor.input,
            self.editor.cursor,
            self.editor.is_pasted,
            &self.footer,
            false,
        )?;
        self.input_cursor = cursor::position()?;
        self.output_cursor = (output_col, output_row);
        self.tail_start = tail_start;
        self.tail_rows = total_rows;
        self.rendered = true;
        Ok(())
    }

    pub(crate) fn apply_output_frame(&mut self, frame: &[u8]) -> Result<()> {
        if frame.is_empty() {
            return Ok(());
        }
        if !self.rendered {
            io::stdout().write_all(frame)?;
            io::stdout().flush()?;
            self.output_cursor = cursor::position()?;
            return Ok(());
        }

        let (columns, terminal_rows) = terminal::size().unwrap_or((80, 24));
        let terminal_rows = terminal_rows.max(1);
        let unbounded = terminal_frame_layout(frame, self.output_cursor, columns, None);
        let natural_tail = unbounded
            .cursor
            .1
            .saturating_add(u16::from(unbounded.cursor.0 > 0));
        let occupied_tail = unbounded
            .occupied_bottom
            .map(|row| row.saturating_add(1))
            .unwrap_or(0);
        let desired_tail = natural_tail.max(occupied_tail);
        let max_tail = max_live_tail_start(terminal_rows, self.tail_rows);
        let next_tail = desired_tail.min(max_tail);
        let shift = i32::from(next_tail) - i32::from(self.tail_start);
        let frame_margin = if shift < 0 {
            self.tail_start
        } else {
            next_tail
        };
        let output_bottom = live_frame_output_bottom(frame_margin, unbounded);
        let leading_scroll = output_bottom
            .map(|bottom| self.output_cursor.1.saturating_sub(bottom))
            .unwrap_or(0);
        let frame_start = if let Some(bottom) = output_bottom.filter(|_| leading_scroll > 0) {
            (0, bottom)
        } else {
            self.output_cursor
        };
        let bounded = terminal_frame_layout(frame, frame_start, columns, output_bottom);

        let mut transaction = Vec::with_capacity(frame.len().saturating_add(96));
        if shift > 0 {
            queue!(
                transaction,
                MoveTo(0, self.tail_start.saturating_add(1)),
                Print(format!("\x1b[{shift}L"))
            )?;
        }
        if let Some(bottom) = output_bottom {
            queue!(
                transaction,
                Print(format!("\x1b[1;{}r", bottom.saturating_add(1)))
            )?;
        }
        if let Some(bottom) = output_bottom.filter(|_| leading_scroll > 0) {
            queue!(transaction, MoveTo(0, bottom))?;
            for _ in 0..leading_scroll {
                queue!(transaction, Print("\n"))?;
            }
        }
        queue!(transaction, MoveTo(frame_start.0, frame_start.1))?;
        transaction.extend_from_slice(frame);
        queue!(transaction, Print("\x1b[r"))?;
        if shift < 0 {
            queue!(
                transaction,
                MoveTo(0, next_tail.saturating_add(1)),
                Print(format!("\x1b[{}M", -shift))
            )?;
        }
        let input_row = (i32::from(self.input_cursor.1) + shift)
            .clamp(0, i32::from(terminal_rows.saturating_sub(1))) as u16;
        queue!(transaction, MoveTo(self.input_cursor.0, input_row))?;
        let mut stdout = io::stdout();
        stdout.write_all(&transaction)?;
        stdout.flush()?;

        self.output_cursor = bounded.cursor;
        self.tail_start = next_tail;
        self.input_cursor.1 = input_row;
        Ok(())
    }

    pub(crate) fn apply_renderer_frame(&mut self, renderer: &mut render::StreamRenderer) -> Result<()> {
        let frame = renderer.take_output_frame();
        self.apply_output_frame(&frame)
    }

    fn redraw(&mut self) -> Result<()> {
        let output_cursor = self.output_cursor;
        self.suspend()?;
        self.resume_at(output_cursor)
    }

    fn clear_screen(&mut self) -> Result<()> {
        self.suspend()?;
        let mut stdout = io::stdout();
        execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
        self.output_cursor = (0, 0);
        self.tail_start = 0;
        self.tail_rows = 0;
        self.resume_at((0, 0))
    }

    fn enqueue(&mut self, prompt: QueuedPrompt) -> Result<()> {
        let output_cursor = self.output_cursor;
        self.suspend()?;
        self.append_queued(prompt);
        self.resume_at(output_cursor)
    }

    fn append_queued(&mut self, prompt: QueuedPrompt) {
        self.queued.push(prompt);
        self.queued.sort_by_key(|prompt| prompt.seq);
    }

    fn queue_stream_chunk(&mut self, chunk: ChatStreamChunk) {
        if let Some(pending) = self
            .pending_chunks
            .last_mut()
            .filter(|pending| pending.kind == chunk.kind)
        {
            pending.text.push_str(&chunk.text);
        } else {
            self.pending_chunks.push(chunk);
        }
    }

    fn flush_pending_chunks(&mut self, renderer: &mut render::StreamRenderer) -> Result<()> {
        for chunk in std::mem::take(&mut self.pending_chunks) {
            renderer.write_chunk(chunk)?;
        }
        Ok(())
    }

    fn discard_pending_chunks(&mut self) {
        self.pending_chunks.clear();
    }

    fn tick_spinner(&mut self, renderer: &mut render::StreamRenderer) -> Result<()> {
        self.flush_pending_chunks(renderer)?;
        renderer.tick_spinner()?;
        self.apply_renderer_frame(renderer)
    }

    fn commit_submission(&mut self, submission: &LiveSubmission) -> Result<()> {
        self.suspend()?;
        write_committed_user_messages(
            &[(submission.display_content.as_str(), self.editor.mode)],
            true,
        )?;
        self.output_cursor = cursor::position()?;
        Ok(())
    }

    fn commit_empty_submission(&mut self) -> Result<()> {
        self.editor.clear();
        let output_cursor = self.output_cursor;
        self.resume_at(output_cursor)
    }

    fn consume_queued(&mut self, prompt_ids: &[String], mode: AgentMode) -> Result<()> {
        self.suspend()?;
        let ids = prompt_ids.iter().collect::<std::collections::HashSet<_>>();
        let consumed = self
            .queued
            .iter()
            .filter(|prompt| ids.contains(&prompt.prompt_id))
            .map(|prompt| (prompt.display_content.as_str(), mode))
            .collect::<Vec<_>>();
        write_committed_user_messages(&consumed, true)?;
        self.queued
            .retain(|prompt| !ids.contains(&prompt.prompt_id));
        let output_cursor = cursor::position()?;
        self.output_cursor = output_cursor;
        self.resume_at(output_cursor)
    }

    fn reload_queue(&mut self, state: &StateStore) -> Result<()> {
        let output_cursor = self.output_cursor;
        self.suspend()?;
        self.queued = state.load_queued_prompts()?;
        self.resume_at(output_cursor)
    }
}

fn repl_input_rendered_rows(
    input: &str,
    is_pasted: bool,
    show_shortcut_hint: bool,
    cols: usize,
) -> u16 {
    let suggestions = repl_command_suggestions(input);
    let lines = repl_input_lines(input);
    let display_lines =
        repl_visible_input_lines("  ", &lines, REPL_MAX_VISIBLE_INPUT_ROWS, is_pasted);
    let input_rows = repl_wrapped_input_rows_for_cols("  ", &display_lines, cols)
        .len()
        .max(1)
        .min(u16::MAX as usize) as u16;
    input_rows.saturating_add(if show_shortcut_hint && suggestions.is_empty() {
        4
    } else {
        3
    })
}

fn queued_prompt_lines(prompts: &[QueuedPrompt], mode: AgentMode, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, prompt) in prompts.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(submitted_echo_lines(mode, &prompt.display_content, cols));
        lines.push(format!(
            "{} {}",
            submitted_echo_bar(mode),
            primary_footer_text(t("Queued", "排队中"))
        ));
    }
    lines
}

fn write_committed_user_messages(messages: &[(&str, AgentMode)], leading_gap: bool) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout();
    let (col, _) = cursor::position()?;
    if col > 0 {
        writeln!(stdout)?;
    }
    let cols = terminal_cols();
    write!(
        stdout,
        "{}",
        committed_user_messages_text(messages, leading_gap, cols)
    )?;
    stdout.flush()?;
    Ok(())
}

fn committed_user_messages_text(
    messages: &[(&str, AgentMode)],
    leading_gap: bool,
    cols: usize,
) -> String {
    let mut output = String::new();
    if leading_gap {
        output.push('\n');
    }
    for (index, (content, mode)) in messages.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        for line in submitted_echo_lines(*mode, content, cols) {
            output.push_str(&line);
            output.push('\n');
        }
    }
    output.push('\n');
    output
}

fn queued_prompt_attachments(
    images: &[Option<crate::clipboard::PastedImage>],
) -> Vec<QueuedPromptAttachment> {
    images
        .iter()
        .filter_map(|image| match image {
            Some(crate::clipboard::PastedImage::Binary(image)) => {
                Some(QueuedPromptAttachment::Binary {
                    mime: image.mime.clone(),
                    data_base64: base64::engine::general_purpose::STANDARD.encode(&image.data),
                })
            }
            Some(crate::clipboard::PastedImage::Path(path)) => {
                Some(QueuedPromptAttachment::Path { path: path.clone() })
            }
            None => None,
        })
        .collect()
}

fn persist_queued_submission(
    state: &StateStore,
    submission: &LiveSubmission,
) -> Result<QueuedPrompt> {
    let prompt_id = format!(
        "queued_{}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        rand::random::<u16>()
    );
    state.enqueue_prompt(
        &prompt_id,
        &submission.content,
        &submission.display_content,
        &queued_prompt_attachments(&submission.images),
    )
}

struct LiveRawMode {
    show_cursor_on_drop: bool,
    restore_terminal_on_drop: bool,
}

struct ReplCursorRestore;

impl Drop for ReplCursorRestore {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste, Show);
        let _ = terminal::disable_raw_mode();
    }
}

impl LiveRawMode {
    fn start() -> Result<Self> {
        enable_live_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(Self {
            show_cursor_on_drop: true,
            restore_terminal_on_drop: true,
        })
    }

    fn adopt() -> Self {
        Self {
            show_cursor_on_drop: true,
            restore_terminal_on_drop: true,
        }
    }

    fn keep_cursor_hidden(&mut self) {
        self.show_cursor_on_drop = false;
    }

    fn handoff(&mut self) {
        self.restore_terminal_on_drop = false;
    }
}

fn enable_live_raw_mode() -> Result<()> {
    terminal::enable_raw_mode()?;
    if let Err(error) = restore_live_output_processing() {
        let _ = terminal::disable_raw_mode();
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn restore_live_output_processing() -> Result<()> {
    let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
    // Raw input is required for key events, but renderer output still relies on newline translation.
    unsafe {
        if libc::tcgetattr(libc::STDOUT_FILENO, attributes.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut attributes = attributes.assume_init();
        attributes.c_oflag |= libc::OPOST | libc::ONLCR;
        if libc::tcsetattr(libc::STDOUT_FILENO, libc::TCSANOW, &attributes) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restore_live_output_processing() -> Result<()> {
    Ok(())
}

impl Drop for LiveRawMode {
    fn drop(&mut self) {
        if !self.restore_terminal_on_drop {
            return;
        }
        let mut stdout = io::stdout();
        if self.show_cursor_on_drop {
            let _ = execute!(stdout, DisableBracketedPaste, Show);
        } else {
            let _ = execute!(stdout, DisableBracketedPaste);
        }
        let _ = terminal::disable_raw_mode();
    }
}

fn read_live_repl_input(
    live: &mut LiveReplTail,
    paths: &GqyPaths,
) -> Result<
    Option<(
        AgentMode,
        String,
        Vec<Option<crate::clipboard::PastedImage>>,
    )>,
> {
    let mut raw = if std::mem::take(&mut live.raw_mode_handoff) {
        LiveRawMode::adopt()
    } else {
        LiveRawMode::start()?
    };
    if !live.rendered {
        synchronized_terminal_update(CursorAfterUpdate::Shown, || live.resume())?;
    }
    loop {
        match live.editor.handle_event(event::read()?, paths, false)? {
            LiveEditorAction::None => {}
            // 对话尚未开始：Ctrl+O 无意义，忽略
            LiveEditorAction::ToggleReasoning => {}
            LiveEditorAction::Redraw => {
                synchronized_terminal_update(CursorAfterUpdate::Preserve, || live.redraw())?
            }
            LiveEditorAction::ClearScreen => {
                synchronized_terminal_update(CursorAfterUpdate::Preserve, || live.clear_screen())?
            }
            LiveEditorAction::EmptySubmit => {
                synchronized_terminal_update(CursorAfterUpdate::Preserve, || {
                    live.commit_empty_submission()
                })?
            }
            LiveEditorAction::Submit(submission) => {
                let mode = live.mode();
                synchronized_terminal_update(CursorAfterUpdate::Hidden, || {
                    live.commit_submission(&submission)
                })?;
                raw.keep_cursor_hidden();
                return Ok(Some((mode, submission.content, submission.images)));
            }
            LiveEditorAction::Interrupt | LiveEditorAction::Exit => {
                synchronized_terminal_update(CursorAfterUpdate::Hidden, || live.suspend())?;
                return Ok(None);
            }
        }
    }
}

pub(crate) fn handle_live_agent_event(
    live: &mut LiveReplTail,
    renderer: &mut render::StreamRenderer,
    event: AgentEvent,
) -> Result<()> {
    let event = match event {
        AgentEvent::Chunk(chunk) => {
            live.queue_stream_chunk(chunk);
            return Ok(());
        }
        event => event,
    };
    if live.external_output_active && matches!(&event, AgentEvent::SpinnerTick) {
        return Ok(());
    }
    if matches!(&event, AgentEvent::SpinnerTick) {
        return live.tick_spinner(renderer);
    }
    match event {
        AgentEvent::PrepareForExternalOutput { ready } => {
            let result = (|| {
                live.flush_pending_chunks(renderer)?;
                renderer.prepare_for_external_output()?;
                live.apply_renderer_frame(renderer)?;
                synchronized_terminal_update(CursorAfterUpdate::Hidden, || live.suspend())?;
                live.external_output_active = true;
                Ok(())
            })();
            if result.is_ok() {
                let _ = ready.send(true);
            }
            result
        }
        AgentEvent::QueuedPromptsConsumed {
            prompt_ids, mode, ..
        } => {
            live.flush_pending_chunks(renderer)?;
            renderer.prepare_for_external_output()?;
            live.apply_renderer_frame(renderer)?;
            synchronized_terminal_update(CursorAfterUpdate::Preserve, || {
                live.suspend()?;
                live.consume_queued(&prompt_ids, mode)
            })
        }
        event => {
            let finishes_external_output =
                live.external_output_active && matches!(&event, AgentEvent::ToolResult { .. });
            if live.external_output_active && !finishes_external_output {
                handle_agent_event(renderer, event)?;
                return live.apply_renderer_frame(renderer);
            }
            let question = matches!(&event, AgentEvent::AskQuestion { .. });
            if question {
                live.flush_pending_chunks(renderer)?;
                renderer.prepare_for_external_output()?;
                live.apply_renderer_frame(renderer)?;
                synchronized_terminal_update(CursorAfterUpdate::Hidden, || live.suspend())?;
                handle_agent_event(renderer, event)?;
                enable_live_raw_mode()?;
                execute!(io::stdout(), EnableBracketedPaste)?;
                synchronized_terminal_update(CursorAfterUpdate::Shown, || live.resume())?;
                return live.apply_renderer_frame(renderer);
            }
            live.flush_pending_chunks(renderer)?;
            handle_agent_event(renderer, event)?;
            live.apply_renderer_frame(renderer)?;
            if finishes_external_output {
                live.external_output_active = false;
                synchronized_terminal_update(CursorAfterUpdate::Shown, || live.resume())?;
            }
            Ok(())
        }
    }
}

pub(crate) async fn run_live_agent_turn(
    live: &mut LiveReplTail,
    paths: &GqyPaths,
    state: &StateStore,
    agent: &mut Agent,
    input: LiveAgentInput<'_>,
    control: &AgentTurnControl,
    renderer: &mut render::StreamRenderer,
) -> Result<Option<crate::llm::ChatResult>> {
    renderer.use_external_cursor_control();
    renderer.use_buffered_output();
    let mut raw = if std::mem::take(&mut live.raw_mode_handoff) {
        LiveRawMode::adopt()
    } else {
        LiveRawMode::start()?
    };
    live.external_output_active = false;
    if !live.rendered {
        live.resume_at(live.output_cursor)?;
    }
    renderer.start_waiting()?;
    live.apply_renderer_frame(renderer)?;

    let result = {
        let llm = agent.llm_client();
        let live_cell = std::cell::RefCell::new(&mut *live);
        let renderer_cell = std::cell::RefCell::new(&mut *renderer);
        let chat = agent.chat_stream_with_control(input.content, input.images, control, |event| {
            handle_live_agent_event(
                &mut live_cell.borrow_mut(),
                &mut renderer_cell.borrow_mut(),
                event,
            )
        });
        tokio::pin!(chat);
        let mut input_tick = tokio::time::interval(Duration::from_millis(16));
        input_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        input_tick.tick().await;
        loop {
            tokio::select! {
                biased;
                _ = input_tick.tick() => {
                    if !event::poll(Duration::ZERO)? {
                        continue;
                    }
                    let event = event::read()?;
                    let mut live = live_cell.borrow_mut();
                    if matches!(
                        &event,
                        Event::Key(KeyEvent {
                            code: KeyCode::Enter,
                            kind,
                            ..
                        }) if *kind != KeyEventKind::Release
                    ) && live.editor.input.trim_start().starts_with('/')
                    {
                        if live.external_output_active {
                            continue;
                        }
                        let mut renderer = renderer_cell.borrow_mut();
                        live.flush_pending_chunks(&mut renderer)?;
                        renderer.write_system_message(t(
                            "REPL commands are available after the current reply finishes",
                            "当前回复结束后才能执行 REPL 命令",
                        ))?;
                        live.apply_renderer_frame(&mut renderer)?;
                        continue;
                    }
                    let mode_before = live.mode();
                    match live.editor.handle_event(event, paths, true)? {
                        LiveEditorAction::None => {}
                        LiveEditorAction::Redraw if !live.external_output_active => {
                            synchronized_terminal_update(CursorAfterUpdate::Preserve, || {
                                live.redraw()
                            })?
                        }
                        LiveEditorAction::ClearScreen if !live.external_output_active => {
                            synchronized_terminal_update(CursorAfterUpdate::Preserve, || {
                                live.clear_screen()
                            })?
                        }
                        LiveEditorAction::Redraw | LiveEditorAction::ClearScreen => {}
                        LiveEditorAction::EmptySubmit => {}
                        LiveEditorAction::Submit(submission) => {
                            let prompt = persist_queued_submission(state, &submission)?;
                            live.editor.record_history(&submission.content);
                            if live.external_output_active {
                                live.append_queued(prompt);
                            } else {
                                synchronized_terminal_update(CursorAfterUpdate::Preserve, || {
                                    live.enqueue(prompt)
                                })?;
                            }
                        }
                        LiveEditorAction::Interrupt | LiveEditorAction::Exit => {
                            let _ = llm.interrupt();
                            break Ok(None);
                        }
                        LiveEditorAction::ToggleReasoning => {
                            // Ctrl+O：展开/收起思考详情（流式输出中即时生效）
                            let mut renderer = renderer_cell.borrow_mut();
                            let mode = renderer.toggle_reasoning_mode()?;
                            let hint = match mode {
                                render::ReasoningDisplayMode::Full => {
                                    t("Thinking details expanded (Ctrl+O to collapse)", "思考详情：已展开（Ctrl+O 收起）")
                                }
                                _ => {
                                    t("Thinking details collapsed (Ctrl+O to expand)", "思考详情：已收起（Ctrl+O 展开）")
                                }
                            };
                            renderer.write_system_message(hint)?;
                            live.apply_renderer_frame(&mut renderer)?;
                        }
                    }
                    if live.mode() != mode_before {
                        control.set_mode(live.mode());
                    }
                },
                result = &mut chat => break result.map(Some),
            }
        }
    };

    if matches!(&result, Ok(None)) {
        live.discard_pending_chunks();
    }
    live.external_output_active = false;
    live.flush_pending_chunks(renderer)?;
    renderer.finish()?;
    live.apply_renderer_frame(renderer)?;
    raw.handoff();
    live.raw_mode_handoff = true;
    result
}

fn read_repl_input(
    paths: &GqyPaths,
    mut mode: AgentMode,
    prefill: Option<String>,
    history: &[String],
    footer: &ReplFooterStatus,
    show_shortcut_hint: bool,
) -> Result<
    Option<(
        AgentMode,
        String,
        Vec<Option<crate::clipboard::PastedImage>>,
    )>,
> {
    let mut stdout = io::stdout();
    let mut input = strip_terminal_control_sequences(&prefill.unwrap_or_default());
    let mut cursor = input.chars().count();
    let mut history_index = history.len();
    let mut history_clean_index: Option<usize> = None;
    let plain_prefix = "  ";
    let (cursor_col, _) = cursor::position()?;
    if cursor_col != 0 {
        writeln!(stdout)?;
        stdout.flush()?;
    }
    terminal::enable_raw_mode()?;
    execute!(stdout, EnableBracketedPaste)?;
    let (_, mut input_row) = cursor::position()?;
    let mut rendered_rows = 0u16;
    let mut is_pasted = false;
    let mut pasted_images: Vec<Option<crate::clipboard::PastedImage>> = Vec::new();
    let mut pasted_texts: Vec<Option<PastedText>> = Vec::new();
    let render_repl_input = |stdout: &mut io::Stdout,
                             input_row: &mut u16,
                             rendered_rows: &mut u16,
                             mode: AgentMode,
                             input: &str,
                             cursor: usize,
                             is_pasted: bool| {
        render_repl_input_with_footer(
            stdout,
            input_row,
            rendered_rows,
            mode,
            input,
            cursor,
            is_pasted,
            footer,
            show_shortcut_hint,
        )
    };
    render_repl_input(
        &mut stdout,
        &mut input_row,
        &mut rendered_rows,
        mode,
        &input,
        cursor,
        is_pasted,
    )?;
    loop {
        match event::read()? {
            Event::Paste(text) => {
                insert_pasted_text_at_cursor(&mut input, &mut cursor, text, &mut pasted_texts);
                history_clean_index = None;
                is_pasted = true;
                render_repl_input(
                    &mut stdout,
                    &mut input_row,
                    &mut rendered_rows,
                    mode,
                    &input,
                    cursor,
                    is_pasted,
                )?;
            }
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => match code {
                KeyCode::Tab => {
                    if input.starts_with('/') {
                        if let Some(completed) = complete_repl_command(&input) {
                            input = completed.to_string();
                            cursor = input.chars().count();
                            history_clean_index = None;
                        }
                    } else {
                        mode = match mode {
                            AgentMode::Normal => AgentMode::Plan,
                            AgentMode::Plan => AgentMode::Chat,
                            AgentMode::Chat => AgentMode::Normal,
                        };
                    }
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Esc => {
                    input.clear();
                    cursor = 0;
                    history_clean_index = None;
                    is_pasted = false;
                    pasted_images.clear();
                    pasted_texts.clear();
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Left => {
                    if let Some((start, _)) = placeholder_at_cursor(&input, cursor) {
                        cursor = start;
                    } else {
                        cursor = cursor.saturating_sub(1);
                    }
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Right => {
                    if let Some((_, end)) = placeholder_at_cursor(&input, cursor) {
                        cursor = end;
                    } else {
                        cursor = (cursor + 1).min(input.chars().count());
                    }
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Home => {
                    cursor = 0;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::End => {
                    cursor = input.chars().count();
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Up => {
                    if !history.is_empty()
                        && repl_should_browse_history(&input, history, history_clean_index)
                    {
                        if input.is_empty() {
                            history_index = history.len();
                        }
                        history_index = history_index.saturating_sub(1);
                        input = history.get(history_index).cloned().unwrap_or_default();
                        cursor = input.chars().count();
                        history_clean_index = Some(history_index);
                        is_pasted = false;
                        pasted_images.clear();
                        pasted_texts.clear();
                    } else {
                        cursor = repl_move_cursor_vertical(&plain_prefix, &input, cursor, -1);
                    }
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Down => {
                    if repl_history_is_clean(&input, history, history_clean_index) {
                        if history_index + 1 < history.len() {
                            history_index += 1;
                            input = history.get(history_index).cloned().unwrap_or_default();
                            cursor = input.chars().count();
                            history_clean_index = Some(history_index);
                        } else {
                            history_index = history.len();
                            input.clear();
                            cursor = 0;
                            history_clean_index = None;
                        }
                        is_pasted = false;
                        pasted_images.clear();
                        pasted_texts.clear();
                    } else {
                        cursor = repl_move_cursor_vertical(&plain_prefix, &input, cursor, 1);
                    }
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Enter => {
                    let submitted_echo = strip_terminal_control_sequences(&input);
                    input = expand_pasted_text_placeholders(&submitted_echo, &pasted_texts);
                    replace_repl_input_with_user_echo(
                        &mut stdout,
                        input_row,
                        rendered_rows,
                        mode,
                        &submitted_echo,
                    )?;
                    execute!(stdout, DisableBracketedPaste)?;
                    terminal::disable_raw_mode()?;
                    return Ok(Some((mode, input, pasted_images)));
                }
                KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                    insert_newline_at_cursor(&mut input, &mut cursor);
                    history_clean_index = None;
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Char('c')
                    if modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    if !input.is_empty() {
                        input.clear();
                        cursor = 0;
                        history_clean_index = None;
                        is_pasted = false;
                        pasted_images.clear();
                        pasted_texts.clear();
                        render_repl_input(
                            &mut stdout,
                            &mut input_row,
                            &mut rendered_rows,
                            mode,
                            &input,
                            cursor,
                            is_pasted,
                        )?;
                        continue;
                    }
                    move_after_repl_input(&mut stdout, input_row, rendered_rows)?;
                    execute!(stdout, DisableBracketedPaste)?;
                    terminal::disable_raw_mode()?;
                    return Ok(None);
                }
                KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) && input.is_empty() =>
                {
                    move_after_repl_input(&mut stdout, input_row, rendered_rows)?;
                    execute!(stdout, DisableBracketedPaste)?;
                    terminal::disable_raw_mode()?;
                    return Ok(None);
                }
                KeyCode::Char('l') if modifiers.contains(KeyModifiers::CONTROL) => {
                    queue!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
                    stdout.flush()?;
                    input_row = 0;
                    rendered_rows = 0;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some((start, end)) = placeholder_before_or_at_cursor(&input, cursor) {
                        clear_placeholder_payload(
                            &input,
                            start,
                            end,
                            &mut pasted_images,
                            &mut pasted_texts,
                        );
                        remove_range_chars(&mut input, start, end);
                        cursor = start;
                    } else {
                        remove_word_before_cursor(&mut input, &mut cursor);
                    }
                    history_clean_index = None;
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Backspace => {
                    if cursor > 0 {
                        if let Some((start, end)) = placeholder_before_or_at_cursor(&input, cursor)
                        {
                            clear_placeholder_payload(
                                &input,
                                start,
                                end,
                                &mut pasted_images,
                                &mut pasted_texts,
                            );
                            remove_range_chars(&mut input, start, end);
                            cursor = start;
                        } else {
                            remove_char_before_cursor(&mut input, &mut cursor);
                        }
                        history_clean_index = None;
                    }
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Delete => {
                    if let Some((start, end)) = placeholder_after_or_at_cursor(&input, cursor) {
                        clear_placeholder_payload(
                            &input,
                            start,
                            end,
                            &mut pasted_images,
                            &mut pasted_texts,
                        );
                        remove_range_chars(&mut input, start, end);
                    } else {
                        remove_char_at_cursor(&mut input, cursor);
                    }
                    history_clean_index = None;
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                KeyCode::Char('c' | 'C')
                    if modifiers.contains(KeyModifiers::CONTROL)
                        && modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    if let Some(selected) =
                        placeholder_text_near_cursor(&input, cursor, &pasted_texts)
                    {
                        let _ = crate::clipboard::write_clipboard_text(&selected)?;
                    }
                }
                KeyCode::Char('v') if modifiers.contains(KeyModifiers::CONTROL) => {
                    match crate::clipboard::read_clipboard() {
                        Ok(crate::clipboard::ClipboardContent::Image(img)) => {
                            let index = pasted_images.len() + 1;
                            let placeholder = match img.write_temp_file(&paths.cache_dir, index) {
                                Ok(path) => {
                                    let filename = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("image");
                                    format!("[Image {}: {}]", index, filename)
                                }
                                Err(_) => format!("[Image {}]", index),
                            };
                            insert_str_at_cursor(&mut input, &mut cursor, &placeholder);
                            history_clean_index = None;
                            pasted_images.push(Some(crate::clipboard::PastedImage::Binary(img)));
                            is_pasted = false;
                            render_repl_input(
                                &mut stdout,
                                &mut input_row,
                                &mut rendered_rows,
                                mode,
                                &input,
                                cursor,
                                is_pasted,
                            )?;
                        }
                        Ok(crate::clipboard::ClipboardContent::ImagePath(path)) => {
                            let index = pasted_images.len() + 1;
                            let filename = std::path::Path::new(&path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("image");
                            let placeholder = format!("[Image {}: {}]", index, filename);
                            insert_str_at_cursor(&mut input, &mut cursor, &placeholder);
                            history_clean_index = None;
                            pasted_images.push(Some(crate::clipboard::PastedImage::Path(path)));
                            is_pasted = false;
                            render_repl_input(
                                &mut stdout,
                                &mut input_row,
                                &mut rendered_rows,
                                mode,
                                &input,
                                cursor,
                                is_pasted,
                            )?;
                        }
                        Ok(crate::clipboard::ClipboardContent::TextPath(path)) => {
                            insert_str_at_cursor(&mut input, &mut cursor, &path);
                            history_clean_index = None;
                            is_pasted = false;
                            render_repl_input(
                                &mut stdout,
                                &mut input_row,
                                &mut rendered_rows,
                                mode,
                                &input,
                                cursor,
                                is_pasted,
                            )?;
                        }
                        _ => {
                            if let Ok(Some(text)) = crate::clipboard::read_clipboard_text() {
                                insert_pasted_text_at_cursor(
                                    &mut input,
                                    &mut cursor,
                                    text,
                                    &mut pasted_texts,
                                );
                                history_clean_index = None;
                                is_pasted = true;
                                render_repl_input(
                                    &mut stdout,
                                    &mut input_row,
                                    &mut rendered_rows,
                                    mode,
                                    &input,
                                    cursor,
                                    is_pasted,
                                )?;
                            }
                        }
                    }
                }
                KeyCode::Char(ch) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    if !is_disallowed_control_char(ch) {
                        if let Some((_, end)) = placeholder_at_cursor(&input, cursor) {
                            cursor = end;
                        }
                        insert_char_at_cursor(&mut input, &mut cursor, ch);
                        history_clean_index = None;
                    }
                    is_pasted = false;
                    render_repl_input(
                        &mut stdout,
                        &mut input_row,
                        &mut rendered_rows,
                        mode,
                        &input,
                        cursor,
                        is_pasted,
                    )?;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn render_repl_input_with_footer(
    stdout: &mut io::Stdout,
    input_row: &mut u16,
    rendered_rows: &mut u16,
    mode: AgentMode,
    input: &str,
    cursor: usize,
    is_pasted: bool,
    footer: &ReplFooterStatus,
    show_shortcut_hint: bool,
) -> Result<()> {
    let suggestions = repl_command_suggestions(input);
    let lines = repl_input_lines(input);
    let prompt_prefix = input_prompt_bar(mode);
    let plain_prefix = "  ";
    let cols = terminal_cols();
    let display_lines = repl_visible_input_lines(
        &plain_prefix,
        &lines,
        REPL_MAX_VISIBLE_INPUT_ROWS,
        is_pasted,
    );
    let display_rows = repl_wrapped_input_rows_for_cols(&plain_prefix, &display_lines, cols);
    let display_rows: Vec<String> = display_rows
        .iter()
        .map(|line| colorize_repl_placeholders(line))
        .collect();
    let input_rows = display_rows.len().max(1).min(u16::MAX as usize) as u16;
    let show_hint = show_shortcut_hint && suggestions.is_empty();
    let current_rows = input_rows.saturating_add(if show_hint { 5 } else { 4 });
    let rows_to_clear = (*rendered_rows).max(current_rows).max(1);
    ensure_repl_space(stdout, input_row, rows_to_clear)?;
    for row_offset in 0..rows_to_clear {
        queue!(
            stdout,
            MoveTo(0, (*input_row).saturating_add(row_offset)),
            Clear(ClearType::CurrentLine)
        )?;
    }

    // 输入区顶部边框
    let border_color = match mode {
        AgentMode::Normal => "\x1b[38;2;51;65;85m",
        AgentMode::Plan => "\x1b[38;2;30;58;138m",
        AgentMode::Chat => "\x1b[38;2;167;139;250m",
    };
    let border_char = "─";
    let border = format!("{}{}{}", border_color, border_char.repeat(cols.min(80) as usize), "\x1b[0m");
    let mut row_offset = 0u16;
    queue!(stdout, MoveTo(0, *input_row), Print(&border))?;
    row_offset = row_offset.saturating_add(1);

    // 输入行
    queue!(stdout, MoveTo(0, (*input_row).saturating_add(row_offset)), Print(&prompt_prefix))?;
    row_offset = row_offset.saturating_add(1);
    for line in &display_rows {
        let row = (*input_row).saturating_add(row_offset);
        queue!(stdout, MoveTo(0, row))?;
        queue!(stdout, Print(&prompt_prefix), Print(line))?;
        row_offset = row_offset.saturating_add(1);
    }

    // 底部信息
    queue!(
        stdout,
        MoveTo(0, (*input_row).saturating_add(row_offset)),
        Print(&prompt_prefix)
    )?;
    row_offset = row_offset.saturating_add(1);
    if !suggestions.is_empty() {
        let suggestion_width = cols.saturating_sub(visible_width(&prompt_prefix)).max(1);
        queue!(
            stdout,
            MoveTo(0, (*input_row).saturating_add(row_offset)),
            Print(&prompt_prefix),
            Print(format!(
                "\x1b[2m{}\x1b[0m",
                repl_command_suggestions_line(&suggestions, suggestion_width)
            ))
        )?;
    } else {
        queue!(
            stdout,
            MoveTo(0, (*input_row).saturating_add(row_offset)),
            Print(repl_footer_line(mode, footer, cols))
        )?;
        if show_hint {
            row_offset = row_offset.saturating_add(1);
            queue!(
                stdout,
                MoveTo(0, (*input_row).saturating_add(row_offset)),
                Print(repl_shortcut_hint_line(mode, cols))
            )?;
        }
    }
    let (cursor_col, cursor_row_offset) = if display_lines.len() == lines.len() {
        repl_cursor_position(&plain_prefix, input, cursor)
    } else {
        let last_line = display_lines.last().map(String::as_str).unwrap_or_default();
        let (col, _) = repl_cursor_position_for_line_for_cols(
            &plain_prefix,
            last_line,
            last_line.chars().count(),
            terminal_cols(),
        );
        (
            col,
            repl_prompt_rows(&plain_prefix, &display_lines).saturating_sub(1),
        )
    };
    queue!(
        stdout,
        MoveTo(
            cursor_col,
            (*input_row)
                .saturating_add(1)
                .saturating_add(cursor_row_offset)
        )
    )?;
    stdout.flush()?;
    *rendered_rows = current_rows;
    Ok(())
}

fn repl_visible_input_lines(
    prefix: &str,
    lines: &[String],
    max_rows: u16,
    is_pasted: bool,
) -> Vec<String> {
    let total_rows = repl_prompt_rows(prefix, lines);
    if total_rows <= max_rows || lines.len() <= 2 || !is_pasted {
        return lines.to_vec();
    }

    let omitted_lines = lines.len().saturating_sub(2);
    let omitted = if is_zh() {
        format!("... 已隐藏 {omitted_lines} 行粘贴内容 ...")
    } else {
        format!("... {omitted_lines} pasted lines hidden ...")
    };
    vec![lines[0].clone(), omitted, lines[lines.len() - 1].clone()]
}

fn ensure_repl_space(stdout: &mut io::Stdout, input_row: &mut u16, needed_rows: u16) -> Result<()> {
    let (_, term_rows) = terminal::size().unwrap_or((80, 24));
    let term_rows = term_rows.max(1);
    if (*input_row).saturating_add(needed_rows) < term_rows {
        return Ok(());
    }
    let overflow = (*input_row)
        .saturating_add(needed_rows)
        .saturating_sub(term_rows.saturating_sub(1));
    queue!(stdout, MoveTo(0, term_rows.saturating_sub(1)))?;
    for _ in 0..overflow {
        queue!(stdout, Print("\n"))?;
    }
    *input_row = (*input_row).saturating_sub(overflow);
    Ok(())
}

fn move_after_repl_input(
    stdout: &mut io::Stdout,
    input_row: u16,
    rendered_rows: u16,
) -> Result<()> {
    queue!(
        stdout,
        MoveTo(0, input_row.saturating_add(rendered_rows.max(1)))
    )?;
    stdout.flush()?;
    Ok(())
}

fn replace_repl_input_with_user_echo(
    stdout: &mut io::Stdout,
    input_row: u16,
    rendered_rows: u16,
    mode: AgentMode,
    input: &str,
) -> Result<()> {
    let cols = terminal_cols();
    let echo_lines = submitted_echo_lines(mode, input.trim_end(), cols);
    let echo_rows = echo_lines.len().min(u16::MAX as usize) as u16;
    let rows_to_clear = rendered_rows.max(echo_rows).max(1);
    for row_offset in 0..rows_to_clear {
        queue!(
            stdout,
            MoveTo(0, input_row.saturating_add(row_offset)),
            Clear(ClearType::CurrentLine)
        )?;
    }
    for (offset, line) in echo_lines.iter().enumerate() {
        queue!(
            stdout,
            MoveTo(
                0,
                input_row.saturating_add(offset.min(u16::MAX as usize) as u16)
            ),
            Print(line)
        )?;
    }
    queue!(
        stdout,
        MoveTo(0, input_row.saturating_add(echo_rows).saturating_add(1))
    )?;
    stdout.flush()?;
    Ok(())
}

fn submitted_echo_lines(mode: AgentMode, input: &str, cols: usize) -> Vec<String> {
    let max_text_width = cols.saturating_sub(3).max(1);
    let bar = submitted_echo_bar(mode);
    let mut output = Vec::new();
    output.push(bar.clone());
    for line in input.split('\n') {
        let mut chunks = wrap_visible_width(line, max_text_width);
        if chunks.is_empty() {
            chunks.push(String::new());
        }
        for chunk in chunks {
            output.push(format!("{bar} {}", colorize_repl_placeholders(&chunk)));
        }
    }
    output.push(bar);
    output
}

fn submitted_echo_bar(mode: AgentMode) -> String {
    // 月夜清影主题的用户消息条：月光银白 ❯
    let moon = "\x1b[1m\x1b[38;2;203;213;225m";
    match mode {
        AgentMode::Normal => format!("{moon}❯\x1b[0m"),
        AgentMode::Plan => "\x1b[1m\x1b[38;2;147;197;253m❯\x1b[0m".to_string(),
        AgentMode::Chat => "\x1b[1m\x1b[38;2;167;139;250m❯\x1b[0m".to_string(),
    }
}

pub(crate) fn input_prompt_bar(mode: AgentMode) -> String {
    format!("{} ", submitted_echo_bar(mode))
}

fn repl_shortcut_hint_line(mode: AgentMode, cols: usize) -> String {
    let bar = input_prompt_bar(mode);
    let text = t(
        "Tab switch mode · Ctrl+J newline · Ctrl+V paste · Esc interrupt",
        "Tab 切换模式 · Ctrl+J 换行 · Ctrl+V 粘贴 · Esc 中断",
    );
    let text_width = cols.saturating_sub(visible_width(&bar)).max(1);
    format!(
        "{bar}\x1b[38;2;51;65;85m{}\x1b[0m",
        truncate_visible_width(text, text_width)
    )
}

fn wrap_visible_width(value: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let char_width = visible_width(&ch.to_string());
        if width > 0 && width.saturating_add(char_width) > max_width {
            lines.push(std::mem::take(&mut current));
            width = 0;
        }
        current.push(ch);
        width = width.saturating_add(char_width);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn repl_wrapped_input_rows_for_cols(prefix: &str, lines: &[String], cols: usize) -> Vec<String> {
    let max_width = repl_content_width_for_cols(prefix, cols);
    let mut rows = Vec::new();
    for line in lines {
        let mut current = String::new();
        let mut width = 0usize;
        for ch in line.chars() {
            let char_width = visible_width(&ch.to_string());
            if width > 0 && width.saturating_add(char_width) > max_width {
                rows.push(std::mem::take(&mut current));
                width = 0;
            }
            current.push(ch);
            width = width.saturating_add(char_width);
        }
        rows.push(current);
        if width > 0 && width % max_width == 0 {
            rows.push(String::new());
        }
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn repl_cursor_position_for_line_for_cols(
    prefix: &str,
    line: &str,
    cursor: usize,
    cols: usize,
) -> (u16, u16) {
    let cols = cols.max(1);
    let prefix_width = repl_prefix_width_for_cols(prefix, cols);
    let content_width = repl_content_width_for_cols(prefix, cols);
    let mut col = 0usize;
    let mut row = 0usize;
    for ch in line.chars().take(cursor) {
        let char_width = visible_width(&ch.to_string()).max(1);
        if col > 0 && col.saturating_add(char_width) > content_width {
            row = row.saturating_add(1);
            col = 0;
        }
        col = col.saturating_add(char_width);
        if col >= content_width {
            row = row.saturating_add(1);
            col = 0;
        }
    }
    (
        prefix_width.saturating_add(col).min(u16::MAX as usize) as u16,
        row.min(u16::MAX as usize) as u16,
    )
}

fn repl_history_is_clean(
    input: &str,
    history: &[String],
    history_clean_index: Option<usize>,
) -> bool {
    history_clean_index
        .and_then(|index| history.get(index))
        .map(|entry| entry == input)
        .unwrap_or(false)
}

fn repl_should_browse_history(
    input: &str,
    history: &[String],
    history_clean_index: Option<usize>,
) -> bool {
    input.is_empty() || repl_history_is_clean(input, history, history_clean_index)
}

fn repl_move_cursor_vertical(prefix: &str, input: &str, cursor: usize, direction: i32) -> usize {
    if input.is_empty() || direction == 0 {
        return cursor.min(input.chars().count());
    }
    repl_move_cursor_vertical_for_cols(prefix, input, cursor, direction, terminal_cols())
}

fn repl_move_cursor_vertical_for_cols(
    prefix: &str,
    input: &str,
    cursor: usize,
    direction: i32,
    cols: usize,
) -> usize {
    let positions = repl_cursor_layout_positions_for_cols(prefix, input, cols);
    let cursor = cursor.min(positions.len().saturating_sub(1));
    let (_, current_row, current_col) = positions[cursor];
    let last_row = positions.last().map(|(_, row, _)| *row).unwrap_or(0);
    let target_row = if direction < 0 {
        current_row.saturating_sub(1)
    } else {
        current_row.saturating_add(1).min(last_row)
    };
    if target_row == current_row {
        return cursor;
    }

    positions
        .iter()
        .filter(|(_, row, _)| *row == target_row)
        .min_by_key(|(index, _, col)| (col.abs_diff(current_col), usize::MAX - *index))
        .map(|(index, _, _)| *index)
        .unwrap_or(cursor)
}

fn repl_cursor_layout_positions_for_cols(
    prefix: &str,
    input: &str,
    cols: usize,
) -> Vec<(usize, usize, usize)> {
    let content_width = repl_content_width_for_cols(prefix, cols);
    let mut positions = Vec::with_capacity(input.chars().count() + 1);
    let mut row = 0usize;
    let mut col = 0usize;
    positions.push((0, row, col));
    for (index, ch) in input.chars().enumerate() {
        if ch == '\n' {
            row = row.saturating_add(1);
            col = 0;
            positions.push((index + 1, row, col));
            continue;
        }
        let char_width = visible_width(&ch.to_string()).max(1);
        if col > 0 && col.saturating_add(char_width) > content_width {
            row = row.saturating_add(1);
            col = 0;
        }
        col = col.saturating_add(char_width);
        if col >= content_width {
            row = row.saturating_add(1);
            col = 0;
        }
        positions.push((index + 1, row, col));
    }
    positions
}

fn repl_prompt_rows(prefix: &str, lines: &[String]) -> u16 {
    repl_prompt_rows_for_cols(prefix, lines, terminal_cols())
}

fn repl_cursor_position(prefix: &str, input: &str, cursor: usize) -> (u16, u16) {
    repl_cursor_position_for_cols(prefix, input, cursor, terminal_cols())
}

fn repl_line_rows_for_cols(prefix: &str, line: &str, cols: usize) -> u16 {
    let content_width = repl_content_width_for_cols(prefix, cols);
    let width = visible_width(line);
    (width / content_width + 1).min(u16::MAX as usize) as u16
}

fn repl_prefix_width_for_cols(prefix: &str, cols: usize) -> usize {
    visible_width(prefix).min(cols.max(1).saturating_sub(1))
}

fn repl_content_width_for_cols(prefix: &str, cols: usize) -> usize {
    cols.max(1)
        .saturating_sub(repl_prefix_width_for_cols(prefix, cols))
        .max(1)
}

fn repl_prompt_rows_for_cols(prefix: &str, lines: &[String], cols: usize) -> u16 {
    let cols = cols.max(1);
    let mut rows = 0usize;
    for line in lines {
        rows += repl_line_rows_for_cols(prefix, line, cols) as usize;
    }
    rows.max(1).min(u16::MAX as usize) as u16
}

fn repl_cursor_position_for_cols(
    prefix: &str,
    input: &str,
    cursor: usize,
    cols: usize,
) -> (u16, u16) {
    let cols = cols.max(1);
    let before_cursor = take_chars(input, cursor);
    let lines = repl_input_lines(&before_cursor);
    let last_index = lines.len().saturating_sub(1);
    let mut row_offset = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if index == last_index {
            let (col, row) =
                repl_cursor_position_for_line_for_cols(prefix, line, line.chars().count(), cols);
            return (
                col,
                row_offset
                    .saturating_add(row as usize)
                    .min(u16::MAX as usize) as u16,
            );
        }
        row_offset += repl_line_rows_for_cols(prefix, line, cols) as usize;
    }
    (
        repl_prefix_width_for_cols(prefix, cols).min(u16::MAX as usize) as u16,
        0,
    )
}

fn insert_char_at_cursor(value: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = byte_index_for_char(value, *cursor);
    value.insert(byte_index, ch);
    *cursor += 1;
}

fn insert_str_at_cursor(value: &mut String, cursor: &mut usize, text: &str) {
    let byte_index = byte_index_for_char(value, *cursor);
    value.insert_str(byte_index, text);
    *cursor += text.chars().count();
}

fn insert_newline_at_cursor(value: &mut String, cursor: &mut usize) {
    insert_char_at_cursor(value, cursor, '\n');
}

fn remove_char_before_cursor(value: &mut String, cursor: &mut usize) {
    let end = byte_index_for_char(value, *cursor);
    let start = byte_index_for_char(value, cursor.saturating_sub(1));
    value.replace_range(start..end, "");
    *cursor -= 1;
}

fn remove_word_before_cursor(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let chars = value.chars().collect::<Vec<_>>();
    let mut start = (*cursor).min(chars.len());
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let byte_start = byte_index_for_char(value, start);
    let byte_end = byte_index_for_char(value, *cursor);
    value.replace_range(byte_start..byte_end, "");
    *cursor = start;
}

fn remove_char_at_cursor(value: &mut String, cursor: usize) {
    if cursor >= value.chars().count() {
        return;
    }
    let start = byte_index_for_char(value, cursor);
    let end = byte_index_for_char(value, cursor + 1);
    value.replace_range(start..end, "");
}

fn byte_index_for_char(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

pub(crate) fn should_summarize_pasted_text(text: &str) -> bool {
    !text.is_empty()
        && (pasted_text_line_count(text) >= REPL_PASTE_PLACEHOLDER_MIN_LINES
            || text.chars().count() > REPL_PASTE_PLACEHOLDER_MIN_CHARS)
}

pub(crate) fn pasted_text_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().filter(|ch| *ch == '\n').count() + 1
    }
}

pub(crate) fn pasted_text_placeholder(index: usize, line_count: usize) -> String {
    if is_zh() {
        format!("[粘贴 {index}: ~{line_count} 行]")
    } else {
        format!("[Pasted {index}: ~{line_count} lines]")
    }
}

fn insert_pasted_text_at_cursor(
    input: &mut String,
    cursor: &mut usize,
    text: String,
    pasted_texts: &mut Vec<Option<PastedText>>,
) {
    let text = strip_terminal_control_sequences(&text);
    if should_summarize_pasted_text(&text) {
        let index = pasted_texts.len() + 1;
        let placeholder = pasted_text_placeholder(index, pasted_text_line_count(&text));
        insert_str_at_cursor(input, cursor, &placeholder);
        pasted_texts.push(Some(PastedText { text }));
    } else {
        insert_str_at_cursor(input, cursor, &text);
    }
}

pub(crate) fn find_repl_placeholders(input: &str) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let prefix_len = if i + 7 <= chars.len()
            && chars[i..i + 7].iter().collect::<String>() == "[Image "
        {
            Some(7)
        } else if i + 8 <= chars.len() && chars[i..i + 8].iter().collect::<String>() == "[Pasted " {
            Some(8)
        } else if i + 4 <= chars.len() && chars[i..i + 4].iter().collect::<String>() == "[粘贴 " {
            Some(4)
        } else {
            None
        };

        if let Some(prefix_len) = prefix_len {
            let mut j = i + prefix_len;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == ':' {
                j += 1;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ']' {
                    result.push((i, j + 1));
                    i = j + 1;
                    continue;
                }
            } else if j < chars.len() && chars[j] == ']' {
                result.push((i, j + 1));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    result
}

pub(crate) fn find_image_placeholders(input: &str) -> Vec<(usize, usize)> {
    find_repl_placeholders(input)
        .into_iter()
        .filter(|(start, end)| parse_image_placeholder_index(input, *start, *end).is_some())
        .collect()
}

pub(crate) fn find_pasted_text_placeholders(input: &str) -> Vec<(usize, usize, usize)> {
    find_repl_placeholders(input)
        .into_iter()
        .filter_map(|(start, end)| {
            parse_pasted_text_placeholder_index(input, start, end).map(|index| (start, end, index))
        })
        .collect()
}

fn placeholder_at_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let placeholders = find_repl_placeholders(input);
    for (start, end) in &placeholders {
        if cursor > *start && cursor < *end {
            return Some((*start, *end));
        }
    }
    None
}

fn placeholder_before_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let placeholders = find_repl_placeholders(input);
    for (start, end) in &placeholders {
        if *end == cursor {
            return Some((*start, *end));
        }
    }
    None
}

fn placeholder_before_or_at_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    placeholder_at_cursor(input, cursor).or_else(|| placeholder_before_cursor(input, cursor))
}

fn placeholder_after_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let placeholders = find_repl_placeholders(input);
    for (start, end) in &placeholders {
        if *start == cursor {
            return Some((*start, *end));
        }
    }
    None
}

fn placeholder_after_or_at_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    placeholder_at_cursor(input, cursor).or_else(|| placeholder_after_cursor(input, cursor))
}

fn remove_range_chars(value: &mut String, char_start: usize, char_end: usize) {
    let byte_start = byte_index_for_char(value, char_start);
    let byte_end = byte_index_for_char(value, char_end);
    value.replace_range(byte_start..byte_end, "");
}

fn parse_image_placeholder_index(input: &str, char_start: usize, char_end: usize) -> Option<usize> {
    let chars: Vec<char> = input.chars().collect();
    let segment: String = chars[char_start..char_end].iter().collect();
    let after_prefix = segment.strip_prefix("[Image ")?;
    let num_str: String = after_prefix
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num_str.parse::<usize>().ok()
}

fn parse_pasted_text_placeholder_index(
    input: &str,
    char_start: usize,
    char_end: usize,
) -> Option<usize> {
    let chars: Vec<char> = input.chars().collect();
    let segment: String = chars[char_start..char_end].iter().collect();
    let after_prefix = segment
        .strip_prefix("[Pasted ")
        .or_else(|| segment.strip_prefix("[粘贴 "))?;
    let num_str: String = after_prefix
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num_str.parse::<usize>().ok()
}

fn clear_placeholder_payload(
    input: &str,
    start: usize,
    end: usize,
    pasted_images: &mut [Option<crate::clipboard::PastedImage>],
    pasted_texts: &mut [Option<PastedText>],
) {
    if let Some(n) = parse_image_placeholder_index(input, start, end) {
        if n > 0 && n <= pasted_images.len() {
            pasted_images[n - 1] = None;
        }
    }
    if let Some(n) = parse_pasted_text_placeholder_index(input, start, end) {
        if n > 0 && n <= pasted_texts.len() {
            pasted_texts[n - 1] = None;
        }
    }
}

fn expand_pasted_text_placeholders(input: &str, pasted_texts: &[Option<PastedText>]) -> String {
    let placeholders = find_pasted_text_placeholders(input);
    if placeholders.is_empty() {
        return input.to_string();
    }

    let chars: Vec<char> = input.chars().collect();
    let mut expanded = String::new();
    let mut last_end = 0;
    for (start, end, index) in placeholders {
        expanded.extend(&chars[last_end..start]);
        if index > 0 {
            if let Some(Some(pasted_text)) = pasted_texts.get(index - 1) {
                expanded.push_str(&pasted_text.text);
            } else {
                expanded.extend(&chars[start..end]);
            }
        } else {
            expanded.extend(&chars[start..end]);
        }
        last_end = end;
    }
    expanded.extend(&chars[last_end..]);
    expanded
}

fn placeholder_text_near_cursor(
    input: &str,
    cursor: usize,
    pasted_texts: &[Option<PastedText>],
) -> Option<String> {
    let (start, end) = placeholder_at_cursor(input, cursor)
        .or_else(|| placeholder_before_cursor(input, cursor))
        .or_else(|| placeholder_after_cursor(input, cursor))?;
    let index = parse_pasted_text_placeholder_index(input, start, end)?;
    pasted_texts
        .get(index.checked_sub(1)?)
        .and_then(Option::as_ref)
        .map(|pasted_text| pasted_text.text.clone())
}

fn take_chars(value: &str, count: usize) -> String {
    value.chars().take(count).collect()
}

fn terminal_cols() -> usize {
    terminal::size()
        .map(|(cols, _)| cols.max(1) as usize)
        .unwrap_or(80)
}

fn repl_input_lines(input: &str) -> Vec<String> {
    let normalized = strip_terminal_control_sequences(input)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = normalized
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(crate) fn strip_terminal_control_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            continue;
        }
        if is_disallowed_control_char(ch) {
            continue;
        }
        output.push(ch);
    }
    output
}

fn is_disallowed_control_char(ch: char) -> bool {
    ch.is_control() && !matches!(ch, '\n' | '\t')
}

pub(crate) fn visible_width(value: &str) -> usize {
    let mut width = 0usize;
    let mut escape = false;
    for ch in value.chars() {
        if escape {
            if ch == 'm' {
                escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            escape = true;
        } else if (ch as u32) >= 0x2e80 {
            width += 2;
        } else {
            width += 1;
        }
    }
    width
}

fn colorize_repl_placeholders(line: &str) -> String {
    let placeholders = find_repl_placeholders(line);
    if placeholders.is_empty() {
        return line.to_string();
    }

    let chars: Vec<char> = line.chars().collect();
    let mut result = String::new();
    let mut last_end = 0;
    for (start, end) in placeholders {
        result.extend(&chars[last_end..start]);
        result.push_str("\x1b[35m");
        result.extend(&chars[start..end]);
        result.push_str("\x1b[0m");
        last_end = end;
    }
    result.extend(&chars[last_end..]);
    result
}

fn repl_commands() -> [&'static str; 9] {
    [
        "/models", "/config", "/variant", "/undo", "/pop", "/compact", "/reset", "/help", "/exit",
    ]
}

fn repl_command_suggestions(input: &str) -> Vec<&'static str> {
    if !input.starts_with('/') {
        return Vec::new();
    }
    repl_commands()
        .into_iter()
        .filter(|command| command.starts_with(input))
        .collect()
}

fn complete_repl_command(input: &str) -> Option<&'static str> {
    let suggestions = repl_command_suggestions(input);
    if suggestions.len() == 1 {
        suggestions.first().copied()
    } else {
        None
    }
}

fn resolve_repl_command<'a>(input: &'a str) -> &'a str {
    if input.starts_with('/') {
        if let Some(command) = complete_repl_command(input) {
            return command;
        }
    }
    input
}

fn repl_command_suggestions_line(suggestions: &[&str], max_width: usize) -> String {
    let line = if suggestions.len() == 1 {
        suggestions[0].to_string()
    } else {
        suggestions.join("  ")
    };
    truncate_visible_width(&line, max_width)
}

pub(crate) fn truncate_visible_width(value: &str, max_width: usize) -> String {
    if visible_width(value) <= max_width {
        return value.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut output = String::new();
    let mut width = 0usize;
    let ellipsis_width = visible_width("...");
    let budget = max_width.saturating_sub(ellipsis_width);
    for ch in value.chars() {
        let ch_width = visible_width(&ch.to_string());
        if width.saturating_add(ch_width) > budget {
            break;
        }
        output.push(ch);
        width = width.saturating_add(ch_width);
    }
    output.push_str("...");
    output
}

#[cfg(test)]
mod repl_input_tests {
    use super::*;
    use crate::cli::args::{localized_command, parse_args};
    use crate::llm::ChatStreamKind;

    fn sample_pop_turn(status: TurnStatus) -> Turn {
        Turn {
            turn_id: "turn-1".to_string(),
            seq: 1,
            user_content: "first prompt line\nsecond prompt line".to_string(),
            user_timestamp: "2026-07-19 10:42".to_string(),
            assistant_content: "first answer line\nsecond answer line".to_string(),
            assistant_reasoning: Some("private reasoning".to_string()),
            assistant_provider_id: None,
            assistant_model: None,
            assistant_timestamp: Some("2026-07-19 10:43".to_string()),
            status,
            tool_reports: vec!["hidden tool report".to_string()],
            question_exchanges: Vec::new(),
            followups: Vec::new(),
            hidden: false,
            is_summary: false,
            owner_pid: None,
            token_total: 0,
            token_usage_estimated: false,
        }
    }

    fn pop_test_paths(root: &std::path::Path) -> GqyPaths {
        GqyPaths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.jsonc"),
            skills_dir: root.join("config/skills"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            state_dir: root.join("state"),
            pictures_dir: root.join("pictures"),
            fish_hook_file: root.join("fish/gqy.fish"),
            bash_hook_file: root.join("shell/bash-hook.sh"),
            zsh_hook_file: root.join("shell/zsh-hook.zsh"),
            scripts_dir: root.join("config/scripts"),
            system_scripts_dir: root.join("system/scripts"),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        }
    }

    #[test]
    fn terminal_frame_tracks_ansi_and_wide_graphemes() {
        let layout = terminal_frame_layout("\x1b[32mAB\x1b[0m\n中👨‍👩‍👧‍👦".as_bytes(), (3, 2), 12, None);

        assert_eq!(layout.cursor, (4, 3));
        assert_eq!(layout.occupied_bottom, Some(3));
    }

    #[test]
    fn terminal_frame_wraps_before_the_next_wide_grapheme() {
        let layout = terminal_frame_layout("中🙂".as_bytes(), (8, 1), 10, None);

        assert_eq!(layout.cursor, (2, 2));
        assert_eq!(layout.occupied_bottom, Some(2));
    }

    #[test]
    fn terminal_frame_applies_cursor_motion_without_losing_bottom_occupancy() {
        let layout = terminal_frame_layout(b"first\nsecond\x1b[1A\x1b[3G!", (0, 4), 20, None);

        assert_eq!(layout.cursor, (3, 4));
        assert_eq!(layout.occupied_bottom, Some(5));
    }

    #[test]
    fn terminal_frame_scroll_margin_keeps_cursor_above_live_input() {
        let layout = terminal_frame_layout("one\n二\nthree".as_bytes(), (0, 5), 20, Some(5));

        assert_eq!(layout.cursor, (5, 5));
        assert_eq!(layout.occupied_bottom, Some(5));
    }

    #[test]
    fn live_frame_uses_the_gap_only_for_a_terminating_newline() {
        let content = terminal_frame_layout(b"answer", (0, 5), 20, None);
        assert_eq!(live_frame_output_bottom(6, content), Some(5));

        let terminated = terminal_frame_layout(b"answer\n", (0, 5), 20, None);
        assert_eq!(live_frame_output_bottom(6, terminated), Some(6));
        let bounded = terminal_frame_layout(
            b"answer\n",
            (0, 5),
            20,
            live_frame_output_bottom(6, terminated),
        );
        assert_eq!(bounded.cursor, (0, 6));
        assert_eq!(bounded.occupied_bottom, Some(5));
    }

    #[test]
    fn models_is_the_cli_model_selector() {
        let matches = localized_command()
            .try_get_matches_from(["gqy", "models", "1"])
            .unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Models(ModelsArgs { index: Some(1) }))
        ));
        let old_matches = localized_command()
            .try_get_matches_from(["gqy", "providers"])
            .unwrap();
        let old_cli = Cli::from_arg_matches(&old_matches).unwrap();
        assert!(old_cli.command.is_none());
        assert_eq!(old_cli.message, ["providers"]);
    }

    #[test]
    fn variant_is_a_cli_subcommand_with_an_optional_name() {
        let cli = parse_args(["gqy", "variant"].map(OsString::from).to_vec()).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Variant(VariantArgs { name: None }))
        ));

        let cli = parse_args(["gqy", "variant", "high"].map(OsString::from).to_vec()).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Variant(VariantArgs { name })) if name.as_deref() == Some("high")
        ));

        assert!(parse_args(
            ["gqy", "variant", "high", "extra"]
                .map(OsString::from)
                .to_vec()
        )
        .is_err());
    }

    #[test]
    fn web_is_a_cli_subcommand_with_local_server_options() {
        let cli = parse_args(
            ["gqy", "web", "--port", "4100", "--no-open"]
                .map(OsString::from)
                .to_vec(),
        )
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Web(WebArgs {
                port: 4100,
                host,
                no_open: true,
                password: None,
                password_file: None,
            })) if host == "127.0.0.1"
        ));

        let cli = parse_args(
            ["gqy", "web", "--host", "0.0.0.0", "--port", "4100", "--no-open", "-p", "secret"]
                .map(OsString::from)
                .to_vec(),
        )
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Web(WebArgs {
                port: 4100,
                host,
                no_open: true,
                password: Some(password),
                password_file: None,
            })) if host == "0.0.0.0" && password == "secret"
        ));

        let cli = parse_args(["gqy", "web"].map(OsString::from).to_vec()).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Web(WebArgs {
                port: 4096,
                host,
                no_open: false,
                password: None,
                password_file: None,
            })) if host == "127.0.0.1"
        ));

        let cli = parse_args(
            ["gqy", "web", "-p", "secret", "--no-open"]
                .map(OsString::from)
                .to_vec(),
        )
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Web(WebArgs {
                password: Some(password),
                no_open: true,
                ..
            })) if password == "secret"
        ));

        let cli = parse_args(
            ["gqy", "web", "-p", "--no-open"]
                .map(OsString::from)
                .to_vec(),
        )
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Web(WebArgs {
                password: Some(password),
                no_open: true,
                ..
            })) if password.is_empty()
        ));
    }

    #[test]
    fn pop_is_a_cli_subcommand_with_an_optional_count() {
        let cli = parse_args(["gqy", "pop"].map(OsString::from).to_vec()).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Pop(PopArgs { count: None }))
        ));

        let cli = parse_args(["gqy", "pop", "3"].map(OsString::from).to_vec()).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Pop(PopArgs { count: Some(3) }))
        ));
        assert!(parse_args(["gqy", "pop", "0"].map(OsString::from).to_vec()).is_err());
        assert!(parse_args(["gqy", "pop", "nope"].map(OsString::from).to_vec()).is_err());
    }

    #[test]
    fn repl_pop_accepts_zero_or_one_positive_integer() {
        assert_eq!(parse_repl_pop_count("").unwrap(), None);
        assert_eq!(parse_repl_pop_count(" 3 ").unwrap(), Some(3));
        assert!(parse_repl_pop_count("0").is_err());
        assert!(parse_repl_pop_count("nope").is_err());
        assert!(parse_repl_pop_count("1 2").is_err());
    }

    #[test]
    fn counted_pop_removes_oldest_turns_and_caps_at_available_count() {
        let temp = tempfile::tempdir().unwrap();
        let paths = pop_test_paths(temp.path());
        let config = AppConfig::default();
        let state = StateStore::new(&paths).unwrap();
        for id in ["t1", "t2", "t3"] {
            state.start_turn(id, id, 999999).unwrap();
            state.complete_turn(id, "reply", None).unwrap();
        }

        let first = execute_pop(&paths, &config, &state, Some(2))
            .unwrap()
            .unwrap();
        assert_eq!(first.turns, 2);
        assert_eq!(
            state
                .load_visible_turns()
                .unwrap()
                .into_iter()
                .map(|turn| turn.turn_id)
                .collect::<Vec<_>>(),
            vec!["t3"]
        );

        let second = execute_pop(&paths, &config, &state, Some(99))
            .unwrap()
            .unwrap();
        assert_eq!(second.turns, 1);
        assert!(state.load_visible_turns().unwrap().is_empty());
    }

    #[test]
    fn pop_menu_uses_three_lines_without_context_metadata() {
        let turn = sample_pop_turn(TurnStatus::Completed);
        let lines = pop_menu_turn_lines(&turn, true, false, 80)
            .map(|line| strip_terminal_control_sequences(&line));

        assert_eq!(lines[0], "› [ ] 2026-07-19 10:42");
        assert!(lines[1].contains("first prompt line"));
        assert!(!lines[1].contains("second prompt line"));
        assert!(lines[2].contains("first answer line"));
        assert!(!lines[2].contains("second answer line"));
        let joined = lines.join(" ");
        assert!(!joined.contains("hidden tool report"));
        assert!(!joined.contains("private reasoning"));
        assert!(lines.iter().all(|line| visible_width(line) <= 80));
    }

    #[test]
    fn pop_menu_labels_an_interrupted_reply_without_showing_the_reminder() {
        let mut turn = sample_pop_turn(TurnStatus::Interrupted);
        turn.assistant_content = crate::state::interrupted_text().to_string();
        let lines = pop_menu_turn_lines(&turn, false, true, 80)
            .map(|line| strip_terminal_control_sequences(&line));

        assert!(lines[2].contains("中断") || lines[2].contains("interrupted"));
        assert!(!lines[2].contains("system-reminder"));
    }

    #[test]
    fn pop_menu_footer_has_controls_but_no_position_counter() {
        let help = strip_terminal_control_sequences(&pop_menu_help_line(120));
        assert!(help.contains("Tab"));
        assert!(help.contains("Enter"));
        assert!(!help.contains("3 / 8"));

        let header = strip_terminal_control_sequences(&pop_menu_header("", 2, 8, 80));
        assert!(header.contains("2 / 8"));
    }

    #[test]
    fn filtered_pop_turns_keep_oldest_first_order() {
        let matcher = SkimMatcherV2::default();
        let items = vec![
            "old matching prompt".to_string(),
            "middle unrelated".to_string(),
            "new matching prompt".to_string(),
        ];

        assert_eq!(pop_matches(&matcher, &items, "matching"), vec![0, 2]);
    }

    #[test]
    fn debug_is_a_global_cli_option() {
        for args in [
            &["gqy", "--debug", "models", "1"][..],
            &["gqy", "models", "--debug", "1"][..],
            &["gqy", "hello", "--debug"][..],
            &["gqy", "ask", "hello", "--debug"][..],
        ] {
            let cli = parse_args(args.iter().map(OsString::from).collect()).unwrap();
            assert!(cli.debug);
        }

        let cli = parse_args(["gqy", "hello", "--debug"].map(OsString::from).to_vec()).unwrap();
        assert_eq!(cli.message, ["hello"]);

        let cli = parse_args(["gqy", "--", "--debug"].map(OsString::from).to_vec()).unwrap();
        assert!(!cli.debug);
        assert_eq!(cli.message, ["--debug"]);
    }

    #[test]
    fn footer_reset_clears_turn_and_cumulative_tokens() {
        let config = AppConfig::default();
        let mut footer = ReplFooterStatus::from_config(&config, 100, Some(250));
        footer.set_token_usage(50, 100, Some(200_000), Some(250));

        footer.reset_token_usage(0, Some(200_000));

        assert_eq!(footer.token_usage.turn_tokens, 0);
        assert_eq!(footer.token_usage.session_tokens, 0);
        assert_eq!(footer.token_usage.context_window, Some(200_000));
        assert_eq!(footer.token_usage.cumulative_tokens, None);

        footer.reset_token_usage(0, None);
        assert_eq!(footer.token_usage.context_window, None);
    }

    #[test]
    fn footer_variant_always_uses_the_fixed_primary_color() {
        let config = AppConfig::default();
        let mut footer = ReplFooterStatus::from_config(&config, 0, None);
        footer.update_thinking_variant(Some("high"));

        for mode in [AgentMode::Normal, AgentMode::Plan, AgentMode::Chat] {
            let line = repl_footer_left(mode, &footer, 120);
            assert!(line.contains("\x1b[1m\x1b[34mhigh\x1b[0m"));
            // 默认 opencode 供应商不显示在 footer
            let provider = if footer.provider.is_empty() || footer.provider == "opencode" {
                String::new()
            } else {
                format!(" {}", footer.provider)
            };
            // 模式标签现在带背景色和空格（badge 样式）
            assert_eq!(
                strip_terminal_control_sequences(&line),
                format!(" {}  · {}{} · high", mode.label(), footer.model, provider)
            );
        }
    }

    #[test]
    fn mixed_footer_uses_dim_provider_and_hides_global_variant() {
        let mut config = AppConfig::default();
        let provider = config
            .providers
            .iter_mut()
            .find(|provider| !provider.models.is_empty())
            .unwrap();
        let provider_id = provider.id.clone();
        let first_model = provider.models[0].clone();
        let second_model = "footer-second-model".to_string();
        provider.models.push(second_model.clone());
        config.active_provider_models = Some(vec![
            ActiveProviderModelConfig {
                provider_id: provider_id.clone(),
                model: first_model,
            },
            ActiveProviderModelConfig {
                provider_id,
                model: second_model,
            },
        ]);
        let mut footer = ReplFooterStatus::from_config(&config, 0, None);
        footer.update_thinking_variant(Some("mixed"));

        let line = repl_footer_left(AgentMode::Normal, &footer, 120);

        assert_eq!(footer.provider, "mixed");
        assert!(footer.thinking.is_none());
        // 模式标签现在带背景色和空格（badge 样式）
        assert_eq!(
            strip_terminal_control_sequences(&line),
            format!(
                " {}  · {} mixed",
                AgentMode::Normal.label(),
                t("Mixed", "混合")
            )
        );
        assert!(line.contains("\x1b[2mmixed\x1b[0m"));
        assert!(!line.contains(&primary_footer_text("mixed")));
    }

    #[test]
    fn committed_user_message_keeps_one_blank_line_before_output() {
        let output = committed_user_messages_text(&[("hello", AgentMode::Normal)], true, 80);

        assert_eq!(
            strip_terminal_control_sequences(&output),
            "\n❯\n❯ hello\n❯\n\n"
        );
    }

    #[test]
    fn queued_message_uses_full_height_bar_and_primary_status() {
        let prompt = QueuedPrompt {
            prompt_id: "q1".to_string(),
            seq: 1,
            content: "follow up".to_string(),
            display_content: "follow up".to_string(),
            attachments: Vec::new(),
            submitted_at: String::new(),
        };

        let normal = queued_prompt_lines(std::slice::from_ref(&prompt), AgentMode::Normal, 80);
        let plan = queued_prompt_lines(&[prompt], AgentMode::Plan, 80);

        assert_eq!(normal.len(), 4);
        assert_eq!(normal[0], submitted_echo_bar(AgentMode::Normal));
        assert_eq!(normal[2], submitted_echo_bar(AgentMode::Normal));
        assert!(normal[3].starts_with(&submitted_echo_bar(AgentMode::Normal)));
        assert!(normal[3].contains(&primary_footer_text(t("Queued", "排队中"))));
        assert!(plan
            .iter()
            .filter(|line| !line.is_empty())
            .all(|line| line.starts_with(&submitted_echo_bar(AgentMode::Plan))));
        assert_ne!(normal[0], plan[0]);
    }

    #[test]
    fn live_tail_moves_naturally_and_releases_after_output_shrinks() {
        assert_eq!(max_live_tail_start(6, 5), 0);
        assert_eq!(max_live_tail_start(24, 5), 18);
        assert_eq!(
            live_tail_placement(0, 4, 5, 24),
            LiveTailPlacement {
                output_row: 4,
                tail_start: 4,
                overflow: 0,
                anchored: false,
            }
        );
        assert_eq!(
            live_tail_placement(0, 20, 5, 24),
            LiveTailPlacement {
                output_row: 18,
                tail_start: 18,
                overflow: 2,
                anchored: true,
            }
        );
        assert_eq!(
            live_tail_placement(0, 6, 5, 24),
            LiveTailPlacement {
                output_row: 6,
                tail_start: 6,
                overflow: 0,
                anchored: false,
            }
        );
        assert_eq!(live_tail_placement(0, 6, 5, 30).tail_start, 6);
    }

    #[test]
    fn live_editor_restores_clear_screen_and_double_escape_controls() {
        let temp = tempfile::tempdir().unwrap();
        let paths = pop_test_paths(temp.path());
        let escape = || Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let mut editor = LiveReplEditor::new(AgentMode::Normal, Vec::new());
        editor.input = "draft".to_string();
        assert!(matches!(
            editor.handle_event(escape(), &paths, true).unwrap(),
            LiveEditorAction::Redraw
        ));
        assert!(editor.input.is_empty());
        assert!(matches!(
            editor.handle_event(escape(), &paths, true).unwrap(),
            LiveEditorAction::Interrupt
        ));

        let clear = Event::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(matches!(
            editor.handle_event(clear, &paths, true).unwrap(),
            LiveEditorAction::ClearScreen
        ));

        assert!(matches!(
            editor.handle_event(escape(), &paths, false).unwrap(),
            LiveEditorAction::Redraw
        ));
        assert!(matches!(
            editor.handle_event(escape(), &paths, false).unwrap(),
            LiveEditorAction::Redraw
        ));

        assert!(matches!(
            editor
                .handle_event(
                    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                    &paths,
                    false,
                )
                .unwrap(),
            LiveEditorAction::EmptySubmit
        ));
        assert!(editor.history.is_empty());

        editor.input = "/help".to_string();
        editor.cursor = editor.input.chars().count();
        assert!(matches!(
            editor
                .handle_event(
                    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                    &paths,
                    false,
                )
                .unwrap(),
            LiveEditorAction::Submit(_)
        ));
        assert!(editor.history.is_empty());
        editor.record_history("ordinary prompt");
        assert_eq!(editor.history, ["ordinary prompt"]);
    }

    #[test]
    fn spinner_does_not_resume_tail_during_external_output() {
        let config = AppConfig::default();
        let mut live = LiveReplTail {
            editor: LiveReplEditor::new(AgentMode::Normal, Vec::new()),
            queued: Vec::new(),
            pending_chunks: Vec::new(),
            footer: ReplFooterStatus::from_config(&config, 0, None),
            output_cursor: (0, 0),
            tail_start: 0,
            tail_rows: 0,
            input_cursor: (0, 0),
            rendered: false,
            external_output_active: true,
            raw_mode_handoff: false,
        };
        let mut renderer = render::StreamRenderer::new(
            render::ReasoningDisplayMode::Hidden,
            render::ToolCallDisplayMode::Hidden,
            true,
            true,
            10,
        );

        handle_live_agent_event(&mut live, &mut renderer, AgentEvent::SpinnerTick).unwrap();

        assert!(live.external_output_active);
        assert!(!live.rendered);
    }

    #[test]
    fn live_tail_coalesces_adjacent_stream_chunks_and_can_discard_them() {
        let config = AppConfig::default();
        let mut live = LiveReplTail {
            editor: LiveReplEditor::new(AgentMode::Normal, Vec::new()),
            queued: Vec::new(),
            pending_chunks: Vec::new(),
            footer: ReplFooterStatus::from_config(&config, 0, None),
            output_cursor: (0, 0),
            tail_start: 0,
            tail_rows: 0,
            input_cursor: (0, 0),
            rendered: false,
            external_output_active: false,
            raw_mode_handoff: false,
        };

        for (kind, text) in [
            (ChatStreamKind::Reasoning, "one"),
            (ChatStreamKind::Reasoning, " two"),
            (ChatStreamKind::Content, "answer"),
            (ChatStreamKind::Content, " text"),
        ] {
            live.queue_stream_chunk(ChatStreamChunk {
                kind,
                text: text.to_string(),
            });
        }

        assert_eq!(live.pending_chunks.len(), 2);
        assert_eq!(live.pending_chunks[0].text, "one two");
        assert_eq!(live.pending_chunks[1].text, "answer text");
        live.discard_pending_chunks();
        assert!(live.pending_chunks.is_empty());
    }

    #[test]
    fn prompt_rows_wrap_at_terminal_width() {
        assert_eq!(repl_prompt_rows_for_cols("", &["1234567".into()], 10), 1);
        assert_eq!(repl_prompt_rows_for_cols("", &["1234567890".into()], 10), 2);
        assert_eq!(
            repl_prompt_rows_for_cols("", &["123".into(), "456".into()], 10),
            2
        );
    }

    #[test]
    fn cursor_position_wraps_at_terminal_width() {
        assert_eq!(repl_cursor_position_for_cols("", "1234567", 7, 10), (7, 0));
        assert_eq!(
            repl_cursor_position_for_cols("", "1234567890", 10, 10),
            (0, 1)
        );
        assert_eq!(repl_cursor_position_for_cols("", "123\n456", 7, 10), (3, 1));
        assert_eq!(repl_cursor_position_for_cols("", "1234567", 3, 10), (3, 0));
    }

    #[test]
    fn cursor_position_keeps_prefix_after_newline() {
        assert_eq!(repl_cursor_position_for_cols("  ", "123\n", 4, 10), (2, 1));
        assert_eq!(
            repl_cursor_position_for_cols("  ", "123\n456", 7, 10),
            (5, 1)
        );
    }

    #[test]
    fn prompt_rows_include_prefix_on_each_line() {
        assert_eq!(
            repl_prompt_rows_for_cols("  ", &["12".into(), "34".into()], 5),
            2
        );
        assert_eq!(
            repl_prompt_rows_for_cols("  ", &["123".into(), "34".into()], 5),
            3
        );
    }

    #[test]
    fn wrapped_input_rows_keep_prefix_outside_content_width() {
        assert_eq!(
            repl_wrapped_input_rows_for_cols("  ", &["123456789".into()], 10),
            vec!["12345678".to_string(), "9".to_string()]
        );
        assert_eq!(
            repl_wrapped_input_rows_for_cols("  ", &["12345678".into()], 10),
            vec!["12345678".to_string(), String::new()]
        );
        assert_eq!(
            repl_cursor_position_for_cols("  ", "12345678", 8, 10),
            (2, 1)
        );
    }

    #[test]
    fn history_browsing_requires_empty_or_clean_history_input() {
        let history = vec!["first".to_string(), "second".to_string()];

        assert!(repl_should_browse_history("", &history, None));
        assert!(repl_should_browse_history("second", &history, Some(1)));
        assert!(!repl_should_browse_history("draft", &history, None));
        assert!(!repl_should_browse_history(
            "second edited",
            &history,
            Some(1)
        ));
    }

    #[test]
    fn vertical_cursor_move_uses_soft_wrapped_rows() {
        assert_eq!(
            repl_move_cursor_vertical_for_cols("  ", "123456789", 9, -1, 10),
            1
        );
        assert_eq!(
            repl_move_cursor_vertical_for_cols("  ", "123456789", 1, 1, 10),
            9
        );
    }

    #[test]
    fn vertical_cursor_move_handles_explicit_newlines() {
        assert_eq!(
            repl_move_cursor_vertical_for_cols("  ", "abc\ndef", 6, -1, 20),
            2
        );
        assert_eq!(
            repl_move_cursor_vertical_for_cols("  ", "abc\ndef", 2, 1, 20),
            6
        );
    }

    #[test]
    fn vertical_cursor_move_handles_wide_chars_near_wrap() {
        assert_eq!(
            repl_cursor_position_for_cols("  ", "1234567你", 8, 11),
            (2, 1)
        );
        assert_eq!(
            repl_cursor_position_for_cols("  ", "12345678你", 9, 11),
            (4, 1)
        );
        assert_eq!(
            repl_move_cursor_vertical_for_cols("  ", "12345678你好", 9, -1, 11),
            2
        );
    }

    #[test]
    fn reset_is_a_repl_command() {
        assert!(repl_commands().contains(&"/reset"));
    }

    #[test]
    fn compact_is_a_repl_command() {
        assert!(repl_commands().contains(&"/compact"));
    }

    #[test]
    fn pop_is_a_repl_command_with_an_optional_count() {
        assert!(repl_commands().contains(&"/pop"));
        assert_eq!(split_repl_command("/pop 3"), ("/pop", "3"));
        assert_eq!(resolve_repl_command("/p"), "/pop");
    }

    #[test]
    fn variant_is_a_repl_command_with_arguments() {
        assert!(repl_commands().contains(&"/variant"));
        assert_eq!(split_repl_command("/variant high"), ("/variant", "high"));
        assert_eq!(split_repl_command("/reset all"), ("/reset", "all"));
        assert_eq!(resolve_repl_command("/var"), "/variant");
    }

    #[test]
    fn variant_menu_checks_pending_selection_before_confirming() {
        let options = ThinkingVariantOptions {
            provider_id: "ririxin".to_string(),
            model: "deepseek-v4-flash".to_string(),
            variants: vec!["high".to_string(), "max".to_string()],
            selected: Some("high".to_string()),
        };
        let mut item = VariantMenuItem::from_options(&options);
        assert_eq!(
            item.options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "high", "max"]
        );
        assert_eq!(item.selection().2.as_deref(), Some("high"));

        item.cursor = 2;
        assert_eq!(item.selection().2.as_deref(), Some("high"));
        item.check_cursor();
        assert_eq!(item.selection().2.as_deref(), Some("max"));
    }

    #[test]
    fn single_variant_menu_uses_content_width() {
        let item = VariantMenuItem::from_options(&ThinkingVariantOptions {
            provider_id: "ririxin".to_string(),
            model: "deepseek-v4-flash".to_string(),
            variants: vec!["high".to_string(), "max".to_string()],
            selected: None,
        });

        assert!(single_variant_content_width(&item) < 30);
    }

    #[test]
    fn mixed_variant_columns_do_not_fill_wide_terminal() {
        let items = ["myopencode", "myopencode6"]
            .into_iter()
            .map(|provider_id| {
                VariantMenuItem::from_options(&ThinkingVariantOptions {
                    provider_id: provider_id.to_string(),
                    model: "deepseek-v4-flash-free".to_string(),
                    variants: vec!["high".to_string(), "max".to_string()],
                    selected: None,
                })
            })
            .collect::<Vec<_>>();

        let (left, right) = variant_menu_column_widths(&items, 120);
        assert!(left + right < 80);
        assert!(left >= visible_width("myopencode6 / deepseek-v4-flash-free") + 2);
        assert!(right >= visible_width("[*] default") + 2);
    }

    #[test]
    fn mixed_endpoint_label_only_omits_unset_variant() {
        assert_eq!(
            mixed_model_endpoint_label("provider", "model", None),
            "provider / model"
        );
        assert_eq!(
            mixed_model_endpoint_label("provider", "model", Some("default")),
            "provider / model · default"
        );
        assert_eq!(
            mixed_model_endpoint_label("provider", "model", Some("high")),
            "provider / model · high"
        );
    }

    #[test]
    fn variant_menu_distinguishes_unset_from_default_effort() {
        let options = ThinkingVariantOptions {
            provider_id: "groq".to_string(),
            model: "qwen/qwen3-32b".to_string(),
            variants: vec!["none".to_string(), "default".to_string()],
            selected: Some("default".to_string()),
        };
        let item = VariantMenuItem::from_options(&options);

        assert_eq!(item.options[0].label, "default");
        assert_eq!(item.options[0].value, None);
        assert_eq!(item.options[2].label, "default (variant)");
        assert_eq!(item.options[2].value.as_deref(), Some("default"));
        assert_eq!(item.selected, 2);
        assert_eq!(item.selection().2.as_deref(), Some("default"));
    }

    #[test]
    fn explicit_variant_prefix_can_select_default_effort() {
        let argument = "variant:default";
        assert_eq!(argument.strip_prefix("variant:"), Some("default"));
        assert_ne!(argument, "default");
    }

    #[test]
    fn variant_name_resolution_handles_default_and_case_insensitive_names() {
        let available = vec!["low".to_string(), "high".to_string(), "default".to_string()];

        assert_eq!(
            resolve_variant_name("HIGH", &available).unwrap(),
            Some("high".into())
        );
        assert_eq!(resolve_variant_name("default", &available).unwrap(), None);
        assert_eq!(
            resolve_variant_name("variant:default", &available).unwrap(),
            Some("default".into())
        );
        assert!(resolve_variant_name("unknown", &available).is_err());
        assert!(resolve_variant_name("Variant:default", &available).is_err());
    }

    #[test]
    fn command_suggestions_are_prefixed_and_truncated() {
        let suggestions = repl_command_suggestions("/");
        let line = repl_command_suggestions_line(&suggestions, 24);
        assert!(line.starts_with("/models"));
        assert!(visible_width(&line) <= 24);

        let line = repl_command_suggestions_line(&["/compact"], 40);
        assert_eq!(line, "/compact");
    }

    #[test]
    fn truncation_respects_very_narrow_widths() {
        assert_eq!(truncate_visible_width("abcdef", 0), "");
        assert_eq!(truncate_visible_width("abcdef", 1), ".");
        assert_eq!(truncate_visible_width("abcdef", 2), "..");
        assert_eq!(truncate_visible_width("abcdef", 3), "...");
    }

    #[test]
    fn shortcut_hint_line_is_bar_aligned_and_truncated() {
        let line = repl_shortcut_hint_line(AgentMode::Normal, 24);
        assert!(strip_terminal_control_sequences(&line).contains("Tab"));
        assert!(visible_width(&line) <= 24);
    }

    #[test]
    fn inline_fuzzy_lines_are_bar_aligned_and_truncated() {
        let header = inline_fuzzy_header("big", 12);
        assert!(strip_terminal_control_sequences(&header).contains(t("Select", "选择模型")));
        assert!(visible_width(&header) <= 12);

        let item = inline_fuzzy_item_line("opencode Zen / big-pickle", true, false, 16);
        let item_plain = strip_terminal_control_sequences(&item);
        assert!(item_plain.starts_with("› [ ]"));
        assert!(item_plain.contains("open"));
        assert!(visible_width(&item) <= 16);

        let item = inline_fuzzy_item_line("opencode Zen / big-pickle", false, true, 18);
        let item_plain = strip_terminal_control_sequences(&item);
        assert!(item_plain.starts_with("  [*]"));
        assert!(item_plain.contains("opencode"));
        assert!(visible_width(&item) <= 18);

        let help = inline_fuzzy_help_line(40);
        let help_plain = strip_terminal_control_sequences(&help);
        assert!(help_plain.contains("j/k"));
        assert!(visible_width(&help) <= 40);
    }

    #[test]
    fn partial_slash_command_resolves_unique_match() {
        assert_eq!(resolve_repl_command("/model"), "/models");
        assert_eq!(resolve_repl_command("/compa"), "/compact");
        assert_eq!(resolve_repl_command("/co"), "/co");
        assert_eq!(resolve_repl_command("hello"), "hello");
    }

    #[test]
    fn drain_stdin_does_not_panic() {
        drain_stdin();
    }

    #[test]
    fn input_helpers_edit_at_cursor() {
        let mut input = "abcd".to_string();
        let mut cursor = 2;
        insert_char_at_cursor(&mut input, &mut cursor, '中');
        assert_eq!(input, "ab中cd");
        assert_eq!(cursor, 3);

        remove_char_before_cursor(&mut input, &mut cursor);
        assert_eq!(input, "abcd");
        assert_eq!(cursor, 2);

        remove_char_at_cursor(&mut input, cursor);
        assert_eq!(input, "abd");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn input_helpers_remove_word_before_cursor() {
        let mut input = "hello world  ".to_string();
        let mut cursor = input.chars().count();
        remove_word_before_cursor(&mut input, &mut cursor);
        assert_eq!(input, "hello ");
        assert_eq!(cursor, 6);

        let mut input = "前面 中间 后面".to_string();
        let mut cursor = 6;
        remove_word_before_cursor(&mut input, &mut cursor);
        assert_eq!(input, "前面 后面");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn input_helpers_insert_paste_at_cursor() {
        let mut input = "前后".to_string();
        let mut cursor = 1;
        insert_str_at_cursor(&mut input, &mut cursor, "中间");
        assert_eq!(input, "前中间后");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn input_helpers_insert_newline_at_cursor() {
        let mut input = "前后".to_string();
        let mut cursor = 1;
        insert_newline_at_cursor(&mut input, &mut cursor);
        assert_eq!(input, "前\n后");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn long_paste_visible_lines_are_collapsed() {
        let lines = (0..20)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>();
        let visible = repl_visible_input_lines("[NORMAL] > ", &lines, 12, true);

        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0], "line 0");
        assert!(visible[1].contains("18") || visible[1].contains("已隐藏 18"));
        assert_eq!(visible[2], "line 19");
        assert_eq!(lines.len(), 20);
    }

    #[test]
    fn long_paste_is_replaced_with_placeholder_and_expanded() {
        let text = "alpha\nbeta\ngamma".to_string();
        let placeholder = pasted_text_placeholder(1, pasted_text_line_count(&text));
        let input = format!("请分析 {placeholder}谢谢");
        let pasted_texts = vec![Some(PastedText { text: text.clone() })];

        assert!(should_summarize_pasted_text(&text));
        assert_eq!(
            expand_pasted_text_placeholders(&input, &pasted_texts),
            "请分析 alpha\nbeta\ngamma谢谢"
        );
    }

    #[test]
    fn short_paste_is_not_summarized() {
        assert!(!should_summarize_pasted_text("short paste"));
    }

    #[test]
    fn insert_pasted_text_summarizes_long_clipboard_text() {
        let mut input = "前后".to_string();
        let mut cursor = 1;
        let mut pasted_texts = Vec::new();

        insert_pasted_text_at_cursor(
            &mut input,
            &mut cursor,
            "alpha\nbeta\ngamma".to_string(),
            &mut pasted_texts,
        );

        assert!(
            input == "前[Pasted 1: ~3 lines]后" || input == "前[粘贴 1: ~3 行]后",
            "unexpected localized placeholder: {input}"
        );
        assert_eq!(pasted_texts.len(), 1);
        assert_eq!(cursor, input.chars().count() - 1);
    }

    #[test]
    fn pasted_placeholder_is_treated_as_atomic_token() {
        let input = "前[Pasted 1: ~3 lines] 后";
        assert_eq!(placeholder_at_cursor(input, 3), Some((1, 21)));
        assert_eq!(placeholder_before_cursor(input, 21), Some((1, 21)));
        assert_eq!(placeholder_after_cursor(input, 1), Some((1, 21)));
        assert_eq!(placeholder_before_or_at_cursor(input, 3), Some((1, 21)));
        assert_eq!(placeholder_after_or_at_cursor(input, 3), Some((1, 21)));
    }

    #[test]
    fn chinese_pasted_placeholder_is_supported() {
        let input = "前[粘贴 1: ~3 行] 后";
        let placeholder = find_pasted_text_placeholders(input);

        assert_eq!(placeholder, vec![(1, 13, 1)]);
        assert_eq!(placeholder_at_cursor(input, 3), Some((1, 13)));
        assert_eq!(placeholder_before_cursor(input, 13), Some((1, 13)));
        assert_eq!(placeholder_after_cursor(input, 1), Some((1, 13)));
    }

    #[test]
    fn colorizes_image_and_pasted_placeholders() {
        let colored = colorize_repl_placeholders("[Image 1] [Pasted 1: ~3 lines]");
        assert!(colored.contains("\x1b[35m[Image 1]\x1b[0m"));
        assert!(colored.contains("\x1b[35m[Pasted 1: ~3 lines]\x1b[0m"));
    }

    #[test]
    fn placeholder_text_near_cursor_expands_pasted_placeholder() {
        let input = "前[Pasted 1: ~3 lines]后";
        let pasted_texts = vec![Some(PastedText {
            text: "alpha\nbeta\ngamma".to_string(),
        })];

        assert_eq!(
            placeholder_text_near_cursor(input, 3, &pasted_texts),
            Some("alpha\nbeta\ngamma".to_string())
        );
    }

    #[test]
    fn strips_terminal_control_sequences_from_repl_text() {
        assert_eq!(
            strip_terminal_control_sequences("\x1b[E表情包\x1b[0m\x07 ok"),
            "表情包 ok"
        );
        assert_eq!(
            strip_terminal_control_sequences("line1\nline2\tend"),
            "line1\nline2\tend"
        );
    }

    #[test]
    fn repl_history_loads_user_messages_from_state() {
        let temp = tempfile::tempdir().unwrap();
        let paths = GqyPaths {
            config_dir: PathBuf::new(),
            config_file: PathBuf::new(),
            skills_dir: PathBuf::new(),
            data_dir: PathBuf::new(),
            cache_dir: PathBuf::new(),
            state_dir: temp.path().to_path_buf(),
            pictures_dir: PathBuf::new(),
            fish_hook_file: PathBuf::new(),
            bash_hook_file: PathBuf::new(),
            zsh_hook_file: PathBuf::new(),
            scripts_dir: PathBuf::new(),
            system_scripts_dir: PathBuf::new(),
            share_dir: PathBuf::new(),
            kb_dir: PathBuf::new(),
        };
        let state = StateStore::new(&paths).unwrap();
        state.start_turn("turn_1", "first", 999999).unwrap();
        state.complete_turn("turn_1", "reply", None).unwrap();
        state.start_turn("turn_2", "second", 999999).unwrap();

        assert_eq!(
            load_repl_input_history(&state).unwrap(),
            vec!["first".to_string(), "second".to_string()]
        );
    }
}
