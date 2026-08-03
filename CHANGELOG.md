# Changelog

本项目所有值得记录的改动都会列在此文件。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

## [0.8.2] - 2026-08-03

### 新增
- **闲聊上下文隔离**：Chat 模式的对话写入独立 `mode='chat'` 存储，与普通/计划模式互不污染——闲聊历史不撑爆正经对话上下文（本地模型友好，闲聊只看最近 12 轮）；WebUI 切换模式时按模式重载历史（`/api/channels/{id}/turns?mode=`）。

### 修复
- **供应商管理（CLI + 对话自然语言）**：`gqy provider add/list/switch/remove` 新增供应商；给 base_url + API Key 自动发现可用模型（GET /models）、写入配置并热切换激活；对话内可直接让顾清影调用 `manage_providers` 工具完成（运行中 WebUI 通过 config watcher 自动刷新出新供应商）；支持任意数量自定义 OpenAI 兼容供应商（Ollama/LM Studio/任意网关）；
- **称呼规则冲突**：内置人格 `gqy.md` 原禁止「宝宝/宝贝」等亲密称呼、默认叫「老板」——与女友人格冲突。改为：默认叫「主人」，亲密称呼仅女友人格/闲聊模式启用时使用；
- **菜单栏左键面板闪退**：`presentQuickPanel` 先 `makeKeyAndOrderFront` 后 `activate` 顺序反了，首次点击时 app 未激活导致面板被 `hidesOnDeactivate` 立即收起——调为先激活再展示。

## [0.8.1] - 2026-08-03

### 修复
- **WebUI 情感标签**：`tool.image` 事件携带的 emotion/action 字段此前被前端忽略——现在图片右上角叠加情感徽标（😊开心/😢难过等 8 种，含动作 tooltip）；
- **女友人格默认启用**：内置 `chat.md` 提醒词过于中性，导致 Chat 闲聊模式没有情感表现——已在 `config/prompts/` 部署女友态提醒 + lover 人格，并默认激活 `active_persona=lover.md`（免编译，改文件即生效）。

## [0.8.0] - 2026-08-03

### 新增
- **SearXNG 本地搜索**：本地优先 + 超时/去重/重试，配置 `searxng_base_url` 后自动使用（隐私友好，不依赖第三方在线 API）；
- **自我进化一期**：每轮对话自动采集训练样本到 `data/finetune/turns.jsonl`（JSONL 格式，只收集不训练，攒够阈值后用 MLX LoRA 批量微调）；
- **WebUI 用量分析 v2**：GitHub 式贡献热力图 + 费用估算（按 provider 单价，可配置）+ token 构成堆叠柱 + 模型维度表 + 调用级明细；
- **菜单栏双模交互**：左键悬浮卡片（NSPanel + WKWebView）+ 右键菜单 + ⌥G 全局唤起；
- **WebUI 悬浮卡片嵌入模式**：`?panel=1` 精简界面（隐藏侧栏/顶栏，单聊卡片形态）；
- **备份可见性**：WebUI 顶栏备份状态徽标（成功/失败/时间），失败也能被发现；
- **SQLite 空闲页回收**：建库 `auto_vacuum=INCREMENTAL`，备份前 best-effort 增量回收（旧库无害 no-op）；
- **通信桥 v2**：TG/NapCat 桥接层接入 GQY 长驻 daemon（HTTP + SSE），支持图片事件、追问、流式输出，daemon 生命周期自动管理；
- **`config set` 自动建段**：中间键不存在时自动创建，支持直接新增 `finetune.*` 等配置段；
- **闲聊模式开放只读记忆查询**：角色扮演时能想起别处的事。

### 修复
- **切换供应商前自动压缩上下文**：避免大上下文多供应商切换烧钱（34 元事件根治）；
- **Chat 模式提醒词免编译**：读 `config/prompts/chat.md`，改提醒词不再需要重新编译；
- **tool 折叠动画**：grid `1fr→0fr` 过渡，自适应高度，不再跳变；
- **废弃补丁防护**：`apply-patches.sh` 自动跳过被 fix 版取代的原始补丁。

## [0.7.1] - 2026-08-02

