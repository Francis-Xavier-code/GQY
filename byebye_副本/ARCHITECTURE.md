# GQY 架构文档

## 项目概览

GQY 是一个用 Rust 编写的命令行 AI 助手，支持多种 LLM 后端，具备丰富的工具系统和自主 Agent 能力。

## 核心架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLI / REPL                              │
│  (clap 解析参数, rustyline 交互式输入, crossterm 终端控制)       │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Agent 层                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ Normal 模式 │  │  Plan 模式  │  │  Chat 模式  │             │
│  │ (完整工具)  │  │ (只读工具)  │  │ (轻量工具)  │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│                                                                 │
│  AgentTurnControl: 管理模式切换与工具注册表                      │
│  Agent: 核心对话循环，管理上下文、工具调用、溢出处理             │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        LLM 客户端层                             │
│  ┌─────────────────────┐  ┌─────────────────────┐              │
│  │ OpenAI Compatible   │  │    Pi RPC Client    │              │
│  │ (直连 API)          │  │ (独立进程通信)      │              │
│  └─────────────────────┘  └─────────────────────┘              │
│                                                                 │
│  LlmClient: 统一接口，支持流式输出、工具调用、思考链             │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        工具系统                                 │
│  ┌────────────────────────────────────────────────────────┐    │
│  │                   ToolRegistry                         │    │
│  │  - 注册/查找/调用工具                                  │    │
│  │  - 权限控制 (读/写)                                    │    │
│  │  - 混合加载模式 (hybrid/lazy)                          │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                 │
│  工具分类:                                                      │
│  ├── 文件操作: read_file, write_file, edit_file, glob, grep   │
│  ├── Shell: run_command (bash/fish/zsh)                        │
│  ├── 网络: web_search, web_fetch, web_images                  │
│  ├── 知识库: knowledge_base CRUD                              │
│  ├── 记忆: remember_fact, recall_memory, search_evicted       │
│  ├── 任务: task (子代理), deep_research                       │
│  ├── Agent集群: spawn_agent, talk_to_agent, parallel_agents   │
│  ├── MCP: 外部工具协议                                        │
│  └── 其他: 计算器、哈希、汇率、天气、表情包...               │
└─────────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        状态与存储                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐            │
│  │ StateStore  │  │ MemoryStore │  │ SQLite DB   │            │
│  │ (对话状态)  │  │ (记忆系统)  │  │ (持久化)    │            │
│  └─────────────┘  └─────────────┘  └─────────────┘            │
│                                                                 │
│  - 对话历史: turns 表，支持归档/压缩/弹出                      │
│  - 记忆分片: MEMORY.md 索引 + 分片文件                         │
│  - 用量统计: usage.json                                        │
│  - 知识库: FTS5 全文搜索                                       │
└─────────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        配置系统                                 │
│  config.jsonc:                                                  │
│  ├── providers[]: 供应商配置 (URL, API Key, 模型列表)          │
│  ├── active_provider: 当前激活供应商                            │
│  ├── context: 上下文窗口配置                                   │
│  ├── tools: 工具开关、加载模式、最大轮数                       │
│  ├── plugins: 各功能插件开关                                   │
│  ├── display: 显示配置 (语言、推理展示、工具调用展示)          │
│  ├── prompt: 人格/身份配置                                     │
│  └── memory: 记忆系统配置                                      │
└─────────────────────────────────────────────────────────────────┘
```

## 核心模块详解

### 1. Agent 系统 (`src/agent/`)

Agent 是对话的核心，负责：
- **对话循环**: 接收用户输入 → 调用 LLM → 处理工具调用 → 返回结果
- **上下文管理**: 加载历史、压缩溢出、归档旧轮次
- **模式切换**: Normal (完整功能) / Plan (只读) / Chat (轻量闲聊)

```rust
pub struct Agent {
    state: StateStore,          // 对话状态
    client: LlmClient,          // LLM 客户端
    system_prompt: String,      // 系统提示词
    tools: ToolRegistry,        // 工具注册表
    memory: MemoryStore,        // 记忆存储
    mode: AgentMode,            // 当前模式
    config: AppConfig,          // 配置
    // ...
}
```

**关键流程**:
1. `chat_stream()`: 主对话入口，处理流式输出
2. `tool_loop`: 工具调用循环，支持多轮工具调用
3. `handle_overflow_after_turn()`: 上下文溢出时自动压缩
4. `consume_queued_prompts()`: 处理排队的用户输入

### 2. LLM 客户端 (`src/llm/`)

统一的 LLM 接口，支持两种后端：

```rust
pub enum LlmClient {
    OpenAi(OpenAiCompatibleClient),  // OpenAI 兼容 API
    Pi(PiRpcClient),                 // Pi 独立进程 RPC
}
```

**OpenAI Compatible**:
- 支持所有 OpenAI 兼容 API (OpenAI, DeepSeek, Claude, 本地模型等)
- 流式 SSE 输出
- 工具调用 (function calling)
- 思考链 (reasoning) 支持
- 前缀缓存 (prompt caching)

**Pi RPC**:
- 独立进程运行，通过 JSON-RPC 通信
- 自管理上下文与压缩
- 工具在子进程内执行

### 3. 工具系统 (`src/tools/`)

工具注册表管理所有可用工具：

```rust
pub struct ToolRegistry {
    tools: HashMap<String, ToolSpec>,
    // ...
}

