//! 供应商管理：自动发现模型 + 热增删改切换（CLI 与 agent 工具共用）。
//!
//! 核心机制：直接读写磁盘 config（`config.jsonc`），GQY 的 config watcher
//! 检测到文件变化后自动 reload 并发布 `config.reloaded`——运行中的 WebUI
//! 无需重启即可看到新供应商/模型，agent 的 LLM client 也随之热切换。
use crate::config::{ActiveProviderModelConfig, AppConfig, ProviderConfig};
use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// 用 api_key 请求 OpenAI 兼容 `/models` 端点，返回可用模型 id 列表。
/// 兼容 `{base}/v1/models` 与 `{base}/models` 两种路径。
pub async fn discover_models(base_url: &str, api_key: &str) -> Result<Vec<String>> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        bail!("base_url is required");
    }
    let url = if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .context("failed to build http client")?;
    let mut req = client.get(&url);
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("failed to reach {url}"))?;
    if !resp.status().is_success() {
        bail!("models endpoint {url} returned HTTP {}", resp.status());
    }
    let data: Value = resp.json().await?;
    let mut models: Vec<String> = data
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    models.sort();
    if models.is_empty() {
        bail!("no models returned from {url}（可能是非 OpenAI 兼容端点）");
    }
    Ok(models)
}

/// 添加或更新一个 OpenAI 兼容供应商，并激活默认模型。
/// `models` 为空时不会自动发现（调用方可先用 discover_models 拉取）。
/// 返回给人/模型看的摘要文本。
pub fn add_provider(
    paths: &GqyPaths,
    id: &str,
    display_name: &str,
    base_url: &str,
    api_key: &str,
    models: Vec<String>,
    default_model: Option<String>,
) -> Result<String> {
    let id = sanitize_id(id)?;
    let mut config = AppConfig::load(paths)?;
    let default_model = default_model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| models.first().cloned());
    let existing = config
        .providers
        .iter_mut()
        .find(|p| p.id == id);
    if let Some(provider) = existing {
        if !base_url.trim().is_empty() {
            provider.base_url = base_url.trim().to_string();
        }
        if !display_name.trim().is_empty() {
            provider.display_name = display_name.trim().to_string();
        }
        provider.api_key = Some(api_key.trim().to_string());
        if !models.is_empty() {
            provider.models = models.clone();
        }
        if let Some(model) = &default_model {
            provider.default_model = model.clone();
            if !provider.models.contains(model) {
                provider.models.push(model.clone());
            }
        }
    } else {
        let provider = ProviderConfig {
            id: id.clone(),
            display_name: if display_name.trim().is_empty() {
                id.clone()
            } else {
                display_name.trim().to_string()
            },
            base_url: base_url.trim().to_string(),
            protocol: "auto".to_string(),
            api_key: Some(api_key.trim().to_string()),
            models: models.clone(),
            model_context_window: std::collections::HashMap::new(),
            model_modalities: std::collections::HashMap::new(),
            default_model: default_model.clone().unwrap_or_default(),
            timeout_seconds: crate::config::default_timeout(),
            temperature: crate::config::default_temperature(),
            anthropic_max_tokens: crate::config::default_anthropic_max_tokens(),
            extra_body: None,
            prompt_caching: None,
        };
        config.providers.push(provider);
    }
    if let Some(model) = &default_model {
        config.active_provider = id.clone();
        config.active_provider_models = Some(vec![ActiveProviderModelConfig {
            provider_id: id.clone(),
            model: model.clone(),
        }]);
    }
    config.save(paths)?;
    Ok(json!({
        "ok": true,
        "provider_id": id,
        "base_url": base_url.trim(),
        "models": models,
        "default_model": default_model,
        "active": default_model.is_some(),
        "hint": "配置已保存；运行中的 gqy web 会自动重载，WebUI 模型下拉即可见。"
    })
    .to_string())
}

