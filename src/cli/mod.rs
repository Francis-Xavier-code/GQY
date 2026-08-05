//! CLI entry: argument re-exports, command dispatch, and shared helpers.
pub mod args;
pub mod commands;
pub mod repl;

pub use args::*;

use repl::{
    find_image_placeholders, find_pasted_text_placeholders, handle_live_agent_event,
    input_prompt_bar, pasted_text_line_count, pasted_text_placeholder, print_mixed_model_endpoint,
    should_summarize_pasted_text, show_mixed_model_endpoint, strip_terminal_control_sequences,
    truncate_visible_width, visible_width, LiveReplTail,
};

use crate::agent::{
    archive_and_delete_visible_turns, Agent, AgentEvent, AgentMode,
};
use crate::backup::{BackupInitOptions, RestoreOptions};
use crate::config::{ActiveProviderModelConfig, AppConfig};
use crate::i18n::{is_zh, text as t};
use crate::llm::{ChatStreamChunk, LlmClient, ThinkingVariantOptions};
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use crate::render;
use crate::shell;
use crate::state::{QueuedPrompt, QueuedPromptAttachment, StateStore, Turn, TurnStatus};
use crate::tools;
use anyhow::{bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Local};
use clap::{Arg, ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use crossterm::cursor::{self, Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::style::Print;
use crossterm::terminal::{self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use crossterm::{execute, queue};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::ffi::OsString;
use std::io::Cursor;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use vte::{Params as VteParams, Parser as VteParser, Perform as VtePerform};

pub async fn run(cli: Cli, paths: GqyPaths) -> Result<()> {
    if cli.shell_classify {
        let shell_name = cli.shell.as_deref().unwrap_or("fish");
        let message = shell_message_from_input(cli.stdin, cli.message)?;
        return run_shell_classify(shell_name, &message);
    }

    if cli.clipboard_paste {
        return run_clipboard_paste(&paths);
    }
    let _logging_guard = match crate::logging::init(&paths, cli.debug) {
        Ok(guard) => Some(guard),
        Err(err) => {
            eprintln!(
                "{}: {err:#}",
                t(
                    "warning: diagnostic logging is unavailable",
                    "警告：诊断日志不可用"
                )
            );
            None
        }
    };
    let mode = if cli.plan {
        AgentMode::Plan
    } else {
        AgentMode::Normal
    };

    crate::models_cache::try_load(&paths);
    crate::models_cache::spawn_background_refresh(paths.clone());

    if cli.shell_intercept {
        let shell_name = cli.shell.as_deref().unwrap_or("fish");
        let message = shell_message_from_input(cli.stdin, cli.message)?;
        return run_shell_intercept(&paths, shell_name, message).await;
    }

    if !paths.config_file.exists()
        && !matches!(
            cli.command,
            Some(Command::Init)
                | Some(Command::FishInit)
                | Some(Command::BashInit)
                | Some(Command::ZshInit)
                | Some(Command::RemoveShellHook)
                | Some(Command::Paths)
                | Some(Command::Backup(_))
                | Some(Command::Preview)
        )
    {
        run_init(&paths, InitKind::FirstRun)?;
    }

    match cli.command {
        Some(Command::Alarm(args)) => commands::alarm::run_alarm_cmd(&paths, args),
        Some(Command::Watch(args)) => commands::watch::run_watch(&paths, args),
        Some(Command::Tts(args)) => commands::tts::run_tts(args),
        Some(Command::Stt(args)) => commands::stt::run_stt(&paths, args),
        Some(Command::Provider(args)) => commands::provider::run_provider(&paths, args).await,
        Some(Command::AlarmWorker(args)) => commands::alarm::run_alarm_worker(args),
        Some(Command::Tool(args)) => commands::tool::run_tool(&paths, mode, args).await,
        Some(Command::Preview) => {
            crate::repl_avatar::print_startup_banner(
                &mut io::stdout(),
                "opencode",
                "big-pickle",
                t("normal", "普通"),
                0,
                0,
            );
            render::print_markdown(
                "# 月夜清影\n\n你好呀，我是**顾清影** —— 活在终端里的二次元少女。\n\
                 \n\
                 ## 今天的待办\n\n\
                 - 修好 [GQY 的 GitHub](https://github.com/Francis-Xavier-code/GQY)\n\
                 - 学习 `Rust` 与 `OSC 8` 超链接\n\
                 - [x] 月夜主题已上线\n\
                 - [ ] 面板动画（下次）\n\n\
                 > 链接在 iTerm2 / Terminal.app / kitty 里都可以点击\n\n\
                 ```rust\nfn greet() -> &'static str {\n    \"清影陪你写代码\"\n}\n```\n\n\
                 | 项目 | 状态 |\n| --- | --- |\n| 月夜主题 | ✅ 完成 |\n| 可点击链接 | ✅ 完成 |\n| 结构化渲染 | ✅ 完成 |\n",
            );
            Ok(())
        }
        Some(Command::Ask(args)) => {
            run_chat_with_options(&paths, join_message(args.message), None, cli.stdout, mode, cli.dry_run).await
        }
        Some(Command::Init) => run_init(&paths, InitKind::Explicit),
        Some(Command::Paths) => {
            paths.print();
            Ok(())
        }
        Some(Command::Config(args)) => run_config(&paths, args).await,
        Some(Command::Models(args)) => run_models(&paths, args),
        Some(Command::Variant(args)) => run_variant(&paths, args),
        Some(Command::FishInit) => shell::fish::install(&paths),
        Some(Command::BashInit) => shell::bash::install(&paths),
        Some(Command::ZshInit) => shell::zsh::install(&paths),
        Some(Command::RemoveShellHook) => remove_shell_hooks(&paths),
        Some(Command::History(args)) => commands::history::run_history(&paths, args),
        Some(Command::Activity(args)) => commands::activity::run_activity(&paths, args),
        Some(Command::Archive(args)) => commands::archive::run_archive(&paths, args),
        Some(Command::Pop(args)) => run_pop(&paths, args),
        Some(Command::Memes(args)) => commands::memes::run_memes(&paths, args),
        Some(Command::Kb(args)) => commands::kb::run_kb(&paths, args).await,
        Some(Command::Memory(args)) => commands::memory::run_memory(&paths, args),
        Some(Command::Backup(args)) => commands::backup::run_backup(&paths, args),
        Some(Command::Skills(args)) => commands::skills::run_skills(&paths, args),
        Some(Command::Tools(args)) => commands::tools::run_tools(&paths, args),
        Some(Command::Reset(args)) => commands::reset::run_reset(&paths, args.scope.as_deref()),
        Some(Command::Web(args)) => crate::web::run(paths, args).await,
        Some(Command::Balance) => commands::balance::run_balance(&paths),
        Some(Command::Napcat(args)) => crate::bridges::napcat::run(&paths, args).await,
        Some(Command::Tg(args)) => crate::bridges::tg::run(&paths, args).await,
        None => {
            let message = join_message(cli.message);
            if message.is_empty() && io::stdin().is_terminal() {
                repl::run_repl(&paths, mode).await
            } else {
                run_chat_with_options(&paths, message, None, cli.stdout, mode, cli.dry_run).await
            }
        }
    }
}


#[derive(Clone, Copy, PartialEq, Eq)]
enum InitKind {
    FirstRun,
    Explicit,
}

fn run_init(paths: &GqyPaths, kind: InitKind) -> Result<()> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if interactive {
        println!(
            "{}\n",
            match kind {
                InitKind::FirstRun => t("GQY first start", "GQY 首次启动"),
                InitKind::Explicit => t("GQY initialization", "GQY 初始化"),
            }
        );
    }
    print_init_step(
        interactive,
        t("Preparing config directory", "正在准备配置目录"),
        &paths.config_dir.display().to_string(),
    )?;
    AppConfig::init_files(paths)?;
    print_init_step(
        interactive,
        t("Writing default config", "正在写入默认配置"),
        &paths.config_file.display().to_string(),
    )?;
    print_init_step(
        interactive,
        t("Creating state files", "正在创建状态文件"),
        &paths.state_dir.display().to_string(),
    )?;
    StateStore::new(paths)?.init_files()?;
    print_init_step(
        interactive,
        t("Preparing data directory", "正在准备数据目录"),
        &paths.data_dir.display().to_string(),
    )?;
    // 默认文件播种：scripts 索引不存在时创建空索引（GQY 扫描依赖它），
    // 用户知识库为空且随包有默认 kb 时自动导入（brew 安装后开箱即用）
    let scripts_index = paths.config_dir.join("scripts/index.json");
    if !scripts_index.exists() {
        std::fs::create_dir_all(paths.config_dir.join("scripts"))?;
        std::fs::write(
            &scripts_index,
            "{\n  \"scripts\": []\n}\n",
        )?;
    }
    if kind == InitKind::FirstRun {
        seed_default_knowledge_base(paths, interactive)?;
    }
    if interactive {
        println!("\n{}\n", t("Initialization complete.", "初始化完成。"));
        std::thread::sleep(Duration::from_millis(420));
        prompt_shell_init_menu(paths)?;
    } else {
        println!(
            "{} {}",
            t("initialized GQY at", "GQY 已初始化于"),
            paths.config_dir.display()
        );
    }
    Ok(())
}

/// 首次运行时：若用户知识库为空且随包携带默认 kb 目录（brew 的 share/gqy/kb），
/// 自动导入，让 `gqy kb search` 开箱即用（幂等：已有内容则跳过）。
fn seed_default_knowledge_base(paths: &GqyPaths, interactive: bool) -> Result<()> {
    let kb_root = paths.data_dir.join("kb");
    let has_content = kb_root.exists()
        && std::fs::read_dir(&kb_root).map(|mut entries| entries.next().is_some()).unwrap_or(false);
    let kb_dir = paths.kb_dir.clone();
    let has_bundled = kb_dir.is_dir()
        && std::fs::read_dir(&kb_dir)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
    if has_content || !has_bundled {
        return Ok(());
    }
    let config = AppConfig::load(paths)?;
    let kb = tools::knowledge_base::KnowledgeBase::new(config, paths.clone())?;
    let imported = match kb.add_path_sync(&kb_dir) {
        Ok(imported) => imported,
        Err(_) => return Ok(()),
    };
    if imported.is_empty() {
        return Ok(());
    }
    let _ = print_init_step(
        interactive,
        t("Importing bundled knowledge base", "正在导入随包知识库"),
        &format!("{} 条文档", imported.len()),
    );
    Ok(())
}

fn print_init_step(interactive: bool, label: &str, value: &str) -> Result<()> {
    if interactive {
        std::thread::sleep(Duration::from_millis(180));
        println!("  {label:<24} ✓ {value}");
        io::stdout().flush()?;
    }
    Ok(())
}

fn prompt_shell_init_menu(paths: &GqyPaths) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Ok(());
    }
    println!("{}", t("Integrate with shell?", "是否集成到 shell？"));
    println!(
        "{}\n",
        t(
            "After integration, you can chat in natural language directly in the terminal.",
            "集成后可在终端直接使用自然语言交流。"
        )
    );
    match select_shell_hook()? {
        Some("fish") => shell::fish::install(paths),
        Some("bash") => shell::bash::install(paths),
        Some("zsh") => shell::zsh::install(paths),
        _ => Ok(()),
    }
}

