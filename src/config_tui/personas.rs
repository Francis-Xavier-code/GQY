//! Persona / identity / custom prompt editors.
use super::widgets::{draw_menu, read_key, run_form, Field};
use crate::config::AppConfig;
use crate::i18n::text as t;
use crate::paths::GqyPaths;
use anyhow::{bail, Result};
use crossterm::event::KeyCode;
use std::io::{self, Write};
use std::path::PathBuf;

pub(crate) fn edit_custom_prompts(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &mut AppConfig,
) -> Result<()> {
    let mut selected = 0usize;
    loop {
        let persona = if config.prompt.active_persona.trim().is_empty() {
            "顾清影".to_string()
        } else {
            persona_display_name(&config.prompt.active_persona).to_string()
        };
        let options = [
            format!(
                "{} ({}: {persona})",
                t("AI persona", "AI 人格"),
                t("Current", "当前")
            ),
            t("User identity", "用户身份").to_string(),
        ];
        draw_menu(
            stdout,
            t(" CUSTOM PROMPTS ", " 自定义提示词 "),
            &options,
            selected,
            t("[Enter]select [q]back", "[Enter]选择 [q]返回"),
        )?;
        match read_key()? {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Enter if selected == 0 => edit_personas(stdout, paths, config)?,
            KeyCode::Enter if selected == 1 => edit_identities(stdout, paths, config)?,
            _ => {}
        }
    }
}