### 修复
- **菜单栏升级路径**：`gqy menubar --install` 安装前清理旧实例（退出运行中的菜单栏、
  卸载旧 LaunchAgent），安装后自动拉起；提示清理旧 cask 版 `/Applications/顾清影.app`。
  修复「升级后误开旧版、打开面板无法连接」；
- **WebUI 载入崩溃**：时间线渲染误用未定义变量 `index`（`for...of` 里引用下标）
  导致 `ReferenceError` 页面白屏——改为带下标的循环。

## [0.7.0] - 2026-08-02

### 新增
- **pi 底座模式（实验性）**：`provider.protocol` 设为 `pi` 后，GQY 通过 `pi --mode rpc`
  把「大脑」整体交给 pi——pi 用自己的模型、agent 循环与内置工具（read/write/edit/bash/find/grep/ls）
  完成对话与工具调用，GQY 负责渲染、记忆、知识库、备份等外围能力。
  支持多轮记忆、工具进度透传、`Esc` 中断（向 pi 发 abort）、人格注入。
- **GQY 工具注入 pi**：`src/scripts/pi-bridge.ts` 扩展 + 本地 HTTP 工具桥
  （`src/pi_bridge.rs`，`127.0.0.1` 随机端口），把 GQY 的 38 个定制工具
  （记忆、表情包、闹钟、玄学、知识库、天气、汇率、man、moegirl、哈希、计算器、剪贴板、
  web_fetch、语音等）以 `gqy_*` 前缀注册进 pi，模型调用时回调 GQY 的 ToolRegistry 执行；
  GQY 侧的联想记忆（`<associative-memory>`）拼入每轮 prompt 的 `<gqy-context>` 前缀。
- **工具引导（提高 gqy_* 使用率）**：每个注入工具带 `promptSnippet`/`promptGuidelines`
  （如「用户要算卦/抽塔罗时用 gqy_draw_*」「设闹钟用 gqy_set_alarm，不要用 bash sleep 模拟」），
  工具清单同步写入 `GQY_HOME/cache/pi-bridge-tools.json`，扩展在 `session_start`
  **同步注册**（避免异步 fetch 与首轮 prompt 的竞态），确保首轮 system prompt 就带 gqy_* 工具。
  详见 `docs/01-指南/pi-底座模式.md`。
- **pi 模式本地视觉**：`gqy_analyze_image_local` 暴露给 pi（Apple Vision，OCR+分类+物体检测，
  免费离线不耗 API 额度）；pi 模式下图片随 prompt 的 `images` 字段直接流入 pi；
  同时捕获 pi 实际使用的 provider/model 用于用量归因与界面显示。

### WebUI
- **图片粘贴/拖拽**：composer 支持粘贴或拖入图片（托盘预览、可移除、可多张），
  随消息一起发送（`/api/turns`、`/api/queue` 均支持 `images` 字段），用户消息气泡内渲染缩略图；
- **pi 底座标识**：bootstrap 新增 `engine` 字段，pi 模式下顶栏模型按钮显示「pi 底座」；
- **浏览器通知**：页面后台运行时回复完成弹出系统通知（首次发送时请求权限）；
- **附件能力声明**：`capabilities.attachments` 置为 `true`。
- **用量准确性**：pi 模式从事件流捕获每条消息的真实 token 用量
  （含 cache_read/cache_write，多个内部 LLM 调用逐条累加），不再用估算值；
  用量记录带真实 provider/model（deepseek / deepseek-v4-flash）。
- **用量可视化**：新增「近 30 天趋势」柱状图（峰值/日均/合计汇总行），
  配合原有 365 天贡献网格与模型占比条。
- **工具调用折叠块**：新增 `ChatStreamKind::ToolProgress/ToolResult`，pi 的工具执行
  （bash/gqy_* 的 start/update/end）转成 AgentEvent 驱动终端与 Web 时间线里的
  可折叠工具块（含参数与输出）。
- **代码块语法高亮**：零依赖轻量高亮器（bash/rust/js/python/json/sql/yaml/diff…），
  关键词/字符串/数字/注释着色，diff 增删行高亮。
- **对话全文搜索**：新增 `GET /api/search`（跨通道 LIKE 检索）+ 侧边栏搜索框，
  结果卡片内联展开查看完整对话。