fn select_shell_hook() -> Result<Option<&'static str>> {
    let options = [
        (t("Skip", "跳过"), None),
        ("fish", Some("fish")),
        ("bash", Some("bash")),
        ("zsh", Some("zsh")),
    ];
    let detected = shell::current_parent_shell();
    let mut selected = detected
        .as_deref()
        .and_then(|shell| options.iter().position(|(_, value)| *value == Some(shell)))
        .unwrap_or(0);
    let mut stdout = io::stdout();
    let (_, menu_row) = cursor::position()?;
    execute!(stdout, Hide)?;
    struct ShellMenuGuard;
    impl Drop for ShellMenuGuard {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), Show);
        }
    }
    let _guard = ShellMenuGuard;
    loop {
        queue!(
            stdout,
            MoveTo(0, menu_row),
            Clear(ClearType::FromCursorDown)
        )?;
        for (index, (label, _)) in options.iter().enumerate() {
            if index == selected {
                queue!(stdout, Print(format!("> {label}\n")))?;
            } else {
                queue!(stdout, Print(format!("  {label}\n")))?;
            }
        }
        queue!(
            stdout,
            Print(format!(
                "\n\x1b[2m{}\x1b[0m",
                t(
                    "Up/Down or j/k to choose, Enter to confirm, Esc/q to skip",
                    "↑/↓ 或 j/k 选择，Enter 确认，Esc/q 跳过"
                )
            ))
        )?;
        stdout.flush()?;
        terminal::enable_raw_mode()?;
        let key = read_shell_menu_key();
        terminal::disable_raw_mode()?;
        match key? {
            KeyCode::Esc | KeyCode::Char('q') => {
                execute!(stdout, Show)?;
                return Ok(None);
            }
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Enter => {
                execute!(stdout, Show)?;
                return Ok(options[selected].1);
            }
            _ => {}
        }
    }
}

fn read_shell_menu_key() -> Result<KeyCode> {
    loop {
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            return Ok(code);
        }
    }
}

fn remove_shell_hooks(paths: &GqyPaths) -> Result<()> {
    let removed = shell::fish::uninstall(paths)?;
    let removed = shell::bash::uninstall(paths)? || removed;
    let removed = shell::zsh::uninstall(paths)? || removed;
    if !removed {
        println!(
            "{}",
            t(
                "no installed GQY shell hooks found",
                "未找到已安装的 GQY shell hook"
            )
        );
    }
    Ok(())
}


/// 管家监控：采样系统进程，检测到异常时给运行中的 WebUI 会话入队主动消息。


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PopOutcome {
    turns: usize,
    archived: bool,
}

fn run_pop(paths: &GqyPaths, args: PopArgs) -> Result<()> {
    let config = AppConfig::load_or_default(paths)?;
    let state = StateStore::new(paths)?;
    state.recover_stale_turns()?;
    if let Some(outcome) = execute_pop(paths, &config, &state, args.count)? {
        print_pop_outcome(outcome);
    }
    Ok(())
}

fn execute_pop(
    paths: &GqyPaths,
    config: &AppConfig,
    state: &StateStore,
    count: Option<usize>,
) -> Result<Option<PopOutcome>> {
    let turns = match count {
        Some(count) => {
            validate_pop_count(count)?;
            state.oldest_evictable_visible_turns(count)?
        }
        None => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                bail!(
                    "{}",
                    t(
                        "interactive pop requires a terminal; use `gqy pop <count>`",
                        "交互 pop 需要终端；请使用 `gqy pop <数量>`",
                    )
                );
            }
            let limit = usize::try_from(i64::MAX).unwrap_or(usize::MAX);
            let candidates = state.oldest_evictable_visible_turns(limit)?;
            if candidates.is_empty() {
                print_nothing_to_pop();
                return Ok(None);
            }
            let Some(selected) = inline_pop_select(&candidates)? else {
                return Ok(None);
            };
            let selected = candidates
                .into_iter()
                .zip(selected)
                .filter_map(|(turn, selected)| selected.then_some(turn))
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Ok(None);
            }
            selected
        }
    };
    if turns.is_empty() {
        print_nothing_to_pop();
        return Ok(None);
    }

    let memory = MemoryStore::new(config, paths);
    archive_and_delete_visible_turns(state, &memory, &turns)?;
    let memory_config = config.memory_config();
    Ok(Some(PopOutcome {
        turns: turns.len(),
        archived: memory_config.enabled && memory_config.evicted_context_enabled,
    }))
}

