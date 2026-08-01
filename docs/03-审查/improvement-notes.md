# GQY 改进意见

审查日期：2026-08-01。基于当前代码状态的改进建议，按优先级分组。

## P0 · 安全

1. **WebUI 默认绑定 0.0.0.0 且无鉴权**（src/web.rs:897）
   WebUI 监听所有网卡，局域网内其他设备可直接访问（能读配置、发消息）。建议默认只绑定 `127.0.0.1`，需要局域网访问时显式传 `--host 0.0.0.0` 并开启访问密码。

2. **自动备份失败静默**（src/agent/mod.rs:622）
   每轮对话后的 `maybe_auto_backup` 失败只 `eprintln!` 警告，用户无感知。建议：失败时在 REPL/WebUI 状态栏提示，或菜单栏「立即备份」按钮显示最近一次备份时间，让「记忆没存上」可被发现。

## P1 · 可靠性

3. **每轮全量快照，无节流**（src/backup.rs `snapshot`）
   每次 `maybe_auto_backup` 都整树复制 config/data/state/pictures（SQLite 用 VACUUM INTO）。高频对话时磁盘 IO 与 CPU 开销明显。建议：节流（如 ≥30 分钟或 ≥10 轮才快照一次）+ 只 commit 有变更的内容（已有 dirty 检查，但复制本身无法避免）。

4. **restore 覆盖语义需文档明确**
   `backup restore` 对已有 live 目录默认拒绝、`--force` 整体覆盖（config.jsonc 除外）。同一台机器误操作会丢本地新数据。建议文档写明「restore 用于新机器/全新 GQY_HOME」，并考虑 `--merge` 或先备份旧目录。

5. **token_estimate 与官方 tiktoken 有偏差**
   基线测试 `counts_match_official_tiktoken_vectors` 对长中文文档偏差约 7%（3229 vs 3144），属上游遗留。影响 token 用量统计准确性。建议核对 `estimate_tokens` 实现（可能是 byte 编码与 UTF-8 边界处理）。

6. **2 个测试依赖外部环境**
   `llm::openai_compatible`（网络 mock）与 `tools::mcp`（外部 mock 二进制）在无网络/无依赖环境必失败。建议在 CI 里显式忽略或提供内置 mock。

## P2 · 功能缺口

7. **缺少 macOS 专属工具**
   目前系统类工具是跨平台的（check_os_info/run_command），没有 brew 管理、LaunchAgents 增删查、磁盘清理、系统设置直达等 macOS 工具。可新增 `tools/macos.rs`：`brew_search/brew_install`、`launchctl_list/load/unload`、`disk_usage`、`open_settings`。配合新加的 macOS 知识库正好闭环。

8. **WebUI 无历史会话浏览**
   conversation.db 里有全部历史，但 WebUI 只能看当前会话。加 `/api/history` + 侧栏会话列表即可，体验提升明显。

9. **auto_fact_enabled / auto_skill_enabled 配置空转**
   config.rs 里两个开关没有消费者（记忆只做自动日记 + 工具触发式记忆）。要么实现自动事实提取，要么从 TUI 移除避免误导。

10. **菜单栏端口硬编码 4096**（macos/GQYMenuBar/main.m）
    端口被占用时菜单栏仍会打开 http://127.0.0.1:4096（可能打开陌生服务）。建议：菜单栏先请求 `/api/health` 确认是 GQY 再打开，或后端支持 `--port 0` 动态端口并回报。

11. **终端 launcher 脚本会进备份快照**
    「打开终端对话」把 launcher 写到 `GQY_HOME/runtime/`，该目录会随快照提交。建议快照排除 `runtime/`（或在 .gitignore 模式里加）。

12. **GQY_HOME/secrets 未自动创建**
    文档要求配 SSH key 前手动 `mkdir -p "$GQY_HOME/secrets/ssh"`。建议 `backup init` / `backup remote --ssh-key` 时自动创建，少一个手工步骤。

## P3 · 工程与体验

13. **版本号沿用上游 0.3.0**
    fork 后功能已大改（GQY_HOME、备份、人格），建议 bump 到 0.4.0 或 1.0.0，并让菜单栏/WebUI/CLI 的版本显示一致。

14. **人格文件运行时覆盖**
    人格以 XOR+base64 嵌入二进制（防误读，无安全性）。若想让她的人格随时可调，可支持 `GQY_HOME/config/persona.md` 覆盖内置人格，改完即生效，配合备份恢复更灵活。

15. **依赖瘦身评估**
    rodio（音频播放）、trash、image、flate2 等依赖在纯终端场景是否全部必要，可评估裁剪以减小二进制（当前 release 约 30MB）。

16. **DMG 未公证**
    本地使用无碍；若打算分发给他人在未受信任的 Mac 上运行，需要 Developer ID 签名 + notarization（需开发者账号），脚本已支持 `CODESIGN_IDENTITY`，缺公证步骤。

17. **内置脚本资产待审视**
    `src/scripts/`（battery-care、crack-search、procusage 等）是上游遗留，建议逐个确认是否保留并更新说明。

## 已随本轮完成的事项

- macOS 知识库（kb/，16 篇）+ `gqy kb add kb/` 导入
- LICENSE 切换 GPL-3.0（保留上游 MIT 声明）
- README 完善（命令速查、FAQ、知识库说明）
- 菜单栏 app 图标 / 版本号 / DMG 打包