- **macOS 风格化**：字体栈换 SF Pro/PingFang、侧边栏与顶栏毛玻璃
  （backdrop-filter，不支持时优雅降级）、消息气泡圆角 macOS 化。
- **Homebrew 打包**：`pi-bridge.ts` 随 `src/scripts` 自动进入 brew 公式与菜单栏
  App 的 share 目录，无需额外改动。
- **表情包发送全链路（pi 模式）**：`gqy_show_meme` 暴露给 pi（含使用引导：
  先 search 取 id 再 show）；工具桥调用时并发读取 `ToolProgress` 事件——
  `PrepareForExternalOutput` 自动应答保持终端 chafa 图片打印，`Image` 事件经
  WebUI 资产链路（`tool.image` SSE）落到浏览器时间线，终端/网页双通道显示表情包。
  `ToolRegistry` 新增 `call_with_progress`。
- **工具矩阵验证 + 桥并发修复**：新增 `exposed_tools_matrix_runs_ok` 测试，16 个本地
  确定性工具（哈希/编解码/计算器/系统信息/骰子/占卜/记忆/表情/闹钟列表/知识库）逐个经
  桥路径调用，毫秒级完成；修复桥事件循环潜在挂起（工具改在独立任务执行，progress 的
  sender 随任务结束释放，事件循环必然收敛）。
- **pi 模式深度研究 / 子 agent**：`task`（子 agent）与 `deep_research`（多阶段报告）
  暴露给 pi 工具桥——子 agent 经独立 pi 子进程隔离执行（与主会话互不污染），
  长任务桥超时 30 分钟、主轮超时放宽到 30 分钟（`GQY_PI_TURN_TIMEOUT` 可调）。
  端到端验证：模型调用 `gqy_task` 完成子主题调研、`gqy_deep_research` 生成带
  引用的 markdown 报告落盘并汇总。
### 进程统一与交付整合
- **统一收尾**：`gqy web` 新增 `/api/shutdown`（优雅退出：停 serve → actor 结束 →
  agent drop → pi 进程组被清理）；菜单栏「退出」改为先调 shutdown 再退出——
  **点菜单栏退出，全部 GQY/pi 进程随之消亡**（不再有孤儿残留）；
- **pi 孤儿修复**：PiRpcClient 的 pi 进程设进程组（process_group），PiProc Drop 时
  `kill(-pid, SIGTERM)` 连 bash 孙进程一起清理；
- **菜单栏客户端化**：main.m 不再强杀 4096 端口进程，改为复用已有 daemon
  （已运行直接连、没有才拉起），退出时统一 shutdown；
- **`gqy menubar --install`**：把菜单栏壳（main.m）现场 clang 编译成
  顾清影.app 装到 `~/Applications`（内置 gqy 二进制 + 共享资源，自包含）——
  **交付合并为单一 formula**，不再维护 DMG/cask 双轨；
- **hook 不打扰**：新增 `shell.auto` 配置（默认开）；关掉后命令未找到一律
  系统报错，只用显式 `gqy <问句>` 对话。

### 真实使用反馈修复（agent 集群体验）
- **页面空白修复**：`rerunButton` 作用域 bug（createTool 局部变量在 tool.finished 分支
  访问 → ReferenceError → 工具卡卡死、时间线渲染断裂）——存进 tool 对象后修复；
- **思考聚合**：pi_rpc 对跨内部 LLM 调用的 thinking 发 `ReasoningPartStart/End`，
  前端只渲染**一个聚合思考块**（此前每段思考各自折叠）；
- **滚动捕获**：tool-detail pre / reasoning 块加 `overscroll-behavior: contain`，
  打开详情后页面仍可滚轮滑动；
- **背景全屏**：顾清影壁纸从角落小图改为**全屏铺满**（cover + 顶部/底部渐变遮罩
  保证文字可读，滚动对话保持不动）；
- **agent 守则强化**：系统提示词要求 agent 只输出可核查信息、优先用工具核实、
  不确定明确标注，降低编造/假信息概率。

### agent 活动实时可视化
- `talk_to_agent` 的 agent 思考/回复增量经 `ToolProgress` 消息 → 桥 progress sink →
  `tool.progress` SSE → Web 时间线「agent 集群活动」卡片**实时滚动**（追加式，带长度上限）；
