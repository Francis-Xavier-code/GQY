use crate::i18n::text as t;
use anyhow::{Context, Result};
use directories::{BaseDirs, UserDirs};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const GQY_HOME_ENV: &str = "GQY_HOME";
pub const GQY_SHARE_DIR_ENV: &str = "GQY_SHARE_DIR";

/// 打包进来的只读资源（scripts/memes/kb）统一放在一个 share 基目录下。
/// 三种入口一致：
/// - brew CLI：`$(brew --prefix)/share/gqy/{scripts,memes,kb}`
/// - 菜单栏 App：`顾清影.app/Contents/Resources/share/gqy/{scripts,memes,kb}`
/// - 源码构建：仓库内 `src/scripts`、`src/memes`、`kb`（share 基目录 = 仓库根）
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
    /// 只读共享资源基目录（内置脚本/表情/知识库源）。
    pub share_dir: PathBuf,
    /// 随包知识库源目录（`gqy kb add <这里>` 一键导入）。
    pub kb_dir: PathBuf,
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
        // 统一使用小写 gqy：升级只替换二进制，目录路径不变即数据不变
        let config_dir = base.config_dir().join("gqy");
        let data_dir = base.data_dir().join("gqy");
        let cache_dir = base.cache_dir().join("gqy");
        let state_dir = base
            .state_dir()
            .unwrap_or_else(|| base.data_dir())
            .join("gqy");
        let pictures_dir = std::env::var_os("XDG_PICTURES_DIR")
            .map(PathBuf::from)
            .or_else(|| UserDirs::new().and_then(|dirs| dirs.picture_dir().map(PathBuf::from)))
            .unwrap_or_else(|| base.home_dir().join("Pictures"))
            .join("gqy");
        let fish_hook_file = base.config_dir().join("fish/conf.d/gqy.fish");
        let bash_hook_file = config_dir.join("shell/bash-hook.sh");
        let zsh_hook_file = config_dir.join("shell/zsh-hook.zsh");
        let scripts_dir = config_dir.join("scripts");
        let share_dir = resolve_share_base();
        let system_scripts_dir = resolve_system_scripts_dir(&share_dir);
        let kb_dir = share_dir.join("kb");

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
            share_dir,
            kb_dir,
        })
    }

    fn from_isolated_home(home: PathBuf) -> Self {
        let config_dir = home.join("config");
        let data_dir = home.join("data");
        let cache_dir = home.join("cache");
        let state_dir = home.join("state");
        let share_dir = resolve_share_base();
        let system_scripts_dir = resolve_system_scripts_dir(&share_dir);

        Self {
            config_file: config_dir.join("config.jsonc"),
            skills_dir: config_dir.join("skills"),
            fish_hook_file: config_dir.join("shell/gqy.fish"),
            bash_hook_file: config_dir.join("shell/bash-hook.sh"),
            zsh_hook_file: config_dir.join("shell/zsh-hook.zsh"),
            scripts_dir: config_dir.join("scripts"),
            system_scripts_dir,
            share_dir: share_dir.clone(),
            kb_dir: share_dir.join("kb"),
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
        println!(
            "{}: {}",
            t("share directory", "共享资源目录"),
            self.share_dir.display()
        );
        println!(
            "{}: {}",
            t("built-in knowledge base", "内置知识库目录"),
            self.kb_dir.display()
        );
    }
}

/// 只读共享资源基目录解析顺序：
/// 1. `GQY_SHARE_DIR` 环境变量（测试/特殊部署）
/// 2. 从可执行文件所在目录逐级向上找 `<祖先>/share/gqy`（brew cellar、app bundle、prefix）
/// 3. 源码树：仓库根（其下为 src/scripts、src/memes、kb）
/// 4. Linux 兜底 `/usr/share/gqy`
pub fn resolve_share_base() -> PathBuf {
    resolve_share_base_from(
        std::env::var_os(GQY_SHARE_DIR_ENV).map(PathBuf::from),
        std::env::current_exe().ok().as_deref(),
        &crate::tools::path_guard::project_dir(),
    )
}

