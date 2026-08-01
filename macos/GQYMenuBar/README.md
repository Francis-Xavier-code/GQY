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

菜单栏提供终端对话、本地 Web 面板、立即备份和打开独立主目录四个入口。发布阶段再注册为登录项；当前版本不会擅自修改用户的登录项。