- 前端 tool.progress 由替换改为追加滚动；桥新增 `progress_sink`（cli 传 None）。

### 自主 agent 集群（Kimi 式）
- **模型可自主创建/管理命名子代理**：`gqy_spawn_agent`（创建 agent，自定义角色）、
  `gqy_talk_to_agent`（点名对话，独立记忆、多轮可连续）、`gqy_list_agents`（名册）、
  `gqy_kill_agent`（销毁）。同一轮多次 `talk_to_agent` 即并行派活。
- **实现**：`src/agents.rs`（AgentManager 全局单例 + 工具 + 持久化到
  `GQY_HOME/data/agents/agents.json`）；pi 模式下每个 agent 是独立 pi 进程
  （自定义系统提示词 → 独立进程，多轮记忆）；直连模式用 OpenAI 客户端 + 实例内历史。
- **递归防护**：agent 进程使用过滤工具清单（不含 spawn/talk/list/kill），
  不能无限再创建 agent。
- 端到端验证：模型自主组建 architect+reviewer 双 agent 团队，并行派活、
  汇总意见、确认名册、销毁——完整闭环。

### WebUI 第三轮
- **回合重试/重新生成**：失败/中断轮及最后一轮的用户消息带「重新生成」按钮，
  剥离注入的图片描述块后重发（抽出公共 `sendTurnContent`，发送与重试共用一条链路）；
- **工具一键重跑**：pi 模式工具块完成/失败后显示「重跑」按钮，经新增的
  `/api/tools/call` 走 GQY 注册表重新执行（长任务 30 分钟超时）；
- **pi 模型切换 + 思考级别**：顶栏模型按钮在 pi 模式弹出 pi 模型选择器
  （`get_available_models` 列表单选 + `set_model`）与思考级别档位
  （off~max，`set_thinking_level`）；新增 `/api/pi/{state|models|model|thinking}`，
  PiRpcClient 增加通用 `rpc_command`（回合运行中也可切换）；
- **TTS 朗读**：助手消息「朗读」按钮（浏览器 SpeechSynthesis，零后端）；
- **对话导出**：`/api/export` 导出当前会话为 markdown + 顶栏下载按钮；
- **移动端适配**：窄屏（≤768/480px）布局走查（消息宽度/菜单/composer/用量图）。

- **pi 工具桥全量注册修复**：`pi-bridge.ts` 对工具名做规范化
  （连字符等非法字符转下划线，如 `battery-care` → `gqy_battery_care`，
  execute 仍按原始名回调桥），此前内置脚本 `battery-care` 因连字符被
  扩展跳过（46/47），现在 **47/47 全注册**。
- **子 agent 递归隔离**：`for_subagent_output` 给 Pi 客户端打子 agent 标记，
  子 agent spawn 的 pi 进程使用过滤工具清单（`pi-bridge-tools-subagent.json`，
  剔除 `gqy_task`/`gqy_deep_research`），防止子 agent 自我递归。端到端验证：
  主进程 47 个工具、子 agent 进程 45 个（恰少 2 个递归工具）。
- **`gqy tools` 更多管理能力**：新增 `show <包名>`（工具详情/禁用状态）、
  `disable <id>` / `enable <id>`（写 index.json 的 disabled 数组，扫描跳过/恢复），
  与既有 `import/list/remove` 构成完整管理面。
- **仓库转工具（`gqy tools`）成熟化**：
  - 修复「导入的脚本工具注册不上」的静默 bug：早期导入在 index.json 写入
    `load_policy: ""`，扫描侧 `LoadPolicy` 枚举反序列化失败被 `.ok()` 静默丢弃；
    `LoadPolicy` 现在容忍空串/未知值（回退 Summary），新导入规范化写入 `group`；
  - 新增 `gqy tools remove <包名>`（删除包目录 + 清理 index.json 注册）；
  - **pi 模式可用**：工具桥放行用户导入的脚本工具（`is_script_tool`），导入的脚本
    以 `gqy_<id>` 出现在 pi 的工具列表并可被模型直接调用（含使用引导）。

## [0.6.1] - 2026-08-02

