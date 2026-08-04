<p align="center">
  <img src="pics/GQY-icon.png" alt="GQY" width="160">
</p>

<h1 align="center">GQY — 顾清影</h1>

<p align="center">
  活在终端与菜单栏里的 AI 助理
</p>

<p align="center">
  <a href="https://github.com/GQYTeam/GQY/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License"></a>
  <img src="https://img.shields.io/badge/Rust-1.97.1-orange?logo=rust" alt="Rust">
  <a href="https://github.com/GQYTeam/GQY/actions"><img src="https://img.shields.io/github/actions/workflow/status/GQYTeam/GQY/ci.yml?label=CI" alt="CI"></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple" alt="Platform">
</p>

---

GQY 是一个大模型驱动的 CLI AI 助理，运行在 macOS 终端和菜单栏中。她不是 Coding Agent，而是偏向日常聊天、系统排障、娱乐互动的桌面伴侣。

默认接入 [opencode](https://github.com/anomalyco/opencode) 公共模型服务，开箱即用；也支持任意 OpenAI 兼容接口、本地模型（llama.cpp / Ollama）和 [pi](https://github.com/earendil-works/pi) 底座模式。

<p align="center">
  <img src="pics/GQY-image.png" alt="GQY Demo" width="680">
</p>

## 安装

### 一键安装（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/GQYTeam/GQY/main/install.sh | bash
```

自动检测平台（macOS Intel/Apple Silicon、Linux x86_64/aarch64），下载预编译二进制并安装共享资源。

### Homebrew

```bash
brew tap GQYTeam/GQY
brew trust GQYTeam/GQY
brew install gqy
```

### 从源码构建

需要 Rust 1.97+、C 编译工具链。图片显示依赖 [chafa](https://github.com/hpjansson/chafa)。

```bash
brew install rust chafa
git clone https://github.com/GQYTeam/GQY.git
cd GQY
cargo build --release --locked
./target/release/gqy --version
```

## 快速开始

```bash
# 进入 REPL 对话
gqy

# 直接问一句
gqy "今天天气怎么样"

# 打开配置 TUI
gqy config

# 启动 Web 面板
gqy web
```

首次运行会自动创建配置目录和默认配置。所有数据（对话、记忆、配置、知识库）统一存放在 `GQY_HOME`（默认 `~/Library/Application Support/gqy`）。

## 主要功能

| 功能 | 说明 |
|------|------|
| **REPL 对话** | 终端交互式对话，支持流式输出、思考过程展示、表情包发送 |
| **Web 面板** | `gqy web` 启动本地 WebUI（默认 127.0.0.1:4096），支持多通道、用量分析 |
| **Shell 集成** | zsh/fish 无缝集成，命令未找到时自动交给 GQY 处理 |
| **知识库** | 内置 macOS 日常排障知识库（16 篇），支持自定义导入 |
| **记忆系统** | 对话记忆自动召回，支持手动记录、搜索、归档 |
| **表情包** | 自带表情库，根据情景自主发送，支持自定义添加 |
| **工具集** | 40+ 内置工具：天气、汇率、计算器、哈希、闹钟、玄学、搜图、生图等 |
| **深度研究** | 多阶段研究报告生成，引经据典，带引用来源 |
| **语音** | TTS 朗读（含克隆音色）、STT 离线识别（macOS 本地） |
| **桥接** | QQ（NapCat）、Telegram 桥接，独立通道对话 |
| **备份** | Git 快照自动备份，支持私有远程仓库，一键恢复 |
| **多供应商** | 任意 OpenAI 兼容服务热切换，自动发现模型 |
| **本地推理** | 支持 llama.cpp / Ollama / LM Studio，Apple Silicon Metal 加速 |

## 命令速查

| 命令 | 作用 |
|------|------|
| `gqy` | 进入 REPL |
| `gqy "问题"` | 单次对话 |
| `gqy config` | 配置 TUI |
| `gqy config set <key> <value>` | 免交互写配置 |
| `gqy web` | 启动 Web 面板 |
| `gqy kb add <目录>` | 导入知识库 |
| `gqy kb search <关键词>` | 搜索知识库 |
| `gqy memory search <词>` | 搜索记忆 |
| `gqy history --search <词>` | 搜索对话记录 |
| `gqy zsh-init` / `gqy fish-init` | 安装 Shell hook |
| `gqy provider add <url> --api-key <key>` | 添加供应商 |
| `gqy napcat status` / `gqy tg status` | 桥接状态 |
| `gqy backup init` / `gqy backup now` | 备份管理 |
| `gqy balance` | 查询 DeepSeek 余额 |
| `gqy tts "文字"` | 语音朗读 |
| `gqy reset --all` | 清空对话与记忆 |

## 界面语言

支持英文与简体中文，自动跟随系统 locale。在 `gqy config` 中切换，或用环境变量临时覆盖：

```bash
GQY_LANG=en gqy    # 强制英文
GQY_LANG=zh gqy    # 强制中文
```

## 配置

配置文件位于 `~/.config/gqy/config.jsonc`（JSONC 格式，支持注释）。可通过 `gqy config` TUI 或 `gqy config set/get` 命令修改。

关键配置项：

```jsonc
{
  "active_provider": "opencode",          // 当前供应商
  "active_provider_models": [             // 模型池
    { "provider_id": "opencode", "model": "big-pickle" }
  ],
  "plugins": {
    "web": { "enabled": true },           // 网络搜索
    "vision": { "enabled": true },        // 图片识别
    "memes": { "enabled": true },         // 表情包
    "knowledge_base": { "enabled": true } // 知识库
  },
  "display": {
    "language": "auto"                    // 界面语言
  }
}
```

完整配置说明见 [docs/01-指南/配置指南.md](docs/01-指南/配置指南.md)。

## 文档

| 文档 | 说明 |
|------|------|
| [快速开始](wiki/快速开始.md) | 首次使用指南 |
| [配置指南](docs/01-指南/配置指南.md) | 完整配置项说明 |
| [架构说明](wiki/架构说明.md) | 技术架构 |
| [本地模型部署](wiki/本地模型部署.md) | llama.cpp / Ollama 部署 |
| [供应商管理](wiki/供应商管理.md) | 多供应商配置 |
| [人格系统](wiki/人格系统.md) | 人格与角色扮演 |
| [WebUI 指南](wiki/WebUI-指南.md) | Web 面板使用 |
| [macOS 集成](docs/01-指南/macos-portable-home-and-backup.md) | 菜单栏、独立主目录、备份 |
| [pi 底座模式](docs/01-指南/pi-底座模式.md) | pi 集成 |
| [发布流程](wiki/发布流程.md) | 版本发布 |
| [常见问题](wiki/常见问题.md) | FAQ |

## 开发

```bash
cargo check              # 编译检查
cargo test               # 运行测试
cargo clippy -- -W warnings   # Lint
cargo build --release    # Release 构建
```

CI 固定 Rust 1.97.1。Linux 需要额外安装 `libasound2-dev pkg-config ripgrep`。

更多开发信息见 [AGENTS.md](AGENTS.md)。

## 贡献

欢迎贡献！请阅读 [设计理念](docs/01-指南/自主行为规范.md) 后提交 PR。

- 一个 PR 只包含一个功能
- 提供设计理念、作用场景和实际意义
- 不变更现有功能语义

## 致谢

- [opencode](https://github.com/anomalyco/opencode) — 最好的开源 Coding Agent
- [Miyu](https://github.com/SHORiN-KiWATA/Miyu) — 本项目 fork 自 Miyu（MIT License）

## 许可

本项目使用 [GPL-3.0](LICENSE) 发布。上游 Miyu 的 MIT 部分仍按 MIT 授权，新增代码与修改按 GPL-3.0 授权。
