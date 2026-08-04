# GQY 快速开始指南

本指南将帮助您快速上手 GQY，了解基本功能和使用方法。

## 第一次对话

```zsh
# 启动 GQY
gqy

# 或者直接提问
gqy "你好，介绍一下你自己"
```

## 基本对话

### 交互式对话

```zsh
# 进入交互式 REPL
gqy

# 输入问题
> 今天天气怎么样？

# 退出对话
# 按 Ctrl+D 或输入 exit
```

### 单次对话

```zsh
# 直接提问
gqy "帮我写一个 Python 脚本"

# 管道输入
echo "分析这段代码" | gqy
```

### 对话模式

GQY 支持三种对话模式：

| 模式 | 说明 | 用途 |
|------|------|------|
| **Normal** | 完整功能模式 | 日常使用，包含所有工具 |
| **Plan** | 只读分析模式 | 代码审查，不修改文件 |
| **Chat** | 轻量闲聊模式 | 简单对话，节省资源 |

```zsh
# 使用计划模式
gqy --plan "分析这个项目的架构"

# 使用聊天模式
gqy --chat "今天心情不错"
```

## Shell 集成

配置 Shell hook 后，可以在终端直接输入自然语言：

```zsh
# 安装 zsh hook
gqy zsh-init

# 安装 fish hook
gqy fish-init

# 安装 bash hook
gqy bash-init

# 移除所有 hook
gqy remove-shell-hook
```

安装后重启终端，直接输入自然语言即可触发对话：

```zsh
# 直接输入自然语言
帮我看下磁盘空间
今天有什么待办事项
```

## Web 面板

```zsh
# 启动 Web 面板（默认 127.0.0.1:4096）
gqy web

# 自定义端口
gqy web --port 8080

# 监听所有地址（需设置密码）
gqy web --host 0.0.0.0 -p mypassword

# 不自动打开浏览器
gqy web --no-open
```

Web 面板功能：
- 多通道对话（终端 / WebUI / QQ / Telegram）
- 流式输出、思考过程展示
- 用量分析（贡献热力图、费用估算）
- 图片粘贴/拖拽发送
- 对话全文搜索
- 对话导出（Markdown）

## 配置管理

### 查看配置

```zsh
# 查看全部配置
gqy config get

# 查看特定配置
gqy config get active_provider

# TUI 配置界面
gqy config
```

### 修改配置

```zsh
# 设置供应商
gqy config set active_provider deepseek

# 启用插件
gqy config set plugins.web.enabled true

# 设置语言
gqy config set display.language zh
```

## 供应商配置

### 添加供应商

```zsh
# 添加 DeepSeek
gqy provider add https://api.deepseek.com/v1 --api-key sk-xxx

# 添加 OpenAI
gqy provider add https://api.openai.com/v1 --api-key sk-xxx
```

### 管理供应商

```zsh
# 列出供应商
gqy provider list

# 切换供应商
gqy provider switch deepseek

# 删除供应商
gqy provider remove <id>
```

### 对话中切换

直接告诉 GQY：
```
帮我加个供应商，地址 https://api.deepseek.com/v1，key 是 sk-xxx
切到 deepseek
```

## 知识库

```zsh
# 导入知识库
gqy kb add /path/to/documents

# 搜索知识库
gqy kb search "关键词"

# 列出知识库
gqy kb list
```

## 记忆系统

```zsh
# 记住事实
gqy memory remember "用户喜欢使用 Rust"

# 搜索记忆
gqy memory search "编程偏好"

# 列出记忆
gqy memory list

# 删除记忆
gqy memory forget <id>
```

## 工具使用

```zsh
# 列出工具
gqy tools list

# 查看工具详情
gqy tools info web_search

# 注册脚本工具
gqy scripts register /path/to/script.sh --name "我的脚本"
```

## 常用命令

| 命令 | 说明 |
|------|------|
| `gqy` | 启动交互式对话 |
| `gqy "问题"` | 单次对话 |
| `gqy web` | 启动 Web 面板 |
| `gqy config` | TUI 配置界面 |
| `gqy provider list` | 列出供应商 |
| `gqy kb list` | 列出知识库 |
| `gqy memory list` | 列出记忆 |
| `gqy tools list` | 列出工具 |
| `gqy doctor` | 运行诊断 |
| `gqy --version` | 查看版本 |

## 快捷键

| 快捷键 | 说明 |
|--------|------|
| `Ctrl+O` | 展开/收起思考详情 |
| `Ctrl+C` | 中断当前回复 |
| `Ctrl+D` | 退出对话 |
| `Tab` | 自动补全 |
| `↑/↓` | 浏览历史命令 |

## 下一步

- [功能详解](../功能详解/README.md) - 了解所有功能
- [配置指南](../配置指南.md) - 详细配置选项
- [故障排除](../故障排除/README.md) - 常见问题解决