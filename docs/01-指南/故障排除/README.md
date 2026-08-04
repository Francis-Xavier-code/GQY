# GQY 故障排除指南

本指南帮助您解决 GQY 使用过程中遇到的常见问题。

## 安装问题

### 1. Homebrew 安装失败

**问题**: `brew install gqy` 失败

**解决方案**:

```zsh
# 检查仓库是否正确添加
brew tap | grep GQY

# 重新添加仓库
brew untap GQYTeam/GQy 2>/dev/null
brew tap GQYTeam/GQY

# 信任仓库
brew trust GQYTeam/GQY

# 更新 Homebrew
brew update

# 重新安装
brew install gqy
```

### 2. 编译失败

**问题**: `cargo build` 失败

**解决方案**:

```zsh
# 检查 Rust 版本
rustc --version
cargo --version

# 更新 Rust
rustup update

# 清理构建缓存
cargo clean

# 重新编译
cargo build --release
```

### 3. 权限问题

**问题**: 权限被拒绝

**解决方案**:

```zsh
# 修复权限
sudo chown -R $(whoami) /opt/homebrew/bin/

# 或者使用用户目录
mkdir -p ~/.local/bin
cp target/release/gqy ~/.local/bin/
export PATH="$HOME/.local/bin:$PATH"
```

## 配置问题

### 1. 配置文件损坏

**问题**: 配置文件无法解析

**解决方案**:

```zsh
# 验证配置
gqy config validate

# 查看配置
gqy config get

# 重新生成配置
rm ~/.config/gqy/config.jsonc
gqy
```

### 2. 环境变量未生效

**问题**: 环境变量设置后未生效

**解决方案**:

```zsh
# 检查环境变量
echo $GQY_HOME

# 重新加载 shell 配置
source ~/.zshrc

# 或者重启终端
```

### 3. 配置路径错误

**问题**: 配置路径不正确

**解决方案**:

```zsh
# 查看配置路径
gqy config paths

# 检查目录是否存在
ls -la ~/.config/gqy/

# 创建目录（如果不存在）
mkdir -p ~/.config/gqy
```

## 连接问题

### 1. 无法连接到供应商

**问题**: 无法连接到 AI 供应商

**解决方案**:

```zsh
# 检查网络连接
curl -I https://api.deepseek.com

# 检查 API 密钥
gqy config get providers

# 测试连接
gqy provider test deepseek

# 增加超时时间
gqy config set providers.0.timeout_seconds 120
```

### 2. API 密钥错误

**问题**: API 密钥无效

**解决方案**:

```zsh
# 检查密钥配置
gqy config get providers.0.api_key

# 使用环境变量
export DEEPSEEK_API_KEY="sk-xxx"
gqy config set providers.0.api_key '$env:DEEPSEEK_API_KEY'

# 重新添加供应商
gqy provider remove deepseek
gqy provider add https://api.deepseek.com/v1 --api-key $DEEPSEEK_API_KEY
```

### 3. 模型不存在

**问题**: 请求的模型不存在

**解决方案**:

```zsh
# 列出可用模型
gqy models

# 检查模型配置
gqy config get providers.0.models

# 更新模型列表
gqy provider refresh deepseek
```

## 对话问题

### 1. 响应缓慢

**问题**: AI 响应速度慢

**解决方案**:

```zsh
# 检查网络延迟
ping api.deepseek.com

# 减少上下文长度
gqy config set context.max_turns 10

# 使用更快的模型
gqy config set providers.0.default_model "deepseek-chat"

# 启用流式输出
gqy config set display.stream true
```

### 2. 上下文丢失

**问题**: 对话上下文丢失

**解决方案**:

```zsh
# 检查对话历史
gqy history

# 检查上下文配置
gqy config get context

# 增加上下文长度
gqy config set context.max_turns 50

# 检查数据库
gqy doctor --check-db
```

### 3. 工具调用失败

**问题**: 工具调用失败

**解决方案**:

```zsh
# 列出可用工具
gqy tools list

# 检查工具权限
gqy tools info <tool-name>

# 启用工具
gqy config set tools.enabled true

# 检查工具日志
cat ~/.config/gqy/logs/tools.log
```

## 记忆问题

### 1. 记忆丢失

**问题**: 记忆数据丢失

**解决方案**:

```zsh
# 检查记忆状态
gqy memory stats

# 检查数据库
ls -la ~/.local/share/gqy/memory/

# 从备份恢复
gqy backup restore --remote <url>

# 重建索引
gqy memory reindex
```

### 2. 搜索结果不准确

**问题**: 记忆搜索结果不准确

**解决方案**:

```zsh
# 更新搜索索引
gqy memory reindex

# 调整搜索参数
gqy memory search --limit 20 --threshold 0.5

# 检查记忆质量
gqy memory stats
```

### 3. 自动记忆不工作

**问题**: 自动记忆功能不工作

**解决方案**:

```zsh
# 检查配置
gqy config get memory.auto_fact_enabled

# 启用自动记忆
gqy config set memory.auto_fact_enabled true
gqy config set memory.auto_diary_enabled true

# 检查日志
cat ~/.config/gqy/logs/memory.log
```

## 知识库问题

### 1. 导入失败

**问题**: 知识库导入失败

**解决方案**:

```zsh
# 检查文件格式
file /path/to/document.pdf

# 检查文件权限
ls -la /path/to/document.pdf

# 检查磁盘空间
df -h

# 重新导入
gqy kb add /path/to/document.pdf
```

