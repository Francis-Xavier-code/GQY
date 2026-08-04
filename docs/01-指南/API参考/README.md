# GQY API 参考

本参考手册提供 GQY 所有工具和命令的详细说明。

## 命令行命令

### 基本命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy` | 启动交互式对话 | `gqy` |
| `gqy "问题"` | 单次对话 | `gqy "今天天气"` |
| `gqy --version` | 查看版本 | `gqy --version` |
| `gqy --help` | 查看帮助 | `gqy --help` |
| `gqy doctor` | 运行诊断 | `gqy doctor` |

### 对话模式

| 模式 | 说明 | 示例 |
|------|------|------|
| `--plan` | 只读分析模式 | `gqy --plan "分析代码"` |
| `--chat` | 轻量闲聊模式 | `gqy --chat "你好"` |
| `--dry-run` | 干运行模式（不执行工具） | `gqy --dry-run "任务"` |

### 配置命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy config` | TUI 配置界面 | `gqy config` |
| `gqy config get` | 查看全部配置 | `gqy config get` |
| `gqy config get <key>` | 查看特定配置 | `gqy config get active_provider` |
| `gqy config set <key> <value>` | 设置配置 | `gqy config set active_provider deepseek` |
| `gqy config validate` | 验证配置 | `gqy config validate` |
| `gqy config paths` | 查看配置路径 | `gqy config paths` |

### 供应商命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy provider list` | 列出供应商 | `gqy provider list` |
| `gqy provider add <url>` | 添加供应商 | `gqy provider add https://api.deepseek.com/v1 --api-key sk-xxx` |
| `gqy provider switch <id>` | 切换供应商 | `gqy provider switch deepseek` |
| `gqy provider remove <id>` | 删除供应商 | `gqy provider remove deepseek` |

### 知识库命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy kb add <path>` | 导入知识库 | `gqy kb add /path/to/docs` |
| `gqy kb search <query>` | 搜索知识库 | `gqy kb search "关键词"` |
| `gqy kb list` | 列出知识库 | `gqy kb list` |
| `gqy kb remove <id>` | 删除文档 | `gqy kb remove doc-123` |

### 记忆命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy memory remember <content>` | 记住事实 | `gqy memory remember "用户喜欢 Rust"` |
| `gqy memory search <query>` | 搜索记忆 | `gqy memory search "编程偏好"` |
| `gqy memory list` | 列出记忆 | `gqy memory list` |
| `gqy memory forget <id>` | 删除记忆 | `gqy memory forget 123` |
| `gqy memory stats` | 记忆统计 | `gqy memory stats` |
| `gqy memory clear` | 清空记忆 | `gqy memory clear` |

### 工具命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy tools list` | 列出工具 | `gqy tools list` |
| `gqy tools info <name>` | 工具详情 | `gqy tools info web_search` |
| `gqy tools stats` | 工具统计 | `gqy tools stats` |

### 脚本命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy scripts register <path>` | 注册脚本 | `gqy scripts register /path/to/script.sh --name "我的脚本"` |
| `gqy scripts list` | 列出脚本 | `gqy scripts list` |
| `gqy scripts unregister <name>` | 注销脚本 | `gqy scripts unregister my-script` |

### 技能命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy skills register <path>` | 注册技能 | `gqy skills register /path/to/skill.json` |
| `gqy skills list` | 列出技能 | `gqy skills list` |
| `gqy skills load <name>` | 加载技能 | `gqy skills load my-skill` |

### Agent 命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy agent create` | 创建 Agent | `gqy agent create --name "研究员" --role "技术调研"` |
| `gqy agent list` | 列出 Agent | `gqy agent list` |
| `gqy agent info <name>` | Agent 详情 | `gqy agent info 研究员` |
| `gqy agent talk <name> <message>` | 与 Agent 对话 | `gqy agent talk 研究员 "调研 Rust"` |
| `gqy agent delete <name>` | 删除 Agent | `gqy agent delete 研究员` |

