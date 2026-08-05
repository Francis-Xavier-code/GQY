//! Interactive configuration TUI (alternate-screen forms and menus).
mod personas;
mod plugins;
mod providers;
mod settings;
mod widgets;

use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::paths::GqyPaths;
use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::event::KeyCode;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use personas::edit_custom_prompts;
use plugins::edit_plugins;
use providers::{
    active_multimodal_label, select_active_multimodal_provider, select_active_provider,
    ProviderBrowser,
};
use settings::edit_settings;
use std::io::{self, Write};
use widgets::{active_label, draw_menu, read_key};

pub fn run(paths: &GqyPaths) -> Result<()> {
    AppConfig::init_files(paths)?;
    crate::models_cache::try_load(paths);
    crate::models_cache::spawn_background_refresh(paths.clone());
    let config = AppConfig::load_or_default(paths)?;
    TerminalSession::start()?.run(paths, config)
}

struct TerminalSession {
    stdout: io::Stdout,
}

impl TerminalSession {
    fn start() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self { stdout })
    }

    fn run(mut self, paths: &GqyPaths, mut config: AppConfig) -> Result<()> {
        let result = run_main_menu(&mut self.stdout, paths, &mut config);
        execute!(self.stdout, Show, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        let _ = result?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn run_main_menu(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &mut AppConfig,
) -> Result<bool> {
    let mut selected = 0usize;
    loop {
        let active = active_label(config);
        let multimodal = active_multimodal_label(config);
        let options = [
            format!(
                "{} ({}: {active})",
                t("Configure text model", "配置文本模型"),
                t("Current", "当前")
            ),
            format!(
                "{} ({}: {multimodal})",
                t("Configure multimodal model", "配置多模态模型"),
                t("Current", "当前")
            ),
            t("Providers and models", "供应商和模型").to_string(),
            t("Plugins", "插件配置").to_string(),
            t("Custom prompts", "自定义提示词").to_string(),
            t("Global settings", "全局参数设置").to_string(),
            t("Save and exit", "保存并退出").to_string(),
        ];
        draw_menu(
            stdout,
            t(" GQY 配置 ", " GQY 配置 "),
            &options,
            selected,
            "",
        )?;

        match read_key()? {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Enter => match selected {
                0 => select_active_provider(stdout, config)?,
                1 => select_active_multimodal_provider(stdout, config)?,
                2 => ProviderBrowser::new(paths, config).run(stdout)?,
                3 => edit_plugins(stdout, config)?,
                4 => edit_custom_prompts(stdout, paths, config)?,
                5 => edit_settings(stdout, config)?,
                6 => {
                    config.save(paths)?;
                    return Ok(true);
                }
                _ => {}
            },
            _ => {}
        }
    }
}


#[cfg(test)]
mod tests {
    use super::providers::parse_extra_body;
    use super::widgets::{
        field_display_value, language_choice_label, language_choice_value, Field,
    };
    use crate::i18n::text as t;

    #[test]
    fn sensitive_field_is_masked_until_actively_edited() {
        let field = Field::new("API Key", "secret-key".to_string()).sensitive();

        assert_eq!(field_display_value(&field, false), "********");
        assert_eq!(field_display_value(&field, true), "secret-key");
    }

    #[test]
    fn empty_sensitive_field_remains_empty() {
        let field = Field::new("API Key", String::new()).sensitive();

        assert_eq!(field_display_value(&field, false), "");
    }

    #[test]
    fn sensitive_textarea_displays_configured_item_count() {
        let field = Field::textarea("API Keys", "first\n\nsecond, third".to_string()).sensitive();

        assert_eq!(
            field_display_value(&field, false),
            t("[3 configured]", "[已配置 3 项]")
        );
    }

    #[test]
    fn empty_sensitive_textarea_keeps_editor_placeholder() {
        let field = Field::textarea("API Keys", String::new()).sensitive();

        assert_eq!(
            field_display_value(&field, false),
            t("(Enter opens $EDITOR)", "(Enter 打开 $EDITOR)")
        );
    }

    #[test]
    fn language_choices_have_locale_specific_labels() {
        assert_eq!(language_choice_label("auto", false), Some("Auto"));
        assert_eq!(language_choice_label("en", false), Some("English"));
        assert_eq!(
            language_choice_label("zh", false),
            Some("Simplified Chinese")
        );
        assert_eq!(language_choice_label("auto", true), Some("自动"));
        assert_eq!(language_choice_label("en", true), Some("英语"));
        assert_eq!(language_choice_label("zh", true), Some("简体中文"));
    }

    #[test]
    fn language_choice_labels_map_to_stable_values() {
        for value in ["auto", "Auto", "自动"] {
            assert_eq!(language_choice_value(value), Some("auto"));
        }
        for value in ["en", "English", "英语"] {
            assert_eq!(language_choice_value(value), Some("en"));
        }
        for value in ["zh", "Simplified Chinese", "简体中文"] {
            assert_eq!(language_choice_value(value), Some("zh"));
        }
        assert_eq!(language_choice_value("unsupported"), None);
    }

    #[test]
    fn extra_body_parser_accepts_only_json_objects() {
        for input in ["true", "\"hello\"", "[1, 2, 3]", "{invalid"] {
            assert!(parse_extra_body(input).is_err());
        }

        let parsed = parse_extra_body(r#"{"enable_thinking":false}"#)
            .unwrap()
            .unwrap();
        assert_eq!(parsed["enable_thinking"], false);
        assert!(parse_extra_body("  ").unwrap().is_none());
    }
}