fn validate_pop_count(count: usize) -> Result<usize> {
    if count == 0 {
        bail!(
            "{}",
            t("pop count must be greater than zero", "pop 数量必须大于 0")
        );
    }
    Ok(count)
}

fn parse_positive_pop_count(value: &str) -> std::result::Result<usize, String> {
    let count = value.parse::<usize>().map_err(|_| {
        t(
            "pop count must be a positive integer",
            "pop 数量必须是正整数",
        )
        .to_string()
    })?;
    if count == 0 {
        return Err(t("pop count must be greater than zero", "pop 数量必须大于 0").to_string());
    }
    Ok(count)
}

fn parse_repl_pop_count(args: &str) -> Result<Option<usize>> {
    let mut parts = args.split_whitespace();
    let Some(value) = parts.next() else {
        return Ok(None);
    };
    if parts.next().is_some() {
        bail!(
            "{}",
            t("usage: /pop [positive integer]", "用法：/pop [正整数]")
        );
    }
    let count = parse_positive_pop_count(value).map_err(anyhow::Error::msg)?;
    validate_pop_count(count).map(Some)
}

fn print_pop_outcome(outcome: PopOutcome) {
    let message = if is_zh() {
        if outcome.archived {
            format!("已弹出 {} 轮 · 已归档", outcome.turns)
        } else {
            format!(
                "已弹出 {} 轮 · 未归档（弹出上下文归档已关闭）",
                outcome.turns
            )
        }
    } else {
        let turns = if outcome.turns == 1 { "turn" } else { "turns" };
        if outcome.archived {
            format!("popped {} {turns} · archived", outcome.turns)
        } else {
            format!(
                "popped {} {turns} · not archived (evicted-context archiving is disabled)",
                outcome.turns
            )
        }
    };
    println!("\x1b[2m{message}\x1b[0m\n");
}

fn print_nothing_to_pop() {
    println!(
        "\x1b[2m{}\x1b[0m\n",
        t(
            "no conversation turns are available to pop",
            "没有可弹出的上下文轮次"
        )
    );
}

fn inline_pop_select(turns: &[Turn]) -> Result<Option<Vec<bool>>> {
    let menu_lines = inline_pop_lines(turns.len());
    let visible_items = menu_lines.saturating_sub(2) as usize / 3;
    reserve_inline_fuzzy_space(menu_lines)?;
    let mut session = InlineRawMode::start()?;
    let matcher = SkimMatcherV2::default();
    let search_items = turns.iter().map(pop_search_text).collect::<Vec<_>>();
    let mut active = vec![false; turns.len()];
    let mut query = String::new();
    let mut selected = 0usize;
    let mut scroll = 0usize;
    let (_, cursor_y) = cursor::position().unwrap_or((0, menu_lines.saturating_sub(1)));
    let anchor_y = cursor_y.saturating_sub(menu_lines.saturating_sub(1));
    loop {
        let matches = pop_matches(&matcher, &search_items, &query);
        if selected >= matches.len() {
            selected = matches.len().saturating_sub(1);
        }
        let visible = matches.len().min(visible_items);
        scroll = inline_fuzzy_scroll(selected, scroll, visible);
        draw_inline_pop(
            &mut session.stdout,
            anchor_y,
            menu_lines,
            turns,
            &matches,
            selected,
            scroll,
            &active,
            &query,
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
                KeyCode::Esc => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Char('q') => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Enter => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(active));
                }
                KeyCode::Tab => {
                    if let Some(index) = matches.get(selected) {
                        if let Some(value) = active.get_mut(*index) {
                            *value = !*value;
                        }
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(matches.len().saturating_sub(1));
                }
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0;
                    scroll = 0;
                }
                KeyCode::Char(ch) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    query.push(ch);
                    selected = 0;
                    scroll = 0;
                }
                _ => {}
            }
        }
    }
}

fn pop_matches(matcher: &SkimMatcherV2, items: &[String], query: &str) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (query.trim().is_empty() || matcher.fuzzy_match(item, query).is_some()).then_some(index)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_inline_pop(
    stdout: &mut io::Stdout,
    anchor_y: u16,
    menu_lines: u16,
    turns: &[Turn],
    matches: &[usize],
    selected: usize,
    scroll: usize,
    active: &[bool],
    query: &str,
) -> Result<()> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let bar = inline_fuzzy_bar();
    let width = (cols as usize).saturating_sub(visible_width(&bar)).max(1);
    let visible_items = menu_lines.saturating_sub(2) as usize / 3;
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
        Print(pop_menu_header(
            query,
            active.iter().filter(|selected| **selected).count(),
            turns.len(),
            width,
        )),
    )?;
    if matches.is_empty() {
        queue!(
            stdout,
            MoveTo(0, anchor_y + 1),
            Print(&bar),
            Print(format!("\x1b[2m{}\x1b[0m", t("no matches", "没有匹配项")))
        )?;
    } else {
        for (row, item_index) in matches.iter().skip(scroll).take(visible_items).enumerate() {
            let focused = scroll + row == selected;
            let checked = active.get(*item_index).copied().unwrap_or(false);
            let lines = pop_menu_turn_lines(&turns[*item_index], focused, checked, width);
            for (line_offset, line) in lines.into_iter().enumerate() {
                queue!(
                    stdout,
                    MoveTo(0, anchor_y + 1 + row as u16 * 3 + line_offset as u16),
                    Print(&bar),
                    Print(line)
                )?;
            }
        }
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y + menu_lines.saturating_sub(1)),
        Print(&bar),
        Print(pop_menu_help_line(width))
    )?;
    stdout.flush()?;
    Ok(())
}

fn pop_menu_header(query: &str, selected: usize, total: usize, width: usize) -> String {
    let title = if query.trim().is_empty() {
        t("Pop context", "弹出上下文").to_string()
    } else {
        format!(
            "{} · {}: {}",
            t("Pop context", "弹出上下文"),
            t("Search", "搜索"),
            query.trim()
        )
    };
    let count = if is_zh() {
        format!("已选 {selected} / {total}")
    } else {
        format!("selected {selected} / {total}")
    };
    let count_width = visible_width(&count);
    if count_width >= width {
        return format!("\x1b[2m{}\x1b[0m", truncate_visible_width(&count, width));
    }
    let title_width = width.saturating_sub(count_width + 1);
    let title = truncate_visible_width(&title, title_width);
    let gap = width
        .saturating_sub(visible_width(&title).saturating_add(count_width))
        .max(1);
    format!(
        "\x1b[1m{title}\x1b[0m{}\x1b[2m{count}\x1b[0m",
        " ".repeat(gap)
    )
}

fn pop_menu_turn_lines(turn: &Turn, focused: bool, checked: bool, width: usize) -> [String; 3] {
    let cursor = if focused { "›" } else { " " };
    let marker = if checked { "[*]" } else { "[ ]" };
    let lines = [
        format!(
            "{cursor} {marker} {}",
            pop_menu_timestamp(&turn.user_timestamp)
        ),
        format!(
            "      {}{}",
            t("You: ", "你："),
            pop_menu_summary(&turn.user_content)
        ),
        format!(
            "      {}{}",
            t("AI: ", "AI："),
            pop_menu_assistant_summary(turn)
        ),
    ];
    lines.map(|line| {
        let line = truncate_visible_width(&line, width);
        if focused {
            format!("\x1b[1m\x1b[35m{line}\x1b[0m")
        } else if checked {
            format!("\x1b[1m\x1b[32m{line}\x1b[0m")
        } else {
            format!("\x1b[2m{line}\x1b[0m")
        }
    })
}

fn pop_menu_timestamp(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| pop_menu_summary(timestamp))
}

fn pop_menu_assistant_summary(turn: &Turn) -> String {
    if turn.status == TurnStatus::Interrupted {
        t("(reply interrupted)", "（回复已中断）").to_string()
    } else {
        pop_menu_summary(&turn.assistant_content)
    }
}