### Web 命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy web` | 启动 Web 面板 | `gqy web` |
| `gqy web --port <port>` | 自定义端口 | `gqy web --port 8080` |
| `gqy web --host <host>` | 监听地址 | `gqy web --host 0.0.0.0` |
| `gqy web -p <password>` | 设置密码 | `gqy web -p mypassword` |
| `gqy web --no-open` | 不自动打开浏览器 | `gqy web --no-open` |

### 桥接命令

#### QQ 桥接（NapCat）

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy napcat status` | 查看状态 | `gqy napcat status` |
| `gqy napcat install` | 安装桥接 | `gqy napcat install` |
| `gqy napcat config` | 配置桥接 | `gqy napcat config` |
| `gqy napcat uninstall` | 卸载桥接 | `gqy napcat uninstall` |

#### Telegram 桥接

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy tg status` | 查看状态 | `gqy tg status` |
| `gqy tg install` | 安装桥接 | `gqy tg install` |
| `gqy tg token <token>` | 设置 Token | `gqy tg token 123456:ABC-DEF` |
| `gqy tg config` | 配置桥接 | `gqy tg config` |
| `gqy tg uninstall` | 卸载桥接 | `gqy tg uninstall` |

### 备份命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy backup init` | 初始化备份 | `gqy backup init` |
| `gqy backup now` | 立即备份 | `gqy backup now` |
| `gqy backup status` | 查看状态 | `gqy backup status` |
| `gqy backup remote <url>` | 绑定远程仓库 | `gqy backup remote https://github.com/user/repo.git` |
| `gqy backup restore --remote <url>` | 从远程恢复 | `gqy backup restore --remote https://github.com/user/repo.git` |

### 语音命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy tts <text>` | 文字转语音 | `gqy tts "你好世界"` |
| `gqy tts --voice <voice>` | 指定音色 | `gqy tts --voice Ting-Ting "你好"` |
| `gqy tts --clone <text>` | 克隆音色 | `gqy tts --clone "你好世界"` |
| `gqy tts --list` | 列出音色 | `gqy tts --list` |
| `gqy stt <file>` | 语音转文字 | `gqy stt audio.wav` |

### 表情包命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy memes list` | 列出表情包 | `gqy memes list` |
| `gqy memes stats` | 统计信息 | `gqy memes stats` |

### 闹钟命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy alarm set <time> <message>` | 设置闹钟 | `gqy alarm set 10m "泡面好了"` |
| `gqy alarm set <time> <message> --repeat` | 周期提醒 | `gqy alarm set 25m "番茄钟" --repeat` |
| `gqy alarm list` | 列出闹钟 | `gqy alarm list` |
| `gqy alarm cancel <id>` | 取消闹钟 | `gqy alarm cancel 123` |
| `gqy alarm stop --all` | 停止所有 | `gqy alarm stop --all` |

### 其他命令

| 命令 | 说明 | 示例 |
|------|------|------|
| `gqy models` | 列出模型 | `gqy models` |
| `gqy models <index>` | 切换模型 | `gqy models 1` |
| `gqy variant` | 设置思考档位 | `gqy variant` |
| `gqy variant <name>` | 指定档位 | `gqy variant high` |
| `gqy pop` | 弹出上下文 | `gqy pop` |
| `gqy pop <count>` | 弹出指定轮数 | `gqy pop 5` |
| `gqy history` | 查看历史 | `gqy history` |
| `gqy activity` | 查看活动日志 | `gqy activity` |
| `gqy archive` | 归档对话 | `gqy archive` |
| `gqy reset` | 重置状态 | `gqy reset` |
| `gqy balance` | 查看余额 | `gqy balance` |
| `gqy paths` | 查看路径 | `gqy paths` |

## 内置工具

### 文件操作工具

#### read_file
读取文件内容。

**参数**:
- `path` (string, required): 文件路径

**返回**: 文件内容

**示例**:
```json
{
  "path": "/path/to/file.txt"
}
```

#### write_file
写入文件内容。

**参数**:
- `path` (string, required): 文件路径
- `content` (string, required): 文件内容

**返回**: 写入结果

**示例**:
```json
{
  "path": "/path/to/file.txt",
  "content": "Hello, World!"
}
```