fn resolve_share_base_from(
    env_override: Option<PathBuf>,
    exe: Option<&Path>,
    project_dir: &Path,
) -> PathBuf {
    if let Some(dir) = env_override {
        return dir;
    }
    if let Some(exe) = exe {
        let mut dir = exe.parent();
        for _ in 0..8 {
            let Some(d) = dir else { break };
            let candidate = d.join("share/gqy");
            if share_base_plausible(&candidate) {
                return candidate;
            }
            dir = d.parent();
        }
    }
    if share_base_plausible(project_dir) {
        return project_dir.to_path_buf();
    }
    PathBuf::from("/usr/share/gqy")
}

/// share 基目录里是否确实放着资源（避免误判 home 目录等）。
fn share_base_plausible(base: &Path) -> bool {
    [
        base.join("scripts"),
        base.join("src/scripts"),
        base.join("memes"),
        base.join("src/memes"),
        base.join("kb"),
    ]
    .iter()
    .any(|path| path.is_dir())
}

/// 系统 scripts 目录：share 布局 `scripts/` 或源码树 `src/scripts`。
fn resolve_system_scripts_dir(share_base: &Path) -> PathBuf {
    let primary = share_base.join("scripts");
    if primary.is_dir() {
        return primary;
    }
    let source = share_base.join("src/scripts");
    if source.is_dir() {
        return source;
    }
    primary
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

    #[test]
    fn share_base_prefers_env_override() {
        let temp = std::env::temp_dir().join("gqy-share-test-env");
        std::fs::create_dir_all(temp.join("scripts")).unwrap();
        let exe = std::env::temp_dir().join("cellar/gqy/0.4.2/bin/gqy");
        let found = resolve_share_base_from(
            Some(temp.clone()),
            Some(&exe),
            Path::new("/nonexistent/project"),
        );
        assert_eq!(found, temp);
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn share_base_walks_up_from_brew_cellar_exe() {
        let temp = std::env::temp_dir().join("gqy-share-test-cellar");
        std::fs::create_dir_all(temp.join("share/gqy/scripts")).unwrap();
        std::fs::create_dir_all(temp.join("Cellar/gqy/0.4.2/bin")).unwrap();
        let exe = temp.join("Cellar/gqy/0.4.2/bin/gqy");
        let found = resolve_share_base_from(None, Some(&exe), Path::new("/nonexistent/project"));
        assert_eq!(found, temp.join("share/gqy"));
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn share_base_finds_app_bundle_resources() {
        let temp = std::env::temp_dir().join("gqy-share-test-bundle");
        std::fs::create_dir_all(temp.join("Resources/share/gqy/memes/gqy/images")).unwrap();
        let exe = temp.join("Resources/gqy");
        let found = resolve_share_base_from(None, Some(&exe), Path::new("/nonexistent/project"));
        assert_eq!(found, temp.join("Resources/share/gqy"));
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn share_base_falls_back_to_source_tree() {
        let temp = std::env::temp_dir().join("gqy-share-test-src");
        std::fs::create_dir_all(temp.join("src/scripts")).unwrap();
        let exe = temp.join("target/release/gqy");
        let found = resolve_share_base_from(None, Some(&exe), &temp);
        assert_eq!(found, temp);
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn share_base_defaults_to_linux_share() {
        let found = resolve_share_base_from(
            None,
            Some(Path::new("/opt/bin/gqy")),
            Path::new("/nonexistent/project"),
        );
        assert_eq!(found, PathBuf::from("/usr/share/gqy"));
    }

    #[test]
    fn system_scripts_dir_matches_share_or_source_layout() {
        let temp = std::env::temp_dir().join("gqy-share-test-layout");
        std::fs::create_dir_all(temp.join("share/gqy/scripts")).unwrap();
        assert_eq!(
            resolve_system_scripts_dir(&temp.join("share/gqy")),
            temp.join("share/gqy/scripts")
        );
        std::fs::create_dir_all(temp.join("src/scripts")).unwrap();
        assert_eq!(
            resolve_system_scripts_dir(&temp),
            temp.join("src/scripts")
        );
        std::fs::remove_dir_all(&temp).ok();
    }
}
