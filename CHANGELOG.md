# Changelog

本项目所有值得记录的改动都会列在此文件。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

## [Unreleased]

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