#### edit_file
编辑文件内容。

**参数**:
- `path` (string, required): 文件路径
- `old_text` (string, required): 要替换的文本
- `new_text` (string, required): 新文本

**返回**: 编辑结果

**示例**:
```json
{
  "path": "/path/to/file.txt",
  "old_text": "old",
  "new_text": "new"
}
```

#### list_directory
列出目录内容。

**参数**:
- `path` (string, required): 目录路径

**返回**: 目录内容列表

**示例**:
```json
{
  "path": "/path/to/dir"
}
```

#### create_directory
创建目录。

**参数**:
- `path` (string, required): 目录路径

**返回**: 创建结果

**示例**:
```json
{
  "path": "/path/to/new/dir"
}
```

#### trash_path
移动到回收站。

**参数**:
- `path` (string, required): 文件/目录路径

**返回**: 操作结果

**示例**:
```json
{
  "path": "/path/to/file.txt"
}
```

#### glob
查找文件。

**参数**:
- `pattern` (string, required): 匹配模式
- `path` (string, optional): 搜索路径

**返回**: 匹配的文件列表

**示例**:
```json
{
  "pattern": "*.rs",
  "path": "/path/to/dir"
}
```

#### grep
搜索文本。

**参数**:
- `pattern` (string, required): 搜索模式
- `path` (string, optional): 搜索路径
- `include` (string, optional): 文件匹配模式

**返回**: 匹配结果

**示例**:
```json
{
  "pattern": "fn main",
  "path": "/path/to/dir",
  "include": "*.rs"
}
```

### 系统工具

#### run_command
运行命令。

**参数**:
- `command` (string, required): 命令
- `workdir` (string, optional): 工作目录

**返回**: 命令输出

**示例**:
```json
{
  "command": "ls -la",
  "workdir": "/path/to/dir"
}
```

#### get_current_directory
获取当前目录。

**参数**: 无

**返回**: 当前目录路径

#### get_current_time
获取当前时间。

**参数**: 无

**返回**: 当前时间

#### check_os_info
检查系统信息。

**参数**: 无

**返回**: 系统信息

#### read_clipboard
读取剪贴板。

**参数**: 无

**返回**: 剪贴板内容

### 网络工具

#### web_search
网络搜索。

**参数**:
- `query` (string, required): 搜索查询
- `max_results` (number, optional): 最大结果数

**返回**: 搜索结果

**示例**:
```json
{
  "query": "Rust 编程",
  "max_results": 10
}
```

#### web_fetch
获取网页内容。

**参数**:
- `url` (string, required): 网页 URL

**返回**: 网页内容

**示例**:
```json
{
  "url": "https://example.com"
}
```

#### search_web_images
搜索网络图片。

**参数**:
- `query` (string, required): 搜索查询
- `max_results` (number, optional): 最大结果数

**返回**: 图片搜索结果

**示例**:
```json
{
  "query": "Rust logo",
  "max_results": 5
}
```

### 知识库工具

#### search_knowledge_base
搜索知识库。

**参数**:
- `query` (string, required): 搜索查询
- `limit` (number, optional): 结果数量限制

**返回**: 搜索结果

**示例**:
```json
{
  "query": "如何优化性能",
  "limit": 10
}
```

#### upload_knowledge_base_file
上传知识库文件。

**参数**:
- `path` (string, required): 文件路径

**返回**: 上传结果

**示例**:
```json
{
  "path": "/path/to/document.pdf"
}
```

#### read_knowledge_base_file
读取知识库文件。

**参数**:
- `id` (string, required): 文档 ID

**返回**: 文档内容

**示例**:
```json
{
  "id": "doc-123"
}
```

### 记忆工具

#### remember_fact
记住事实。

**参数**:
- `content` (string, required): 事实内容
- `source` (string, optional): 来源

**返回**: 记忆 ID

**示例**:
```json
{
  "content": "用户喜欢使用 Rust",
  "source": "对话"
}
```

#### recall_memory
回忆记忆。

**参数**:
- `query` (string, required): 查询
- `limit` (number, optional): 结果数量限制

