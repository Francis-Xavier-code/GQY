mod activity;
mod agents;
mod agent;
mod alarm;
mod backup;
mod balance;
mod bridges;
mod cli;
mod clipboard;
mod config;
mod config_tui;
mod default_models;
mod finetune;
mod i18n;
mod learning;
mod llm;
mod menubar;
mod logging;
mod memory;
mod models_cache;
mod paths;
mod pi_bridge;
mod prompts;
mod question;
mod question_tui;
mod render;
mod repl_avatar;
mod shell;
mod speech;
mod state;
mod token_counter;
mod token_estimate;
mod tools;
mod watch;
mod web;

use anyhow::Result;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}: {error:#}", i18n::text("error", "错误"));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let paths = paths::GqyPaths::new()?;
    let language = config::AppConfig::display_language_hint(&paths);
    i18n::init(language.as_deref().unwrap_or("auto"));
    let cli = cli::parse();
    cli::run(cli, paths).await
}
