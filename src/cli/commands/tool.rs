use anyhow::Result;
use crate::cli::ToolArgs;
use crate::agent::AgentMode;
use crate::config::AppConfig;
use crate::paths::GqyPaths;

pub async fn run_tool(paths: &GqyPaths, _mode: AgentMode, args: ToolArgs) -> Result<()> {
    let config = AppConfig::load_or_default(paths)?;
    let registry = crate::tools::builtin_registry(&config, paths);
    let output = registry
        .call(&args.name, args.arguments.as_deref().unwrap_or("{}"))
        .await?;
    println!("{output}");
    Ok(())
}