/// 热切换激活供应商（可指定模型；不指定用其 default_model 或第一个）。
pub fn switch_provider(paths: &GqyPaths, provider_id: &str, model: Option<String>) -> Result<String> {
    let mut config = AppConfig::load(paths)?;
    let provider = config
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .with_context(|| format!("provider '{provider_id}' not found（可用 gqy provider list 查看）"))?;
    let model = model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            let m = provider.default_model.trim();
            if m.is_empty() {
                None
            } else {
                Some(m.to_string())
            }
        })
        .or_else(|| provider.models.first().cloned())
        .with_context(|| format!("provider '{provider_id}' has no model to activate"))?;
    config.active_provider = provider_id.to_string();
    config.active_provider_models = Some(vec![ActiveProviderModelConfig {
        provider_id: provider_id.to_string(),
        model: model.clone(),
    }]);
    config.save(paths)?;
    Ok(json!({
        "ok": true,
        "active_provider": provider_id,
        "model": model,
        "display_name": provider.display_name,
    })
    .to_string())
}

/// 列出全部供应商（脱敏 key）与当前激活项。
pub fn list_providers(paths: &GqyPaths) -> Result<String> {
    let config = AppConfig::load(paths)?;
    let active = config.active_provider.clone();
    let rows: Vec<Value> = config
        .providers
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "display_name": p.display_name,
                "base_url": p.base_url,
                "has_key": p.api_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false),
                "models": p.models,
                "default_model": p.default_model,
                "active": p.id == active,
            })
        })
        .collect();
    Ok(json!({ "ok": true, "active_provider": active, "providers": rows }).to_string())
}

/// 移除供应商（当前激活时自动切换到第一个剩余供应商）。
pub fn remove_provider(paths: &GqyPaths, provider_id: &str) -> Result<String> {
    let mut config = AppConfig::load(paths)?;
    let before = config.providers.len();
    config.providers.retain(|p| p.id != provider_id);
    if config.providers.len() == before {
        bail!("provider '{provider_id}' not found");
    }
    if config.active_provider == provider_id {
        if let Some(next) = config.providers.first() {
            config.active_provider = next.id.clone();
            config.active_provider_models = next.models.first().map(|m| {
                vec![ActiveProviderModelConfig {
                    provider_id: next.id.clone(),
                    model: m.clone(),
                }]
            });
        } else {
            config.active_provider = String::new();
            config.active_provider_models = None;
        }
    }
    config.save(paths)?;
    Ok(json!({
        "ok": true,
        "removed": provider_id,
        "remaining": config.providers.len(),
        "active_provider": config.active_provider,
    })
    .to_string())
}