**返回**: 记忆列表

**示例**:
```json
{
  "query": "编程偏好",
  "limit": 5
}
```

#### forget_memory
遗忘记忆。

**参数**:
- `id` (number, required): 记忆 ID

**返回**: 操作结果

**示例**:
```json
{
  "id": 123
}
```

#### list_memory
列出记忆。

**参数**:
- `limit` (number, optional): 结果数量限制

**返回**: 记忆列表

**示例**:
```json
{
  "limit": 20
}
```

### 媒体工具

#### analyze_image
分析图片。

**参数**:
- `path` (string, required): 图片路径
- `prompt` (string, optional): 分析提示

**返回**: 分析结果

**示例**:
```json
{
  "path": "/path/to/image.jpg",
  "prompt": "描述这张图片"
}
```

#### generate_image
生成图片。

**参数**:
- `prompt` (string, required): 生成提示
- `size` (string, optional): 图片尺寸

**返回**: 图片路径

**示例**:
```json
{
  "prompt": "一只可爱的猫",
  "size": "1024x1024"
}
```

#### print_image
显示图片。

**参数**:
- `path` (string, required): 图片路径

**返回**: 显示结果

**示例**:
```json
{
  "path": "/path/to/image.jpg"
}
```

### 专业工具

#### weather
天气查询。

**参数**:
- `city` (string, required): 城市名

**返回**: 天气信息

**示例**:
```json
{
  "city": "北京"
}
```

#### exchange_rate
汇率查询。

**参数**:
- `from` (string, required): 源货币
- `to` (string, required): 目标货币

**返回**: 汇率信息

**示例**:
```json
{
  "from": "USD",
  "to": "CNY"
}
```

#### calculate
科学计算。

**参数**:
- `expression` (string, required): 计算表达式

**返回**: 计算结果

**示例**:
```json
{
  "expression": "sqrt(16) + 2^3"
}
```

### 玄学工具

#### xuanxue_pick
玄学选择。

**参数**:
- `options` (string, required): 选项列表（逗号分隔）

**返回**: 选择结果

**示例**:
```json
{
  "options": "选项1,选项2,选项3"
}
```

#### xuanxue_divine
玄学占卜。

**参数**:
- `question` (string, required): 占卜问题

**返回**: 占卜结果

**示例**:
```json
{
  "question": "今天运势如何"
}
```

## 配置参考

### 主配置文件

配置文件位置: `~/.config/gqy/config.jsonc`

```jsonc
{
  // 供应商配置
  "active_provider": "opencode",
  "providers": [...],
  
  // 工具配置
  "tools": {
    "enabled": true,
    "max_rounds": 10,
    "loading_mode": "hybrid"
  },
  
  // 插件配置
  "plugins": {
    "web": { "enabled": true },
    "knowledge_base": { "enabled": true },
    "memory": { "enabled": true }
  },
  
  // 显示配置
  "display": {
    "language": "auto",
    "reasoning": "summary",
    "tool_calls": "summary"
  },
  
  // 记忆配置
  "memory": {
    "enabled": true,
    "auto_fact_enabled": true,
    "auto_diary_enabled": true
  }
}
```

## 环境变量参考

| 变量名 | 默认值 | 说明 |
|--------|--------|------|
| `GQY_HOME` | `~/Library/Application Support/gqy` | 数据根目录 |
| `GQY_SHARE_DIR` | 自动检测 | 共享资源目录 |
| `GQY_PROJECT_DIR` | `~/Desktop/GQY` | 项目源码目录 |
| `GQY_WORKSPACE` | `~/gqy-workspace` | 临时工作区 |
| `GQY_LANG` | `auto` | 界面语言 |
| `GQY_CHANNEL` | 自动检测 | 通信通道 |
| `GQY_ALLOW_PROJECT_WRITES` | `0` | 允许写入项目目录 |

## 下一步

- [故障排除](../故障排除/README.md) - 常见问题解决
- [架构说明](../架构说明/README.md) - 系统架构概述
- [开发指南](../开发指南/README.md) - 开发者指南