### 新增
- **历史对话列表**：侧边栏保留每个通道的完整会话历史——「新对话」后旧对话
  仍可点击查看（`turns` 表新增 `conversation_id` 分组，旧的连续对话归入一个
  「历史对话」，新对话显示在上方）；点击历史对话只读浏览完整记录，不影响
  当前对话；浏览其他通道时同样显示该通道的会话列表
- **余额实时显示**：侧边栏显示当前 provider 余额（DeepSeek 等有公开余额接口
  的 provider，API key 支持 `$env:` 引用解析，请求 `/user/balance` 并清洗
  展示 `¥12.34`，多币种并列，悬停看充值/赠送明细）；**每次对话完成后刷新
  一次**（前端 10s 防抖 + 后端 60s 缓存，不做轮询），不支持的 provider 自动
  隐藏

## [0.6.0] - 2026-08-02

### 新增
- **多终端会话（通道隔离）**：终端 / WebUI / QQ / Telegram 各自独立的
  对话上下文（`turns` 表新增 `channel` 列，`GQY_CHANNEL` 环境变量指定；
  桥接脚本自动注入 qq/tg）。WebUI 左侧新增通道列表，可点击切换查看
  各通道的历史记录（其他通道只读浏览，不影响各自上下文）；「新对话」
  只清空当前通道
- **KV 缓存策略**：Anthropic 协议请求自动下发 `cache_control: ephemeral`
  （system 提示词 + 工具定义 + 对话历史前缀断点），多轮对话命中前缀缓存
  省 token；DeepSeek/OpenAI 兼容协议的
  `prompt_tokens_details.cached_tokens` / `prompt_cache_hit_tokens` 自动
  归一化记录。provider 可设 `"prompt_caching": false` 关闭
- **用量明细**：用量页新增「最近调用」列表（时间/模型/输入/输出/缓存命中/
  记忆徽标，含记忆归档压缩等辅助消耗）；点击模型明细行进入详情面板
  （该模型按日消耗柱状图 + 最近调用明细）

### 修复
- WebUI 直播中「思考后无正文」（刷新才显示）：运行结束后强制用服务端
  数据对账重渲染，事件丢失不再导致正文永久缺失；SSE 事件处理增加异常
  隔离，单个事件失败不影响后续
- 用量贡献图月份标签错位一列（按列首周日标记）与首月无标签：改为按列内
  周六标记「该列包含的新月份」，首列也显示
- 菜单栏「打开面板」改为默认浏览器打开 WebUI，删除内置 NSPanel/WKWebView
  独立窗口（避免双份界面冗余），⌥H 快捷键与「打开配置」行为同步
- 菜单栏「重启面板服务」：此前只终止自己 spawn 的子进程（服务为旧版 App
  自启/手动启动时实际没重启），现在会找到并终止监听 4096 端口的全部旧
  gqy web 进程，再用当前二进制重新启动，轮询健康检查通过后才提示成功

## [0.5.2] - 2026-08-02

### 新增
- **用量统计视图**（侧栏「用量」按钮）：GitHub 风格贡献图展示最近 365 天
  每日 token 消耗（月夜主题分级着色、悬停显示日期与 token、月份标签），
  下方为详细消耗统计——累计/今日/本周/本月卡片 + 按模型明细表
  （请求数、输入/输出/总 token、占比条，模型带品牌 logo）；
  数据来自新增的 `usage-history.jsonl`（每次调用追加一条记录，原子追加）

### 修复
- 「只有思考没有文字」：模型只输出 reasoning 时（如 DeepSeek reasoner
  偶尔思考完直接结束），补一句「（本轮思考完成，没有额外的文字回复。）」，
  面板与历史记录都有可读内容
- 备份推送后菜单栏图标被换成 sparkles：备份结束恢复顾清影头像图标
- 迷你对话窗口整体移除（⌥G、/mini 独立页面、放大按钮、迷你模式样式全删）
- 「新对话」不再弹「清空全部对话？」确认框、不再删除数据——点击后把当前
  对话归档（hidden=1，数据完整保留），面板从空会话开始；旧对话不显示但
  gqy 历史/记忆查询仍可读到（多会话隔离，主对话记录不受影响）

## [0.5.1] - 2026-08-02

