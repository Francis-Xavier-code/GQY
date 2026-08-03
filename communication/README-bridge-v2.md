# communication/ — 消息平台桥接层（v2：接 GQY 长驻 daemon）

> v2 核心变更：桥接层不再 spawn 一次性 `gqy ask`，而是连接 GQY 的**长驻 daemon**
> （`gqy web`），走 HTTP + SSE 事件流。这是「通信做完整」的前提——
> 只有 daemon 模式能拿到图片事件（tool.image）、流式输出、工具进度与追问。

## 架构

```
QQ/NapCat ──┐
Telegram ───┼─→ bridge.cjs（长驻 Node 进程）──HTTP/SSE──> GQY daemon（gqy web）
            │        │ 1. 收到消息 → POST /api/turns 交给 GQY 完整 agent 循环
            │        │ 2. 订阅 GET /api/events（SSE）实时拿事件
            │        │ 3. assistant.delta → 收齐文字；tool.image → 下载资产发图
            │        │ 4. run.completed → 分片发文字 + 逐张发图片
            └────────┘ 5. question.requested → 转发追问，下条消息回答后继续
```

**关键点：**
- **daemon 生命周期自动管理**（`lib/daemon-client.cjs` 的 `ensureDaemon`）：启动时探测
  `/api/health`，已有 daemon 复用；没有就自己 spawn
  `gqy web --host 127.0.0.1 --port <port> --no-open`（只绑本机、不弹浏览器），
  退出时只杀自己拉起的（进程组清理，不留孤儿）。
- **通道隔离**：每个平台一个 daemon 通道（TG=`GQY_CHANNEL=tg` 端口 4101，
  QQ=`GQY_CHANNEL=qq` 端口 4102），平台间上下文互不串扰；
  与菜单栏/WebUI 的 4096（webui 通道）也互不影响。
- **同平台内所有聊天共享一份对话上下文**（daemon 单活动会话模型）。
  若未来需要按用户/群隔离，可演进为「每聊天一个 daemon 通道」，成本是内存。
- **串行提交**：桥内 FIFO 队列，同一时刻只跑一个 run，避免排队时 run_id
  匹配歧义（排队后 run_id 会变）。

## 新增/修改文件

| 文件 | 说明 |
|---|---|
| `lib/daemon-client.cjs` | **新增**：daemon 生命周期 + HTTP + SSE 客户端 |
| `tg/bridge.cjs` | **重写**：接 daemon，支持图片/追问/流式文字 |
| `napcat/bridge.cjs` | **重写**：接 daemon，支持图片（base64 段）/追问 |
| `lib/bridge-common.cjs` | 保留：splitReply / log 仍被使用（askGqy 等不再被 v2 使用） |

## 部署

前置：本机已装 GQY（brew 或源码），且能正常对话；Node ≥ 18（全局 fetch）。

```zsh
cd communication

# Telegram
GQY_TG_TOKEN=123456:ABC... GQY_TG_OWNER_ID=你的TG数字ID node tg/bridge.cjs

# QQ（NapCat 已跑在 ws://127.0.0.1:3001）
GQY_SELF_ID=你的QQ号 node napcat/bridge.cjs
```

建议用 LaunchAgent 托管（与 v1 相同的 `~/Library/LaunchAgents/*.plist` 方式，
程序入口指向对应 bridge.cjs）。

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `GQY_TG_TOKEN` | 空 | Telegram Bot Token（TG 必需） |
| `GQY_TG_OWNER_ID` | 空 | 主人 TG 数字 ID（仅用于按钮回调归属，不隔离会话） |
| `GQY_SELF_ID` | 空 | 你的 QQ 号；未设置时群聊 @ 响应不可用 |
| `GQY_WS_URL` | `ws://127.0.0.1:3001` | NapCat OneBot WebSocket 地址 |
| `GQY_TG_WEB_PORT` | `4101` | TG daemon 端口 |
| `GQY_QQ_WEB_PORT` | `4102` | QQ daemon 端口 |
| `GQY_RUN_TIMEOUT_MS` | `600000` | 单轮等待上限（秒×1000），长工具执行时可调大 |
| `GQY_BIN` | `/opt/homebrew/bin/gqy` | gqy 可执行文件路径 |
| `GQY_HOME` | `~/Library/Application Support/gqy` | 主数据目录（daemon 读配置） |
| `GQY_BRIDGE_LOG` | `~/napcat/bridge.log`（TG 为 `tg-bridge.log`） | 日志路径 |

## 行为说明

- **图片/表情包**：GQY 调 `show_meme` 等工具时，SSE 推 `tool.image`，桥下载资产后
  发回聊天（TG `sendPhoto`；QQ base64 图片段）。回复顺序：先文字后图片。
- **追问**：GQY 需要澄清时会推 `question.requested`，桥转发问题；发起该 run 的
  聊天下一条消息作为回答提交（`/api/questions/{id}/answer`），run 继续。
- **超时**：单轮超过 `GQY_RUN_TIMEOUT_MS` 未完成，发已收集到的内容并结束。
- **重连**：SSE 断线自动按最后事件 id 续传（`Last-Event-ID`），不丢事件；
  NapCat WS 断开 3 秒重连。

## 已知限制

- 同平台共享上下文：A 群和 B 群、私聊与群聊都在同一 TG/QQ 对话里。
- 图片仅支持 tool.image 事件产出（表情包/生图等）；用户发来的图片暂不送模型。
- 长回复为「完成后一次性发送」，非逐字流式（TG/QQ 均无低成本流式 UI）。
