//! `manage_providers` 工具：让顾清影在对话里直接管理供应商——
//! 给了 URL + API Key，它能自动发现可用模型、写入配置并热切换激活。
//! 配置落盘后 GQY 的 config watcher 自动 reload，WebUI 无需重启即刷新。
use super::{ToolRegistry, ToolSpec};
use crate::paths::GqyPaths;
use crate::provider;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub fn register(registry: &mut ToolRegistry, paths: GqyPaths) {
    registry.register(ToolSpec::new(
        "manage_providers",
        "管理 AI 供应商（模型服务商）：添加/列出/切换/移除。给 base_url（OpenAI 兼容端点）和 API key，可自动发现可用模型并热切换激活；切换后运行中的 WebUI 会自动刷新。",
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "switch", "remove"],
                    "description": "操作：add=新增/更新供应商（可自动发现模型）；list=列出全部；switch=热切换激活；remove=移除。"
                },
                "base_url": { "type": "string", "description": "add 时必填：OpenAI 兼容端点，如 https://api.deepseek.com/v1 或 http://127.0.0.1:11434/v1" },
                "api_key": { "type": "string", "description": "add 时必填：API Key（本地服务可填占位符）" },
                "provider_id": { "type": "string", "description": "供应商 id（小写字母/数字/连字符）；add 时不填则从 base_url 推断；switch/remove 必填" },
                "display_name": { "type": "string", "description": "显示名，默认取 provider_id" },
                "model": { "type": "string", "description": "switch 时指定要激活的模型；add 时不填则自动发现模型并选第一个" }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
        move |args| {
            let paths = paths.clone();
            async move { run(args, paths).await }
        },
    ));
}

async fn run(args: Value, paths: GqyPaths) -> Result<String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    match action.as_str() {
        "list" => provider::list_providers(&paths),
        "switch" => {
            let id = required_str(&args, "provider_id")?;
            let model = args.get("model").and_then(Value::as_str).map(String::from);
            provider::switch_provider(&paths, &id, model)
        }
        "remove" => {
            let id = required_str(&args, "provider_id")?;
            provider::remove_provider(&paths, &id)
        }
        "add" => {
            let base_url = required_str(&args, "base_url")?;
            let api_key = args
                .get("api_key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let id = args
                .get("provider_id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| infer_id(&base_url));
            let display_name = args
                .get("display_name")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| id.clone());
            // 自动发现模型（本地/在线 OpenAI 兼容端点都支持 /models）
            let discovered = match provider::discover_models(&base_url, api_key).await {
                Ok(models) => models,
                Err(err) => {
                    return Ok(json!({
                        "ok": false,
                        "error": format!("模型发现失败：{err}（可手动指定 model 参数重试）"),
                    })
                    .to_string());
                }
            };
            let model = args
                .get("model")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| discovered.first().cloned().unwrap_or_default());
            provider::add_provider(
                &paths,
                &id,
                &display_name,
                &base_url,
                api_key,
                discovered.clone(),
                Some(model),
            )
        }
        _ => bail!("action 必须是 add/list/switch/remove 之一"),
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .with_context(|| format!("参数 {key} 必填"))
}

fn infer_id(base_url: &str) -> String {
    let cleaned: String = base_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .split('.')
        .next()
        .unwrap_or("custom")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "custom".to_string()
    } else {
        cleaned
    }
}

use anyhow::Context;
