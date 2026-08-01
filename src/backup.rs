use crate::paths::MiyuPaths;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const SETTINGS_VERSION: u32 = 1;
const SNAPSHOT_DIRS: [&str; 4] = ["config", "data", "state", "pictures"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSettings {
    pub version: u32,
    pub remote: String,
    pub branch: String,
    pub git_name: String,
    pub git_email: String,
    pub auto_push: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BackupInitOptions {
    pub remote: String,
    pub branch: String,
    pub git_name: String,
    pub git_email: String,
    pub auto_push: bool,
    pub ssh_key: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BackupOutcome {
    pub committed: bool,
    pub pushed: bool,
    pub commit: Option<String>,
}

pub fn init(paths: &MiyuPaths, options: BackupInitOptions) -> Result<()> {
    let home = required_isolated_home(paths)?;
    validate_init_options(&home, &options)?;

    let backup_dir = home.join("backup");
    let repo = backup_dir.join("repository");
    std::fs::create_dir_all(&repo)?;
    std::fs::create_dir_all(backup_dir.join("no-hooks"))?;
    ensure_isolated_global_config(&backup_dir)?;

    let settings = BackupSettings {
        version: SETTINGS_VERSION,
        remote: options.remote.trim().to_string(),
        branch: options.branch.trim().to_string(),
        git_name: options.git_name.trim().to_string(),
        git_email: options.git_email.trim().to_string(),
        auto_push: options.auto_push,
        ssh_key: options.ssh_key,
    };
    write_settings(&backup_dir, &settings)?;

    if !repo.join(".git").exists() {
        run_git(
            &backup_dir,
            &settings,
            ["init", "-b", settings.branch.as_str()],
        )?;
    }
    run_git(
        &backup_dir,
        &settings,
        ["config", "--local", "user.name", settings.git_name.as_str()],
    )?;
    run_git(
        &backup_dir,
        &settings,
        [
            "config",
            "--local",
            "user.email",
            settings.git_email.as_str(),
        ],
    )?;
    let hooks_path = backup_dir.join("no-hooks");
    let hooks_path = hooks_path.to_string_lossy().to_string();
    run_git(
        &backup_dir,
        &settings,
        ["config", "--local", "core.hooksPath", hooks_path.as_str()],
    )?;

    let has_origin = git_output(&backup_dir, &settings, ["remote", "get-url", "origin"]).is_ok();
    if has_origin {
        run_git(
            &backup_dir,
            &settings,
            ["remote", "set-url", "origin", settings.remote.as_str()],
        )?;
    } else {
        run_git(
            &backup_dir,
            &settings,
            ["remote", "add", "origin", settings.remote.as_str()],
        )?;
    }

    write_repository_files(&repo)?;
    snapshot(paths, &repo)?;
    Ok(())
}

pub fn backup_now(paths: &MiyuPaths, push: bool) -> Result<BackupOutcome> {
    let home = required_isolated_home(paths)?;
    let backup_dir = home.join("backup");
    let settings = load_settings(&backup_dir)?;
    let repo = backup_dir.join("repository");
    if !repo.join(".git").is_dir() {
        bail!("backup repository is not initialized; run `miyu backup init` first");
    }

    snapshot(paths, &repo)?;
    run_git(&backup_dir, &settings, ["add", "--all"])?;
    let dirty = !git_output(&backup_dir, &settings, ["status", "--porcelain"])?
        .trim()
        .is_empty();
    if dirty {
        let message = format!("GQY snapshot {}", Utc::now().to_rfc3339());
        run_git(&backup_dir, &settings, ["commit", "-m", message.as_str()])?;
    }

    let commit = git_output(&backup_dir, &settings, ["rev-parse", "--short", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let pushed = push && commit.is_some();
    if pushed {
        run_git(
            &backup_dir,
            &settings,
            ["push", "--set-upstream", "origin", settings.branch.as_str()],
        )?;
    }

    Ok(BackupOutcome {
        committed: dirty,
        pushed,
        commit,
    })
}

pub fn maybe_auto_backup(paths: &MiyuPaths) -> Result<Option<BackupOutcome>> {
    let Some(home) = paths.isolated_home()? else {
        return Ok(None);
    };
    let backup_dir = home.join("backup");
    if !settings_path(&backup_dir).is_file() {
        return Ok(None);
    }
    let settings = load_settings(&backup_dir)?;
    if !settings.auto_push {
        return Ok(None);
    }
    backup_now(paths, true).map(Some)
}

pub fn status(paths: &MiyuPaths) -> Result<String> {
    let home = required_isolated_home(paths)?;
    let backup_dir = home.join("backup");
    let settings = load_settings(&backup_dir)?;
    let repo = backup_dir.join("repository");
    let git_status = git_output(&backup_dir, &settings, ["status", "--short", "--branch"])?;
    Ok(format!(
        "home: {}\nrepository: {}\nremote: {}\nbranch: {}\nauto push: {}\n{}",
        home.display(),
        repo.display(),
        settings.remote,
        settings.branch,
        settings.auto_push,
        git_status.trim_end()
    ))
}

fn required_isolated_home(paths: &MiyuPaths) -> Result<PathBuf> {
    paths
        .isolated_home()?
        .context("Git backup requires an isolated GQY_HOME; set it to an absolute directory first")
}

fn validate_init_options(home: &Path, options: &BackupInitOptions) -> Result<()> {
    for (label, value) in [
        ("remote", options.remote.as_str()),
        ("branch", options.branch.as_str()),
        ("git name", options.git_name.as_str()),
        ("git email", options.git_email.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("{label} must not be empty");
        }
    }
    if options.branch.contains(char::is_whitespace) {
        bail!("branch must not contain whitespace");
    }
    if options.branch.starts_with('-') || options.remote.trim_start().starts_with('-') {
        bail!("branch and remote must not start with '-'");
    }
    if options
        .remote
        .chars()
        .any(|character| character.is_control())
    {
        bail!("remote must not contain control characters");
    }
    if http_remote_contains_credentials(&options.remote) {
        bail!("remote URLs must not contain credentials; use an isolated SSH key instead");
    }
    if is_ssh_remote(&options.remote) && options.ssh_key.is_none() {
        bail!("SSH remotes require --ssh-key so backup authentication stays isolated");
    }
    if let Some(key) = &options.ssh_key {
        if !key.is_absolute() {
            bail!("--ssh-key must be an absolute path");
        }
        let secrets = std::fs::canonicalize(home.join("secrets"))
            .context("GQY_HOME/secrets must exist before configuring an SSH key")?;
        let real_key = std::fs::canonicalize(key)
            .with_context(|| format!("SSH key does not exist: {}", key.display()))?;
        if !real_key.starts_with(&secrets) {
            bail!("--ssh-key must live below GQY_HOME/secrets");
        }
        if !real_key.is_file() {
            bail!("SSH key does not exist: {}", key.display());
        }
    }
    Ok(())
}

fn is_ssh_remote(remote: &str) -> bool {
    remote.trim_start().starts_with("ssh://") || remote.contains('@') && remote.contains(':')
}

fn http_remote_contains_credentials(remote: &str) -> bool {
    let Some(authority) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
    else {
        return false;
    };
    authority.contains('@')
}

fn settings_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join("settings.json")
}

fn write_settings(backup_dir: &Path, settings: &BackupSettings) -> Result<()> {
    std::fs::create_dir_all(backup_dir)?;
    let raw = serde_json::to_string_pretty(settings)?;
    std::fs::write(settings_path(backup_dir), format!("{raw}\n"))?;
    Ok(())
}

fn load_settings(backup_dir: &Path) -> Result<BackupSettings> {
    let path = settings_path(backup_dir);
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read backup settings: {}", path.display()))?;
    let settings: BackupSettings = serde_json::from_str(&raw)
        .with_context(|| format!("invalid backup settings: {}", path.display()))?;
    if settings.version != SETTINGS_VERSION {
        bail!("unsupported backup settings version: {}", settings.version);
    }
    Ok(settings)
}

fn ensure_isolated_global_config(backup_dir: &Path) -> Result<()> {
    let path = backup_dir.join("gitconfig");
    if !path.exists() {
        std::fs::write(path, "# GQY isolated Git configuration\n")?;
    }
    Ok(())
}

fn write_repository_files(repo: &Path) -> Result<()> {
    std::fs::write(
        repo.join(".gitignore"),
        "*.db-wal\n*.db-shm\n*.log\n.DS_Store\n",
    )?;
    std::fs::write(
        repo.join("README.md"),
        "# GQY private state snapshot\n\nThis repository contains a consistent, redacted snapshot of GQY's portable state. API keys, Git credentials, caches, and live SQLite WAL files are intentionally excluded.\n",
    )?;
    Ok(())
}

fn snapshot(paths: &MiyuPaths, repo: &Path) -> Result<()> {
    for name in SNAPSHOT_DIRS {
        let destination = repo.join(name);
        if destination.exists() {
            std::fs::remove_dir_all(&destination)
                .with_context(|| format!("failed to refresh {}", destination.display()))?;
        }
    }

    copy_tree(&paths.config_dir, &repo.join("config"), true)?;
    copy_tree(&paths.data_dir, &repo.join("data"), false)?;
    copy_tree(&paths.state_dir, &repo.join("state"), false)?;
    copy_tree(&paths.pictures_dir, &repo.join("pictures"), false)?;
    write_redacted_config(&paths.config_file, &repo.join("config/config.jsonc"))?;

    let manifest = json!({
        "format": 1,
        "generated_by": "GQY isolated backup",
        "contains": ["redacted configuration", "personas and skills", "memory", "conversation state", "pictures"],
        "excludes": ["API keys and secrets", "Git credentials", "cache and logs", "SQLite WAL/SHM files"]
    });
    std::fs::write(
        repo.join("snapshot.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, skip_live_config: bool) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let name = entry.file_name();
        if skip_live_config && name == OsStr::new("config.jsonc") {
            continue;
        }
        if is_sqlite_sidecar(&source_path)
            || is_obvious_secret_file(&source_path)
            || file_type.is_symlink()
        {
            continue;
        }
        let destination_path = destination.join(&name);
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path, false)?;
        } else if file_type.is_file() && source_path.extension() == Some(OsStr::new("db")) {
            snapshot_sqlite(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn is_sqlite_sidecar(path: &Path) -> bool {
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    name.ends_with(".db-wal") || name.ends_with(".db-shm")
}

fn is_obvious_secret_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(name.as_str(), "id_rsa" | "id_ed25519" | "credentials.json")
        || matches!(extension.as_str(), "pem" | "key" | "p12" | "pfx")
}

fn snapshot_sqlite(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if destination.exists() {
        std::fs::remove_file(destination)?;
    }
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open SQLite database {}", source.display()))?;
    connection
        .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])
        .with_context(|| format!("failed to snapshot SQLite database {}", source.display()))?;
    Ok(())
}

fn write_redacted_config(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_file() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(source)?;
    let stripped = json_comments::StripComments::new(raw.as_bytes());
    let mut value: Value = serde_json::from_reader(stripped)?;
    redact_secrets(&mut value);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        destination,
        format!("{}\n", serde_json::to_string_pretty(&value)?),
    )?;
    Ok(())
}

fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if is_secret_key(key) {
                    *value = match value {
                        Value::Array(_) => Value::Array(Vec::new()),
                        Value::Object(_) => Value::Object(serde_json::Map::new()),
                        Value::String(_) => Value::String(String::new()),
                        _ => Value::Null,
                    };
                } else {
                    redact_secrets(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_secrets(value);
            }
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '.'], "_");
    let token_value = (normalized == "token"
        || normalized == "tokens"
        || normalized.ends_with("_token")
        || normalized.ends_with("_tokens"))
        && !normalized.contains("max_token")
        && !normalized.contains("token_usage")
        && !normalized.contains("token_count")
        && !normalized.contains("token_limit")
        && !normalized.contains("token_budget");
    normalized.contains("api_key")
        || normalized.contains("apikey")
        || token_value
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("credential")
        || normalized == "authorization"
        || normalized.ends_with("_auth")
}

fn run_git<I, S>(backup_dir: &Path, settings: &BackupSettings, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_output(backup_dir, settings, args).map(|_| ())
}

fn git_output<I, S>(backup_dir: &Path, settings: &BackupSettings, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let repo = backup_dir.join("repository");
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", backup_dir.join("gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND");

    if let Some(key) = &settings.ssh_key {
        let known_hosts = backup_dir
            .parent()
            .unwrap_or(backup_dir)
            .join("secrets/ssh/known_hosts");
        if let Some(parent) = known_hosts.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let ssh = format!(
            "ssh -i {} -o IdentitiesOnly=yes -o UserKnownHostsFile={} -o StrictHostKeyChecking=accept-new",
            shell_quote(key),
            shell_quote(&known_hosts)
        );
        command.env("GIT_SSH_COMMAND", ssh);
    }

    let output = command
        .output()
        .with_context(|| "failed to start isolated git command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "isolated git command failed ({}): {}{}",
            output.status,
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stdout.trim())
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursively_redacts_known_secret_names() {
        let mut value = json!({
            "api_key": "abc",
            "nested": {"OPENAI_API_KEY": "def", "safe": "kept"},
            "tokens": ["one", "two"],
            "anthropic_max_tokens": 4096,
            "show_token_usage": true,
            "model_context_window": {"model": 1000}
        });
        redact_secrets(&mut value);

        assert_eq!(value["api_key"], "");
        assert_eq!(value["nested"]["OPENAI_API_KEY"], "");
        assert_eq!(value["nested"]["safe"], "kept");
        assert_eq!(value["tokens"], json!([]));
        assert_eq!(value["anthropic_max_tokens"], 4096);
        assert_eq!(value["show_token_usage"], true);
        assert_eq!(value["model_context_window"]["model"], 1000);
    }

    #[test]
    fn redacted_default_config_remains_loadable() {
        let mut value = serde_json::to_value(crate::config::AppConfig::default()).unwrap();
        redact_secrets(&mut value);
        serde_json::from_value::<crate::config::AppConfig>(value).unwrap();
    }

    #[test]
    fn detects_ssh_remote_forms() {
        assert!(is_ssh_remote("git@github.com:owner/private.git"));
        assert!(is_ssh_remote("ssh://git@github.com/owner/private.git"));
        assert!(!is_ssh_remote("https://github.com/owner/private.git"));
    }

    #[test]
    fn detects_credentials_in_http_remote() {
        assert!(http_remote_contains_credentials(
            "https://token@github.com/owner/private.git"
        ));
        assert!(!http_remote_contains_credentials(
            "https://github.com/owner/private.git"
        ));
    }

    #[test]
    fn recognizes_obvious_secret_files() {
        assert!(is_obvious_secret_file(Path::new(".env.local")));
        assert!(is_obvious_secret_file(Path::new("deploy-key.pem")));
        assert!(is_obvious_secret_file(Path::new("id_ed25519")));
        assert!(!is_obvious_secret_file(Path::new("persona.md")));
    }
}
