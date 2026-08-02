# GQY menu bar

这是一个只依赖 AppKit 的轻量菜单栏壳。它不保存第二份状态，而是为所有后端进程设置同一个 `GQY_HOME`。

构建：

```zsh
zsh macos/GQYMenuBar/build.sh
```

开发运行：

```zsh
export GQY_HOME="$HOME/Library/Application Support/gqy"
open "macos/GQYMenuBar/.build/顾清影.app"
```

构建脚本会优先把 `target/release/gqy` 打进 `.app`，开发时则回退到 `target/debug/gqy`。也可以用 `GQY_BIN=/absolute/path/to/gqy` 显式指定后端。脚本会用 `GQY-icon.png` 生成 App 图标（`.icns`），版本号跟随 `Cargo.toml`；默认 ad-hoc 签名，设置 `CODESIGN_IDENTITY` 环境变量可用 Developer ID 正式签名。

菜单栏提供终端对话、打开 WebUI（默认浏览器）、立即备份、打开独立主目录和开机自启五个入口。

> 说明：WebUI 不再内置独立的 NSPanel/WKWebView 窗口（与浏览器界面双份冗余），
> 「打开 WebUI」/「打开配置」/⌥H 均确保本地面板服务启动后在默认浏览器打开
> `http://127.0.0.1:4096`（配置直达带 `?open=settings`）。

## 打包 DMG

```zsh
zsh macos/GQYMenuBar/build.sh
zsh macos/GQYMenuBar/make-dmg.sh
```

产物为 `macos/GQYMenuBar/.build/GQY-<版本>.dmg`（如 `GQY-0.4.0.dmg`），内含 `顾清影.app` 与 `Applications` 快捷方式，挂载后拖入 Applications 即可安装。

## 开机自启（登录项）

菜单中的“开机自启”会安装一个 LaunchAgent（`~/Library/LaunchAgents/dev.gqy.menubar.plist`），
下次登录时自动用 `open` 启动 `.app`。再次点击可移除登录项。当前版本只在用户主动点击时
才修改登录项，不会擅自注册。
