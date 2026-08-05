use anyhow::{bail, Result};
use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::memory::MemoryStore;
use crate::paths::GqyPaths;
use crate::state::StateStore;

pub fn run_reset(paths: &GqyPaths, scope: Option<&str>) -> Result<()> {
    let all = match scope {
        None => false,
        Some("all") => true,
        Some(scope) => bail!("{}: {scope}", t("unknown reset scope", "未知 reset 范围")),
    };
    let config = AppConfig::load_or_default(paths)?;
    StateStore::new(paths)?.reset_conversation()?;
    let memory = MemoryStore::new(&config, paths);
    if all {
        memory.reset_all(false)?;
    } else {
        memory.clear_evicted_context()?;
        memory.clear_pending_events()?;
    }
    let message = if all {
        t(
            "cleared current conversation history and all memory",
            "已清空当前会话历史与全部记忆",
        )
    } else {
        t("cleared current conversation history", "已清空当前会话历史")
    };
    println!("\x1b[2m{message}\x1b[0m\n");
    Ok(())
}
