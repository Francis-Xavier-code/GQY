# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

GQY (顾清影) — Rust CLI AI assistant with REPL, web UI, macOS menu bar app, and chat bridges (QQ/NapCat, Telegram). Single crate, no workspace. Not a coding agent: oriented toward daily chat, system troubleshooting, and desktop companion use.

## Build & Test

```bash
cargo check                        # compile check (lib + bin, no tests)
cargo test                         # all inline unit tests
cargo test <name>                  # run tests matching <name>
cargo test -- --test-threads=1     # serialize if tests contend on env/fs
cargo clippy -- -W warnings        # lint (report-only in CI, not blocking)
cargo build                        # debug
cargo build --release --locked     # release (what install/CI package)
```

- CI pins Rust **1.97.1** (`.github/workflows/ci.yml`).
- Linux needs: `libasound2-dev pkg-config ripgrep` (rodio/cpal audio; rg for glob/grep tool tests).
- `cargo check` in CI does **not** use `--all-targets` — some inline `#[cfg(test)]` blocks have test rot; keep app compile gate separate from test gate.
- Cross aarch64 Linux: `Cross.toml` + `cross build --release --target aarch64-unknown-linux-gnu` (release workflow).

## Build Script (`build.rs`)

- XOR-obfuscates `src/prompts/gqy.md` → `OBFUSCATED_DEFAULT_SYSTEM_PROMPT` in `$OUT_DIR`.
- Also rebuilds on `plan.md` / `chat.md` changes.
- Compiles `assets/o200k_base.tiktoken` → `$OUT_DIR/o200k_base.bin`.
- Edit the markdown prompts; do not hand-edit the generated obfuscated blob.

## Architecture

```
用户输入 (REPL / WebUI / QQ / TG)
  → agent::run_turn
      → start_turn (SQLite, records mode)
      → chat_messages (history filtered by AgentMode)
      → LlmClient (provider → OpenAI-compatible or pi RPC)
      → stream chunks + tool loop (ToolRegistry)
      → complete_turn (DB + usage + finetune sample + memory)
```

| Path | Role |
|------|------|
| `src/main.rs` | Entrypoint → `GqyPaths::new()` → i18n → `cli::run()` |
| `src/cli/` | CLI surface (was a monolithic `cli.rs`; now split) |
| `src/cli/args.rs` | clap `Cli` / `Command` definitions |
| `src/cli/mod.rs` | Subcommand dispatch + shared helpers |
| `src/cli/repl.rs` | Interactive REPL (largest CLI piece) |
| `src/cli/commands/` | Subcommand implementations (`provider`, `kb`, `backup`, …) |
| `src/agent/` | Turn loop, tool loop (`tool_loop.rs`), compaction, overflow |
| `src/agents.rs` | Named multi-agent swarm (spawn/talk/list; separate from `agent/`) |
| `src/llm/` | `openai_compatible.rs` (streaming SSE), `pi_rpc.rs` |
| `src/tools/` | ~40 tools + `ToolRegistry`; tiers below |
| `src/state/` | SQLite conversations (`conversation_db.rs`), usage |
| `src/memory/` | Persistent memory store (not the same as conversation DB) |
| `src/config.rs` | `AppConfig` JSONC: providers, models, plugins, MCP, shell, display |
| `src/config_tui/` | Config TUI: `widgets` / `plugins` / `personas` / `providers` / `settings` |
| `src/question_tui/` | Ask-question panel: `state` / `session` / `render` / `helpers` |
| `src/provider.rs` | Provider CRUD / model discovery / active switch |
| `src/render/` | Terminal markdown, streaming, spinners (crossterm) |
| `src/web.rs` | Axum + SSE; embeds `web/*` via `include_str!` / `include_bytes!` |
| `src/bridges/` | `napcat.rs` (QQ), `tg.rs` (Telegram) |
| `src/paths.rs` | All filesystem paths; `GQY_HOME` / `GQY_SHARE_DIR` |
| `src/i18n.rs` | Bilingual UI; `GQY_LANG` override |
| `src/prompts/` | System prompts (gqy/plan/chat/compact/subagent-*) |
| `src/scripts/`, `src/memes/`, `kb/` | Bundled share resources |

### Agent modes

`AgentMode::{Normal, Plan, Chat}` — history is mode-isolated in SQLite (`mode` column). Chat keeps a short window (~12 turns). Tool sets differ by mode (see registry tiers).

### Tool registry tiers (`src/tools/mod.rs`)

- `builtin_registry` — full toolset (Normal)
- `readonly_registry` — no write/patch tools (Plan-oriented)
- `chat_registry` — minimal (web/vision/memes/readonly memory); empty if `prompt.chat_pure_text`

Each tool module exposes `register(&mut ToolRegistry, …)`. Plugins gate via `config.plugins.*`. Hybrid/lazy tool loading adds `load_tools`.

### Channels & hot reload

- Process channel: terminal / webui / qq / tg; isolate with `GQY_CHANNEL`.
- Config mtime watcher reloads agent + LLM client and emits `config.reloaded` over SSE.

### Paths & data layout

- Config: `~/.config/gqy/config.jsonc` (JSONC via `json_comments`).
- Default data root (macOS): `~/Library/Application Support/gqy` via XDG helpers.
- `GQY_HOME` → fully isolated layout (tests, portable installs, menu bar app).
- Share dir (scripts/memes/kb): brew prefix, app bundle `Contents/Resources/share/gqy`, or repo root for source builds. Override with `GQY_SHARE_DIR`.
- Write guard: `src/tools/path_guard.rs` blocks writes into project source unless `GQY_ALLOW_PROJECT_WRITES=1`. Workspace default `~/gqy-workspace` (`GQY_WORKSPACE`).

## Conventions

- **UI strings**: always `i18n::text("english", "中文")` (or `t(...)`).
- **Tests**: inline `#[cfg(test)] mod tests` next to code; use `tempfile::tempdir()` + `GQY_HOME` — no shared fixtures.
- **Embedded assets**: `web/*`, `pics/GQY-avatar.png`, `pics/GQY-icon.png` are compiled in — do not delete.
- **System prompt**: change `src/prompts/gqy.md` only (build-time obfuscation).
- **PR scope**: one feature per PR; do not change existing feature semantics without intent (see `docs/01-指南/自主行为规范.md`).

## Distribution

- Homebrew: `Formula/gqy.rb` / tap `GQYTeam/GQY`
- One-liner: `install.sh`
- Release: tag `v*` → `.github/workflows/release.yml` (darwin aarch64/x86_64, linux x86_64 + cross aarch64)
- Web UI: `gqy web` → default `127.0.0.1:4096`

## Gotchas

- `src/cli/repl.rs` and `src/agent/mod.rs` / `src/llm/openai_compatible.rs` are very large — prefer targeted reads over whole-file loads.
- `agent/` (turn loop) ≠ `agents.rs` (multi-agent swarm tools).
- Image display in terminal depends on optional `chafa` at runtime.
- Forked from Miyu (MIT); project is GPL-3.0 for new/changed code.