### 新增
- 面板与终端会话同步：WebUI 空闲时每 5s 轮询 `/api/state`，
  终端 `gqy` 对话的 seq 变化后面板自动重载历史（进行中/已完成都同步）
- 回复动画增强：流式回复期间（无论有无 reasoning）头像呼吸 + 月青光晕，
  回复完成移除
- 修复面板「每次打开都是新对话」：`reset_if_prompt_changed` 在系统提示变化时
  会清空全部对话历史（开发迭代 prompt 误伤真实数据）——现在只更新指纹，
  历史轮次完整保留
- 终端对话实时同步面板：终端流式回复每 ~1s 写入 conversation.db，
  面板在外部有运行轮次时 2s 高频轮询，逐字回复实时可见
- 思考动画修复/增强：思考中顾清影头像呼吸 + 月青光晕脉冲（live reasoning 期间）
- 菜单栏状态显示增强：模型/记忆/备份 状态项带彩色圆点（月青=就绪、淡紫=记忆、
  绿=备份已同步、灰=未配置）；备份中状态栏图标旋转动画
- 修复：zsh 中「URL 开头/含 ://」的输入不触发 command_not_found_handler
  （zsh 按路径查找直接报错），导致「给 https://…」类自然语言无法拦截——
  现在 accept-line 提前识别 URL 开头的输入并交给 GQY

### 修复
- 面板 UI 五处修复：
  - 对话壁纸位置重做：按图片真实比例铺满、圆角淡显在输入区上方，不再
    contain 留白贴死在视口右下角
  - 侧栏折叠：补折叠按钮图标（此前缺失显示警告圈）、悬停展开时网格同步
    回流（`:has`）、双向平滑过渡、恢复状态同步按钮标题
  - 设置界面重构：导航按「外观/模型/自动化/扩展/系统」分组；当前模型移入
    模型池面板、能力状态并入高级、版本号入页脚
  - 去重顶栏重复的主题/设置按钮（只保留侧栏带标签版本）
  - 供应商品牌 logo（devicons 22 个，本地 sprite 离线加载）；未收录的
    供应商保持字母缩写

## [0.5.0] - 2026-08-02

### 新增
- `gqy balance`：查询 DeepSeek 账户余额（`¥ x.xx（总）· 充值 · 赠送`），终端随时可看
- `gqy napcat` / `gqy tg`：桥接管理 CLI（`status` / `install` / `uninstall` / `config`），
  配置统一存 `GQY_HOME/config/bridges.json`，LaunchAgent 自启动一键托管（KeepAlive 自动重启）
- `gqy config set <key> <value>` / `gqy config get [key]`：免交互读写配置（点号路径、密钥脱敏），
  顾清影与脚本可直接调用
- 菜单栏「打开配置」：面板直达设置抽屉（等价 `gqy config` 的 GUI 版）
- 终端启动横幅：彩色渐变文字版 GQY logo（深蓝夜空→冷蓝→月白→淡紫→银灰）
- REPL：`Ctrl+O` 流式输出中即时展开/收起思考详情
- 面板打开时 Dock 显示图标（关闭收回）
- WebUI 对话区右下角淡显顾清影壁纸背景
- `gqy __preview`：终端预览月夜主题 logo 与 markdown 渲染效果
- `gqy tools import` 许可证检查：识别仓库 LICENSE（MIT/Apache/BSD/GPL 等），
  随包保留 LICENSE 文件，`gqy tools list` 显示许可证，无许可证/非宽松许可时警告
- WebUI 流式输出打字机光标 + 新消息块淡入动画
- WebUI 增强（参考 llama.cpp tools/ui 设计，保留月夜特色）：
  - 桌面侧栏可折叠为 48px 图标条（hover 自动展开，状态持久化）
  - 细滚动条（hover 才显示，界面干净不抖动）
  - 更多自研动画：消息滑入/上浮、工具卡片弹入、思考进度条渐变流光、
    上下文条填充过渡、输入框聚焦月青光晕、菜单弹入、回到底部按钮弹出
    （全部尊重 prefers-reduced-motion）
- shell 意图判断增强：歧义命令清单扩展（time/test/date/which/type/command/history/help/man），
  新增聊天开场词检测（帮/请/怎么/为什么/如何/能不能/写/查/搜/翻译/推荐…），
  后接中文即判为自然语言，命令拦截更准
