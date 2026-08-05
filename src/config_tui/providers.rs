//! Provider / model browser and active-model selection.
use super::widgets::{
    column_scroll, column_visible_rows, draw_column, draw_menu, message, modality_field_value,
    normalize_base_url, parse_modalities, read_key, read_key_with_timeout, run_form, truncate,
    Field,
};
use crate::config::{AppConfig, ProviderConfig};
use crate::default_models::{OPENCODE_DEFAULT_VISION_MODEL, OPENCODE_PROVIDER_ID};
use crate::i18n::{is_zh, text as t};
use crate::paths::GqyPaths;
use anyhow::{bail, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::KeyCode;
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub(crate) struct ProviderBrowser<'a> {
    paths: &'a GqyPaths,
    config: &'a mut AppConfig,
    active_col: usize,
    provider_idx: usize,
    provider_scroll: usize,
    org_idx: usize,
    org_scroll: usize,
    model_idx: usize,
    model_scroll: usize,
    filter: String,
    filter_mode: bool,
    raw_models: Vec<String>,
    orgs: Vec<String>,
    models: Vec<ModelEntry>,
    status: String,
    loading: bool,
    fetch_seq: u64,
    fetch_rx: Option<Receiver<FetchResult>>,
}

impl<'a> ProviderBrowser<'a> {
    pub(crate) fn new(paths: &'a GqyPaths, config: &'a mut AppConfig) -> Self {
        Self {
            paths,
            config,
            active_col: 0,
            provider_idx: 0,
            provider_scroll: 0,
            org_idx: 0,
            org_scroll: 0,
            model_idx: 0,
            model_scroll: 0,
            filter: String::new(),
            filter_mode: false,
            raw_models: Vec::new(),
            orgs: Vec::new(),
            models: Vec::new(),
            status: String::new(),
            loading: false,
            fetch_seq: 0,
            fetch_rx: None,
        }
    }

    pub(crate) fn run(mut self, stdout: &mut io::Stdout) -> Result<()> {
        self.refresh_models();
        loop {
            self.poll_fetch_result();
            self.draw(stdout)?;
            match read_key_with_timeout(if self.loading {
                Some(Duration::from_millis(100))
            } else {
                None
            })? {
                None => continue,
                Some(key) => match key {
                    key if self.filter_mode => self.handle_filter_key(key),
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Left | KeyCode::Char('h') => self.move_left(),
                    KeyCode::Right | KeyCode::Char('l') => self.move_right(),
                    KeyCode::Up | KeyCode::Char('k') => self.move_up(),
                    KeyCode::Down | KeyCode::Char('j') => self.move_down(),
                    KeyCode::Char('/') => {
                        self.filter_mode = true;
                        self.filter.clear();
                        self.rebuild_models();
                    }
                    KeyCode::Char('r') => self.refresh_models(),
                    KeyCode::Char('a') => self.add_provider(stdout)?,
                    KeyCode::Char('d') => self.delete_provider(),
                    KeyCode::Tab if self.active_col == 2 => self.toggle_model_activation(),
                    KeyCode::Enter | KeyCode::Char('i') => self.select_or_edit(stdout)?,
                    _ => {}
                },
            }
        }
    }