pub struct ToolSpec {
    name: String,
    description: String,
    parameters: Value,  // JSON Schema
    handler: Box<dyn Fn(Value) -> Future<Output = Result<String>>>,
    permission: ToolPermission,  // Read / Write
    always_loaded: bool,         // 是否始终加载
}
```

**工具加载模式**:
- `eager`: 全部加载（默认）
- `hybrid` / `lazy`: 按需加载，通过 `load_tools` 工具动态加载

**核心工具**:
- **文件操作**: `read_file`, `write_file`, `edit_file`, `apply_patch`
- **Shell**: `run_command` (支持 bash/fish/zsh)
- **搜索**: `web_search`, `web_fetch`, `search_knowledge_base`
- **记忆**: `remember_fact`, `recall_memory`, `search_evicted_context`
- **任务**: `task` (子代理), `deep_research` (深度研究)
- **Agent集群**: `spawn_agent`, `talk_to_agent`, `parallel_agents`

### 4. 子代理集群 (`src/agents.rs`)

Kimi 式自主 Agent 系统，模型可自建/管理命名子代理：

```rust
pub struct AgentManager {
    agents: RwLock<HashMap<String, AgentInstance>>,
    tools: ToolRegistry,
}

struct AgentInstance {
    def: AgentDef,           // 名称、角色
    client: LlmClient,       // 独立 LLM 客户端
    history: Vec<ChatMessage>, // 多轮记忆
    talk_lock: Mutex<()>,    // 串行化 talk
}
```

**能力**:
- `spawn_agent`: 创建/更新命名 agent
- `talk_to_agent`: 与 agent 对话
- `parallel_agents`: 并发执行多个 agent 任务
- `kill_agent`: 销毁 agent

**递归防护**: 子 agent 不可调用 `spawn_agent`, `task`, `deep_research` 等递归工具

### 5. 状态管理 (`src/state/`)

SQLite 存储对话历史：

```rust
pub struct StateStore {
    conv_db: Arc<ConversationDb>,
    queue_session_id: Arc<str>,
    // ...
}
```

**核心功能**:
- **Turn 生命周期**: `start_turn` → `complete_turn` / `interrupt_turn`
- **可见轮次**: `load_visible_turns_for_mode()` 按模式隔离
- **归档**: `archive_and_delete_visible_turns()` 归档到 evicted_context.db
- **排队提示**: `enqueue_prompt()` 支持 WebUI 排队输入

### 6. 上下文管理

**溢出处理**:
```rust
fn trim_visible_context(&self) -> Result<Vec<StoredConversationEntry>> {
    // 1. 计算当前 token 数
    // 2. 超过 trim_at_ratio 时触发
    // 3. 删除最旧的轮次直到低于 target
    // 4. 归档到 evicted_context.db
}
```

**压缩 (Compact)**:
- 当上下文接近窗口限制时，自动触发压缩
- 调用 LLM 总结历史，生成摘要轮次

### 7. 记忆系统 (`src/memory/`)

```rust
pub struct MemoryStore {
    // MEMORY.md 索引 + 分片文件
}
```

**功能**:
- `remember_fact`: 记录事实
- `recall_memory`: 检索记忆
- `search_evicted_context`: 搜索归档上下文
- 分片存储，避免单文件过大

### 8. 配置系统 (`src/config.rs`)

```rust
pub struct AppConfig {
    pub active_provider: String,
    pub providers: Vec<ProviderConfig>,
    pub context: ContextConfig,
    pub tools: ToolsConfig,
    pub plugins: PluginsConfig,
    pub display: DisplayConfig,
    pub prompt: PromptConfig,
    pub memory: MemoryConfig,
    // ...
}
```

**供应商配置**:
```rust
pub struct ProviderConfig {
    pub id: String,
    pub base_url: String,
    pub protocol: String,  // "openai-chat" | "pi"
    pub api_key: Option<String>,
    pub models: Vec<String>,
    pub model_context_window: HashMap<String, usize>,
    pub default_model: String,
    pub temperature: f32,
    // ...
}
```

### 9. 渲染系统 (`src/render/`)

流式渲染器，处理：
- Markdown 格式化输出
- 思考链 (reasoning) 展示
- 工具调用展示
- 命令输出流
- 等待动画

### 10. Shell 集成 (`src/shell/`)

支持 fish/bash/zsh 的自然语言命令拦截：

```bash
# fish 示例
function gqy_intercept
    gqy --shell-intercept --shell fish -- $argv
