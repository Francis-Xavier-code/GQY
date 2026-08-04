# Agent 集群

GQY 的 Agent 集群系统允许创建多个独立的 Agent，每个 Agent 有自己的角色、工具和对话历史。Agent 之间可以协作完成复杂任务，支持并行执行和通信。

## 核心概念

### 1. Agent 定义
- **名称**：Agent 的唯一标识符
- **角色**：Agent 的职责和专长描述
- **工具**：Agent 可使用的工具集
- **历史**：Agent 的对话历史

### 2. Agent 实例
- **独立进程**：每个 Agent 运行在独立的进程中
- **独立记忆**：每个 Agent 有自己的对话历史
- **工具隔离**：Agent 使用过滤后的工具清单，防止递归创建

### 3. Agent 管理
- **全局管理器**：进程级单例管理所有 Agent
- **持久化存储**：Agent 定义存储在 `GQY_HOME/data/agents/agents.json`
- **懒加载**：Agent 进程按需启动

## 使用方法

### 命令行工具

```zsh
# 创建新 Agent
gqy agent create --name "研究员" --role "负责技术调研和资料收集"

# 列出所有 Agent
gqy agent list

# 查看 Agent 详情
gqy agent info 研究员

# 与 Agent 对话
gqy agent talk 研究员 "请调研 Rust 异步编程最佳实践"

# 删除 Agent
gqy agent delete 研究员
```

### 对话中使用

在对话中，GQY 可以自主创建和管理 Agent：

```
用户：帮我调研一下 Rust 异步编程的最佳实践
GQY：我来创建一个研究员 Agent 帮您调研。
     [创建 Agent "研究员"，角色：技术调研专家]
     研究员正在调研...
     调研完成，以下是主要发现：...
```

### 并行任务

多个 Agent 可以并行执行任务：

```
用户：同时调研 Rust 和 Go 的并发模型
GQY：我将创建两个 Agent 并行调研：
     1. Rust专家 - 调研 Rust 并发模型
     2. Go专家 - 调研 Go 并发模型
     
     [两个 Agent 并行执行]
     
     Rust专家：Rust 使用 async/await 和 tokio...
     Go专家：Go 使用 goroutine 和 channel...
```

## Agent 类型

### 1. 通用 Agent
- **角色**：通用助手
- **工具**：完整工具集
- **用途**：处理各种任务

### 2. 专业 Agent
- **角色**：特定领域专家
- **工具**：领域相关工具
- **用途**：处理专业任务

### 3. 子 Agent
- **角色**：临时任务执行者
- **工具**：过滤后的工具集
- **用途**：处理特定子任务

## 配置选项

### Agent 配置（config.jsonc）

```jsonc
{
  "agents": {
    "max_agents": 16,
    "max_history_turns": 20,
    "auto_create": true,
    "parallel_execution": true
  }
}
```

### 配置说明

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `max_agents` | `16` | 最大 Agent 数量 |
| `max_history_turns` | `20` | 每个 Agent 最大历史轮数 |
| `auto_create` | `true` | 允许自动创建 Agent |
| `parallel_execution` | `true` | 允许并行执行任务 |

## 存储结构

### 数据目录
```
GQY_HOME/data/agents/
├── agents.json        # Agent 定义文件
├── 研究员/            # Agent 专属目录
│   ├── history.json   # 对话历史
│   └── state.json     # 状态信息
└── ...
```

### Agent 定义格式
```json
[
  {
    "name": "研究员",
    "role": "负责技术调研和资料收集",
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

## 高级功能

### 1. Agent 通信
- Agent 之间可以发送消息
- 支持请求-响应模式
- 支持广播消息

### 2. 任务分配
- 自动将任务分配给合适的 Agent
- 支持任务优先级
- 支持任务依赖关系

### 3. 结果聚合
- 收集所有 Agent 的结果
- 合并和总结结果
- 生成最终报告

## 使用场景

### 1. 技术调研
```
用户：调研 2024 年最流行的编程语言
GQY：创建多个 Agent 并行调研：
     - Web开发专家 - 调研 Web 开发语言
     - 数据科学专家 - 调研数据科学语言
     - 系统编程专家 - 调研系统编程语言
```

### 2. 代码审查
```
用户：审查这个 PR 的代码质量
GQY：创建多个 Agent 并行审查：
     - 安全专家 - 检查安全问题
     - 性能专家 - 检查性能问题
     - 可读性专家 - 检查代码可读性
```

### 3. 文档生成
```
用户：为这个项目生成文档
GQY：创建多个 Agent 并行生成：
     - API文档专家 - 生成 API 文档
     - 用户指南专家 - 生成用户指南
     - 架构文档专家 - 生成架构文档
```

## 最佳实践

### 1. 明确角色定义
为每个 Agent 定义清晰的角色和职责：
```
✅ "研究员：负责技术调研，擅长搜索和总结资料"
❌ "助手：帮忙干活"
```

### 2. 合理分配任务
根据 Agent 的专长分配任务：
```
✅ 将技术调研任务分配给研究员
❌ 将技术调研任务分配给客服 Agent
```

### 3. 控制 Agent 数量
避免创建过多 Agent：
```zsh
# 查看当前 Agent 数量
gqy agent list

# 清理不需要的 Agent
gqy agent delete 旧Agent
```

## 故障排除

### Agent 创建失败
1. 检查 Agent 数量限制：`gqy agent list`
2. 检查配置：`gqy config get agents.max_agents`
3. 清理不需要的 Agent

### Agent 响应缓慢
1. 检查并行执行配置：`gqy config get agents.parallel_execution`
2. 减少同时运行的 Agent 数量
3. 检查网络连接

### Agent 通信失败
1. 检查 Agent 状态：`gqy agent info Agent名`
2. 重启 Agent：`gqy agent restart Agent名`
3. 检查日志：`cat GQY_HOME/logs/agent.log`

## 相关文档

- [记忆系统](记忆系统.md) - 记忆存储与搜索
- [工具系统](工具系统.md) - GQY 工具系统概述
- [架构说明](../架构说明.md) - GQY 系统架构