fn pop_menu_summary(content: &str) -> String {
    strip_terminal_control_sequences(content)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| t("(empty)", "（空）"))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn pop_search_text(turn: &Turn) -> String {
    format!(
        "{} {} {}",
        pop_menu_timestamp(&turn.user_timestamp),
        pop_menu_summary(&turn.user_content),
        pop_menu_assistant_summary(turn)
    )
}

fn pop_menu_help_line(width: usize) -> String {
    let line = t(
        "Up/Down or j/k move · Tab toggle · Enter pop · Esc/q cancel",
        "↑/↓ 或 j/k 移动 · Tab 勾选 · Enter 弹出 · Esc/q 取消",
    );
    format!("\x1b[2m{}\x1b[0m", truncate_visible_width(line, width))
}

fn inline_pop_lines(item_count: usize) -> u16 {
    let (_, terminal_rows) = terminal::size().unwrap_or((80, 24));
    let available_items = terminal_rows.saturating_sub(2).saturating_div(3).max(1) as usize;
    let visible_items = item_count.min(5).min(available_items).max(1);
    (visible_items as u16).saturating_mul(3).saturating_add(2)
}

fn run_models(paths: &GqyPaths, args: ModelsArgs) -> Result<()> {
    let mut config = AppConfig::load(paths)?;
    let choices = config.text_provider_model_choices();
    if choices.is_empty() {
        bail!(
            "{}",
            t(
                "no active provider models; configure or activate a model first",
                "没有已激活的 provider 模型；请先配置或激活模型",
            )
        );
    }
    if let Some(index) = args.index {
        if index == 0 || index > choices.len() {
            bail!(
                "{}: {index}",
                t("provider index out of range", "provider 序号超出范围")
            );
        }
        let choice = &choices[index - 1];
        let provider_id = choice.provider_id.clone();
        let model = choice.model.clone();
        let label = choice.label();
        config.set_active_provider_model(&provider_id, &model)?;
        config.save(paths)?;
        println!(
            "{}: {index}. {label}",
            t("active provider", "当前 provider")
        );
        return Ok(());
    }
    if io::stdout().is_terminal() && io::stdin().is_terminal() {
        let active = choices
            .iter()
            .map(|choice| config.is_active_provider_model(&choice.provider_id, &choice.model))
            .collect::<Vec<_>>();
        if let Some(active) = inline_fuzzy_select(
            &choices
                .iter()
                .map(|choice| choice.label())
                .collect::<Vec<_>>(),
            active,
        )? {
            config.active_provider_models = Some(
                choices
                    .iter()
                    .zip(active)
                    .filter_map(|(choice, active)| {
                        active.then(|| ActiveProviderModelConfig {
                            provider_id: choice.provider_id.clone(),
                            model: choice.model.clone(),
                        })
                    })
                    .collect(),
            );
            config.save(paths)?;
            println!("{}", t("active provider models updated", "已更新激活模型"));
        }
        return Ok(());
    }
    for (index, choice) in choices.iter().enumerate() {
        let marker = if config.is_active_provider_model(&choice.provider_id, &choice.model) {
            "[*]"
        } else {
            "[ ]"
        };
        println!("{marker} {}. {}", index + 1, choice.label());
    }
    Ok(())
}

fn inline_fuzzy_select(items: &[String], mut active: Vec<bool>) -> Result<Option<Vec<bool>>> {
    let menu_lines = inline_fuzzy_lines(items.len());
    reserve_inline_fuzzy_space(menu_lines)?;
    let mut session = InlineRawMode::start()?;
    let matcher = SkimMatcherV2::default();
    let mut query = String::new();
    let mut selected = 0usize;
    let mut scroll = 0usize;
    let (_, cursor_y) = cursor::position().unwrap_or((0, menu_lines.saturating_sub(1)));
    let anchor_y = cursor_y.saturating_sub(menu_lines.saturating_sub(1));
    loop {
        let matches = fuzzy_matches(&matcher, items, &query);
        if selected >= matches.len() {
            selected = matches.len().saturating_sub(1);
        }
        let visible = matches.len().min(menu_lines.saturating_sub(2) as usize);
        scroll = inline_fuzzy_scroll(selected, scroll, visible);
        draw_inline_fuzzy(
            &mut session.stdout,
            anchor_y,
            menu_lines,
            &query,
            items,
            &matches,
            selected,
            scroll,
            &active,
        )?;
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        {
            match code {
                KeyCode::Char('c')
                    if modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(None);
                }
                KeyCode::Esc => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(active));
                }
                KeyCode::Char('q') if query.is_empty() => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(active));
                }
                KeyCode::Enter => {
                    clear_inline_fuzzy(&mut session.stdout, anchor_y, menu_lines)?;
                    return Ok(Some(active));
                }
                KeyCode::Tab => {
                    if let Some((_, index)) = matches.get(selected) {
                        if let Some(value) = active.get_mut(*index) {
                            *value = !*value;
                        }
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(matches.len().saturating_sub(1));
                }
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0;
                    scroll = 0;
                }
                KeyCode::Char(ch) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    query.push(ch);
                    selected = 0;
                    scroll = 0;
                }
                _ => {}
            }
        }
    }
}

fn fuzzy_matches(matcher: &SkimMatcherV2, items: &[String], query: &str) -> Vec<(i64, usize)> {
    let mut matches = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if query.trim().is_empty() {
                Some((0, index))
            } else {
                matcher.fuzzy_match(item, query).map(|score| (score, index))
            }
        })
        .collect::<Vec<_>>();
    if !query.trim().is_empty() {
        matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    }
    matches
}

fn draw_inline_fuzzy(
    stdout: &mut io::Stdout,
    anchor_y: u16,
    menu_lines: u16,
    query: &str,
    items: &[String],
    matches: &[(i64, usize)],
    selected: usize,
    scroll: usize,
    active: &[bool],
) -> Result<()> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let bar = inline_fuzzy_bar();
    let width = (cols as usize).saturating_sub(visible_width(&bar)).max(1);
    let visible = matches.len().min(menu_lines.saturating_sub(2) as usize);
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
        Print(inline_fuzzy_header(query, width)),
    )?;
    if matches.is_empty() {
        queue!(
            stdout,
            MoveTo(0, anchor_y + 1),
            Print(&bar),
            Print(format!("\x1b[2m{}\x1b[0m", t("no matches", "没有匹配项")))
        )?;
    } else {
        for (row, (_, item_index)) in matches.iter().skip(scroll).take(visible).enumerate() {
            queue!(
                stdout,
                MoveTo(0, anchor_y + row as u16 + 1),
                Print(&bar),
                Print(inline_fuzzy_item_line(
                    items[*item_index].as_str(),
                    scroll + row == selected,
                    active.get(*item_index).copied().unwrap_or(false),
                    width
                ))
            )?;
        }
    }
    queue!(
        stdout,
        MoveTo(0, anchor_y + menu_lines.saturating_sub(1)),
        Print(&bar),
        Print(inline_fuzzy_help_line(width))
    )?;
    stdout.flush()?;
    Ok(())
}

fn inline_fuzzy_scroll(selected: usize, scroll: usize, visible: usize) -> usize {
    if visible == 0 || selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected + 1 - visible
    } else {
        scroll
    }
}

fn inline_fuzzy_bar() -> String {
    input_prompt_bar(AgentMode::Normal)
}

fn inline_fuzzy_header(query: &str, width: usize) -> String {
    let title = t("Select model", "选择模型");
    let line = if query.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title} · {}", query.trim())
    };
    format!("\x1b[1m{}\x1b[0m", truncate_visible_width(&line, width))
}

fn inline_fuzzy_item_line(item: &str, selected: bool, active: bool, width: usize) -> String {
    let marker = if active { "[*]" } else { "[ ]" };
    let line = if selected {
        format!("› {marker} {item}")
    } else {
        format!("  {marker} {item}")
    };
    let line = truncate_visible_width(&line, width);
    if selected {
        format!(
            "\x1b[1m\x1b[35m›\x1b[0m\x1b[1m{}\x1b[0m",
            line.strip_prefix('›').unwrap_or(&line)
        )
    } else if active {
        format!("\x1b[1m\x1b[32m{}\x1b[0m", line)
    } else {
        format!("\x1b[2m{}\x1b[0m", line)
    }
}

