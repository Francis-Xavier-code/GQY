//! `gqy menubar install`：把菜单栏壳（main.m，AppKit，约 300 行）编译成
//! `顾清影.app` 安装到 `~/Applications`，内置当前 gqy 二进制与共享资源，自包含。
//!
//! 交付策略：CLI（formula）为主，菜单栏降级为 CLI 附属——不再单独维护
//! DMG/cask 双轨。菜单栏只做「连接守护进程 + 打开面板 + 退出时统一收尾」。

use crate::paths::GqyPaths;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn install(paths: &GqyPaths) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("gqy menubar 仅支持 macOS");
    }

    // 定位菜单栏源码：brew = share/gqy/macos/GQYMenuBar；源码树 = <repo>/macos/GQYMenuBar
    let menubar_src = [
        paths.share_dir.join("macos/GQYMenuBar"),
        paths.share_dir.join("menubar"),
        paths.share_dir.join("src/../macos/GQYMenuBar"),
    ]
    .into_iter()
    .find(|p| p.join("main.m").is_file())
    .ok_or_else(|| anyhow::anyhow!("找不到菜单栏源码 main.m（预期 share/gqy/menubar 或源码树 macos/GQYMenuBar）"))?;
    let main_m = menubar_src.join("main.m");
    let info_plist = menubar_src.join("Info.plist");
    if !info_plist.is_file() {
        bail!("缺少 Info.plist：{}", info_plist.display());
    }
    let icon_png = [
        paths.share_dir.join("pics/GQY-icon.png"),
        paths.share_dir.join("GQY-icon.png"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .unwrap_or_else(|| paths.share_dir.join("pics/GQY-icon.png"));

    let home_apps = dirs_applications_dir()?;
    let app_dir = home_apps.join("顾清影.app");

    // 清理旧实例：先退出正在运行的菜单栏（无论新旧），并卸载旧 LaunchAgent，
    // 避免「打开面板」跑到旧 App/旧二进制上（升级场景最常见的坑）。
    cleanup_old_instances();

    println!(
        "{}: {} → {}",
        crate::i18n::text("building menu bar app", "编译菜单栏 App"),
        app_dir.display(),
        "…"
    );

    // 用临时目录组装，成功后再原子替换
    let temp = tempfile::tempdir().context("创建临时目录失败")?;
    let contents = temp.path().join("Contents");
    let binary_dir = contents.join("MacOS");
    let resources_dir = contents.join("Resources");
    let share_dir = resources_dir.join("share/gqy");
    std::fs::create_dir_all(&binary_dir)?;
    std::fs::create_dir_all(&resources_dir)?;
    std::fs::create_dir_all(&share_dir)?;

    // 1) 编译菜单栏壳
    let module_cache = temp.path().join("module-cache");
    std::fs::create_dir_all(&module_cache)?;
    let cache_arg = format!("-fmodules-cache-path={}", module_cache.display());
    let output = Command::new("xcrun")
        .arg("clang")
        .args(["-fobjc-arc", "-fmodules"])
        .arg(&cache_arg)
        .args([
            "-framework", "AppKit",
            "-framework", "Foundation",
            "-framework", "QuartzCore",
            "-framework", "Carbon",
            "-mmacosx-version-min=13.0",
        ])
        .arg(&main_m)
        .arg("-o")
        .arg(binary_dir.join("GQYMenuBar"))
        .output()
        .context("调用 xcrun clang 失败")?;
    if !output.status.success() {
        bail!(
            "菜单栏壳编译失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // 2) Info.plist
    std::fs::copy(&info_plist, contents.join("Info.plist"))?;

    // 3) 图标（失败则回退到系统符号图标，不阻塞安装）
    if icon_png.is_file() {
        let _ = build_icns(&icon_png, &resources_dir);
    }

    // 4) 版本号写入 Info.plist
    let plist = contents.join("Info.plist");
    let _ = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Set :CFBundleShortVersionString {}", env!("CARGO_PKG_VERSION"))])
        .arg(&plist)
        .status();
    let _ = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Set :CFBundleVersion {}", env!("CARGO_PKG_VERSION"))])
        .arg(&plist)
        .status();

    // 5) 内置当前 gqy 二进制
    let current_exe = std::env::current_exe().context("定位当前 gqy 二进制失败")?;
    std::fs::copy(&current_exe, resources_dir.join("gqy"))?;

    // 6) 共享资源（scripts/memes/kb/communication）
    copy_share(paths, &share_dir);

    // 7) 签名（ad-hoc）
    let _ = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(temp.path())
        .status();

    // 8) 原子替换到 ~/Applications
    if app_dir.exists() {
        std::fs::remove_dir_all(&app_dir)?;
    }
    std::fs::create_dir_all(home_apps)?;
    let staged = temp.path().to_path_buf();
    std::fs::rename(&staged, &app_dir)?;

    println!(
        "{}: {}",
        crate::i18n::text("installed", "已安装"),
        app_dir.display()
    );
    // 安装后直接拉起新菜单栏（无需用户手动双击）
    let _ = Command::new("open").arg(&app_dir).status();
    println!(
        "{}",
        crate::i18n::text(
            "menu bar launched; ⌥H opens the WebUI",
            "菜单栏已启动（⌥H 打开 WebUI）"
        )
    );
    // 若 /Applications 存在旧 cask 版，提示清理，避免误开旧版
    let system_app = Path::new("/Applications").join("顾清影.app");
    if system_app.is_dir() {
        println!(
            "{}",
            crate::i18n::text(
                "note: /Applications/顾清影.app is an older cask install; remove it with `brew uninstall --cask gqy` or manually, then re-run `gqy menubar --install`",
                "提示：/Applications/顾清影.app 是旧版 cask 安装；请 `brew uninstall --cask gqy` 或手动删除后重跑 `gqy menubar --install`"
            )
        );
    }
    Ok(())
}

/// 退出正在运行的菜单栏（pkill GQYMenuBar）并卸载旧 LaunchAgent。
fn cleanup_old_instances() {
    let _ = Command::new("pkill")
        .args(["-f", "GQYMenuBar"])
        .status();
    if let Ok(home) = std::env::var("HOME") {
        let uid = unsafe { libc::getuid() };
        let _ = Command::new("/bin/launchctl")
            .args(["bootout", &format!("gui/{uid}"), "dev.gqy.menubar"])
            .status();
        let plist = std::path::Path::new(&home)
            .join("Library/LaunchAgents/dev.gqy.menubar.plist");
        let _ = std::fs::remove_file(plist);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
}

fn dirs_applications_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME 未设置")?;
    Ok(PathBuf::from(home).join("Applications"))
}

/// 生成 AppIcon.icns（16~1024 各档），失败静默。
fn build_icns(icon_png: &Path, resources_dir: &Path) -> Result<()> {
    let iconset = resources_dir.join("AppIcon.iconset");
    std::fs::create_dir_all(&iconset)?;
    let sizes = [
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ];
    for (size, name) in sizes {
        let out = iconset.join(name);
        let status = Command::new("sips")
            .args(["-z", &size.to_string(), &size.to_string()])
            .arg(icon_png)
            .args(["--out"])
            .arg(&out)
            .status();
        if status.is_err() {
            return Ok(());
        }
    }
    let icns = resources_dir.join("AppIcon.icns");
    let _ = Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&iconset)
        .args(["-o"])
        .arg(&icns)
        .status();
    let _ = std::fs::remove_dir_all(&iconset);
    Ok(())
}

/// 复制共享资源到 App 内（与 brew share/gqy 布局一致，bundle 自包含）。
fn copy_share(paths: &GqyPaths, share_dir: &Path) {
    let sources = [
        (paths.share_dir.join("scripts"), "scripts"),
        (paths.share_dir.join("src/scripts"), "scripts"),
        (paths.share_dir.join("src/memes"), "memes"),
        (paths.share_dir.join("memes"), "memes"),
        (paths.share_dir.join("kb"), "kb"),
        (paths.share_dir.join("communication"), "bridges"),
    ];
    let mut copied = std::collections::HashSet::new();
    for (source, target) in sources {
        if copied.contains(target) || !source.is_dir() {
            continue;
        }
        let dest = share_dir.join(target);
        let _ = std::fs::create_dir_all(dest.parent().unwrap_or(share_dir));
        copy_dir_recursive(&source, &dest);
        copied.insert(target.to_string());
    }
}

fn copy_dir_recursive(source: &Path, dest: &Path) {
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let _ = std::fs::create_dir_all(dest);
    for entry in entries.flatten() {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            let _ = std::fs::copy(&from, &to);
        }
    }
}
