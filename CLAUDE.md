# AGENTS.md

## Project

GQY (顾清影) — Rust CLI AI assistant with REPL, web UI, macOS menu bar app, and chat bridges (QQ/NapCat, Telegram). Single crate, no workspace.

## Build & Test

```bash
cargo check              # compile check (lib + bin, no tests)
cargo test               # inline unit tests
cargo clippy -- -W warnings   # lint (report-only, not blocking)
cargo build              # debug build
cargo build --release    # release build
```

CI pins Rust **1.97.1** (see `.github/workflows/ci.yml`).

Linux CI requires system deps: `libasound2-dev pkg-config ripgrep` (audio via rodio/cpal, rg for glob/grep tool tests).

## Build Script (build.rs)

- Obfuscates `src/prompts/gqy.md` with XOR mask → compiled into binary as `OBFUSCATED_DEFAULT_SYSTEM_PROMPT`
- Compiles `assets/o200k_base.tiktoken` → binary vocab at `$OUT_DIR/o200k_base.bin`
- Editing either file triggers rebuild automatically (`cargo:rerun-if-changed`)

## Architecture

| Path | Role |
|------|------|
| `src/main.rs` | Entrypoint → `paths::GqyPaths::new()` → `cli::run()` |
| `src/cli.rs` | All CLI subcommands, REPL loop (~9.5k lines) |
| `src/agent/` | Agent loop: conversation, compaction, overflow handling |
| `src/llm/` | LLM clients: `openai_compatible.rs` (OpenAI API), `pi_rpc.rs` (Pi RPC) |
| `src/tools/` | ~40 built-in tools registered via `ToolRegistry` (file, web, vision, memes, knowledge, memory, etc.) |
| `src/state/` | SQLite conversation DB (`conversation_db.rs`), usage tracking |
| `src/memory/` | Persistent memory (separate from conversation state) |
| `src/config.rs` | `AppConfig` — JSONC config, provider/model/plugins/mcp/shell/display |
| `src/render/` | Terminal rendering with crossterm (markdown, streaming, spinners) |
| `src/web.rs` | Axum web server with SSE, serves embedded HTML/CSS/JS from `web/` |
| `src/bridges/` | Chat bridges: `napcat.rs` (QQ), `tg.rs` (Telegram) |
| `src/i18n.rs` | Bilingual zh/en, auto-detected from locale, overridable via `GQY_LANG` |
| `src/prompts/` | System prompts as markdown files (gqy.md, plan.md, chat.md, etc.) |
| `src/scripts/` | Bundled script tools (battery-care, vision-tool.swift, etc.) |
| `src/memes/` | Bundled meme images and data |

## Key Conventions

- **Language**: All UI strings use `i18n::text("en", "中")` — always provide both English and Chinese
- **Tool registration**: Each tool module has a `register()` function that takes `&mut ToolRegistry`. Tools are conditionally enabled based on `config.plugins.*`
- **Three registry tiers**: `builtin_registry` (full), `readonly_registry` (read-only tools), `chat_registry` (chat/roleplay mode, minimal tools)
- **Config**: JSONC format at `~/.config/gqy/config.jsonc` (XDG), parsed with `json_comments` crate for comments
- **Paths**: `GqyPaths` centralizes all filesystem paths. Override with `GQY_HOME` env var
- **Share dir**: Bundled read-only resources (scripts, memes, kb) resolve from brew prefix, app bundle, or repo root depending on entry point

## Distribution

- **Homebrew**: `Formula/gqy.rb` — `brew install gqy` (installs binary + share resources)
- **One-liner**: `curl -fsSL ...install.sh | bash` (downloads prebuilt binary)
- **Web UI**: `gqy web` starts Axum server on port 4096, serves embedded frontend from `web/`

## Gotchas

- `src/cli.rs` is ~9.5k lines — the REPL, all subcommands, and input handling live here
- The system prompt is XOR-obfuscated at build time — editing `src/prompts/gqy.md` is the intended way to change it
- Tests use `tempfile::tempdir()` for isolated config/state — no shared test fixtures
- `GQY_HOME` env var overrides all XDG paths (useful for isolated testing or portable installs)
- `cargo check` in CI does NOT use `--all-targets` to avoid test rot in inline `#[cfg(test)]` blocks
- Web frontend files (`web/*.html/css/js`) are compiled into the binary via `include_str!`/`include_bytes!`
- `pics/GQY-avatar.png` and `pics/GQY-icon.png` are embedded via `include_bytes!` in `repl_avatar.rs` and `web.rs` — don't delete them
