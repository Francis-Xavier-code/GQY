use anyhow::Result;
use crate::cli::{ProviderAction, ProviderArgs};
use crate::i18n::text as t;
use crate::paths::GqyPaths;

pub async fn run_provider(paths: &GqyPaths, args: ProviderArgs) -> Result<()> {
    match args.action {
        ProviderAction::List => {
            let summary = crate::provider::list_providers(paths)?;
            let data: serde_json::Value = serde_json::from_str(&summary)?;
            println!("当前激活: {}\n", data["active_provider"]);
            for p in data["providers"].as_array().unwrap_or(&vec![]).iter() {
                let marker = if p["active"] == true { "*" } else { " " };
                println!(
                    "{marker} {} / {}  {}  key={}  models={}  default={}",
                    p["id"],
                    p["display_name"],
                    p["base_url"],
                    if p["has_key"] == true { "✓" } else { "无" },
                    p["models"].as_array().map(|m| m.len()).unwrap_or(0),
                    p["default_model"],
                );
            }
            Ok(())
        }
        ProviderAction::Add {
            id,
            name,
            base_url,
            api_key,
            model,
        } => {
            let models = crate::provider::discover_models(&base_url, &api_key).await?;
            let id = id.unwrap_or_else(|| infer_provider_id(&base_url));
            let summary = crate::provider::add_provider(
                paths,
                &id,
                name.as_deref().unwrap_or(&id),
                &base_url,
                &api_key,
                models.clone(),
                Some(model.unwrap_or_else(|| {
                    models.first().cloned().unwrap_or_default()
                })),
            )?;
            println!("{}", pretty_json(&summary));
            Ok(())
        }
        ProviderAction::Switch {
            provider_id,
            model,
        } => {
            let summary = crate::provider::switch_provider(paths, &provider_id, model)?;
            println!("{}", pretty_json(&summary));
            Ok(())
        }
        ProviderAction::Remove { provider_id } => {
            let summary = crate::provider::remove_provider(paths, &provider_id)?;
            println!("{}", pretty_json(&summary));
            Ok(())
        }
        ProviderAction::Templates { category } => {
            let templates = crate::provider::templates_by_category(category.as_deref());
            if templates.is_empty() {
                println!("{}", t("No templates found.", "没有找到模板。"));
            } else {
                let cat_label = category.as_deref().unwrap_or("all");
                println!(
                    "{} ({}) — {}",
                    t("Provider templates", "供应商模板"),
                    cat_label,
                    templates.len()
                );
                println!();
                for tmpl in &templates {
                    let tag = match tmpl.category {
                        "china" => "[CN]",
                        "overseas" => "[Intl]",
                        "local" => "[Local]",
                        _ => "[Default]",
                    };
                    println!(
                        "  {:<16} {} {:<30} {}",
                        tmpl.id, tag, tmpl.base_url, tmpl.description
                    );
                }
                println!(
                    "\n{}",
                    t(
                        "Use `gqy provider add <id>` to configure a provider.",
                        "使用 `gqy provider add <id>` 来配置供应商。"
                    )
                );
            }
            Ok(())
        }
    }
}

fn infer_provider_id(base_url: &str) -> String {
    base_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .split('.')
        .next()
        .unwrap_or("custom")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
}

fn pretty_json(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_string())
}
