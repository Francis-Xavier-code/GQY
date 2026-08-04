# 使用指南

## 目录

- [首次运行](#首次运行)
- [REPL 对话](#repl-对话)
- [Shell 集成](#shell-集成)
- [Web 面板](#web-面板)
- [配置管理](#配置管理)
- [供应商与模型](#供应商与模型)
- [知识库](#知识库)
- [记忆系统](#记忆系统)
- [表情包](#表情包)
- [语音](#语音)
- [桥接（QQ / Telegram）](#桥接)
- [备份与恢复](#备份与恢复)
- [本地模型](#本地模型)
- [高级功能](#高级功能)
- [FAQ](#faq)

## 首次运行

```bash
gqy
```

首次启动会自动：
- 创建配置目录 `~/.config/gqy/`
- 生成默认配置文件 `config.jsonc`
- 创建数据目录 `~/.local/share/gqy/`（对话、记忆、知识库）

推荐设置 `GQY_HOME` 环境变量统一管理所有数据：

```bash
# 添加到 ~/.zshrc 或 ~/.bashrc
export GQY_HOME="$HOME/Library/Application Support/gqy"
```

## REPL 对话

```bash
gqy                    # 进入交互式 REPL
gqy "今天天气怎么样"    # 单次对话
gqy --plan "重构方案"   # 计划模式（只读分析）
```

REPL 快捷键：
- `Ctrl+O` — 流式输出中展开/收起思考详情
- `Ctrl+C` — 中断当前回复
- `Ctrl+D` — 退出

### 自然语言输入

配置 Shell hook 后，在终端直接输入自然语言即可触发对话（无需 `gqy` 前缀）：

```bash
gqy zsh-init    # 安装 zsh hook
gqy fish-init   # 安装 fish hook
```

输入非命令的自然语言（如「帮我看下磁盘空间」），GQY 会自动拦截并处理。

## Shell 集成

```bash
gqy zsh-init           # 安装 zsh hook
gqy fish-init          # 安装 fish hook
gqy bash-init          # 安装 bash hook（实验性）
gqy remove-shell-hook  # 移除所有 hook
```

安装后重启终端生效。hook 通过 `command_not_found_handler` 实现：
- 输入已知命令 → 正常执行
- 输入自然语言 → 交给 GQY 处理

关闭自动拦截（只用 `gqy` 前缀）：

```bash
gqy config set shell.auto false
```

## Web 面板

```bash
gqy web                    # 启动（默认 127.0.0.1:4096）
gqy web --port 8080        # 自定义端口
gqy web --host 0.0.0.0     # 监听所有地址（需设置密码）
gqy web -p mypassword      # 设置访问密码
gqy web --no-open          # 不自动打开浏览器
```

功能：
- 多通道对话（终端 / WebUI / QQ / Telegram 各自独立上下文）
- 流式输出、思考过程展示
- 用量分析（贡献热力图、费用估算、模型明细）
- 图片粘贴/拖拽发送
- 对话全文搜索
- 对话导出（Markdown）
- 定时任务面板（闹钟/番茄钟）

默认绑定 `127.0.0.1`，仅本机可访问。绑定非回环地址必须设置密码。

## 配置管理

```bash
gqy config                          # TUI 配置界面
gqy config set active_provider deepseek   # 设置供应商
gqy config get active_provider            # 读取配置
gqy config set plugins.web.enabled true   # 启用插件
gqy config get                            # 查看全部配置
```

配置文件：`~/.config/gqy/config.jsonc`（JSONC 格式，支持注释）

密钥支持环境变量引用：

```jsonc
{
  "providers": [{
    "id": "deepseek",
    "api_key": "$env:DEEPSEEK_API_KEY"
  }]
}
```

## 供应商与模型

### 添加供应商

```bash
gqy provider add https://api.deepseek.com/v1 --api-key sk-xxx
```

自动发现可用模型并激活。支持任意 OpenAI 兼容接口。

### 管理供应商

```bash
gqy provider list           # 列出所有供应商
gqy provider switch deepseek  # 切换供应商
gqy provider remove <id>    # 删除供应商
```

### 对话中切换

直接告诉 GQY「帮我加个供应商」或「切到 deepseek」，她会自动调用工具完成。

### 热切换

运行中修改配置，WebUI 通过 config watcher 自动刷新，无需重启。

## 知识库

```bash
gqy kb add <目录>          # 批量导入
gqy kb add kb/             # 导入随包知识库
gqy kb search <关键词>     # 搜索
gqy kb list                # 列出所有
```

内置 macOS 知识库覆盖：Homebrew、磁盘清理、开机自启、终端代理、网络排障、系统权限等 16 个主题。

知识库随 Git 备份一起快照，换机器恢复后重新 `kb add` 即可。

## 记忆系统

记忆由两部分组成：
1. **对话记忆** — 曾经发生的事（自动记录）
2. **知识记忆** — 信息中的知识点（自动 + 手动）

```bash
gqy memory search <词>      # 搜索记忆
gqy memory remember <内容>  # 手动记录
gqy memory stats            # 记忆统计
gqy memory list             # 列出所有
gqy memory forget <id>      # 删除记忆
```

对话时会根据用户消息自动召回相关记忆（联想功能）。每轮对话结束后记忆自动落盘。

## 表情包

GQY 会根据情景自主发送表情包。配置项：

```bash
gqy config set plugins.memes.enabled true       # 启用
gqy config set plugins.memes.probability 0.3     # 发送概率
gqy config set plugins.memes.cooldown 30         # 冷却时间（秒）
```

管理表情库：

```bash
gqy memes list       # 列出表情
gqy memes stats      # 统计信息
```

在对话中直接告诉 GQY 添加表情：「把这个图片加到表情库」。

## 语音

### TTS（文字转语音）

```bash
gqy tts "你好世界"              # macOS 本地朗读
gqy tts --voice Ting-Ting "你好"  # 指定音色
gqy tts --clone "你好世界"        # 克隆音色（需要 Qwen3-TTS）
gqy tts --list                   # 列出可用音色
```

### STT（语音转文字）

```bash
gqy stt audio.wav    # 离线识别（macOS SFSpeechRecognizer）
```

模型也可以自主调用 `speak` / `listen_audio` 工具。

## 桥接

### QQ（NapCat）

```bash
gqy napcat status      # 查看状态
gqy napcat install     # 安装桥接
gqy napcat config      # 配置
gqy napcat uninstall   # 卸载
```

### Telegram

```bash
gqy tg status          # 查看状态
gqy tg install         # 安装桥接
gqy tg token <token>   # 设置 Bot Token
gqy tg config          # 配置
gqy tg uninstall       # 卸载
```

桥接安装后自动注册 LaunchAgent 开机自启。每个桥接有独立的对话通道。

## 备份与恢复

```bash
gqy backup init                    # 初始化备份仓库
gqy backup now                     # 立即备份
gqy backup status                  # 查看状态
gqy backup remote <url>            # 绑定远程仓库
gqy backup remote owner/repo       # 自动创建私有仓库（需 gh CLI）
gqy backup restore --remote <url>  # 从远程恢复
```

备份内容：配置、记忆、对话历史、知识库。自动排除 API Key 等敏感信息。

每轮对话结束后自动生成 Git 快照（30 分钟节流）。

## 本地模型

支持 Apple Silicon 本地推理（Metal 加速）：

### llama.cpp

```bash
# 安装
brew install llama.cpp

# 启动服务
llama-server -m /path/to/model.gguf --port 8080

# 配置 GQY
gqy provider add http://127.0.0.1:8080/v1 --api-key local
```

### Ollama

```bash
# 安装
brew install ollama

# 拉取模型
ollama pull qwen3:8b

# 配置 GQY
gqy provider add http://127.0.0.1:11434/v1 --api-key ollama
```

## 高级功能

### 深度研究

```bash
# 在对话中让 GQY 研究
gqy "帮我研究一下 Rust 异步运行时的实现原理"
```

GQY 会自动调用深度研究工具，多阶段收集资料，生成带引用的研究报告。

### pi 底座模式

将「大脑」交给 [pi](https://github.com/earendil-works/pi)，GQY 负责渲染、记忆、知识库：

```jsonc
{
  "providers": [{
    "id": "pi",
    "protocol": "pi"
  }]
}
```

详见 [pi 底座模式文档](01-指南/pi-底座模式.md)。

### 子代理

GQY 可以自主创建命名子代理并组队协作（Kimi 式）：

- `gqy_spawn_agent` — 创建子代理
- `gqy_talk_to_agent` — 派活（可并行）
- `gqy_list_agents` — 查看名册
- `gqy_kill_agent` — 销毁

### 工具包管理

```bash
gqy tools list                  # 列出工具
gqy tools import <仓库>         # 导入工具包
gqy tools show <包名>           # 查看详情
gqy tools disable <id>          # 禁用工具
gqy tools enable <id>           # 启用工具
gqy tools remove <包名>         # 删除工具包
```

### 定时任务

```bash
gqy alarm set 10m "泡面好了"     # 10 分钟闹钟
gqy alarm set 25m "番茄钟" --repeat  # 周期提醒
gqy alarm list                  # 列出闹钟
gqy alarm cancel <id>           # 取消闹钟
gqy alarm stop --all            # 停止所有
```

## FAQ

**Q：换电脑怎么迁移？**
A：所有状态在 `GQY_HOME`。配好远程仓库后 `gqy backup restore --remote <url>` 一键恢复。

**Q：远程仓库会泄露 API Key 吗？**
A：不会。备份自动清空所有 api_key/token/password 字段。

**Q：默认模型要钱吗？**
A：默认接入 opencode 公共模型服务，开箱即用免费使用。也可以配置自己的 API。

**Q：怎么让她记住特定的事？**
A：直接说「记住：xxx」，或用 `gqy memory remember <内容>`。

**Q：卸载会删记忆吗？**
A：不会。`brew uninstall gqy` 只移除程序，`GQY_HOME` 下的用户数据不受影响。

更多问题见 [常见问题](../wiki/常见问题.md)。