fn inline_fuzzy_help_line(width: usize) -> String {
    let line = t(
        "type search · j/k move · Tab toggle · Enter/q confirm",
        "输入搜索 · j/k 移动 · Tab 切换 · Enter/q 确认",
    );
    format!("\x1b[2m{}\x1b[0m", truncate_visible_width(line, width))
}

fn clear_inline_fuzzy(stdout: &mut io::Stdout, anchor_y: u16, lines: u16) -> Result<()> {
    for row in 0..lines {
        queue!(
            stdout,
            MoveTo(0, anchor_y + row),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(stdout, MoveTo(0, anchor_y), Show)?;
    stdout.flush()?;
    Ok(())
}

fn reserve_inline_fuzzy_space(lines: u16) -> Result<()> {
    for _ in 1..lines {
        println!();
    }
    io::stdout().flush()?;
    Ok(())
}

fn inline_fuzzy_lines(item_count: usize) -> u16 {
    ((item_count.min(10) + 2) as u16).max(3)
}

#[allow(dead_code)]
fn truncate_display(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}

struct InlineRawMode {
    stdout: io::Stdout,
}

impl InlineRawMode {
    fn start() -> Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self {
            stdout: io::stdout(),
        })
    }
}

impl Drop for InlineRawMode {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show);
        let _ = terminal::disable_raw_mode();
    }
}

async fn run_config(paths: &GqyPaths, args: ConfigArgs) -> Result<()> {
    match args.command {
        Some(ConfigCommand::Validate) => {
            AppConfig::load(paths)?;
            println!(
                "{}: {}",
                t("config is valid", "配置有效"),
                paths.config_file.display()
            );
            Ok(())
        }
        Some(ConfigCommand::Paths) => {
            paths.print();
            Ok(())
        }
        Some(ConfigCommand::Set(set)) => {
            let mut config = AppConfig::load(paths)?;
            let mut node = serde_json::to_value(&config).context("serializing config")?;
            let value = parse_config_value(&set.value);
            set_config_path(&mut node, &set.key, value)
                .with_context(|| format!("setting config key: {}", set.key))?;
            config = serde_json::from_value(node).context("deserializing config")?;
            config.save(paths)?;
            println!(
                "{}: {} = {}",
                t("config updated", "已更新配置"),
                set.key,
                set.value
            );
            Ok(())
        }
        Some(ConfigCommand::Get(get)) => {
            let config = AppConfig::load(paths)?;
            let node = serde_json::to_value(&config).context("serializing config")?;
            if let Some(key) = get.key.as_deref() {
                let value = get_config_path(&node, key)?;
                let value = redact_config_value(value);
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                let redacted = redact_config_value(&node);
                println!("{}", serde_json::to_string_pretty(&redacted)?);
            }
            Ok(())
        }
        Some(ConfigCommand::PromptSource) => {
            let config = AppConfig::load(paths)?;
            let persona = config.prompt.active_persona.trim();
            let identity = config.prompt.active_identity.trim();
            let persona_path = (!persona.is_empty()).then(|| config.persona_path(paths, persona));
            let legacy_prompt = config.custom_system_prompt(paths)?;
            let legacy_prompt_path = config.system_prompt_path(paths);
            let base_prompt_source =
                if let Some(path) = persona_path.as_ref().filter(|path| path.exists()) {
                    format!("persona ({})", path.display())
                } else if !legacy_prompt.trim().is_empty() {
                    format!("legacy_custom ({})", legacy_prompt_path.display())
                } else {
                    "built-in".to_string()
                };
            println!("base_prompt_source: {}", base_prompt_source);
            println!(
                "active_persona: {}",
                if persona.is_empty() {
                    "(none)"
                } else {
                    persona
                }
            );
            if let Some(path) = persona_path {
                println!("active_persona_file: {}", path.display());
            }
            println!(
                "active_identity: {}",
                if identity.is_empty() {
                    "(none)"
                } else {
                    identity
                }
            );
            println!("prompts_dir: {}", config.prompts_dir_path(paths).display());
            println!(
                "identities_dir: {}",
                config.identities_dir_path(paths).display()
            );
            let system_prompt = config.system_prompt(paths)?;
            println!(
                "system_prompt_first_line: {}",
                system_prompt.lines().next().unwrap_or("")
            );
            println!("system_prompt_chars: {}", system_prompt.chars().count());
            Ok(())
        }
        None => crate::config_tui::run(paths),
    }
}

/// 值解析：先按 JSON 解析（true/数字/null/数组/对象），失败按字符串。
fn parse_config_value(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

/// 按点号路径写入配置（如 display.language、tools.max_rounds）。
fn set_config_path(node: &mut serde_json::Value, key: &str, value: serde_json::Value) -> Result<()> {
    let segments = key.split('.').collect::<Vec<_>>();
    if segments.is_empty() || segments.iter().any(|s| s.is_empty()) {
        anyhow::bail!("invalid config key: {key}");
    }
    let mut current = node;
    for (index, segment) in segments.iter().enumerate() {
        if index + 1 == segments.len() {
            current[segment] = value;
            return Ok(());
        }
        if !current.get(segment).is_some_and(serde_json::Value::is_object) {
            // 中间键不存在或不是对象：自动创建空对象，支持新增配置段（如 finetune.*）
            current[segment] = serde_json::Value::Object(serde_json::Map::new());
        }
        current = &mut current[segment];
    }
    Ok(())
}

/// 按点号路径读取配置。
fn get_config_path<'a>(node: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value> {
    let mut current = node;
    for segment in key.split('.') {
        if segment.is_empty() {
            anyhow::bail!("invalid config key: {key}");
        }
        current = current
            .get(segment)
            .ok_or_else(|| anyhow::anyhow!("config key not found: {segment}"))?;
    }
    Ok(current)
}

/// 递归脱敏 api_key/token/password 等敏感字段（config get 输出用）。
fn redact_config_value(node: &serde_json::Value) -> serde_json::Value {
    match node {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                let redacted = if regex_matches_secret_key(key)
                    && value.is_string()
                    && !value.as_str().is_some_and(|s| s.starts_with("$env:"))
                {
                    serde_json::Value::String("<redacted>".to_string())
                } else {
                    redact_config_value(value)
                };
                out.insert(key.clone(), redacted);
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_config_value).collect())
        }
        other => other.clone(),
    }
}

fn regex_matches_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ["api_key", "api-key", "token", "password", "secret", "credential"]
        .iter()
        .any(|pattern| lower.contains(pattern))
}


fn run_clipboard_paste(paths: &GqyPaths) -> Result<()> {
    match crate::clipboard::read_clipboard() {
        Ok(crate::clipboard::ClipboardContent::Image(img)) => {
            let path = img.write_temp_file(&paths.cache_dir, 0)?;
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");
            print!("[Image 1: {}]", filename);
            io::stdout().flush()?;
            Ok(())
        }
        Ok(crate::clipboard::ClipboardContent::ImagePath(path)) => {
            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image");
            let dir = paths.cache_dir.join("clipboard_images");
            std::fs::create_dir_all(&dir)?;
            crate::clipboard::cleanup_clipboard_images(&dir);
            let link_path = dir.join(filename);
            let need_create = if link_path.is_symlink() {
                !link_path.exists()
            } else {
                !link_path.exists()
            };
            if need_create {
                if link_path.exists() || link_path.is_symlink() {
                    std::fs::remove_file(&link_path)?;
                }
                std::os::unix::fs::symlink(&path, &link_path)?;
            }
            print!("[Image 1: {}]", filename);
            io::stdout().flush()?;
            Ok(())
        }
        Ok(crate::clipboard::ClipboardContent::TextPath(path)) => {
            print!("{}", path);
            io::stdout().flush()?;
            Ok(())
        }
        Ok(crate::clipboard::ClipboardContent::Text(text)) => {
            if should_summarize_pasted_text(&text) {
                let index = shell_pasted_text_index(&paths.cache_dir, &text)?;
                let placeholder = pasted_text_placeholder(index, pasted_text_line_count(&text));
                print!("{}", placeholder);
            } else {
                print!("{}", text);
            }
            io::stdout().flush()?;
            Ok(())
        }
        _ => {
            std::process::exit(1);
        }
    }
}

