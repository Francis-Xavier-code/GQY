# GQY 安装指南

本指南将帮助您在 macOS 系统上安装和配置 GQY。

## 系统要求

- **操作系统**: macOS 12.0 (Monterey) 或更高版本
- **架构**: Apple Silicon (M1/M2/M3/M4) 或 Intel
- **磁盘空间**: 至少 500MB 可用空间
- **网络**: 首次安装需要网络连接

## 安装方式

### 方式一：Homebrew 安装（推荐）

```zsh
# 添加 GQY 仓库
brew tap GQYTeam/GQY

# 信任仓库
brew trust GQYTeam/GQY

# 安装 GQY
brew install gqy
```

### 方式二：一键安装脚本

```zsh
curl -fsSL https://raw.githubusercontent.com/GQYTeam/GQY/main/install.sh | bash
```

### 方式三：源码编译

```zsh
# 克隆仓库
git clone https://github.com/GQYTeam/GQY.git
cd GQY

# 编译安装
cargo build --release --locked

# 安装到系统路径
cp target/release/gqy /opt/homebrew/bin/
```

## 验证安装

```zsh
# 检查版本
gqy --version

# 检查安装路径
which gqy

# 运行诊断
gqy doctor
```

## 首次运行

```zsh
# 启动 GQY
gqy
```

首次启动会自动：
1. 创建配置目录 `~/.config/gqy/`
2. 生成默认配置文件 `config.jsonc`
3. 创建数据目录 `~/.local/share/gqy/`
4. 初始化记忆和知识库

## 环境变量配置

推荐设置 `GQY_HOME` 环境变量统一管理所有数据：

```zsh
# 添加到 ~/.zshrc 或 ~/.bashrc
export GQY_HOME="$HOME/Library/Application Support/gqy"
```

### 可选环境变量

| 变量名 | 默认值 | 说明 |
|--------|--------|------|
| `GQY_HOME` | `~/Library/Application Support/gqy` | 数据根目录 |
| `GQY_SHARE_DIR` | 自动检测 | 共享资源目录 |
| `GQY_PROJECT_DIR` | `~/Desktop/GQY` | 项目源码目录 |
| `GQY_WORKSPACE` | `~/gqy-workspace` | 临时工作区 |
| `GQY_LANG` | `auto` | 界面语言 (auto/en/zh) |
| `GQY_CHANNEL` | 自动检测 | 通信通道 (terminal/webui/qq/tg) |

## 卸载

### Homebrew 安装的版本

```zsh
brew uninstall gqy
brew untap GQYTeam/GQY
```

### 清理数据（可选）

```zsh
# 删除配置和数据
rm -rf ~/.config/gqy
rm -rf ~/Library/Application Support/gqy
rm -rf ~/Library/Caches/gqy
```

## 更新

### Homebrew 安装的版本

```zsh
brew update
brew upgrade gqy
```

### 源码编译的版本

```zsh
cd GQY
git pull
cargo build --release --locked
cp target/release/gqy /opt/homebrew/bin/
```

## 故障排除

### 安装失败

1. **Homebrew 仓库不存在**
   ```zsh
   # 检查仓库是否正确添加
   brew tap | grep GQY
   
   # 重新添加仓库
   brew untap GQYTeam/GQy 2>/dev/null
   brew tap GQYTeam/GQY
   ```

2. **编译失败**
   ```zsh
   # 检查 Rust 工具链
   rustc --version
   cargo --version
   
   # 更新 Rust
   rustup update
   ```

3. **权限问题**
   ```zsh
   # 修复权限
   sudo chown -R $(whoami) /opt/homebrew/bin/
   ```

### 运行时错误

1. **配置文件损坏**
   ```zsh
   # 重新生成配置
   gqy config validate
   # 或删除配置文件重新初始化
   rm ~/.config/gqy/config.jsonc
   gqy
   ```

2. **数据库错误**
   ```zsh
   # 检查数据库完整性
   gqy doctor --check-db
   
   # 重建数据库
   gqy reset --scope state
   ```

## 下一步

- [快速开始](../快速开始/README.md) - 学习基本使用
- [配置指南](../配置指南.md) - 详细配置选项
- [功能详解](../功能详解/README.md) - 了解所有功能