    fn handle_filter_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.filter_mode = false;
                self.filter.clear();
            }
            KeyCode::Enter => self.filter_mode = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(ch) => self.filter.push(ch),
            _ => {}
        }
        self.rebuild_models();
    }

    fn move_left(&mut self) {
        self.active_col = self.active_col.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.active_col = (self.active_col + 1).min(2);
    }

    fn move_up(&mut self) {
        match self.active_col {
            0 => {
                self.provider_idx = self.provider_idx.saturating_sub(1);
                self.provider_scroll = column_scroll(
                    self.provider_idx,
                    self.provider_scroll,
                    column_visible_rows(),
                );
                self.refresh_models();
            }
            1 => {
                self.org_idx = self.org_idx.saturating_sub(1);
                self.org_scroll =
                    column_scroll(self.org_idx, self.org_scroll, column_visible_rows());
                self.rebuild_models();
            }
            2 => {
                self.model_idx = self.model_idx.saturating_sub(1);
                self.model_scroll =
                    column_scroll(self.model_idx, self.model_scroll, column_visible_rows());
            }
            _ => {}
        }
    }

    fn move_down(&mut self) {
        match self.active_col {
            0 => {
                self.provider_idx =
                    (self.provider_idx + 1).min(self.config.providers.len().saturating_sub(1));
                self.provider_scroll = column_scroll(
                    self.provider_idx,
                    self.provider_scroll,
                    column_visible_rows(),
                );
                self.refresh_models();
            }
            1 => {
                self.org_idx = (self.org_idx + 1).min(self.orgs.len().saturating_sub(1));
                self.org_scroll =
                    column_scroll(self.org_idx, self.org_scroll, column_visible_rows());
                self.rebuild_models();
            }
            2 => {
                self.model_idx = (self.model_idx + 1).min(self.models.len().saturating_sub(1));
                self.model_scroll =
                    column_scroll(self.model_idx, self.model_scroll, column_visible_rows());
            }
            _ => {}
        }
    }

    fn refresh_models(&mut self) {
        self.provider_idx = self
            .provider_idx
            .min(self.config.providers.len().saturating_sub(1));
        self.raw_models.clear();
        self.orgs = vec!["All".to_string()];
        self.models.clear();
        self.fetch_seq += 1;
        if let Some(provider) = self.config.providers.get(self.provider_idx).cloned() {
            let seq = self.fetch_seq;
            let (tx, rx) = mpsc::channel();
            self.fetch_rx = Some(rx);
            self.loading = true;
            self.status = t("Fetching model list...", "正在获取模型列表...").to_string();
            std::thread::spawn(move || {
                let result = fetch_models(&provider).map_err(|err| err.to_string());
                let _ = tx.send((seq, result));
            });
        } else {
            self.fetch_rx = None;
            self.loading = false;
            self.status.clear();
        }
        self.org_idx = 0;
        self.model_idx = 0;
        self.org_scroll = 0;
        self.model_scroll = 0;
    }

    fn poll_fetch_result(&mut self) {
        let Some(rx) = &self.fetch_rx else {
            return;
        };
        let Ok((seq, result)) = rx.try_recv() else {
            return;
        };
        if seq != self.fetch_seq {
            return;
        }
        self.loading = false;
        self.fetch_rx = None;
        match result {
            Ok(models) => {
                self.status = if is_zh() {
                    format!("已获取 {} 个模型", models.len())
                } else {
                    format!("Fetched {} models", models.len())
                };
                self.raw_models = models;
            }
            Err(err) => {
                let status = if is_zh() {
                    format!("获取模型失败: {err}")
                } else {
                    format!("Failed to fetch models: {err}")
                };
                self.status = format_status_line(&status);
                self.raw_models.clear();
            }
        }
        self.rebuild_models();
    }

    fn rebuild_models(&mut self) {
        let filter = self.filter.to_ascii_lowercase();
        let mut grouped: BTreeMap<String, Vec<ModelEntry>> = BTreeMap::new();
        for model in &self.raw_models {
            if !filter.is_empty() && !model.to_ascii_lowercase().contains(&filter) {
                continue;
            }
            let org = model
                .split_once('/')
                .map(|(org, _)| org)
                .unwrap_or("All")
                .to_string();
            let name = model
                .split_once('/')
                .map(|(_, name)| name)
                .unwrap_or(model)
                .to_string();
            grouped
                .entry("All".to_string())
                .or_default()
                .push(ModelEntry::new(model, model));
            if org != "All" {
                grouped
                    .entry(org)
                    .or_default()
                    .push(ModelEntry::new(&name, model));
            }
        }
        self.orgs = grouped.keys().cloned().collect();
        if self.orgs.is_empty() {
            self.orgs.push("All".to_string());
        }
        self.org_idx = self.org_idx.min(self.orgs.len().saturating_sub(1));
        self.models = grouped.remove(&self.orgs[self.org_idx]).unwrap_or_default();
        self.model_idx = self.model_idx.min(self.models.len().saturating_sub(1));
        self.org_scroll = column_scroll(self.org_idx, self.org_scroll, column_visible_rows());
        self.model_scroll = column_scroll(self.model_idx, self.model_scroll, column_visible_rows());
    }

    fn add_provider(&mut self, stdout: &mut io::Stdout) -> Result<()> {
        if let Some(provider) = edit_provider_form(stdout, ProviderConfig::new_custom())? {
            self.config.upsert_provider(provider);
            self.provider_idx = self.config.providers.len().saturating_sub(1);
            self.refresh_models();
        }
        Ok(())
    }

    fn delete_provider(&mut self) {
        if self.config.providers.is_empty() {
            return;
        }
        let removed = self.config.providers.remove(self.provider_idx);
        if self.config.active_provider == removed.id {
            self.config.active_provider = self
                .config
                .providers
                .first()
                .map(|provider| provider.id.clone())
                .unwrap_or_default();
        }
        self.provider_idx = self
            .provider_idx
            .min(self.config.providers.len().saturating_sub(1));
        self.refresh_models();
    }

    fn select_or_edit(&mut self, stdout: &mut io::Stdout) -> Result<()> {
        match self.active_col {
            0 => {
                if let Some(provider) = self.config.providers.get(self.provider_idx).cloned() {
                    if let Some(provider) = edit_provider_form(stdout, provider)? {
                        let old_id = self.config.providers[self.provider_idx].id.clone();
                        self.config.providers[self.provider_idx] = provider.clone();
                        if self.config.active_provider == old_id {
                            self.config.active_provider = provider.id.clone();
                        }
                        self.refresh_models();
                    }
                }
            }
            2 => {
                if let Some(model) = self.models.get(self.model_idx).cloned() {
                    if let Some(provider) = self.config.providers.get_mut(self.provider_idx) {
                        auto_configure_model_tags(self.paths, provider, &model.full);
                    }
                    if let Some(provider) = self.config.providers.get_mut(self.provider_idx) {
                        if edit_model_form(stdout, provider, &model.full)? {
                            self.config.active_provider = provider.id.clone();
                            self.status = if is_zh() {
                                format!("已更新模型设置: {}", model.full)
                            } else {
                                format!("Updated model settings: {}", model.full)
                            };
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn toggle_model_activation(&mut self) {
        if self.active_col != 2 {
            return;
        }
        let mut removed = None;
        if let (Some(provider), Some(model)) = (
            self.config.providers.get_mut(self.provider_idx),
            self.models.get(self.model_idx),
        ) {
            if let Some(index) = provider.models.iter().position(|item| item == &model.full) {
                let provider_id = provider.id.clone();
                let model = model.full.clone();
                provider.models.remove(index);
                if provider.default_model == model {
                    provider.default_model = provider.models.first().cloned().unwrap_or_default();
                }
                self.status = if is_zh() {
                    format!("已取消激活模型: {model}")
                } else {
                    format!("Deactivated model: {model}")
                };
                removed = Some((provider_id, model));
            } else {
                provider.models.push(model.full.clone());
                auto_configure_model_tags(self.paths, provider, &model.full);
                if provider.default_model.trim().is_empty() {
                    provider.default_model = model.full.clone();
                }
                self.status = if is_zh() {
                    format!("已激活模型: {}", model.full)
                } else {
                    format!("Activated model: {}", model.full)
                };
            }
        }
        if let Some((provider_id, model)) = removed {
            self.config
                .remove_active_model_references(&provider_id, &model);
        }
    }

    fn draw(&self, stdout: &mut io::Stdout) -> Result<()> {
        let (cols, rows) = terminal::size()?;
        let inner_x = 0;
        let inner_y = 0;
        let inner_w = cols;
        let inner_h = rows.saturating_sub(2);
        let left_w = inner_w.saturating_mul(28).saturating_div(100).max(20);
        let mid_w = inner_w.saturating_mul(22).saturating_div(100).max(16);
        let right_w = inner_w
            .saturating_sub(left_w)
            .saturating_sub(mid_w)
            .saturating_sub(2)
            .max(18);
        let providers = self
            .config
            .providers
            .iter()
            .map(|provider| {
                let active = if provider.id == self.config.active_provider {
                    "* "
                } else {
                    "  "
                };
                format!("{active}{}", provider.display_name)
            })
            .collect::<Vec<_>>();
        let models = self
            .models
            .iter()
            .map(|model| {
                let active = self
                    .config
                    .providers
                    .get(self.provider_idx)
                    .map(|provider| provider.models.iter().any(|item| item == &model.full))
                    .unwrap_or(false);
                format!("{} {}", if active { "[*]" } else { "[ ]" }, model.name)
            })
            .collect::<Vec<_>>();
        let orgs = self
            .orgs
            .iter()
            .map(|org| {
                if org == "All" {
                    t("All", "全部").to_string()
                } else {
                    org.clone()
                }
            })
            .collect::<Vec<_>>();

        queue!(stdout, Clear(ClearType::All))?;
        draw_column(
            stdout,
            inner_x,
            inner_y,
            left_w,
            inner_h,
            t(" PROVIDERS ", " 供应商 "),
            &providers,
            self.provider_idx,
            self.provider_scroll,
            self.active_col == 0,
        )?;
        draw_column(
            stdout,
            inner_x + left_w + 1,
            inner_y,
            mid_w,
            inner_h,
            t(" ORGANIZATION ", " 组织 "),
            &orgs,
            self.org_idx,
            self.org_scroll,
            self.active_col == 1,
        )?;
        let title = if self.filter.is_empty() {
            t(" MODELS ", " 模型 ").to_string()
        } else if is_zh() {
            format!(" 模型 /{} ", self.filter)
        } else {
            format!(" MODELS /{} ", self.filter)
        };
        draw_column(
            stdout,
            inner_x + left_w + mid_w + 2,
            inner_y,
            right_w,
            inner_h,
            &title,
            &models,
            self.model_idx,
            self.model_scroll,
            self.active_col == 2,
        )?;
        let help = if self.filter_mode {
            if is_zh() {
                format!("搜索: {}_  [Enter]确认 [Esc]取消", self.filter)
            } else {
                format!("Search: {}_  [Enter]confirm [Esc]cancel", self.filter)
            }
        } else {
            t(
                "[h/l]column [j/k]move [Tab]activate model [Enter]model settings [/]search [r]refresh [a]add [d]delete [q]back",
                "[h/l]切栏 [j/k]移动 [Tab]激活模型 [Enter]模型设置 [/]搜索 [r]刷新 [a]添加 [d]删除 [q]返回",
            )
            .to_string()
        };
        let status = if self.loading {
            format!("{}", self.status)
        } else {
            self.status.clone()
        };
        queue!(
            stdout,
            MoveTo(0, rows.saturating_sub(2)),
            Clear(ClearType::CurrentLine),
            Print(truncate(&status, cols as usize))
        )?;
        queue!(
            stdout,
            MoveTo(0, rows.saturating_sub(1)),
            Clear(ClearType::CurrentLine),
            Print(truncate(&help, cols as usize))
        )?;
        stdout.flush()?;
        Ok(())
    }
}

type FetchResult = (u64, Result<Vec<String>, String>);

fn format_status_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone)]
struct ModelEntry {
    name: String,
    full: String,
}

impl ModelEntry {
    fn new(name: &str, full: &str) -> Self {
        Self {
            name: name.to_string(),
            full: full.to_string(),
        }
    }
}

fn fetch_models(provider: &ProviderConfig) -> Result<Vec<String>> {
    let api_key = provider.api_key.as_deref().unwrap_or_default();
    let mut api_key = if let Some(env_name) = api_key.strip_prefix("$env:") {
        std::env::var(env_name).unwrap_or_default()
    } else {
        api_key.to_string()
    };
    if api_key.is_empty() && provider.is_opencode_zen() {
        api_key = "public".to_string();
    }
    let url = models_url(&provider.base_url);
    let mut request = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(provider.timeout_seconds))
        .build()?
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "gqy-config");
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }
    let response = request.send()?;
    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        bail!("{status}: {body}");
    }
    let parsed: ModelsResponse = serde_json::from_str(&body)?;
    Ok(parsed
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|id| !id.is_empty())
        .collect())
}