fn shell_pasted_text_index(cache_dir: &std::path::Path, text: &str) -> Result<usize> {
    let dir = cache_dir.join("clipboard_texts");
    std::fs::create_dir_all(&dir)?;
    const MAX_INDEX: usize = 100_000;
    for index in 1..=MAX_INDEX {
        let path = dir.join(format!("{index}.txt"));
        if !path.exists() {
            std::fs::write(path, text)?;
            return Ok(index);
        }
    }
    anyhow::bail!("clipboard_texts directory overflow (>{MAX_INDEX} files)")
}

fn shell_message_from_input(use_stdin: bool, message: Vec<String>) -> Result<String> {
    if use_stdin {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        Ok(input)
    } else {
        Ok(join_message(message))
    }
}

fn run_shell_classify(shell_name: &str, message: &str) -> Result<()> {
    if !matches!(shell_name, "fish" | "bash" | "zsh") {
        std::process::exit(2);
    }
    // History expansion（!!、!$、!n 等）透传给 shell 处理
    if shell::is_history_expansion(message) {
        std::process::exit(0);
    }
    // heredoc 和管道链整条透传给 shell
    if shell::is_heredoc_start(message) || shell::is_pipe_chain(message) {
        std::process::exit(0);
    }
    // shell.auto = false：hook 一律放行（系统报错），只用显式 `gqy <问句>` 对话
    if let Ok(paths) = crate::paths::GqyPaths::new() {
        if let Ok(config) = AppConfig::load_or_default(&paths) {
            if !config.shell.auto {
                std::process::exit(0);
            }
        }
    }
    let result = shell::classify_with_confidence(message, shell_name);
    std::process::exit(result.exit_code());
}

async fn run_shell_intercept(paths: &GqyPaths, shell_name: &str, message: String) -> Result<()> {
    if !matches!(shell_name, "fish" | "bash" | "zsh") {
        bail!("{}: {shell_name}", t("unsupported shell", "不支持的 shell"));
    }
    if message.trim().is_empty() {
        bail!(
            "{}",
            t("not a natural language command", "不是自然语言命令")
        );
    }

    let message = expand_shell_pasted_text_placeholders(paths, &message)?;
    let (clean_message, pasted_images) = extract_image_placeholders(&message);

    let result = if pasted_images.is_empty() {
        run_chat_with_options(paths, clean_message, None, false, AgentMode::Normal, false).await
    } else {
        run_chat_with_images(paths, clean_message, pasted_images, false).await
    };
    drain_stdin();
    if let Err(err) = &result {
        println!("\x1b[31m{}: {err}\x1b[0m", t("error", "错误"));
    }
    result
}

fn expand_shell_pasted_text_placeholders(paths: &GqyPaths, message: &str) -> Result<String> {
    let placeholders = find_pasted_text_placeholders(message);
    if placeholders.is_empty() {
        return Ok(message.to_string());
    }

    let chars: Vec<char> = message.chars().collect();
    let mut expanded = String::new();
    let mut last_end = 0;
    let dir = paths.cache_dir.join("clipboard_texts");
    for (start, end, index) in placeholders {
        expanded.extend(&chars[last_end..start]);
        let path = dir.join(format!("{index}.txt"));
        match std::fs::read_to_string(&path) {
            Ok(text) => expanded.push_str(&text),
            Err(_) => expanded.extend(&chars[start..end]),
        }
        last_end = end;
    }
    expanded.extend(&chars[last_end..]);
    Ok(expanded)
}

fn extract_image_placeholders(
    message: &str,
) -> (String, Vec<Option<crate::clipboard::PastedImage>>) {
    let placeholders = find_image_placeholders(message);
    if placeholders.is_empty() {
        return (message.to_string(), Vec::new());
    }

    let cache_images_dir = GqyPaths::new()
        .map(|p| p.cache_dir.join("clipboard_images"))
        .ok();

    let chars: Vec<char> = message.chars().collect();
    let mut clean = String::new();
    let mut images: Vec<Option<crate::clipboard::PastedImage>> = Vec::new();
    let mut last_end = 0;

    for (start, end) in &placeholders {
        clean.extend(&chars[last_end..*start]);
        let segment: String = chars[*start..*end].iter().collect();
        let name_str = segment
            .strip_prefix("[Image ")
            .and_then(|s| s.strip_prefix(|c: char| c.is_ascii_digit()))
            .and_then(|s| s.strip_prefix(':'))
            .and_then(|s| s.strip_suffix(']'))
            .map(|s| s.trim().to_string());

        if let Some(name_str) = name_str {
            if let Some(dir) = &cache_images_dir {
                let candidate = dir.join(&name_str);
                if candidate.exists() {
                    images.push(Some(crate::clipboard::PastedImage::Path(
                        candidate.display().to_string(),
                    )));
                } else {
                    images.push(None);
                }
            } else {
                images.push(None);
            }
        } else {
            images.push(None);
        }
        clean.push_str(&format!("[Image {}]", images.len()));
        last_end = *end;
    }
    clean.extend(&chars[last_end..]);

    (clean, images)
}

async fn run_chat_with_images(
    paths: &GqyPaths,
    message: String,
    pasted_images: Vec<Option<crate::clipboard::PastedImage>>,
    dry_run: bool,
) -> Result<()> {
    AppConfig::init_files(paths)?;
    let config = AppConfig::load_or_default(paths)?;
    let state = StateStore::new(paths)?;
    state.init_files()?;
    let client = LlmClient::from_config(&config, paths)?;
    let registry = build_tool_registry(
        &config,
        paths,
        AgentMode::Normal,
        crate::question_tui::available(false),
    )?;
    let reasoning_mode = render::ReasoningDisplayMode::from_config(&config.display.reasoning);
    let tool_call_mode = render::ToolCallDisplayMode::from_config(&config.display.tool_calls);
    let readable_tool_names = config.display.readable_tool_names;
    let command_output_lines = config.display.command_output_lines;
    let show_token_usage = config.display.show_token_usage;
    let show_mixed_model_endpoint = show_mixed_model_endpoint(&config, false);
    let display_config = config.clone();
    let mut agent = Agent::new_with_dry_run(
        config,
        paths,
        state.clone(),
        client,
        registry,
        AgentMode::Normal,
        dry_run,
    )?;
    crate::pi_bridge::ensure_for_agent(&agent, None, None).await?;
    let mut renderer = render::StreamRenderer::new(
        reasoning_mode,
        tool_call_mode,
        false,
        readable_tool_names,
        command_output_lines,
    );
    renderer.start_waiting()?;
    let result = agent
        .chat_stream_with_images(&message, &pasted_images, |event| {
            handle_agent_event(&mut renderer, event)
        })
        .await;
    renderer.finish()?;
    let result = match result {
        Ok(result) => result,
        Err(err) if crate::question::is_question_cancelled(&err) => return Ok(()),
        Err(err) => return Err(err),
    };
    print_mixed_model_endpoint(show_mixed_model_endpoint, &result, None);
    let mut cumulative_tokens = result.usage.as_ref().map(render::usage_total).unwrap_or(0);
    let context_tokens = agent.effective_context_tokens()?;
    print_chat_token_usage(
        &result,
        show_token_usage,
        context_tokens,
        result_context_window(&display_config, &result).or(agent.context_window()),
        Some(cumulative_tokens),
    )?;
    let overflow_result = handle_post_turn_overflow(
        &agent,
        &mut renderer,
        context_tokens,
        show_token_usage,
        Some(&mut cumulative_tokens),
    )
    .await?;
    let updated_context_tokens = agent.effective_context_tokens()?;
    if overflow_result.is_none() && updated_context_tokens != context_tokens {
        print_chat_token_usage(
            &result,
            show_token_usage,
            updated_context_tokens,
            result_context_window(&display_config, &result).or(agent.context_window()),
            Some(cumulative_tokens),
        )?;
    }
    // 一次性对话：等后台自动备份完成再退出，避免进程先退导致快照丢失
    agent.settle_pending_backup().await;
    Ok(())
}

