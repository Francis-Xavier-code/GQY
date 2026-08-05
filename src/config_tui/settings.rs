//! Global settings form.
use super::providers::{
    kb_embedding_provider_value, parse_provider_model_choice, provider_model_choice_values,
    vision_provider_value,
};
use super::widgets::{
    language_choice_label, language_choice_value, normalize_tools_loading_mode,
    parse_bool_field, parse_mixed_endpoint_display, run_form_without_buttons, Field,
};
use crate::config::{AppConfig, MAX_COMMAND_OUTPUT_LINES};
use crate::i18n::{is_zh, text as t};
use anyhow::Result;
use std::io;

pub(crate) fn edit_settings(stdout: &mut io::Stdout, config: &mut AppConfig) -> Result<()> {
    let language = language_choice_value(&config.display.language).unwrap_or("auto");
    let mut fields = vec![
        Field::boolean(t("Enable tools", "工具启用"), config.tools.enabled),
        Field::new(
            t("Maximum tool rounds", "工具最大轮数"),
            config.tools.max_rounds.to_string(),
        ),
        Field::new(
            t("Tool loading mode", "工具加载模式"),
            config.tools.loading_mode.clone(),
        )
        .choices(&["full", "hybrid"]),
        Field::boolean(
            t("Remember loaded tools", "记住已加载工具"),
            config.tools.persist_loaded_tools,
        ),
        Field::boolean(t("Enable skills", "Skills 启用"), config.skills.enabled),
        Field::boolean(
            t("Allow command execution", "允许执行命令"),
            config.skills.allow_command_execution,
        ),
        Field::new(t("Interface language", "界面语言"), language.to_string())
            .choices(&["auto", "en", "zh"]),
        Field::new(
            t("Show reasoning", "显示思考过程"),
            config.display.reasoning.clone(),
        )
        .choices(&["summary", "full", "hidden"]),
        Field::new(
            t("Show tool call details", "显示工具调用信息"),
            config.display.tool_calls.clone(),
        )
        .choices(&["summary", "full", "hidden"]),
        Field::new(
            t("Command output lines", "命令输出显示行数"),
            config.display.command_output_lines.to_string(),
        ),
        Field::boolean(
            t("Readable tool names", "工具名可读显示"),
            config.display.readable_tool_names,
        ),
        Field::boolean(
            t(
                "Show token usage in shell conversations",
                "Shell 无缝对话显示 Token 计数",
            ),
            config.display.show_token_usage,
        ),
        Field::new(
            t(
                "Show current provider/model in Mixed mode",
                "Mixed 时显示本次供应商/模型",
            ),
            parse_mixed_endpoint_display(&config.display.mixed_model_endpoint_display),
        )
        .choices(&["off", "interactive", "all"]),
        Field::new(
            t("When context reaches its limit", "上下文到达上限后"),
            config.context.on_overflow.clone(),
        )
        .choices(&["pop", "compact"]),
    ];
    run_form_without_buttons(stdout, t(" GLOBAL SETTINGS ", " 全局设置 "), &mut fields)?;
    config.tools.enabled = parse_bool_field(&fields[0].value)?;
    config.tools.max_rounds = fields[1].value.trim().parse::<usize>()?;
    config.tools.loading_mode = normalize_tools_loading_mode(&fields[2].value);
    config.tools.persist_loaded_tools = parse_bool_field(&fields[3].value)?;
    config.skills.enabled = parse_bool_field(&fields[4].value)?;
    config.skills.allow_command_execution = parse_bool_field(&fields[5].value)?;
    config.display.language = language_choice_value(&fields[6].value)
        .unwrap_or("auto")
        .to_string();
    config.display.reasoning = fields[7].value.trim().to_string();
    config.display.tool_calls = fields[8].value.trim().to_string();
    config.display.command_output_lines = fields[9]
        .value
        .trim()
        .parse::<usize>()?
        .min(MAX_COMMAND_OUTPUT_LINES);
    config.display.readable_tool_names = parse_bool_field(&fields[10].value)?;
    config.display.show_token_usage = parse_bool_field(&fields[11].value)?;
    config.display.mixed_model_endpoint_display = parse_mixed_endpoint_display(&fields[12].value);
    config.context.on_overflow = fields[13].value.trim().to_string();
    Ok(())
}