fn auto_configure_model_tags(paths: &GqyPaths, provider: &mut ProviderConfig, model: &str) {
    if provider.model_modalities.contains_key(model) {
        return;
    }
    if let Some(modalities) =
        crate::models_cache::input_modalities_blocking(paths, &provider.id, model)
            .filter(|modalities| !modalities.is_empty())
    {
        provider
            .model_modalities
            .insert(model.to_string(), modalities);
    }
}

fn models_url(base_url: &str) -> String {
    let mut url = base_url.trim().trim_end_matches('/').to_string();
    if url.ends_with("/chat/completions") {
        url.truncate(url.len() - "/chat/completions".len());
    }
    if url.ends_with("/v1") {
        format!("{url}/models")
    } else {
        format!("{url}/v1/models")
    }
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    id: String,
}

pub(crate) fn select_active_provider(stdout: &mut io::Stdout, config: &mut AppConfig) -> Result<()> {
    let mut choices = config.text_provider_model_choices();
    if choices.is_empty() {
        message(
            stdout,
            t(
                "No text models are selected. Activate one with Tab under Providers and models first.",
                "没有已勾选的文本模型，请先在供应商和模型里用 Tab 激活模型。",
            ),
        )?;
        return Ok(());
    }
    let mut selected = choices
        .iter()
        .position(|choice| config.is_active_provider_model(&choice.provider_id, &choice.model))
        .unwrap_or(0);
    loop {
        let options = choices
            .iter()
            .map(|choice| {
                let marker = if config.is_active_provider_model(&choice.provider_id, &choice.model)
                {
                    "[*] "
                } else {
                    "[ ] "
                };
                format!("{marker}{}", choice.label())
            })
            .collect::<Vec<_>>();
        draw_menu(
            stdout,
            t(" SELECT TEXT MODEL ", " 选择文本模型 "),
            &options,
            selected,
            t(
                "[Tab]activate/deactivate [Enter/q]confirm [d]remove",
                "[Tab]激活/取消 [Enter/q]确认 [d]移除",
            ),
        )?;
        match read_key()? {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Enter => return Ok(()),
            KeyCode::Tab => {
                let choice = choices[selected].clone();
                config.toggle_active_provider_model(&choice.provider_id, &choice.model)?;
            }
            KeyCode::Char('d') => {
                let choice = choices[selected].clone();
                config.remove_active_provider_model(&choice.provider_id, &choice.model)?;
                choices = config.text_provider_model_choices();
                if choices.is_empty() {
                    message(
                        stdout,
                        t(
                            "The active model was removed; no models are currently available.",
                            "已移除激活模型，当前没有可用模型。",
                        ),
                    )?;
                    return Ok(());
                }
                selected = selected.min(choices.len().saturating_sub(1));
            }
            _ => {}
        }
    }
}