fn drain_stdin() {
    use std::os::fd::AsRawFd;

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return;
    }
    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return;
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return;
    }

    let mut handle = stdin.lock();
    let mut buffer = [0_u8; 4096];
    loop {
        match handle.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }

    let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
}

const STDIN_MAX_CHARS: usize = 50_000;
const STDIN_TIMEOUT_SECS: u64 = 5;

async fn append_stdin_if_piped(message: String) -> String {
    if io::stdin().is_terminal() {
        return message;
    }
    let read_result = tokio::time::timeout(
        std::time::Duration::from_secs(STDIN_TIMEOUT_SECS),
        tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            let mut stdin = std::io::stdin().lock();
            let mut limited = (&mut stdin).take(STDIN_MAX_CHARS as u64);
            limited.read_to_string(&mut buf).map(|_| buf)
        }),
    )
    .await;

    let stdin_content = match read_result {
        Ok(Ok(Ok(content))) if !content.trim().is_empty() => content.trim().to_string(),
        _ => return message,
    };

    if message.is_empty() {
        stdin_content
    } else {
        format!("{message}\n\n---\n(stdin)\n{stdin_content}")
    }
}

async fn run_chat_with_options(
    paths: &GqyPaths,
    message: String,
    show_reasoning: Option<bool>,
    plain: bool,
    mode: AgentMode,
    dry_run: bool,
) -> Result<()> {
    let message = append_stdin_if_piped(message).await;
    if message.is_empty() {
        return repl::run_repl(paths, mode).await;
    }
    AppConfig::init_files(paths)?;
    let config = AppConfig::load_or_default(paths)?;
    let state = StateStore::new(paths)?;
    state.init_files()?;
    let client = LlmClient::from_config(&config, paths)?;
    let registry =
        build_tool_registry(&config, paths, mode, crate::question_tui::available(plain))?;
    let reasoning_mode = if show_reasoning == Some(false) {
        render::ReasoningDisplayMode::Hidden
    } else {
        render::ReasoningDisplayMode::from_config(&config.display.reasoning)
    };
    let tool_call_mode = if plain {
        render::ToolCallDisplayMode::Hidden
    } else {
        render::ToolCallDisplayMode::from_config(&config.display.tool_calls)
    };
    let readable_tool_names = config.display.readable_tool_names;
    let command_output_lines = config.display.command_output_lines;
    let show_token_usage = config.display.show_token_usage && !plain;
    let show_mixed_model_endpoint = show_mixed_model_endpoint(&config, false);
    let display_config = config.clone();
    let mut agent = Agent::new_with_dry_run(config, paths, state.clone(), client, registry, mode, dry_run)?;
    crate::pi_bridge::ensure_for_agent(&agent, None, None).await?;
    let mut renderer = render::StreamRenderer::new(
        reasoning_mode,
        tool_call_mode,
        plain,
        readable_tool_names,
        command_output_lines,
    );
    renderer.start_waiting()?;
    let result = agent
        .chat_stream(&message, |event| handle_agent_event(&mut renderer, event))
        .await;
    renderer.finish()?;
    let result = match result {
        Ok(result) => result,
        Err(err) if crate::question::is_question_cancelled(&err) => return Ok(()),
        Err(err) => return Err(err),
    };
    print_mixed_model_endpoint(show_mixed_model_endpoint, &result, None);
    let mut cumulative_tokens = result.usage.as_ref().map(render::usage_total).unwrap_or(0);
    let context_tokens = agent.effective_context_tokens()?;
    print_chat_token_usage(
        &result,
        show_token_usage,
        context_tokens,
        result_context_window(&display_config, &result).or(agent.context_window()),
        Some(cumulative_tokens),
    )?;
    let overflow_result = handle_post_turn_overflow(
        &agent,
        &mut renderer,
        context_tokens,
        show_token_usage,
        Some(&mut cumulative_tokens),
    )
    .await?;
    let updated_context_tokens = agent.effective_context_tokens()?;
    if overflow_result.is_none() && updated_context_tokens != context_tokens {
        print_chat_token_usage(
            &result,
            show_token_usage,
            updated_context_tokens,
            result_context_window(&display_config, &result).or(agent.context_window()),
            Some(cumulative_tokens),
        )?;
    }
    // 一次性对话：等后台自动备份完成再退出，避免进程先退导致快照丢失
    agent.settle_pending_backup().await;
    Ok(())
}

fn print_chat_token_usage(
    result: &crate::llm::ChatResult,
    enabled: bool,
    session_token_total: u64,
    context_window: Option<usize>,
    cumulative_tokens: Option<u64>,
) -> Result<()> {
    if enabled {
        if let Some(usage) = &result.usage {
            let turn_tokens = render::usage_total(usage);
            render::print_token_usage(
                turn_tokens,
                session_token_total,
                context_window,
                cumulative_tokens,
                result.usage_estimated,
            )?;
        }
    }
    Ok(())
}

fn result_context_window(config: &AppConfig, result: &crate::llm::ChatResult) -> Option<usize> {
    if config.active_provider_model_choices().len() > 1 {
        return None;
    }
    let provider = result.provider_id.as_deref()?;
    let model = result.model.as_deref()?;
    config
        .context_window_for_provider_model(provider, model)
        .ok()
        .flatten()
}

async fn handle_post_turn_overflow(
    agent: &Agent,
    renderer: &mut render::StreamRenderer,
    context_tokens: u64,
    show_token_usage: bool,
    cumulative_tokens: Option<&mut u64>,
) -> Result<Option<crate::llm::ChatResult>> {
    let compact_result = agent
        .handle_overflow_after_turn(context_tokens, |event| handle_agent_event(renderer, event))
        .await?;
    renderer.finish()?;
    if let Some(compact_result) = compact_result {
        let mut cumulative_display = None;
        if let Some(total) = cumulative_tokens {
            if let Some(usage) = compact_result.usage.as_ref() {
                *total = total.saturating_add(render::usage_total(usage));
                cumulative_display = Some(*total);
            }
        }
        print_chat_token_usage(
            &compact_result,
            show_token_usage,
            agent.effective_context_tokens()?,
            agent.context_window(),
            cumulative_display,
        )?;
        return Ok(Some(compact_result));
    }
    Ok(None)
}