- `gqy history --search <词>`：关键词搜索会话记录（当前会话全部轮次 + 已归档轮次），
  不占对话上下文
- `gqy activity [--search <词>]`：活动日志查询（GQY 干了什么的流水账，
  默认不进 LLM 上下文，零 token 开销；工具调用与子代理完成自动记录）
- `pomodoro` 工具：番茄钟专注循环（工作 25 分钟 + 休息 5 分钟，周期响铃可取消）
- `set_alarm` 支持 `repeat` 周期提醒（如每 25 分钟响一次）
- `log_mood` / `recall_mood`：心情日志与情绪记忆（情感场景专用，不参与代码任务）
- 记忆关联注入极简相对时间（「3天前」等，每条约 2-4 token），模型感知时效不耗上下文
- 子代理新增 `researcher` 类型（深度研究，80 步工具预算 / 20 分钟超时），
  `task` 工具支持 `model` 参数指定子代理模型（如 `provider/model-name` 或纯模型名）
- WebUI 新增「定时任务」面板：闹钟/番茄钟/周期提醒可视化，可一键取消
- 远程备份支持 gh CLI：`gqy backup remote owner/repo` 自动创建私有仓库
  并用 gh 凭据推送（无需 SSH key）
- 首次运行自动播种：创建 scripts 索引、随包知识库自动导入（brew 安装后开箱即用）
- `gqy watch` 管家监控：后台采样进程 CPU/内存，检测异常（≥150% CPU 或 ≥2 个高占用）
  时给运行中的会话入队「主动消息」——顾清影先自己判断再决定是否打扰用户
- 自我成长（知识库反哺）：对话中用户明确说「记住这个方法/记下来」时，
  自动把结论沉淀为可加载技能（SKILL.md + skill_records 记录），规则匹配零模型开销
- `gqy memes list` / `gqy memes stats`：查看表情库数量与格式分布
- 情绪感知：系统提示新增情绪规则——感知用户情绪变化时用 `log_mood` 记心情日志，
  情感场景允许自然道歉（代码场景保持克制不道歉）
- 语音能力（本地、零 API 成本）：
  - `gqy tts "文字"` / `speak` 工具：macOS `say` 朗读（`--voice` 选音色、`-o` 存文件、`--list` 列语音）
  - `gqy stt <音频>` / `listen_audio` 工具：speech-tool.swift 本地离线识别
    （注意：macOS 语音识别是 TCC 敏感权限，裸脚本无法自动授权——CLI 场景会给出指引，
    真正启用需集成进带 bundle 的菜单栏 App）
- 记忆定期归档：`gqy archive [--keep-days N]` 把超过保留期的轮次归档到
  evicted_context（不占对话上下文，`history --search` 可检索）；对话开始前自动触发
  （默认保留 7 天，节流静默）
- 修复：`gqy history --search` 检索归档轮次时读错字段（snippet），现在可搜到归档内容

### 变更
- 菜单栏面板改为独立 App 窗口（可拖动缩放，不再依赖浏览器）；窗口 720px 宽，
  WebUI 移动端断点降至 640px，侧栏默认展开
- 菜单栏状态区每次打开菜单即时刷新（模型/记忆/备份时间），配置改动同步可见
- REPL 渲染全面切换「月夜清影」主题：深蓝夜空底色 + 月光银白正文 + 冷蓝标题 +
  淡紫列表/代码 + 月青链接（呼应顾清影清冷人设，替换原紫色系）
- markdown 渲染去标记化（基于 pulldown-cmark 结构化解析）：
  标题不再显示 `#`（h1/h2 亮蓝加粗）、列表 `-` 渲染为 `•`、任务列表 `☑/☐`、
  引用块 `>` 渲染为 `│`、表格 Unicode 边框对齐、代码块带框
- markdown 链接与裸 URL 输出 OSC 8 超链接：iTerm2/Terminal.app/kitty 中
  直接点击即可打开浏览器（链接文字月青下划线）
- 用户消息 ❯ 条、思考详情小字同步月夜配色
- 省 token：工具描述精简（56 个工具描述 24.3k→20.4k 字符，每轮请求省约 1.3k tokens）；
  历史压缩与回放不再重放模型思考全文（reasoning），仅缺失正文时保留首行占位