fn edit_provider_form(
    stdout: &mut io::Stdout,
    provider: ProviderConfig,
) -> Result<Option<ProviderConfig>> {
    let current_context_window = provider
        .model_context_window
        .get(&provider.default_model)
        .copied()
        .unwrap_or_default();

    // 将 extra_body 格式化为 JSON 字符串，方便编辑
    let extra_body_string = provider
        .extra_body
        .as_ref()
        .and_then(|v| serde_json::to_string_pretty(v).ok())
        .unwrap_or_default();

    let mut fields = vec![
        Field::new(t("Configuration ID", "配置 ID"), provider.id.clone()),
        Field::new(t("Display name", "显示名称"), provider.display_name.clone()),
        Field::new("Base URL", provider.base_url.clone()),
        Field::new(t("Protocol", "协议"), provider.protocol.clone()).choices(&[
            "auto",
            "openai-chat",
            "openai-responses",
            "anthropic",
            "pi",
        ]),
        Field::new(
            t("API Key or $env:NAME", "API Key 或 $env:NAME"),
            provider.api_key.clone().unwrap_or_default(),
        )
        .sensitive(),
        Field::new(
            t("Current model", "当前模型"),
            provider.default_model.clone(),
        ),
        Field::new(
            t(
                "Model context window (tokens, 0=auto)",
                "模型上下文窗口 (tokens, 0=自动)",
            ),
            current_context_window.to_string(),
        ),
        Field::new(
            t("Timeout (seconds)", "超时秒数"),
            provider.timeout_seconds.to_string(),
        ),
        Field::new("Temperature", provider.temperature.to_string()),
        Field::textarea(
            t("Extra request body (JSON)", "额外请求体 (JSON)"),
            extra_body_string,
        ),
    ];

    // 循环直到用户取消或输入合法 JSON 对象
    loop {
        if !run_form(stdout, t(" EDIT PROVIDER ", " 编辑供应商 "), &mut fields)? {
            return Ok(None);
        }

        // 提取各个字段的值（索引保持不变）
        let default_model = fields[5].value.trim().to_string();
        let model_context_window_raw = fields[6].value.trim().parse::<usize>().unwrap_or_default();
        let timeout = fields[7].value.trim().parse().unwrap_or(60);
        let temperature = fields[8].value.trim().parse().unwrap_or(0.7);

        let extra_body = match parse_extra_body(&fields[9].value) {
            Ok(extra_body) => extra_body,
            Err(error) => {
                message(stdout, &error)?;
                continue;
            }
        };

        // 构建 model_context_window
        let mut model_context_window = provider.model_context_window.clone();
        match model_context_window_raw {
            0 => {
                model_context_window.remove(&default_model);
            }
            value => {
                model_context_window.insert(default_model.clone(), value);
            }
        }

        let mut models = provider.models.clone();
        if !default_model.trim().is_empty() && !models.iter().any(|item| item == &default_model) {
            models.push(default_model.clone());
        }

        // 所有验证通过，返回新的 ProviderConfig
        return Ok(Some(ProviderConfig {
            id: fields[0].value.trim().to_string(),
            display_name: fields[1].value.trim().to_string(),
            base_url: normalize_base_url(&fields[2].value),
            protocol: fields[3].value.trim().to_string(),
            api_key: Some(fields[4].value.trim().to_string()).filter(|value| !value.is_empty()),
            models,
            model_context_window,
            model_modalities: provider.model_modalities.clone(),
            default_model,
            timeout_seconds: timeout,
            temperature: temperature,
            anthropic_max_tokens: provider.anthropic_max_tokens,
            extra_body,
            prompt_caching: provider.prompt_caching,
        }));
    }
}

