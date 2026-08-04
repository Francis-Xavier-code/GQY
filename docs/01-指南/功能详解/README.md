# GQY 功能详解

本指南详细介绍 GQY 的所有功能模块。

## 功能概览

GQY 是一个功能丰富的 AI 助理，主要功能包括：

1. **对话系统** - 多模式对话、流式输出、思考过程展示
2. **记忆系统** - 持久化记忆、自动记忆、记忆搜索
3. **知识库** - 文档导入、语义搜索、知识管理
4. **工具系统** - 40+ 内置工具、自定义脚本、技能系统
5. **Agent 集群** - 多 Agent 协作、并行任务执行
6. **Web 面板** - 实时对话、用量分析、多通道管理
7. **语音系统** - TTS/STT、音色克隆
8. **表情包** - 自动表情、表情库管理
9. **桥接系统** - QQ/Telegram 集成
10. **备份恢复** - Git 快照、远程同步

## 对话系统

### 对话模式

GQY 支持三种对话模式：

#### Normal 模式
- 完整功能模式
- 包含所有工具
- 适合日常使用

```zsh
gqy "帮我写一个 Python 脚本"
```

#### Plan 模式
- 只读分析模式
- 不修改文件
- 适合代码审查

```zsh
gqy --plan "分析这个项目的架构"
```

#### Chat 模式
- 轻量闲聊模式
- 节省资源
- 适合简单对话

```zsh
gqy --chat "今天心情不错"
```

### 流式输出

GQY 支持实时流式输出，可以看到 AI 的思考过程：

```zsh
# 启用思考过程展示
gqy config set display.reasoning full

# 启用工具调用展示
gqy config set tool_calls full
```

### 上下文管理

GQY 自动管理对话上下文：

- **自动压缩**：上下文过长时自动压缩
- **记忆关联**：自动关联相关记忆
- **模式隔离**：不同模式的对话上下文独立

## 记忆系统

### 记忆类型

1. **事实记忆** - 用户提供的信息
2. **情景记忆** - 对话历史
3. **知识记忆** - 从知识库提取的信息

### 记忆操作

```zsh
# 记住事实
gqy memory remember "用户喜欢使用 Rust"

# 搜索记忆
gqy memory search "编程偏好"

# 列出记忆
gqy memory list

# 删除记忆
gqy memory forget <id>

# 清空记忆
gqy memory clear
```

### 自动记忆

GQY 会自动识别并记忆重要信息：
- "请记住..."
- "别忘了..."
- "我是一名..."
- "我喜欢..."

### 记忆搜索

```zsh
# 语义搜索
gqy memory search "机器学习"

# 关键词搜索
gqy memory search --keyword "Rust"

# 按时间搜索
gqy memory search --since "2024-01-01"
```

## 知识库

### 知识库操作

```zsh
# 导入文档
gqy kb add /path/to/documents

# 搜索知识库
gqy kb search "关键词"

# 列出知识库
gqy kb list

# 删除文档
gqy kb remove <doc-id>
```

### 支持格式

- Markdown (.md)
- 文本文件 (.txt)
- PDF (.pdf)
- Word (.docx)
- HTML (.html)

### 语义搜索

知识库支持语义搜索，可以找到相关内容：

```zsh
# 语义搜索
gqy kb search "如何优化性能"

# 精确搜索
gqy kb search --exact "performance optimization"
```

## 工具系统

### 内置工具

GQY 包含 40+ 内置工具：

| 类别 | 工具 | 说明 |
|------|------|------|
| 文件操作 | `read_file`, `write_file`, `edit_file` | 文件读写 |
| 系统工具 | `run_command`, `check_os_info` | 系统操作 |
| 网络工具 | `web_search`, `web_fetch` | 网络搜索 |
| 知识库 | `search_knowledge_base` | 知识库搜索 |
| 记忆 | `remember_fact`, `recall_memory` | 记忆操作 |
| 媒体 | `analyze_image`, `generate_image` | 图片处理 |
| 专业工具 | `weather`, `exchange_rate` | 专业查询 |

### 工具权限