async fn handle_live_post_turn_overflow(
    live: &mut LiveReplTail,
    agent: &Agent,
    renderer: &mut render::StreamRenderer,
    context_tokens: u64,
    show_token_usage: bool,
    cumulative_tokens: Option<&mut u64>,
) -> Result<Option<crate::llm::ChatResult>> {
    let compact_result = agent
        .handle_overflow_after_turn(context_tokens, |event| {
            handle_live_agent_event(live, renderer, event)
        })
        .await?;
    renderer.finish()?;
    live.apply_renderer_frame(renderer)?;
    if let Some(compact_result) = compact_result {
        let mut cumulative_display = None;
        if let Some(total) = cumulative_tokens {
            if let Some(usage) = compact_result.usage.as_ref() {
                *total = total.saturating_add(render::usage_total(usage));
                cumulative_display = Some(*total);
            }
        }
        if show_token_usage {
            if let Some(usage) = compact_result.usage.as_ref() {
                let frame = render::token_usage_output(
                    render::usage_total(usage),
                    agent.effective_context_tokens()?,
                    agent.context_window(),
                    cumulative_display,
                    compact_result.usage_estimated,
                );
                live.apply_output_frame(frame.strip_suffix('\n').unwrap_or(&frame).as_bytes())?;
            }
        }
        return Ok(Some(compact_result));
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VariantOutcome {
    Updated,
    Cancelled,
    Rejected(String),
}

fn run_variant(paths: &GqyPaths, args: VariantArgs) -> Result<()> {
    let selected = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if selected.is_none() && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
        bail!(
            "{}",
            t(
                "interactive variant selection requires a terminal; use `gqy variant <name>`",
                "交互 variant 选择需要终端；请使用 `gqy variant <名称>`",
            )
        );
    }
    if !crate::models_cache::is_loaded() {
        crate::models_cache::refresh_blocking(paths).map_err(|error| {
            anyhow::anyhow!(
                "{}: {error:#}",
                t("failed to load model metadata", "无法加载模型元数据")
            )
        })?;
    }

    let config = AppConfig::load_or_default(paths)?;
    let mut client = LlmClient::from_config(&config, paths)?;
    match execute_variant(paths, &mut client, selected, "gqy variant")? {
        VariantOutcome::Updated => print_variant_updated(),
        VariantOutcome::Cancelled => {}
        VariantOutcome::Rejected(message) => bail!("{message}"),
    }
    Ok(())
}

fn execute_variant(
    paths: &GqyPaths,
    client: &mut LlmClient,
    selected: Option<&str>,
    selector_command: &str,
) -> Result<VariantOutcome> {
    if let Some(selected) = selected {
        if client.thinking_variant_options().len() != 1 {
            let message = if is_zh() {
                format!("当前激活了多个模型；请使用 {selector_command} 在 TUI 中分别设置")
            } else {
                format!(
                    "multiple models are active; use {selector_command} and configure them in the TUI"
                )
            };
            return Ok(VariantOutcome::Rejected(message));
        }
        let available = client.available_thinking_variants();
        let variant = match resolve_variant_name(selected, &available) {
            Ok(variant) => variant,
            Err(message) => return Ok(VariantOutcome::Rejected(message)),
        };
        client.set_thinking_variant(variant)?;
    } else {
        let options = client.thinking_variant_options();
        let Some(selections) = repl::inline_variant_select(&options)? else {
            return Ok(VariantOutcome::Cancelled);
        };
        client.set_thinking_variants(&selections)?;
    }

    client.save_thinking_variants(paths)?;
    Ok(VariantOutcome::Updated)
}

fn resolve_variant_name(
    selected: &str,
    available: &[String],
) -> std::result::Result<Option<String>, String> {
    let explicit_variant = selected.strip_prefix("variant:");
    if explicit_variant.is_none() && selected.eq_ignore_ascii_case("default") {
        return Ok(None);
    }
    let selected = explicit_variant.unwrap_or(selected);
    available
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(selected))
        .cloned()
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{}: {selected}",
                t("unknown thinking variant", "未知思考档位")
            )
        })
}

fn print_variant_updated() {
    println!("{}\n", t("thinking variants updated", "已更新思考档位"));
}


/// 记忆定期归档：把超过保留期的可见轮次归档到 evicted_context.db
/// （不占对话上下文，随时可用 `gqy history --search` 或 recall 检索）。
/// 返回归档的轮次数。

/// 查看活动日志（`gqy activity`）：GQY 干了什么的流水账。
/// 默认不进对话上下文（零 token），需要时查询。


/// 表情格式分布统计（jpg/gif/png/webp…）。


pub fn join_message(parts: Vec<String>) -> String {
    parts.join(" ").trim().to_string()
}

pub(crate) fn build_tool_registry(
    config: &AppConfig,
    paths: &GqyPaths,
    mode: AgentMode,
    interactive_questions: bool,
) -> Result<tools::ToolRegistry> {
    let mut registry = if config.tools.enabled {
        match mode {
            AgentMode::Normal => tools::builtin_registry(config, paths),
            AgentMode::Plan => tools::readonly_registry(config, paths),
            AgentMode::Chat => tools::chat_registry(config, paths),
        }
    } else {
        tools::ToolRegistry::new()
    };
    if config.tools.enabled && config.skills.enabled && mode != AgentMode::Chat {
        tools::register_skills(&mut registry, config, paths)?;
    }
    // 供应商管理：自然语言热切换（对话里给 URL+Key 即可添加/切换；仅正经模式，闲聊保持轻量）
    if config.tools.enabled && mode != AgentMode::Chat {
        tools::providers::register(&mut registry, paths.clone());
    }
    if config.tools.enabled && interactive_questions {
        tools::register_ask_question(&mut registry);
    }
    // 注册用户/系统脚本工具（导入的工具包即时可用；对话循环内也会每轮重扫）
    if config.tools.enabled && mode == AgentMode::Normal {
        tools::rescan_scripts(&mut registry, paths);
    }
    tools::register_script_display_names(&registry);
    Ok(registry)
}

fn handle_agent_event(renderer: &mut render::StreamRenderer, event: AgentEvent) -> Result<()> {
    match event {
        AgentEvent::TurnStarted { .. } => Ok(()),
        AgentEvent::Chunk(chunk) => {
            renderer.write_chunk(chunk)?;
            renderer.tick_spinner()
        }
        AgentEvent::ReasoningStart { received_at } => renderer.start_reasoning_phase(received_at),
        AgentEvent::ReasoningReset { received_at } => renderer.reset_reasoning_phase(received_at),
        AgentEvent::ReasoningPartStart { received_at } => {
            renderer.start_reasoning_part(received_at)
        }
        AgentEvent::ReasoningPartEnd { received_at } => renderer.finish_reasoning_part(received_at),
        AgentEvent::ReasoningTitle(title) => {
            renderer.write_reasoning_title(&title)?;
            renderer.tick_spinner()
        }
        AgentEvent::ToolCall { name, arguments } => {
            renderer.write_tool_call(&name, &arguments)?;
            renderer.tick_spinner()
        }
        AgentEvent::ToolResult { name, ok, output } => {
            renderer.write_tool_result(&name, ok, &output)?;
            renderer.tick_spinner()
        }
        AgentEvent::ToolProgress { name, message } => {
            renderer.write_tool_progress(&name, &message)?;
            renderer.tick_spinner()
        }
        AgentEvent::CommandOutput {
            name,
            stream,
            chunk,
        } => {
            renderer.write_command_output(&name, stream, &chunk)?;
            renderer.tick_spinner()
        }
        AgentEvent::PrepareForExternalOutput { ready } => {
            renderer.prepare_for_external_output()?;
            let _ = ready.send(true);
            Ok(())
        }
        AgentEvent::Image { .. } => Ok(()),
        AgentEvent::AskQuestion { request, responder } => {
            renderer.prepare_for_external_output()?;
            let response = crate::question_tui::ask(&request).unwrap_or_else(|err| {
                crate::question::QuestionResponse::Unavailable(err.to_string())
            });
            if !matches!(&response, crate::question::QuestionResponse::Cancelled) {
                renderer.start_waiting()?;
            }
            let _ = responder.send(response);
            Ok(())
        }
        AgentEvent::QueuedPromptsConsumed { .. } => Ok(()),
        AgentEvent::SpinnerTick => renderer.tick_spinner(),
        AgentEvent::CompactStart => {
            renderer.write_system_message(t("Compacting context...", "正在压缩上下文..."))?;
            renderer.tick_spinner()
        }
        AgentEvent::CompactChunk(chunk) => {
            renderer.write_compact_chunk(&chunk)?;
            renderer.tick_spinner()
        }
        AgentEvent::CompactEnd => {
            renderer.finish_compact()?;
            renderer.tick_spinner()
        }
        AgentEvent::PopStart => renderer.tick_spinner(),
        AgentEvent::PopEnd => renderer.tick_spinner(),
    }
}

pub use commands::archive::maybe_auto_archive;