pub(crate) fn parse_extra_body(
    value: &str,
) -> std::result::Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(value) {
        Ok(serde_json::Value::Object(object)) => Ok(Some(object)),
        Ok(_) => Err(t(
            "The extra request body must be a JSON object (for example {\"key\": \"value\"})",
            "额外请求体必须是 JSON 对象 (如 {\"key\": \"value\"})",
        )
        .to_string()),
        Err(error) => Err(if is_zh() {
            format!("无效 JSON: {error}")
        } else {
            format!("Invalid JSON: {error}")
        }),
    }
}

fn edit_model_form(
    stdout: &mut io::Stdout,
    provider: &mut ProviderConfig,
    model: &str,
) -> Result<bool> {
    let context_window = provider
        .model_context_window
        .get(model)
        .copied()
        .unwrap_or_default();
    let mut fields = vec![
        Field::modalities(
            t("Supported input", "支持输入"),
            modality_field_value(provider, model),
        ),
        Field::new(
            t(
                "Model context window (tokens, 0=auto)",
                "模型上下文窗口 (tokens, 0=自动)",
            ),
            context_window.to_string(),
        ),
        Field::new("Temperature", provider.temperature.to_string()),
    ];
    if !run_form(stdout, t(" EDIT MODEL ", " 编辑模型 "), &mut fields)? {
        return Ok(false);
    }
    provider
        .model_modalities
        .insert(model.to_string(), parse_modalities(&fields[0].value));
    match fields[1].value.trim().parse::<usize>().unwrap_or_default() {
        0 => {
            provider.model_context_window.remove(model);
        }
        value => {
            provider
                .model_context_window
                .insert(model.to_string(), value);
        }
    }
    provider.temperature = fields[2].value.trim().parse().unwrap_or(0.7);
    Ok(true)
}

