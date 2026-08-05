//! Shared helpers for the WebUI.
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use std::collections::BTreeSet;
use std::io::{self, IsTerminal};
use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

pub(crate) const JSON_BODY_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const MAX_CONTENT_CHARS: usize = 20_000;
pub(crate) const MAX_PROMPT_DOCUMENT_CHARS: usize = 200_000;
pub(crate) const MAX_PROMPT_DOCUMENTS: usize = 128;
pub(crate) const MAX_SECRET_CHARS: usize = 100_000;
pub(crate) const EVENT_CAPACITY: usize = 4096;
pub(crate) const AUTH_COOKIE: &str = "gqy_session";
pub(crate) const LOGIN_WINDOW: Duration = Duration::from_secs(60);
pub(crate) const LOGIN_ATTEMPT_LIMIT: u8 = 5;

/// Recover from a poisoned mutex instead of panicking request paths.
pub(crate) fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn web_access_urls(port: u16, include_lan: bool) -> Vec<String> {
    let mut addresses = BTreeSet::new();
    addresses.insert(Ipv4Addr::LOCALHOST);
    if include_lan {
        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            for interface in interfaces {
                if let if_addrs::IfAddr::V4(address) = interface.addr {
                    if !address.ip.is_unspecified() {
                        addresses.insert(address.ip);
                    }
                }
            }
        }
    }
    addresses
        .into_iter()
        .map(|address| format!("http://{address}:{port}"))
        .collect()
}
pub(crate) fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub(crate) fn random_id(prefix: &str, bytes: usize) -> String {
    format!("{prefix}_{}", random_token(bytes))
}

pub(crate) fn safe_error_message(error: impl std::fmt::Display) -> String {
    let message = error
        .to_string()
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .take(1000)
        .collect::<String>();
    if message.trim().is_empty() {
        "operation failed".to_string()
    } else {
        message
    }
}

pub(crate) async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    if let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) fn open_browser(_url: &str) {}

