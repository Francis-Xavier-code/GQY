# macOS、独立主目录与记忆备份

## 一个后端，多种入口

GQY 的终端、WebUI 和菜单栏不应各自保存一套状态。三种入口都启动同一个 Rust 后端，并通过 `GQY_HOME` 指向同一个独立主目录：

```text
GQY_HOME/
├── config/      配置、人格、用户身份、skills、脚本
├── data/        长期记忆与知识库数据
├── state/       对话、用量和可召回上下文
├── pictures/    由助理管理的图片资产
├── cache/       日志和可重新生成的缓存
├── secrets/     独立 SSH 私钥等本机秘密，不进入备份
└── backup/      独立 Git 配置、快照仓库和备份设置
```

建议在 macOS 上固定使用：

```zsh
export GQY_HOME="$HOME/Library/Application Support/GQY"
```

`GQY_HOME` 必须是绝对路径。未设置时程序仍兼容上游原有的系统目录布局，但 Git 备份会拒绝启动，因为分散布局无法保证备份边界。

## 终端入口

构建后可直接进入 REPL：

```zsh
GQY_HOME="$HOME/Library/Application Support/GQY" ./target/release/miyu
```

安装 zsh 自然语言 hook：

```zsh
GQY_HOME="$HOME/Library/Application Support/GQY" ./target/release/miyu zsh-init
```

hook 本体位于 `GQY_HOME/config/shell/`。安装动作只会在宿主机 `~/.zshrc` 中加入一个带边界标记的 `source` 块，`miyu remove-shell-hook` 可以移除它。

## 菜单栏入口

轻量 AppKit 壳位于 `macos/GQYMenuBar`，不依赖第三方 GUI 框架或完整 Xcode：

```zsh
zsh macos/GQYMenuBar/build.sh
open "macos/GQYMenuBar/.build/顾清影.app"
```

构建脚本会把现有的 release 后端打进 `.app`；找不到 release 时会使用 debug 后端。菜单提供终端对话、本地 Web 面板、立即备份和打开独立主目录入口。

## 独立 Git 备份

远程仓库应设置为 private。记忆和对话本身可能包含私人内容，即使配置文件已经脱敏，也不适合公开。

有两种起步方式：

- **本地模式**（不需要任何账号）：`miyu backup init` 不带 `--remote`，先启用本地自动快照，之后随时用 `miyu backup remote <url>` 绑定远程。
- **直接配置远程**：创建 private 仓库后按下面的流程一次到位。

推荐为她单独创建 SSH key：

```zsh
mkdir -p "$GQY_HOME/secrets/ssh"
ssh-keygen -t ed25519 -f "$GQY_HOME/secrets/ssh/id_ed25519" -C "gqy-memory-backup"
```

把公钥添加为远程私有仓库的 deploy key，然后初始化：

```zsh
miyu backup init \
  --remote git@github.com:YOUR_NAME/gqy-memory.git \
  --ssh-key "$GQY_HOME/secrets/ssh/id_ed25519" \
  --name "GQY Memory" \
  --email "gqy@localhost"
```

本地模式起步后再绑定远程：

```zsh
miyu backup remote git@github.com:YOUR_NAME/gqy-memory.git \
  --ssh-key "$GQY_HOME/secrets/ssh/id_ed25519" \
  --auto-push
```

手动检查和备份：

```zsh
miyu backup status
miyu backup now
```

在另一台机器上从远程恢复（还原人格、记忆、对话状态和图片）：

```zsh
export GQY_HOME="$HOME/Library/Application Support/GQY"
miyu backup restore \
  --remote git@github.com:YOUR_NAME/gqy-memory.git \
  --ssh-key "$GQY_HOME/secrets/ssh/id_ed25519"
```

`backup restore` 会跳过首次初始化，直接把快照还原进独立主目录；已存在本地配置时需 `--force` 覆盖，且不会覆盖已有 `config.jsonc`（避免用脱敏配置冲掉真实密钥，恢复后再重新填写 API key 即可）。恢复完成后自动推送默认开启（与 `init` 一致），传入 `--no-auto-push` 可关闭；`backup now` 可随时继续增量备份。

默认开启自动保存：每次成功完成一轮对话并落盘记忆后，程序刷新快照并提交；绑定远程后同一开关会同时推送到远程。传入 `backup init --no-auto-push` 可改为只保留手动备份。`backup status` 在未绑定远程时会明确显示本地模式。

### 隔离与安全边界

- Git 使用 `GQY_HOME/backup/gitconfig` 作为唯一 global config，并设置 `GIT_CONFIG_NOSYSTEM=1`，不会读取宿主机的 Git 全局/系统配置。
- SSH remote 必须显式提供位于 `GQY_HOME/secrets` 下的 key；known_hosts 也保存在该目录。
- 禁止把用户名、token 或密码嵌入 HTTP remote URL。
- live SQLite 文件通过 `VACUUM INTO` 生成一致快照，不直接提交 WAL/SHM 文件。
- `config.jsonc` 会递归清除 API key、token、password、secret、credential、authorization 等字段后再进入快照。
- `.env*`、私钥/证书、缓存、日志和 Git 凭据不会进入提交。

当前备份覆盖人格、用户身份、skills、长期记忆、对话状态和图片资产；`backup restore` 可从同一远程还原全部内容。Keychain 管理仍属于后续阶段；在实现恢复前，远端仓库已经是可检查的标准 Git 快照，不是私有二进制格式。

## 开机自启

菜单栏应用的“开机自启”菜单项会安装 `~/Library/LaunchAgents/dev.gqy.menubar.plist`，让顾清影在下次登录时自动启动；再次点击可移除。该 LaunchAgent 会为后端设置同一个 `GQY_HOME`。
