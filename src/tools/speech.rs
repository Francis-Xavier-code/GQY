//! 语音工具（本地、零 API 成本）：
//! - `speak`：macOS `say` 朗读一段文字（TTS）
//! - `listen_audio`：识别本地音频文件（STT，speech-tool.swift 离线识别）
//! 与本地视觉同思路：不消耗模型额度，适合语音交互场景。

use super::{ToolRegistry, ToolSpec};
use crate::i18n::agent_text as t;
use crate::paths::GqyPaths;
use anyhow::Result;
use serde_json::{json, Value};

pub fn register(registry: &mut ToolRegistry, paths: GqyPaths) {
    let speak_paths = paths.clone();
    registry.register(ToolSpec::new(
        "speak",
        t(
            "Speak a short text aloud using the system voice (macOS `say`, offline, free). Use for greetings, reminders, or when the user asks you to say something. Keep text short.",
            "用系统语音朗读一段文字（macOS `say`，离线免费）。用于问候、提醒，或用户让你说话时。文字保持简短。",
        ),
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": t("Text to speak.", "要朗读的文字。") },
                "voice": { "type": "string", "description": t("Optional voice name (e.g. Ting-Ting, Samantha). Defaults to system voice.", "可选语音名（如 Ting-Ting、Samantha）。默认系统语音。") }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        move |args| {
            let paths = speak_paths.clone();
            async move { speak(args, paths).await }
        },
    ));

    let listen_paths = paths.clone();
    registry.register(ToolSpec::new(
        "listen_audio",
        t(
            "Transcribe a local audio file to text using on-device speech recognition (offline, free). Use when the user sends a voice message file.",
            "把本地音频文件识别成文字（设备端离线识别，免费）。用户发来语音文件时使用。",
        ),
        json!({
            "type": "object",
            "properties": {
                "audio": { "type": "string", "description": t("Path to the audio file (m4a/wav/aiff).", "音频文件路径（m4a/wav/aiff）。") },
                "locale": { "type": "string", "description": t("Recognition language, default zh-Hans.", "识别语言，默认 zh-Hans。") }
            },
            "required": ["audio"],
            "additionalProperties": false
        }),
        move |args| {
            let paths = listen_paths.clone();
            async move { listen_audio(args, paths).await }
        },
    ));
}

async fn speak(args: Value, _paths: GqyPaths) -> Result<String> {
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(json!({"ok": false, "error": "text is required"}).to_string());
    }
    let voice = args.get("voice").and_then(Value::as_str);
    match crate::speech::speak(&text, voice, None) {
        Ok(()) => Ok(json!({"ok": true, "spoken": text.len()}).to_string()),
        Err(err) => Ok(json!({"ok": false, "error": err.to_string()}).to_string()),
    }
}

async fn listen_audio(args: Value, paths: GqyPaths) -> Result<String> {
    let audio = args
        .get("audio")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if audio.is_empty() {
        return Ok(json!({"ok": false, "error": "audio path is required"}).to_string());
    }
    let locale = args
        .get("locale")
        .and_then(Value::as_str)
        .unwrap_or("zh-Hans")
        .to_string();
    match crate::speech::transcribe(&paths, &audio, Some(&locale)) {
        Ok(text) => Ok(json!({"ok": true, "text": text, "locale": locale}).to_string()),
        Err(err) => Ok(json!({
            "ok": false,
            "error": err.to_string(),
            "hint": "STT 需要带 bundle 身份的应用授权（macOS 语音识别是 TCC 敏感权限，裸脚本无法自动授权）。CLI 下建议用 macOS 系统听写（快捷键 fn 双击或 系统设置 → 键盘 → 听写）。"
        })
        .to_string()),
    }
}
