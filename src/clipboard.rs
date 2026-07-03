use anyhow::Result;
use base64::Engine;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CLIPBOARD_IMAGE_BYTES: usize = 10 * 1024 * 1024;

pub struct ClipboardImage {
    pub mime: String,
    pub data: Vec<u8>,
}

impl ClipboardImage {
    pub fn data_url(&self) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&self.data);
        format!("data:{};base64,{}", self.mime, encoded)
    }

    pub fn write_temp_file(&self, cache_dir: &std::path::Path, index: usize) -> Result<PathBuf> {
        let dir = cache_dir.join("clipboard_images");
        std::fs::create_dir_all(&dir)?;
        let ext = self
            .mime
            .split('/')
            .nth(1)
            .filter(|e| !e.is_empty())
            .unwrap_or("png");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("img_{ts}_{index}.{ext}"));
        std::fs::write(&path, &self.data)?;
        Ok(path)
    }
}

pub fn read_clipboard_image() -> Result<Option<ClipboardImage>> {
    if let Some(img) = try_command("wl-paste", &["-t", "image/png"], "image/png")? {
        return Ok(Some(img));
    }
    if let Some(img) = try_command(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
        "image/png",
    )? {
        return Ok(Some(img));
    }
    if let Some(img) = try_command(
        "xsel",
        &["--clipboard", "--output"],
        "image/png",
    )? {
        return Ok(Some(img));
    }
    Ok(None)
}

fn try_command(cmd: &str, args: &[&str], mime: &str) -> Result<Option<ClipboardImage>> {
    let output = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => {
            if output.stdout.len() > MAX_CLIPBOARD_IMAGE_BYTES {
                return Ok(None);
            }
            Ok(Some(ClipboardImage {
                mime: mime.to_string(),
                data: output.stdout,
            }))
        }
        _ => Ok(None),
    }
}