fn edit_personas(stdout: &mut io::Stdout, paths: &GqyPaths, config: &mut AppConfig) -> Result<()> {
    std::fs::create_dir_all(config.prompts_dir_path(paths))?;
    let mut selected = 0usize;
    loop {
        let personas = list_personas(paths, config)?;
        let mut options = Vec::with_capacity(personas.len() + 1);
        let default_marker = if config.prompt.active_persona.trim().is_empty() {
            "* "
        } else {
            "  "
        };
        options.push(format!("{default_marker}顾清影"));
        options.extend(personas.iter().map(|name| {
            let display = persona_display_name(name);
            if *name == config.prompt.active_persona {
                format!("* {display}")
            } else {
                format!("  {display}")
            }
        }));
        selected = selected.min(options.len().saturating_sub(1));
        draw_menu(
            stdout,
            t(" AI PERSONA ", " AI 人格 "),
            &options,
            selected,
            t(
                "[Tab]activate [Enter]edit [a]add [d]delete [j/k]move [q]back",
                "[Tab]激活 [Enter]编辑 [a]新增 [d]删除 [j/k]移动 [q]返回",
            ),
        )?;
        match read_key()? {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Tab => {
                config.prompt.active_persona = if selected == 0 {
                    String::new()
                } else {
                    personas.get(selected - 1).cloned().unwrap_or_default()
                };
            }
            KeyCode::Char('a') => {
                if let Some(name) = new_persona(stdout, paths, config)? {
                    config.prompt.active_persona = name;
                }
            }
            KeyCode::Enter if selected > 0 => {
                if let Some(name) = personas.get(selected - 1) {
                    if let Some(new_name) = edit_persona(stdout, paths, config, name)? {
                        move_persona_scope(paths, config, name, &new_name)?;
                        if config.prompt.active_persona == *name {
                            config.prompt.active_persona = new_name;
                        }
                    }
                }
            }
            KeyCode::Char('d') if selected > 0 => {
                if let Some(name) = personas.get(selected - 1) {
                    let path = config.persona_path(paths, name);
                    if path.exists() {
                        std::fs::remove_file(path)?;
                    }
                    remove_persona_scope(paths, config, name)?;
                    if config.prompt.active_persona == *name {
                        config.prompt.active_persona.clear();
                    }
                    selected = selected.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

fn new_persona(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &AppConfig,
) -> Result<Option<String>> {
    edit_prompt_file_form(
        stdout,
        t(" NEW PERSONA ", " 新建人格 "),
        None,
        String::new(),
        |name, content| write_persona(paths, config, name, content),
    )
}

fn edit_persona(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &AppConfig,
    current_name: &str,
) -> Result<Option<String>> {
    let content = read_persona(paths, config, current_name)?;
    edit_prompt_file_form(
        stdout,
        t(" EDIT PERSONA ", " 编辑人格 "),
        Some(current_name),
        content,
        |name, content| {
            if name != current_name {
                let old_path = config.persona_path(paths, current_name);
                if old_path.exists() {
                    std::fs::remove_file(old_path)?;
                }
            }
            write_persona(paths, config, name, content)
        },
    )
}

fn move_persona_scope(
    paths: &GqyPaths,
    config: &AppConfig,
    old_name: &str,
    new_name: &str,
) -> Result<()> {
    if old_name == new_name {
        return Ok(());
    }
    move_dir_if_exists(
        config.persona_memory_data_dir(paths, old_name),
        config.persona_memory_data_dir(paths, new_name),
    )?;
    move_dir_if_exists(
        config.persona_memory_state_dir(paths, old_name),
        config.persona_memory_state_dir(paths, new_name),
    )?;
    move_dir_if_exists(
        config.persona_skills_dir(paths, old_name),
        config.persona_skills_dir(paths, new_name),
    )?;
    Ok(())
}

fn remove_persona_scope(paths: &GqyPaths, config: &AppConfig, name: &str) -> Result<()> {
    remove_dir_if_exists(config.persona_memory_data_dir(paths, name))?;
    remove_dir_if_exists(config.persona_memory_state_dir(paths, name))?;
    remove_dir_if_exists(config.persona_skills_dir(paths, name))?;
    Ok(())
}

fn move_dir_if_exists(from: PathBuf, to: PathBuf) -> Result<()> {
    if !from.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if to.exists() {
        std::fs::remove_dir_all(&to)?;
    }
    std::fs::rename(from, to)?;
    Ok(())
}

fn remove_dir_if_exists(path: PathBuf) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn edit_identities(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &mut AppConfig,
) -> Result<()> {
    std::fs::create_dir_all(config.identities_dir_path(paths))?;
    let mut selected = 0usize;
    loop {
        let identities = list_identities(paths, config)?;
        let mut options = Vec::with_capacity(identities.len() + 1);
        let default_marker = if config.prompt.active_identity.trim().is_empty() {
            "* "
        } else {
            "  "
        };
        options.push(format!(
            "{default_marker}{}",
            t("Do not use a user identity", "不使用用户身份")
        ));
        options.extend(identities.iter().map(|name| {
            let display = persona_display_name(name);
            if *name == config.prompt.active_identity {
                format!("* {display}")
            } else {
                format!("  {display}")
            }
        }));
        selected = selected.min(options.len().saturating_sub(1));
        draw_menu(
            stdout,
            t(" USER IDENTITY ", " 用户身份 "),
            &options,
            selected,
            t(
                "[Tab]activate [Enter]edit [a]add [d]delete [j/k]move [q]back",
                "[Tab]激活 [Enter]编辑 [a]新增 [d]删除 [j/k]移动 [q]返回",
            ),
        )?;
        match read_key()? {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(options.len() - 1),
            KeyCode::Tab => {
                config.prompt.active_identity = if selected == 0 {
                    String::new()
                } else {
                    identities.get(selected - 1).cloned().unwrap_or_default()
                };
            }
            KeyCode::Char('a') => {
                if let Some(name) = new_identity(stdout, paths, config)? {
                    config.prompt.active_identity = name;
                }
            }
            KeyCode::Enter if selected > 0 => {
                if let Some(name) = identities.get(selected - 1) {
                    if let Some(new_name) = edit_identity(stdout, paths, config, name)? {
                        if config.prompt.active_identity == *name {
                            config.prompt.active_identity = new_name;
                        }
                    }
                }
            }
            KeyCode::Char('d') if selected > 0 => {
                if let Some(name) = identities.get(selected - 1) {
                    let path = config.identity_path(paths, name);
                    if path.exists() {
                        std::fs::remove_file(path)?;
                    }
                    if config.prompt.active_identity == *name {
                        config.prompt.active_identity.clear();
                    }
                    selected = selected.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

fn new_identity(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &AppConfig,
) -> Result<Option<String>> {
    edit_prompt_file_form(
        stdout,
        t(" NEW IDENTITY ", " 新建用户身份 "),
        None,
        String::new(),
        |name, content| write_identity(paths, config, name, content),
    )
}

fn edit_identity(
    stdout: &mut io::Stdout,
    paths: &GqyPaths,
    config: &AppConfig,
    current_name: &str,
) -> Result<Option<String>> {
    let content = read_identity(paths, config, current_name)?;
    edit_prompt_file_form(
        stdout,
        t(" EDIT IDENTITY ", " 编辑用户身份 "),
        Some(current_name),
        content,
        |name, content| {
            if name != current_name {
                let old_path = config.identity_path(paths, current_name);
                if old_path.exists() {
                    std::fs::remove_file(old_path)?;
                }
            }
            write_identity(paths, config, name, content)
        },
    )
}

fn list_identities(paths: &GqyPaths, config: &AppConfig) -> Result<Vec<String>> {
    list_markdown_files(&config.identities_dir_path(paths))
}

fn read_identity(paths: &GqyPaths, config: &AppConfig, name: &str) -> Result<String> {
    let path = config.identity_path(paths, name);
    if path.exists() {
        Ok(std::fs::read_to_string(path)?)
    } else {
        Ok(String::new())
    }
}

fn write_identity(paths: &GqyPaths, config: &AppConfig, name: &str, content: &str) -> Result<()> {
    let path = config.identity_path(paths, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format_text_file(content))?;
    Ok(())
}

fn edit_prompt_file_form<F>(
    stdout: &mut io::Stdout,
    title: &str,
    current_name: Option<&str>,
    content: String,
    write: F,
) -> Result<Option<String>>
where
    F: FnOnce(&str, &str) -> Result<()>,
{
    let mut fields = vec![
        Field::new(
            t("Name", "名称"),
            current_name
                .map(persona_display_name)
                .unwrap_or("")
                .to_string(),
        ),
        Field::textarea(t("Content", "内容"), content),
    ];
    if !run_form(stdout, title, &mut fields)? {
        return Ok(None);
    }
    let name = sanitize_persona_name(&fields[0].value)?;
    write(&name, &fields[1].value)?;
    Ok(Some(name))
}

fn list_personas(paths: &GqyPaths, config: &AppConfig) -> Result<Vec<String>> {
    list_markdown_files(&config.prompts_dir_path(paths))
}

fn list_markdown_files(dir: &std::path::Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

fn read_persona(paths: &GqyPaths, config: &AppConfig, name: &str) -> Result<String> {
    let path = config.persona_path(paths, name);
    if path.exists() {
        Ok(std::fs::read_to_string(path)?)
    } else {
        Ok(String::new())
    }
}

fn write_persona(paths: &GqyPaths, config: &AppConfig, name: &str, content: &str) -> Result<()> {
    let path = config.persona_path(paths, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format_text_file(content))?;
    Ok(())
}

fn sanitize_persona_name(value: &str) -> Result<String> {
    let mut name = value
        .trim()
        .trim_end_matches(".md")
        .replace(['/', '\\'], "-");
    if name.is_empty() {
        bail!("{}", t("Persona name cannot be empty", "人格名称不能为空"));
    }
    name.push_str(".md");
    Ok(name)
}

fn persona_display_name(name: &str) -> &str {
    name.strip_suffix(".md").unwrap_or(name)
}

fn format_text_file(content: &str) -> String {
    let content = content.trim_end();
    if content.is_empty() {
        String::new()
    } else {
        format!("{content}\n")
    }
}