/// 供应商 id 清洗：小写字母数字连字符，最长 32。
fn sanitize_id(id: &str) -> Result<String> {
    let cleaned: String = id
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned
        .trim_matches('-')
        .chars()
        .take(32)
        .collect::<String>();
    if cleaned.is_empty() {
        bail!("invalid provider id: '{id}'（用字母/数字，可含连字符）");
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_id_works() {
        assert_eq!(sanitize_id("My Provider!").unwrap(), "my-provider");
        assert_eq!(sanitize_id("deepseek").unwrap(), "deepseek");
        assert!(sanitize_id("###").is_err());
    }
}

/// 供应商模板目录：按区域分类列出所有内置模板。
/// 用于 CLI `gqy provider templates` 和 agent 工具浏览。
pub fn list_templates() -> Vec<ProviderTemplate> {
    vec![
        ProviderTemplate {
            id: "opencode",
            display_name: "opencode Zen",
            base_url: "https://zen.opencode.ai/v1",
            category: "default",
            description: "默认内置供应商，无需 API key",
        },
        ProviderTemplate {
            id: "openai",
            display_name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            category: "overseas",
            description: "GPT-4o / o1 / o3 系列",
        },
        ProviderTemplate {
            id: "anthropic",
            display_name: "Anthropic",
            base_url: "https://api.anthropic.com/v1",
            category: "overseas",
            description: "Claude 系列（原生 Anthropic 协议）",
        },
        ProviderTemplate {
            id: "gemini",
            display_name: "Gemini",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            category: "overseas",
            description: "Google Gemini 系列（OpenAI 兼容端点）",
        },
        ProviderTemplate {
            id: "groq",
            display_name: "Groq",
            base_url: "https://api.groq.com/openai/v1",
            category: "overseas",
            description: "Groq LPU 推理，低延迟",
        },
        ProviderTemplate {
            id: "together",
            display_name: "Together AI",
            base_url: "https://api.together.xyz/v1",
            category: "overseas",
            description: "开源模型云推理",
        },
        ProviderTemplate {
            id: "fireworks",
            display_name: "Fireworks AI",
            base_url: "https://api.fireworks.ai/inference/v1",
            category: "overseas",
            description: "高性能开源模型推理",
        },
        ProviderTemplate {
            id: "cerebras",
            display_name: "Cerebras",
            base_url: "https://api.cerebras.ai/v1",
            category: "overseas",
            description: "Cerebras Wafer-Scale 推理",
        },
        ProviderTemplate {
            id: "sambanova",
            display_name: "SambaNova",
            base_url: "https://api.sambanova.ai/v1",
            category: "overseas",
            description: "SambaNova RDU 推理",
        },
        ProviderTemplate {
            id: "mistral",
            display_name: "Mistral AI",
            base_url: "https://api.mistral.ai/v1",
            category: "overseas",
            description: "Mistral / Codestral 系列",
        },
        ProviderTemplate {
            id: "openrouter",
            display_name: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            category: "overseas",
            description: "聚合路由，数百模型一键切换",
        },
        ProviderTemplate {
            id: "deepseek",
            display_name: "DeepSeek",
            base_url: "https://api.deepseek.com",
            category: "china",
            description: "DeepSeek V3 / R1 系列",
        },
        ProviderTemplate {
            id: "siliconflow",
            display_name: "SiliconFlow",
            base_url: "https://api.siliconflow.cn/v1",
            category: "china",
            description: "硅基流动，开源模型聚合推理",
        },
        ProviderTemplate {
            id: "moonshot",
            display_name: "Moonshot",
            base_url: "https://api.moonshot.cn/v1",
            category: "china",
            description: "月之暗面 Kimi 系列",
        },
        ProviderTemplate {
            id: "zhipu",
            display_name: "Zhipu",
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            category: "china",
            description: "智谱 GLM / ChatGLM 系列",
        },
        ProviderTemplate {
            id: "volcengine",
            display_name: "Volcengine",
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            category: "china",
            description: "火山引擎豆包大模型",
        },
        ProviderTemplate {
            id: "bailian",
            display_name: "Bailian",
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            category: "china",
            description: "阿里云百炼 / 通义千问",
        },
        ProviderTemplate {
            id: "xiaomi",
            display_name: "Xiaomi",
            base_url: "https://token-plan-sgp.xiaomimimo.com/v1",
            category: "china",
            description: "小米 MiMo 系列",
        },
        ProviderTemplate {
            id: "minimax",
            display_name: "Minimax",
            base_url: "https://api.minimaxi.com/v1",
            category: "china",
            description: "MiniMax 系列",
        },
        ProviderTemplate {
            id: "ollama",
            display_name: "Ollama",
            base_url: "http://localhost:11434/v1",
            category: "local",
            description: "本地 Ollama 服务",
        },
        ProviderTemplate {
            id: "lmstudio",
            display_name: "LMStudio",
            base_url: "http://localhost:1234/v1",
            category: "local",
            description: "本地 LM Studio 服务",
        },
    ]
}

#[derive(Debug, Clone)]
pub struct ProviderTemplate {
    pub id: &'static str,
    pub display_name: &'static str,
    pub base_url: &'static str,
    pub category: &'static str,
    pub description: &'static str,
}

/// 按分类列出模板：不传 category 列全部；传 "china" / "overseas" / "local" / "default" 筛选。
pub fn templates_by_category(category: Option<&str>) -> Vec<ProviderTemplate> {
    list_templates()
        .into_iter()
        .filter(|t| category.map_or(true, |c| t.category == c))
        .collect()
}
