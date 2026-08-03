# GQY（顾清影）

> 一个活在 macOS 终端与菜单栏里的 AI 助理 —— 会记得你、会陪你聊、能干活、能自己换脑子（供应商）。

## 这是什么

GQY 是一个由大模型驱动、深度集成 macOS 的个人 AI 助理。她不是专业的 Coding Agent，而是偏「日用」的助手：聊天日常、游戏娱乐、系统排障、记忆陪伴。她有自己的**人格**（默认是女友人设）、有**记忆**（Git 快照备份）、有**本地模型**（llama.cpp / Ollama 跑在你自己机器上）、还能在对话里**自己切换供应商**。

核心特性：

- **多通道**：终端 REPL（`gqy`）、WebUI（`gqy web`）、菜单栏悬浮卡片（⌥G / 左键）、Telegram / QQ（通信桥）
- **本地优先**：支持 llama.cpp / Ollama 本地推理（Apple Silicon Metal 加速），2 秒级回复；也支持任意 OpenAI 兼容云端服务
- **供应商热切换**：对话里说「帮我加个供应商，地址 xx，key 是 yy」→ 自动发现模型 → 写入配置 → 激活，WebUI 即时刷新
- **人格系统**：`GQY_HOME/prompts/` 下的 `lover.md`（女友人格）+ `chat.md`（闲聊态提醒）；`active_persona` 切换
- **闲聊隔离**：闲聊模式与正经对话上下文互不污染（turns 按 mode 存储，闲聊只看最近 12 轮）
- **记忆与备份**：每轮对话后生成 Git 快照，绑定私有远程自动推送，换机器一键恢复
- **工具生态**：表情包、玄学、知识库、网络搜索（SearXNG 本地优先）、生图、语音（TTS/STT）、闹钟、深研等

## 快速开始

```zsh
# 安装（Homebrew）
brew tap Francis-Xavier-code/GQY
brew trust Francis-Xavier-code/GQY
brew install gqy

# 直接开聊（终端）
gqy

# WebUI
gqy web
# 或按 ⌥H 打开浏览器面板
```

本地模型（可选）：

```zsh
# llama.cpp 方案（推荐，跟随菜单栏启停）
brew install llama.cpp
# 放好 GGUF 模型后启动菜单栏，本地推理自动拉起
gqy menubar --install

# Ollama 方案
brew install ollama && ollama pull qwen3:8b
```

## 文档索引

- [快速开始](快速开始)
- [本地模型部署](本地模型部署)
- [人格系统](人格系统)
- [供应商管理](供应商管理)
- [WebUI 指南](WebUI-指南)
- [架构说明](架构说明)
- [发布流程](发布流程)
- [常见问题](常见问题)