工具分为三种权限级别：
- **自动授权** - 安全工具，无需确认
- **用户授权** - 需要用户确认
- **禁止使用** - 被禁用的工具

### 自定义工具

#### 脚本工具

```zsh
# 注册脚本工具
gqy scripts register /path/to/script.sh --name "我的脚本"

# 列出脚本工具
gqy scripts list

# 删除脚本工具
gqy scripts unregister <name>
```

#### 技能系统

```zsh
# 注册技能
gqy skills register /path/to/skill.json

# 列出技能
gqy skills list

# 加载技能
gqy skills load <skill-name>
```

## Agent 集群

### Agent 操作

```zsh
# 创建 Agent
gqy agent create --name "研究员" --role "负责技术调研"

# 列出 Agent
gqy agent list

# 与 Agent 对话
gqy agent talk 研究员 "请调研 Rust 异步编程"

# 删除 Agent
gqy agent delete 研究员
```

### 并行任务

多个 Agent 可以并行执行任务：

```
用户：同时调研 Rust 和 Go 的并发模型
GQY：我将创建两个 Agent 并行调研...
```

## Web 面板

### 启动 Web 面板

```zsh
# 默认启动
gqy web

# 自定义配置
gqy web --port 8080 --host 0.0.0.0 -p password
```

### Web 面板功能

- **多通道对话** - 终端、WebUI、QQ、Telegram
- **流式输出** - 实时显示 AI 回复
- **用量分析** - 贡献热力图、费用估算
- **图片支持** - 粘贴/拖拽发送图片
- **对话搜索** - 全文搜索对话历史
- **对话导出** - 导出为 Markdown

## 语音系统

### TTS（文字转语音）

```zsh
# 朗读文字
gqy tts "你好世界"

# 指定音色
gqy tts --voice Ting-Ting "你好"

# 克隆音色
gqy tts --clone "你好世界"

# 列出音色
gqy tts --list
```

### STT（语音转文字）

```zsh
# 语音识别
gqy stt audio.wav
```

## 表情包

### 表情包配置

```zsh
# 启用表情包
gqy config set plugins.memes.enabled true

# 设置发送概率
gqy config set plugins.memes.probability 0.3

# 设置冷却时间
gqy config set plugins.memes.cooldown 30
```

### 表情包管理

```zsh
# 列出表情包
gqy memes list

# 统计信息
gqy memes stats

# 添加表情包
# 在对话中告诉 GQY 添加
```

## 桥接系统

### QQ 桥接（NapCat）

```zsh
# 查看状态
gqy napcat status

# 安装桥接
gqy napcat install

# 配置桥接
gqy napcat config

# 卸载桥接
gqy napcat uninstall
```

### Telegram 桥接

```zsh
# 查看状态
gqy tg status

# 安装桥接
gqy tg install

# 设置 Token
gqy tg token <bot-token>

# 配置桥接
gqy tg config

# 卸载桥接
gqy tg uninstall
```

## 备份恢复

### 备份操作

```zsh
# 初始化备份
gqy backup init

# 立即备份
gqy backup now

# 查看状态
gqy backup status

# 绑定远程仓库
gqy backup remote <url>

# 从远程恢复
gqy backup restore --remote <url>
```

### 备份内容

- 配置文件
- 记忆数据
- 对话历史
- 知识库
- 自定义工具

## 高级功能

### 深度研究

```zsh
# 在对话中使用
gqy "帮我研究一下 Rust 异步运行时的实现原理"
```

### 定时任务

```zsh
# 设置闹钟
gqy alarm set 10m "泡面好了"

# 设置周期提醒
gqy alarm set 25m "番茄钟" --repeat

# 列出闹钟
gqy alarm list

# 取消闹钟
gqy alarm cancel <id>
```

### 玄学功能

```zsh
# 玄学选择
gqy tool xuanxue_pick --options "选项1,选项2,选项3"

# 玄学占卜
gqy tool xuanxue_divine --question "今天运势如何"
```

## 下一步

- [配置指南](../配置指南.md) - 详细配置选项
- [开发指南](../开发指南/README.md) - 开发者指南
- [API参考](../API参考/README.md) - 工具和命令参考