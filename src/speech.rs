//! 语音能力（本地、零 API 成本）：
//! - TTS：macOS 自带 `say` 命令直接播放/生成音频文件（零依赖）
//! - STT：speech-tool.swift（SFSpeechRecognizer 本地离线识别）
//!
//! 工具：`speak`（读一段文字）、`listen_audio`（识别音频文件）
//! CLI：`gqy tts "文字"`、`gqy stt 音频文件`

use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use std::process::Command;

/// 顾清影克隆音色朗读（Qwen3-TTS 本地服务，见 scripts/tts-server.py）：
/// text → 8091 服务合成 → afplay 播放（终端直接播，不开 App）。
/// 服务未启动时返回错误（可回退到 `speak`）。
pub fn speak_clone(text: &str, tts_url: Option<&str>) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        bail!("text is required");
    }
    let base = tts_url.unwrap_or("http://127.0.0.1:8091");
    // URL 编码（中文等非 ASCII → %XX）
    let mut encoded = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            encoded.push(c);
        } else if c == ' ' {
            encoded.push('+');
        } else {
            for b in c.to_string().as_bytes() {
                encoded.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let url = format!("{base}/tts?text={encoded}");
    let resp = reqwest::blocking::get(&url)
        .with_context(|| format!("TTS 服务不可达（{base}）——先启动：venv/bin/python scripts/tts-server.py") )?;
    if !resp.status().is_success() {
        bail!("TTS 服务返回 HTTP {}", resp.status());
    }
    let bytes = resp.bytes().context("读取 TTS 音频失败")?;
    if bytes.len() < 100 {
        bail!("TTS 返回内容过短，可能合成失败");
    }
    let tmp = std::env::temp_dir().join(format!("gqy-tts-{}.wav", std::process::id()));
    std::fs::write(&tmp, &bytes)?;
    let status = Command::new("afplay")
        .arg(&tmp)
        .status()
        .with_context(|| "failed to run afplay")?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        bail!("afplay exited with status {status}");
    }
    Ok(())
}

/// 文字转语音：默认直接播放，可指定输出文件（.aiff/.m4a）。
/// 用 macOS 内置 `say`，零依赖、零 API 成本。
pub fn speak(text: &str, voice: Option<&str>, output: Option<&str>) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        bail!("text is required");
    }
    let mut command = Command::new("say");
    if let Some(voice) = voice.filter(|v| !v.trim().is_empty()) {
        command.arg("-v").arg(voice);
    }
    if let Some(output) = output {
        command.arg("-o").arg(output);
    }
    let status = command
        .arg(text)
        .status()
        .with_context(|| "failed to run `say`; TTS requires macOS")?;
    if !status.success() {
        bail!("say exited with status {status}");
    }
    Ok(())
}

/// 列出可用的系统语音（`say -v '?'` 解析）。
pub fn list_voices() -> Result<Vec<String>> {
    let output = Command::new("say")
        .args(["-v", "?"])
        .output()
        .with_context(|| "failed to list voices")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let voices = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    Ok(voices)
}

/// 语音转文字：把本地音频文件交给 speech-tool.swift 识别（离线）。
pub fn transcribe(paths: &GqyPaths, audio_path: &str, locale: Option<&str>) -> Result<String> {
    let tool = speech_tool_path(paths);
    if !tool.is_file() {
        bail!("speech-tool.swift not found at {}", tool.display());
    }
    let locale = locale.unwrap_or("zh-Hans");
    let output = Command::new("swift")
        .arg(&tool)
        .arg(audio_path)
        .arg(locale)
        .output()
        .with_context(|| "failed to run speech-tool.swift (requires macOS + swift)")?;
    if !output.status.success() {
        bail!(
            "speech-tool failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("speech-tool returned invalid JSON: {stdout}"))?;
    if parsed.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let error = parsed
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error");
        bail!("speech recognition failed: {error}");
    }
    parsed
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("speech-tool returned no text"))
}

/// 定位 speech-tool.swift：brew 装 = share/gqy/scripts；源码 = <repo>/src/scripts。
fn speech_tool_path(paths: &GqyPaths) -> std::path::PathBuf {
    let candidates = [
        paths.share_dir.join("scripts/speech-tool.swift"),
        paths.share_dir.join("src/scripts/speech-tool.swift"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| paths.share_dir.join("scripts/speech-tool.swift"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 依赖 macOS 自带 `say`；Linux 上无此命令，跳过
    #[cfg(target_os = "macos")]
    #[test]
    fn tts_generates_audio_file() {
        let out = std::env::temp_dir().join(format!("gqy-tts-test-{}.aiff", std::process::id()));
        let _ = std::fs::remove_file(&out);
        speak("test", None, Some(out.to_str().unwrap())).unwrap();
        assert!(out.is_file(), "say should produce an audio file");
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        assert!(size > 1000, "audio file should have content, got {size} bytes");
        let _ = std::fs::remove_file(&out);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lists_system_voices() {
        let voices = list_voices().unwrap();
        assert!(!voices.is_empty(), "macOS should have voices");
    }

    #[test]
    fn rejects_empty_text() {
        assert!(speak("", None, None).is_err());
    }
}