pub(crate) fn provider_model_choice_values(config: &AppConfig, include_current: bool) -> Vec<String> {
    let mut choices = vec![String::new()];
    if include_current {
        choices.push(format!(
            "{OPENCODE_PROVIDER_ID}\t{OPENCODE_DEFAULT_VISION_MODEL}"
        ));
    }
    choices.extend(
        config
            .provider_model_choices()
            .into_iter()
            .map(|choice| choice.value()),
    );
    choices
}

pub(crate) fn active_multimodal_label(config: &AppConfig) -> String {
    let choices = config.active_multimodal_provider_model_choices();
    if choices.is_empty() {
        format!(
            "{} / {}",
            OPENCODE_PROVIDER_ID, OPENCODE_DEFAULT_VISION_MODEL
        )
    } else if choices.len() == 1 {
        choices[0].label()
    } else {
        t("Mixed", "混合").to_string()
    }
}




pub(crate) fn select_active_multimodal_provider(
    stdout: &mut io::Stdout,
    config: &mut AppConfig,
) -> Result<()> {
    let choices = config.multimodal_provider_model_choices();
    if choices.is_empty() {
        message(
            stdout,
            t(
                "No models are marked as multimodal. Configure Supported input under Edit model first.",
                "没有已标记为多模态的模型，请先在编辑模型里配置支持输入。",
            ),
        )?;
        return Ok(());
    }
    let mut selected = choices
        .iter()
        .position(|choice| {
            config.is_active_multimodal_provider_model(&choice.provider_id, &choice.model)
        })
        .unwrap_or(0);
    loop {
        let options = choices
            .iter()
            .map(|choice| {
                let marker = if config
                    .is_active_multimodal_provider_model(&choice.provider_id, &choice.model)
                {
                    "[*] "
                } else {
                    "[ ] "
                };
                format!("{marker}{}", choice.label())
            })
            .collect::<Vec<_>>();
        draw_menu(
            stdout,
            t(" SELECT MULTIMODAL MODEL ", " 选择多模态模型 "),
            &options,
            selected,
            t(
                "[Tab]activate/deactivate [Enter/q]confirm",
                "[Tab]激活/取消 [Enter/q]确认",
            ),
        )?;
        match read_key()? {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => return Ok(()),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Tab => {
                let choice = choices[selected].clone();
                config
                    .toggle_active_multimodal_provider_model(&choice.provider_id, &choice.model)?;
            }
            _ => {}
        }
    }
}

