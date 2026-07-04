use anyhow::Result;
use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CLIPBOARD_IMAGE_BYTES: usize = 10 * 1024 * 1024;

pub struct ClipboardImage {
    pub mime: String,
    pub data: Vec<u8>,
}

pub enum PastedImage {
    Binary(ClipboardImage),
    Path(String),
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

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

pub enum ClipboardContent {
    None,
    Image(ClipboardImage),
    ImagePath(String),
    TextPath(String),
}

pub fn read_clipboard() -> Result<ClipboardContent> {
    let targets = list_clipboard_targets()?;
    let has_uri_list = targets.iter().any(|t| {
        t == "text/uri-list" || t == "x-special/gnome-copied-files" || t == "application/glfw+clipboard-32678"
    });
    let has_image = targets
        .iter()
        .any(|t| t.starts_with("image/"));
    if has_uri_list || targets.iter().any(|t| t == "text/plain" || t == "TEXT" || t == "STRING" || t == "UTF8_STRING") {
        if let Some(text) = read_clipboard_text()? {
            if has_uri_list || text.starts_with("file://") || text.starts_with('/') {
                if let Some(cp) = parse_clipboard_path(&text) {
                    if cp.is_image {
                        return Ok(ClipboardContent::ImagePath(cp.path));
                    } else {
                        return Ok(ClipboardContent::TextPath(cp.path));
                    }
                }
            }
        }
    }
    if has_image {
        if let Some(img) = read_clipboard_image()? {
            return Ok(ClipboardContent::Image(img));
        }
    }
    Ok(ClipboardContent::None)
}

fn list_clipboard_targets() -> Result<Vec<String>> {
    if let Some(targets) = try_targets_command("wl-paste", &["-l"])? {
        return Ok(targets);
    }
    if let Some(targets) = try_targets_command(
        "xclip",
        &["-selection", "clipboard", "-t", "TARGETS", "-o"],
    )? {
        return Ok(targets);
    }
    Ok(Vec::new())
}

fn try_targets_command(cmd: &str, args: &[&str]) -> Result<Option<Vec<String>>> {
    let output = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            let targets = String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            if targets.is_empty() {
                Ok(None)
            } else {
                Ok(Some(targets))
            }
        }
        _ => Ok(None),
    }
}

pub struct ClipboardPath {
    pub path: String,
    pub is_image: bool,
}

pub fn read_clipboard_text() -> Result<Option<String>> {
    if let Some(text) = try_text_command("wl-paste", &[])? {
        return Ok(Some(text));
    }
    if let Some(text) = try_text_command("xclip", &["-selection", "clipboard", "-o"])? {
        return Ok(Some(text));
    }
    if let Some(text) = try_text_command("xsel", &["--clipboard", "--output"])? {
        return Ok(Some(text));
    }
    Ok(None)
}

fn try_text_command(cmd: &str, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text))
            }
        }
        _ => Ok(None),
    }
}

pub fn parse_clipboard_path(text: &str) -> Option<ClipboardPath> {
    let text = text.trim();
    if text.is_empty() || text.contains('\n') || text.contains('\r') {
        return None;
    }
    let raw = text.strip_prefix("file://").unwrap_or(text);
    let path_str = if raw.starts_with('/') {
        raw.to_string()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
            home.join(rest).display().to_string()
        } else {
            return None;
        }
    } else {
        return None;
    };
    let path = Path::new(&path_str);
    if !path.exists() {
        return None;
    }
    let is_image = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    Some(ClipboardPath {
        path: path_str,
        is_image,
    })
}
