# pi 底座模式（实验性）

GQY 的「大脑」可以整体换成 [pi](https://github.com/earendil-works/pi)（`@earendil-works/pi-coding-agent`）。
GQY 通过 `pi --mode rpc` 把每一轮对话交给 pi，由 pi 用自己的模型、自己的 agent 循环和内置工具
（read / write / edit / bash / find / grep / ls）完成回复与工具调用，GQY 负责渲染、记忆、知识库、备份等外围能力。

> 状态：实验性。已完成：纯对话链路、GQY 定制工具经 pi extension 注入（`gqy_*`）、
> 记忆/知识库/表情包/闹钟/玄学等工具端到端可用；工具带使用引导（promptSnippet/guidelines）
> 且同步注册，首轮对话模型即会主动选用 gqy_* 工具。

## 前置条件

1. 本机已安装 pi 且能正常对话：

   ```bash
   pi --version
   pi          # 确认模型已配置（/login 或 API key）
   ```

2. pi 的可执行文件在 PATH 中（菜单栏 App 场景可用 `GQY_PI_BIN` 指定绝对路径）。

## 开启 pi 底座

在 `gqy config` 中添加一个 provider，或直接编辑 `$GQY_HOME/config/config.jsonc`：

```jsonc
{
  "active_provider": "pi",
  "providers": [
    {
      "id": "pi",
      "display_name": "pi (RPC)",
      "base_url": "pi://local",
      "protocol": "pi"
    }
  ]
}
```

要点：

- `protocol` 必须是 `pi`（TUI 的协议下拉里也有该选项）；
- `base_url`、`api_key`、`models` 等字段在 pi 模式下**全部忽略**——模型由 pi 自己管理，
  GQY 配置里的模型/密钥一概不参与；
- pi 模式不需要 `default_model`（已豁免空模型时的默认 provider 回退逻辑）。

## 工作原理

```text
GQY (Rust) ──spawn──▶ pi --mode rpc --no-session --no-context-files \
                        --extension src/scripts/pi-bridge.ts \
                        --append-system-prompt <顾清影人格.md> --name gqy
   │                        │
   │  JSONL prompt(消息)     │  pi 自己跑 agent 循环：
   │◀─── 流式事件 ───────────┤  模型 → 思考 → 工具调用(bash/read/edit/gqy_*) → 结论
   │                        │
   └── 本地 HTTP (127.0.0.1:随机端口) ◀── pi-bridge.ts 扩展回调：
        GET  /tools  工具清单        │   模型调用 gqy_* 工具时，扩展 POST /tool
        POST /tool   执行工具        │   回 GQY，由 GQY 自己的 ToolRegistry 执行
```

- **进程生命周期**：首次对话按 GQY 的 system prompt（人格）spawn 一个长驻 pi 进程，
  之后人格不变则复用（pi 在进程内维护对话历史）；人格切换（换 persona / 子 agent / 压缩）
  会换新进程，天然隔离。
- **工具执行**：pi 的内置编码工具在 pi 进程内部执行；GQY 的定制工具
  （记忆、表情包、闹钟、玄学、知识库、天气、汇率、man、moegirl、哈希、计算器、剪贴板、web_fetch、语音…
  共 38 个，均以 `gqy_` 前缀注册）通过 `src/scripts/pi-bridge.ts` 扩展 + 本地 HTTP 回调执行，
  GQY 自己的 ToolRegistry 负责实际运行，结果回传给 pi。工具进度事件透传给 GQY 渲染层展示。
  工具清单由 GQY 写入 `GQY_HOME/cache/pi-bridge-tools.json`，扩展在 `session_start`
  同步读取注册（含 `promptSnippet`/`promptGuidelines` 使用引导），保证首轮即可被模型选用。
- **联想记忆**：GQY agent 循环按输入自动召回的 `<associative-memory>` 会拼进每轮 prompt
  的 `<gqy-context>` 前缀，pi 也能利用 GQY 侧的记忆库。
- **上下文压缩**：由 pi 自己管理（pi 模式 GQY 的 `context_window` 视为未知，
  GQY 侧溢出/压缩自动停用；手动 `/compact` 会提示由 pi 管理）。
- **会话持久化**：pi 使用 `--no-session`（内存态），对话历史在 pi 进程退出后即丢弃；
  GQY 侧的记忆（SQLite）、Git 快照、备份不受影响，照常工作。

## 环境变量

| 变量 | 作用 |
|---|---|
| `GQY_PI_BIN` | pi 可执行文件路径（默认 `pi`） |
| `GQY_PI_TURN_TIMEOUT` | 单轮等待上限秒数（默认 600），长工具执行时可调大 |
| `GQY_PI_DEBUG` | 设为任意值打印 pi RPC 事件流（诊断用） |
| `GQY_PI_TOOL_API` | pi 工具桥地址（由 GQY 自动设置，一般不需要手动配） |
| `GQY_PI_TOOL_LIST` | 工具清单文件路径（由 GQY 自动写入 `cache/pi-bridge-tools.json`） |
| `GQY_PI_EXTENSION` | 覆盖 pi-bridge 扩展文件路径 |

## 已知限制

- pi 模式需要 `tools.enabled = true`（默认开启）：GQY 侧工具循环不会运行
  （pi 返回空 tool_calls），但工具注册表需要存在，供工具桥使用；
- 模型是否使用 `gqy_*` 工具取决于模型本身的工具调用能力（DeepSeek flash 在带
  promptSnippet/guidelines 引导后可靠，个别模型可能仍倾向 bash，可在提示里明确要求）；
- 图片类工具（表情包、看图）经 HTTP 桥后：终端场景图片会直接 chafa 打印；
  Web 场景图片经 `tool.image` 事件落成 WebUI 资产显示在时间线（双通道均已实现）；
  纯看图理解仍建议配合本地视觉模型；
- GQY 记忆检索是关键词重叠匹配（中文按连续串匹配），查询词需与记忆内容有重叠；
- 单轮中途 `Esc` 中断会向 pi 发送 abort，但 pi 已消耗的 token 无法回收；
- 一次性 `gqy "问题"` 每次都是全新进程，没有跨进程记忆（与 opencode 直连模式一致）；
- 模型切换请用 pi 自己的方式（`/login`、`pi --model ...`），`gqy variant` 等命令在 pi 模式下为无操作。

## 仓库转工具（gqy tools）与 pi 模式

`gqy tools import <目录|仓库>` 导入的脚本工具在 pi 模式下同样可用：
工具桥放行用户脚本工具，导入的脚本以 `gqy_<id>` 出现在 pi 的工具列表，
模型可以直接调用。导入后需重启 GQY（新会话进程）让工具清单生效。

```zsh
gqy tools inspect <仓库>          # 先看候选脚本
gqy tools import <仓库> --name my-tools
gqy tools list                    # 已导入包
gqy tools remove my-tools         # 删除
```

## 深度研究 / 子 agent（pi 模式）

`gqy_task`（子 agent）与 `gqy_deep_research`（深度研究）在 pi 模式可用：
子 agent 通过独立的 pi 子进程隔离执行（与主会话互不污染），深度研究分
多阶段（规划→多路调研→审查→撰写）并生成带引用的 markdown 报告。

- 长任务超时：桥侧 30 分钟、主轮 30 分钟（`GQY_PI_TURN_TIMEOUT` 可调）；
- **递归隔离**：子 agent 的 pi 子进程使用**过滤后的工具清单**
  （`cache/pi-bridge-tools-subagent.json`，剔除 `gqy_task`/`gqy_deep_research`），
  子 agent 无法自我递归触发任务/深度研究。

## 自主 agent 集群（Kimi 式）

模型可以在对话中自主创建命名子代理并组队协作：

- `gqy_spawn_agent(name, role)` — 创建/更新一个命名 agent（自定义角色设定；同名改 role 会清空历史）；
- `gqy_talk_to_agent(name, message)` — 给 agent 派活，独立多轮记忆；
  同一轮多次调用即并行执行（每个 agent 独立进程/会话）；
- `gqy_parallel_agents(tasks)` — 一次并发向多个 agent 派活并汇总结果；
- `gqy_list_agents` / `gqy_kill_agent` — 名册与销毁；
- 定义持久化在 `GQY_HOME/data/agents/agents.json`，重启后 agent 仍在（进程懒启动）；
- 直连模式：子 agent 走 GQY 工具循环（可 web_search/web_fetch 等）；pi 模式工具在过滤后的 bridge 清单内执行；
- 递归防护：agent 工具清单不含 spawn/talk/list/kill/parallel/task/deep_research，不能无限套娃。
