use anyhow::{bail, Result};
use crate::config::AppConfig;
use crate::paths::GqyPaths;

pub fn run_balance(paths: &GqyPaths) -> Result<()> {
    let config = AppConfig::load(paths)?;
    match crate::balance::fetch_balance(&config, paths)? {
        Some(infos) => {
            println!("{}", crate::balance::format_balances(&infos));
            Ok(())
        }
        None => {
            let provider = config
                .provider(None)
                .map(|p| p.id.clone())
                .unwrap_or_default();
            bail!(
                "当前 provider（{provider}）没有公开的余额查询接口；目前支持 DeepSeek（gqy config set active_provider deepseek）"
            );
        }
    }
}