### 2. 搜索结果为空

**问题**: 知识库搜索无结果

**解决方案**:

```zsh
# 列出知识库
gqy kb list

# 重建索引
gqy kb reindex

# 检查搜索关键词
gqy kb search "更宽泛的关键词"
```

### 3. 文档无法读取

**问题**: 知识库文档无法读取

**解决方案**:

```zsh
# 检查文档 ID
gqy kb list

# 检查文档权限
ls -la ~/.local/share/gqy/kb/

# 重新导入文档
gqy kb remove <doc-id>
gqy kb add /path/to/document.pdf
```

## Web 面板问题

### 1. 无法启动

**问题**: Web 面板无法启动

**解决方案**:

```zsh
# 检查端口是否被占用
lsof -i :4096

# 使用其他端口
gqy web --port 8080

# 检查防火墙设置
sudo pfctl -s rules
```

### 2. 无法访问

**问题**: 无法访问 Web 面板

**解决方案**:

```zsh
# 检查是否正在运行
ps aux | grep gqy

# 检查监听地址
netstat -an | grep 4096

# 使用正确地址
curl http://127.0.0.1:4096
```

### 3. 认证失败

**问题**: Web 面板认证失败

**解决方案**:

```zsh
# 重置密码
gqy web --reset-password

# 或者删除认证配置
rm ~/.config/gqy/web-auth.json
gqy web
```

## 桥接问题

### 1. QQ 桥接失败

**问题**: QQ 桥接无法连接

**解决方案**:

```zsh
# 检查状态
gqy napcat status

# 重新安装
gqy napcat uninstall
gqy napcat install

# 检查配置
gqy napcat config

# 查看日志
cat ~/.config/gqy/logs/napcat.log
```

### 2. Telegram 桥接失败

**问题**: Telegram 桥接无法连接

**解决方案**:

```zsh
# 检查状态
gqy tg status

# 检查 Token
gqy tg token <new-token>

# 重新安装
gqy tg uninstall
gqy tg install

# 查看日志
cat ~/.config/gqy/logs/telegram.log
```

## 备份问题

### 1. 备份失败

**问题**: 备份操作失败

**解决方案**:

```zsh
# 检查 Git 状态
cd ~/.local/share/gqy
git status

# 初始化备份
gqy backup init

# 检查远程仓库
gqy backup status

# 手动备份
gqy backup now
```

### 2. 恢复失败

**问题**: 从备份恢复失败

**解决方案**:

```zsh
# 检查远程仓库
git remote -v

# 手动克隆
git clone <url> ~/.local/share/gqy-backup

# 恢复文件
cp -r ~/.local/share/gqy-backup/* ~/.local/share/gqy/
```

### 3. 同步冲突

**问题**: 备份同步冲突

**解决方案**:

```zsh
# 查看冲突
cd ~/.local/share/gqy
git status

# 解决冲突
git checkout --theirs .
git add .
git commit -m "resolve conflicts"

# 强制推送
git push --force
```

## 性能问题

### 1. 内存占用高

**问题**: GQY 内存占用过高

**解决方案**:

```zsh
# 检查内存使用
ps aux | grep gqy

# 减少上下文长度
gqy config set context.max_turns 20

# 禁用不必要的插件
gqy config set plugins.memes.enabled false

# 重启 GQY
```

### 2. CPU 占用高

**问题**: GQY CPU 占用过高

**解决方案**:

```zsh
# 检查 CPU 使用
top -o cpu

# 减少工具调用
gqy config set tools.max_rounds 5

# 禁用自动记忆
gqy config set memory.auto_fact_enabled false

# 重启 GQY
```

### 3. 磁盘空间不足

**问题**: 磁盘空间不足

**解决方案**:

```zsh
# 检查磁盘空间
df -h

# 清理缓存
rm -rf ~/.cache/gqy/

# 清理旧日志
rm -f ~/.config/gqy/logs/*.log.*

# 压缩数据库
gqy doctor --compact-db
```

## 诊断工具

### 1. 运行诊断

```zsh
# 完整诊断
gqy doctor

# 检查特定组件
gqy doctor --check-config
gqy doctor --check-db
gqy doctor --check-tools
gqy doctor --check-memory
```

### 2. 查看日志

```zsh
# 主日志
cat ~/.config/gqy/logs/gqy.log

# 错误日志
cat ~/.config/gqy/logs/error.log

# 实时监控
tail -f ~/.config/gqy/logs/gqy.log
```

### 3. 检查配置

```zsh
# 查看全部配置
gqy config get

# 验证配置
gqy config validate

# 查看路径
gqy paths
```

## 获取帮助

### 1. 查看帮助

```zsh
# 查看命令帮助
gqy --help

# 查看子命令帮助
gqy <command> --help
```

### 2. 社区支持

- **GitHub Issues**: https://github.com/GQYTeam/GQY/issues
- **讨论区**: https://github.com/GQYTeam/GQY/discussions

### 3. 报告问题

报告问题时请提供：
1. GQY 版本 (`gqy --version`)
2. 操作系统版本
3. 错误信息
4. 复现步骤
5. 相关日志

## 下一步

- [安装指南](../安装指南/README.md) - 重新安装
- [配置指南](../配置指南.md) - 详细配置
- [开发指南](../开发指南/README.md) - 开发者指南