end
```

**分类器**: `shell::classify_with_confidence()` 判断是否为自然语言命令

### 11. WebUI (`src/web/`)

Axum 提供的 Web 接口：
- REST API 对话
- SSE 流式输出
- 会话管理
- 配置操作

### 12. 桥接系统 (`src/bridges/`)

- **Napcat**: QQ 机器人桥接
- **Telegram**: Telegram 机器人桥接

### 13. MCP 协议 (`src/tools/mcp.rs`)

Model Context Protocol 支持，可连接外部工具服务器

## 数据流

```
用户输入
    │
    ▼
CLI/REPL 解析
    │
    ▼
Agent.chat_stream()
    │
    ├─→ 加载历史上下文
    │
    ├─→ 构建消息 (system + history + user)
    │
    ▼
LlmClient.chat_stream()
    │
    ├─→ 流式接收 LLM 输出
    │
    ├─→ 检测工具调用
    │       │
    │       ▼
    │   ToolRegistry.call()
    │       │
    │       ▼
    │   执行工具，返回结果
    │       │
    │       ▼
    │   继续对话循环 (直到无工具调用)
    │
    ▼
保存 Turn 到 StateStore
    │
    ▼
检查上下文溢出
    │
    ├─→ 溢出则压缩/归档
    │
    ▼
返回结果给用户
```

## 关键设计决策

### 1. 双 LLM 后端
- **OpenAI Compatible**: 通用，支持所有兼容 API
- **Pi RPC**: 独立进程，自管理上下文，适合本地模型

### 2. 工具混合加载
- 大量工具时，不全部加载到上下文
- 通过 `load_tools` 按需加载，节省 token

### 3. 子代理集群
- 模型可自主创建/管理命名 agent
- 并行执行任务
- 递归防护，避免无限创建

### 4. 上下文三层管理
- **可见轮次**: 当前对话上下文
- **归档轮次**: evicted_context.db，可检索
- **压缩摘要**: LLM 生成的历史摘要

### 5. 模式隔离
- Normal: 完整功能
- Plan: 只读工具，适合规划
- Chat: 轻量闲聊，独立历史

## 文件结构

```
~/.config/gqy/           # 配置目录
├── config.jsonc         # 主配置
├── skills/              # 技能定义
├── scripts/             # 用户脚本
└── prompts/             # 自定义提示词

~/.local/share/gqy/      # 数据目录
├── data/
│   ├── agents/          # Agent 定义
│   └── kb/              # 知识库
├── state/
│   ├── conversation.db  # 对话历史
│   ├── evicted_context.db # 归档上下文
│   └── usage.json       # 用量统计
└── cache/               # 缓存

~/.local/state/gqy/      # 状态目录
└── logs/                # 诊断日志
```

## 扩展点

1. **自定义工具**: 通过 scripts/ 目录添加脚本工具
2. **技能系统**: skills/ 目录定义复合技能
3. **MCP 服务器**: 连接外部工具服务器
4. **人格系统**: prompts/ 目录自定义人格
5. **桥接**: 实现新的消息平台桥接

## 性能优化

1. **前缀缓存**: 支持 Anthropic/DeepSeek 的 prompt caching
2. **混合工具加载**: 减少上下文 token 消耗
3. **流式输出**: 实时显示，改善用户体验
4. **后台备份**: 异步备份，不阻塞对话
5. **SQLite WAL**: 并发读写优化

## 安全考虑

1. **API Key 保护**: 配置脱敏，不暴露在日志
2. **工具权限**: Read/Write 分级控制
3. **路径保护**: 危险路径需确认
4. **递归防护**: 子 agent 不能创建无限递归
