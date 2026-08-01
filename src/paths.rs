use crate::i18n::text as t;
use anyhow::{Context, Result};
use directories::{BaseDirs, UserDirs};
use std::ffi::OsString;
use std::path::PathBuf;

pub const GQY_HOME_ENV: &str = "GQY_HOME";

#[derive(Debug, Clone)]
pub struct GqyPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub skills_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub state_dir: PathBuf,
    pub pictures_dir: PathBuf,
    pub fish_hook_file: PathBuf,
    pub bash_hook_file: PathBuf,
    pub zsh_hook_file: PathBuf,
    pub scripts_dir: PathBuf,
    pub system_scripts_dir: PathBuf,
}

impl GqyPaths {
    pub fn new() -> Result<Self> {
        if let Some(home) = isolated_home_from_env()? {
            return Ok(Self::from_isolated_home(home));
        }

        let base = BaseDirs::new().context(t(
            "could not determine XDG base directories",
            "无法确定 XDG 基础目录",
        ))?;
        // macOS 上与文档/菜单栏约定保持一致（~/Library/Application Support/GQY）
        let app_dir = if cfg!(target_os = "macos") { "GQY" } else { "gqy" };
        let config_dir = base.config_dir().join(app_dir);
        let data_dir = base.data_dir().join(app_dir);
        let cache_dir = base.cache_dir().join(app_dir);
        let state_dir = base
            .state_dir()
            .unwrap_or_else(|| base.data_dir())
            .join(app_dir);
        let pictures_dir = std::env::var_os("XDG_PICTURES_DIR")
            .map(PathBuf::from)
            .or_else(|| UserDirs::new().and_then(|dirs| dirs.picture_dir().map(PathBuf::from)))
            .unwrap_or_else(|| base.home_dir().join("Pictures"))
            .join(app_dir);
        let fish_hook_file = base.config_dir().join("fish/conf.d/gqy.fish");
        let bash_hook_file = config_dir.join("shell/bash-hook.sh");
        let zsh_hook_file = config_dir.join("shell/zsh-hook.zsh");
        let scripts_dir = config_dir.join("scripts");
        let system_scripts_dir = PathBuf::from("/usr/share/gqy/scripts");

        Ok(Self {
            config_file: config_dir.join("config.jsonc"),
            skills_dir: config_dir.join("skills"),
            config_dir,
            data_dir,
            cache_dir,
            state_dir,
            pictures_dir,
            fish_hook_file,
            bash_hook_file,
            zsh_hook_file,
            scripts_dir,
            system_scripts_dir,
        })
    }

    fn from_isolated_home(home: PathBuf) -> Self {
        let config_dir = home.join("config");
        let data_dir = home.join("data");
        let cache_dir = home.join("cache");
        let state_dir = home.join("state");

        Self {
            config_file: config_dir.join("config.jsonc"),
            skills_dir: config_dir.join("skills"),
            fish_hook_file: config_dir.join("shell/gqy.fish"),
            bash_hook_file: config_dir.join("shell/bash-hook.sh"),
            zsh_hook_file: config_dir.join("shell/zsh-hook.zsh"),
            scripts_dir: config_dir.join("scripts"),
            system_scripts_dir: PathBuf::from("/usr/share/gqy/scripts"),
            pictures_dir: home.join("pictures"),
            config_dir,
            data_dir,
            cache_dir,
            state_dir,
        }
    }

    pub fn isolated_home(&self) -> Result<Option<PathBuf>> {
        isolated_home_from_env()
    }

    pub fn create_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.skills_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::create_dir_all(&self.pictures_dir)?;
        std::fs::create_dir_all(&self.scripts_dir)?;
        Ok(())
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.cache_dir.join("logs")
    }

    pub fn print(&self) {
        if let Ok(Some(home)) = self.isolated_home() {
            println!("{}: {}", t("isolated home", "独立主目录"), home.display());
        }
        println!(
            "{}: {}",
            t("config directory", "配置目录"),
            self.config_dir.display()
        );
        println!(
            "{}: {}",
            t("config file", "配置文件"),
            self.config_file.display()
        );
        println!(
            "{}: {}",
            t("skills directory", "skills 目录"),
            self.skills_dir.display()
        );
        println!(
            "{}: {}",
            t("data directory", "数据目录"),
            self.data_dir.display()
        );
        println!(
            "{}: {}",
            t("cache directory", "缓存目录"),
            self.cache_dir.display()
        );
        println!(
            "{}: {}",
            t("state directory", "状态目录"),
            self.state_dir.display()
        );
        println!(
            "{}: {}",
            t("log directory", "日志目录"),
            self.logs_dir().display()
        );
        println!(
            "{}: {}",
            t("pictures directory", "图片目录"),
            self.pictures_dir.display()
        );
        println!(
            "{}: {}",
            t("fish hook file", "fish hook 文件"),
            self.fish_hook_file.display()
        );
        println!(
            "{}: {}",
            t("bash hook file", "bash hook 文件"),
            self.bash_hook_file.display()
        );
        println!(
            "{}: {}",
            t("zsh hook file", "zsh hook 文件"),
            self.zsh_hook_file.display()
        );
        println!(
            "{}: {}",
            t("scripts directory", "scripts 目录"),
            self.scripts_dir.display()
        );
        println!(
            "{}: {}",
            t("system scripts directory", "系统 scripts 目录"),
            self.system_scripts_dir.display()
        );
    }
}

fn isolated_home_from_env() -> Result<Option<PathBuf>> {
    let Some(raw) = std::env::var_os(GQY_HOME_ENV) else {
        return Ok(None);
    };
    validate_isolated_home(raw).map(Some)
}

fn validate_isolated_home(raw: OsString) -> Result<PathBuf> {
    let home = PathBuf::from(raw);
    if home.as_os_str().is_empty() {
        anyhow::bail!("{GQY_HOME_ENV} must not be empty");
    }
    if !home.is_absolute() {
        anyhow::bail!("{GQY_HOME_ENV} must be an absolute path");
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_layout_stays_under_one_home() {
        let home = PathBuf::from("/tmp/gqy-test-home");
        let paths = GqyPaths::from_isolated_home(home.clone());

        for path in [
            &paths.config_dir,
            &paths.data_dir,
            &paths.cache_dir,
            &paths.state_dir,
            &paths.pictures_dir,
            &paths.zsh_hook_file,
        ] {
            assert!(
                path.starts_with(&home),
                "{} escaped the home",
                path.display()
            );
        }
    }

    #[test]
    fn isolated_home_must_be_absolute() {
        let error = validate_isolated_home(OsString::from("relative/home")).unwrap_err();
        assert!(error.to_string().contains("absolute"));
    }
}