- 终端图片显示统一标准：探测图片宽高比后按比例适配（chafa 不再拉伸变形），
  显式 symbols 字符集输出，不同分辨率/格式图片显示一致
- 「新对话」确认文案强化（提示先备份）
- 桥接脚本随 brew 打包（`share/gqy/bridges`），`gqy napcat/tg` 开箱即用

### 修复
- 闹钟 worker 孤儿/无限响铃问题（三重防护）：
  - 周期闹钟响铃上限（max_rings，默认 20 次）自动停止
  - 孤儿检测：周期闹钟的父进程退出后 worker 自动退出（一次性闹钟不受影响）
  - `gqy alarm stop --all` 全局停止：按 pid 文件扫描所有 worker 并终止，
    即使记录丢失也能兜底；`gqy alarm list` / `gqy alarm cancel <id>` 同步提供
  - cancel 统一走 `alarm::cancel`（删记录 + kill + pid 文件兜底）
- 内置脚本 procusage / battery-care 的 `mapfile`（bash 4+ 特性）改为 while-read 循环，
  macOS 自带 bash 3.2 下可正常使用
- 备份快照排除 macOS `._*` AppleDouble 文件（从备份 tar 恢复后快照不再卡死）
- Cask 安装后自动移除 quarantine，Gatekeeper 不再静默拦截启动
- 双配置不同步问题：统一 `GQY_HOME/config/config.jsonc` 单一配置源

## [0.4.5] - 2026-08-01

### 新增
- 菜单栏「打开配置」入口（面板直达设置抽屉）
- `gqy config set/get` 免交互配置命令

### 变更
- 面板改为独立 App 窗口（NSPanel + WKWebView），Dock 图标随窗口显隐
- 面板窗口加宽至 720px，WebUI 响应式断点 760→640（侧栏默认展开）
- 菜单每次打开刷新状态区

### 修复
- 备份快照排除 `._*` AppleDouble 元数据文件
- Cask postflight 移除 quarantine，Gatekeeper 不再拦截启动

## [0.4.4] - 2026-08-01

### 变更
- 备份远程：绑定私有仓库 `GQY-backup` 并首次推送（后续自动 commit + push）

### 修复
- 恢复数据后备份失败（AppleDouble 文件被误当 SQLite）

## [0.4.3] - 2026-08-01

### 新增
- 菜单栏内置面板（WKWebView popover），不再唤起浏览器
- 菜单状态区：模型 / 记忆条数 / 上次备份时间
- 备份中状态栏图标切换为时钟

### 变更
- 统一 `GQY_HOME` 布局（菜单栏/CLI/桥接同一份数据），消除双配置分裂

## [0.4.2] - 2026-08-01

### 新增
- 只读资源统一收进 `share/gqy`（scripts / memes / kb），brew 与 app bundle 布局一致
- `gqy kb add "$(brew --prefix)/share/gqy/kb"` 一键导入随包知识库

### 变更
- WebUI 默认绑定 127.0.0.1，非回环地址强制密码
- path_guard 覆盖 edit_file / trash_path；apply_patch 删除进回收站
- Cask 卸载不再删除用户数据
- 自动备份 30 分钟节流并移出对话热路径
- shell hook 接入 `--shell-classify`；zsh 支持多行自然语言整块拦截

### 修复
- 闹钟 PID 复用防护（flock 判定存活）
- 流式读取 60s 空闲超时
- memory.db 并发加固（WAL + busy_timeout）

## [0.4.1] - 2026-08-01

### 修复
- path_guard 代码级护栏（GQY 无法写入项目源码目录）
- Homebrew formula/cask 打包修正

## [0.4.0] - 2026-08-01

### 新增
- macOS 知识库（16 篇）与 `gqy kb` 命令
- 菜单栏 App（顾清影.app）与 DMG 打包
- 独立主目录 `GQY_HOME` 与 Git 备份（本地/远程）

[Unreleased]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.5...HEAD
[0.4.5]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/Francis-Xavier-code/GQY/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Francis-Xavier-code/GQY/releases/tag/v0.4.0