pub(crate) fn vision_provider_value(config: &AppConfig) -> String {
    let vision = &config.plugins.vision;
    if vision.vision_provider_id.trim().is_empty() {
        format!("{OPENCODE_PROVIDER_ID}\t{OPENCODE_DEFAULT_VISION_MODEL}")
    } else if vision.vision_model.trim().is_empty() {
        config
            .provider(Some(vision.vision_provider_id.trim()))
            .map(|provider| format!("{}\t{}", provider.id, provider.default_model))
            .unwrap_or_else(|_| vision.vision_provider_id.clone())
    } else {
        format!("{}\t{}", vision.vision_provider_id, vision.vision_model)
    }
}

pub(crate) fn kb_embedding_provider_value(config: &AppConfig) -> String {
    let kb = &config.plugins.knowledge_base;
    if kb.embedding_provider_id.trim().is_empty() {
        String::new()
    } else if kb.embedding_model.trim().is_empty() {
        config
            .provider(Some(kb.embedding_provider_id.trim()))
            .map(|provider| format!("{}\t{}", provider.id, provider.default_model))
            .unwrap_or_else(|_| kb.embedding_provider_id.clone())
    } else {
        format!("{}\t{}", kb.embedding_provider_id, kb.embedding_model)
    }
}

pub(crate) fn parse_provider_model_choice(value: &str) -> (String, String) {
    let value = value.trim();
    if value.is_empty() {
        return (String::new(), String::new());
    }
    if let Some((provider, model)) = value.split_once('\t') {
        return (provider.trim().to_string(), model.trim().to_string());
    }
    (value.to_string(), String::new())
}

