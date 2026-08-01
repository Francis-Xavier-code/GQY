# communication/ — 消息平台桥接层源码

这里只放**桥接层的源代码**，不放运行时二进制、日志、会话数据。

## 目录结构

```
communication/
├── README.md
├── lib/
│   └── bridge-common.cjs   # 公共模块：askGqy / 会话隔离 / 输出清理 / 回复分片 / 串行队列
├── napcat/
│   └── bridge.cjs          # OneBot 11 WebSocket 客户端，路由消息到 `gqy ask`
└── tg/
    └── bridge.cjs          # Telegram Bot API 长轮询桥接
```

## 部署约定

- 源码以 git 管理，运行时产物**不要**提交进仓库。
- NapCat 本体、QQ 副本、日志、会话数据保持在部署区：
  - `~/napcat/`            — NapCat 运行时 + 日志
  - `~/qq-napcat/`         — 隔离 QQ 副本
  - `~/Library/Application Support/gqy/sessions/` — 隔离会话目录（每个会话一个独立 GQY_HOME）
- bridge 由 LaunchAgent 托管：
  - `~/Library/LaunchAgents/com.gqy.napcat-bridge.plist`
  - 程序入口指向本目录 `napcat/bridge.cjs`
  - 日志写 `~/napcat/bridge.log`（TG 桥接写 `tg-bridge.log`）

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `GQY_WS_URL` | `ws://127.0.0.1:3001` | NapCat OneBot WebSocket 地址 |
| `GQY_SELF_ID` | 空 | 你的 QQ 号；未设置时**群聊 @ 响应不可用**（启动有警告） |
| `GQY_TG_TOKEN` | 空 | Telegram Bot Token（TG 桥接必需） |
| `GQY_TG_OWNER_ID` | 空 | 主人 TG 数字 ID；私聊时走主 GQY_HOME 全局上下文，其他用户按 ID 隔离 |
| `GQY_BIN` | `/opt/homebrew/bin/gqy` | gqy 可执行文件路径 |
| `GQY_TIMEOUT_MS` | `120000` | 单次 ask 超时（超时自动终止并回提示） |
| `GQY_SESSIONS_DIR` | `~/Library/Application Support/gqy/sessions` | 隔离会话根目录 |
| `GQY_BRIDGE_LOG` | `~/napcat/bridge.log` | 日志路径 |

## 会话隔离设计

- **每个隔离会话一个独立 GQY_HOME**：对话历史（conversation.db）、记忆（memory.db）互不串扰。
  私聊按用户隔离（`qq-private-<id>` / `tg-private-<id>`），群聊按群隔离（`qq-group-<id>` / `tg-group-<id>`）。
- **同一会话的消息串行处理**（`enqueueSession`）：同一 GQY_HOME 下同一时刻只有一个 gqy 进程，
  避免并发读写 SQLite 的竞态与回复乱序。
- **密钥不进会话目录**：首次创建会话时复制主 `config.jsonc`，但 api_key/token/password 等敏感字段
  会被替换为 `$env:GQY_BRIDGE_KEY_n` 引用（GQY 原生支持 `$env:` 引用），真实密钥只经进程环境
  注入子进程。主配置更新（换模型/换 key）后，会话自动跟随，无需重建。
- 会话目录权限 `0700`；主 GQY_HOME（主人上下文）永远不写入会话目录。

## 修改流程

改完 `napcat/bridge.cjs` 后重启桥接服务生效：

```bash
launchctl kickstart -k gui/$(id -u)/com.gqy.napcat-bridge
```

冒烟测试（本地、无需网络）：

```bash
node -e "const m = require('./lib/bridge-common.cjs'); console.log(m.splitReply('你好'.repeat(2001), 4000).length)"
```

## TG 桥接

- 私聊：主人（`GQY_TG_OWNER_ID`）走全局上下文；其他用户按 ID 隔离会话
- 群聊：只在被 @bot、回复 bot 消息、或 `/command@bot` 时响应，按群隔离
- 长回复按 4096 字符分片；处理中发 `typing` 提示；`edited_message` 不触发问答
