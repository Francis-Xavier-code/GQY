# GQY menu bar

这是一个只依赖 AppKit 的轻量菜单栏壳。它不保存第二份状态，而是为所有后端进程设置同一个 `GQY_HOME`。

构建：

```zsh
zsh macos/GQYMenuBar/build.sh
```

开发运行：

```zsh
export GQY_HOME="$HOME/Library/Application Support/GQY"
open "macos/GQYMenuBar/.build/顾清影.app"
```

构建脚本会优先把 `target/release/miyu` 打进 `.app`，开发时则回退到 `target/debug/miyu`。也可以用 `GQY_BIN=/absolute/path/to/miyu` 显式指定后端。

菜单栏提供终端对话、本地 Web 面板、立即备份、打开独立主目录和开机自启五个入口。

## 开机自启（登录项）

菜单中的“开机自启”会安装一个 LaunchAgent（`~/Library/LaunchAgents/dev.gqy.menubar.plist`），
下次登录时自动用 `open` 启动 `.app`。再次点击可移除登录项。当前版本只在用户主动点击时
才修改登录项，不会擅自注册。
