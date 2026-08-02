//! 模型服务余额查询（目前支持 DeepSeek 官方余额接口）。
//!
//! DeepSeek 提供公开的余额查询 API（`GET /user/balance`，Bearer 鉴权），
//! 无需额外付费即可显示账户余额。其他 provider 无公开余额接口时返回 None。

use crate::config::AppConfig;
use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct BalanceInfo {
    pub currency: String,
    pub total_balance: String,
    pub granted_balance: String,
    pub topped_up_balance: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BalanceResponse {
    is_available: bool,
    balance_infos: Vec<BalanceInfo>,
}

/// 查询当前激活 provider 的余额；不支持时返回 Ok(None)。
/// API key 支持 `$env:NAME` 引用（与模型请求一致的解析方式）。
pub fn fetch_balance(config: &AppConfig, paths: &crate::paths::GqyPaths) -> Result<Option<Vec<BalanceInfo>>> {
    let provider = config.provider(None)?;
    let Some(api_key) = provider
        .resolved_api_keys(paths)
        .ok()
        .and_then(|keys| keys.into_iter().next().map(|key| key.value))
        .filter(|key| !key.is_empty())
    else {
        return Ok(None);
    };
    let base = provider.base_url.trim_end_matches('/');
    if !base.to_ascii_lowercase().contains("deepseek") {
        return Ok(None);
    }
    // deepseek 余额接口在域名根：/user/balance（去掉可能的 /v1 后缀）
    let root = base
        .strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/');
    let url = format!("{root}/user/balance");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building balance client")?;
    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .send()
        .with_context(|| format!("requesting balance from {url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!("余额接口返回 {status}: {}", body.chars().take(200).collect::<String>());
    }
    let payload: BalanceResponse = response.json().context("parsing balance response")?;
    if !payload.is_available {
        return Ok(None);
    }
    Ok(Some(payload.balance_infos))
}

/// 人类可读输出：`¥ 12.34（总）· 充值 12.34 · 赠送 0.00`
pub fn format_balances(infos: &[BalanceInfo]) -> String {
    if infos.is_empty() {
        return "余额信息为空".to_string();
    }
    infos
        .iter()
        .map(|info| {
            format!(
                "{} {}（总）· 充值 {} · 赠送 {}",
                info.currency,
                info.total_balance,
                info.topped_up_balance,
                info.granted_balance
            )
        })
        .collect::<Vec<_>>()
        .join("；")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_balances_readably() {
        let infos = vec![BalanceInfo {
            currency: "CNY".to_string(),
            total_balance: "12.34".to_string(),
            granted_balance: "0.00".to_string(),
            topped_up_balance: "12.34".to_string(),
        }];
        let text = format_balances(&infos);
        assert!(text.contains("CNY 12.34"));
        assert!(text.contains("充值 12.34"));
        assert!(text.contains("赠送 0.00"));
    }
}
