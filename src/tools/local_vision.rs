//! 本地视觉工具：Apple Vision 离线图片分析（OCR + 分类标签 + 对象检测）。
//!
//! 不消耗任何模型 API 额度——opencode/DeepSeek 等模型超额时，
//! GQY 仍然可以"看"图片（读文字、识别画面内容）。
//! 依赖 macOS 自带的 swift 与 Vision 框架，免费、离线、无隐私外泄。

use super::{ToolRegistry, ToolSpec};
use crate::i18n::agent_text as t;
use crate::paths::GqyPaths;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

/// 定位 vision-tool.swift：brew 装 = share/gqy/scripts；源码 = <repo>/src/scripts。
fn vision_tool_path(paths: &GqyPaths) -> Result<PathBuf> {
    let candidates = [
        paths.share_dir.join("scripts/vision-tool.swift"),
        paths.share_dir.join("src/scripts/vision-tool.swift"),
    ];
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    anyhow::bail!(
        "找不到本地视觉工具 vision-tool.swift（预期位置：{}）",
        candidates[0].display()
    )
}

pub fn register(registry: &mut ToolRegistry, paths: GqyPaths) {
    registry.register(ToolSpec::new(
        "analyze_image_local",
        t(
            "Analyze a local image offline via Apple Vision: OCR text, classification, object detection. Free, no API quota. Use when the vision model is rate-limited or unavailable.",
            "用 Apple Vision 本地离线分析图片：OCR 文字、分类标签、对象检测。免费不耗 API 额度。视觉模型限流或不可用时看图。",
        ),
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": t("Local image file path.", "本地图片文件路径。") }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        move |args| {
            let paths = paths.clone();
            async move { analyze_image_local(args, &paths).await }
        },
    ));
}

async fn analyze_image_local(args: Value, paths: &GqyPaths) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("path is required"))?
        .to_string();
    let script = vision_tool_path(paths)?;
    let display_path = path.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("swift")
            .arg(&script)
            .arg(&path)
            .arg("all")
            .output()
            .context("running vision-tool (swift + Vision)")
    })
    .await??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "本地视觉分析失败（{}）：{}",
            display_path,
            stderr.chars().take(200).collect::<String>()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|_| json!({ "raw": stdout.trim().to_string() }));

    let ocr = parsed
        .get("ocr")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let labels = parsed
        .get("labels")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let objects = parsed
        .get("objects")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut sections = Vec::new();
    if !labels.is_empty() {
        sections.push(format!(
            "{}：{}",
            t("Scene labels", "画面识别"),
            labels.join("、")
        ));
    }
    if !ocr.is_empty() {
        sections.push(format!(
            "{}：\n{}",
            t("Text in image", "图片中的文字"),
            ocr.join("\n")
        ));
    }
    if !objects.is_empty() {
        sections.push(format!(
            "{}：{}",
            t("Detected objects", "检测到的对象"),
            objects.join("、")
        ));
    }
    if sections.is_empty() {
        return Ok(t(
            "Local analysis found nothing notable in this image.",
            "本地分析未从图片中识别出明显内容。",
        )
        .to_string());
    }
    Ok(sections.join("\n\n"))
}
