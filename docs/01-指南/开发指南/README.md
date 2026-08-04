# GQY 开发指南

本指南为 GQY 开发者提供开发环境搭建、代码结构说明和贡献指南。

## 开发环境搭建

### 系统要求

- **操作系统**: macOS 12.0 或更高版本
- **Rust**: 1.97.1 或更高版本
- **Git**: 2.30 或更高版本
- **Xcode Command Line Tools**: 最新版本

### 安装 Rust

```zsh
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 安装 nightly 工具链（某些功能需要）
rustup install nightly
rustup default stable

# 验证安装
rustc --version
cargo --version
```

### 克隆仓库

```zsh
# 克隆仓库
git clone https://github.com/GQYTeam/GQY.git
cd GQY

# 检查项目状态
cargo check
```

### 安装依赖

```zsh
# 安装系统依赖（macOS）
brew install pkg-config

# 安装开发工具
cargo install cargo-watch
cargo install cargo-expand
```

## 项目结构

```
GQY/
├── src/
│   ├── main.rs              # 入口点
│   ├── cli/                 # CLI 命令
│   │   ├── mod.rs           # CLI 模块
│   │   ├── args.rs          # 参数定义
│   │   ├── repl.rs          # REPL 实现
│   │   └── commands/        # 子命令实现
│   ├── agent/               # Agent 核心
│   │   ├── mod.rs           # Agent 模块
│   │   ├── compact.rs       # 上下文压缩
│   │   ├── tool_loop.rs     # 工具循环
│   │   ├── conversation.rs  # 对话管理
│   │   └── overflow.rs      # 溢出处理
│   ├── agents.rs            # Agent 集群
│   ├── llm/                 # LLM 客户端
│   │   ├── mod.rs           # LLM 模块
│   │   ├── openai_compatible.rs  # OpenAI 兼容客户端
│   │   └── pi_rpc.rs        # pi RPC 客户端
│   ├── tools/               # 工具系统
│   │   ├── mod.rs           # 工具模块
│   │   ├── registry.rs      # 工具注册
│   │   ├── scripts.rs       # 脚本工具
│   │   ├── skills.rs        # 技能系统
│   │   └── ...              # 各种工具实现
│   ├── memory/              # 记忆系统
│   │   └── mod.rs           # 记忆模块
│   ├── state/               # 状态管理
│   │   ├── mod.rs           # 状态模块
│   │   ├── conversation_db.rs  # 对话数据库
│   │   └── usage.rs         # 用量统计
│   ├── config.rs            # 配置管理
│   ├── paths.rs             # 路径管理
│   ├── i18n.rs              # 国际化
│   ├── render/              # 渲染系统
│   │   ├── mod.rs           # 渲染模块
│   │   └── wait_spinner.rs  # 等待动画
│   ├── web.rs               # Web 面板
│   ├── bridges/             # 桥接系统
│   │   ├── mod.rs           # 桥接模块
│   │   ├── napcat.rs        # QQ 桥接
│   │   └── tg.rs            # Telegram 桥接
│   ├── prompts/             # 系统提示
│   ├── scripts/             # 内置脚本
│   ├── memes/               # 表情包
│   └── ...                  # 其他模块
├── web/                     # Web 前端
│   ├── index.html
│   ├── styles.css
│   └── app.js
├── kb/                      # 知识库
├── docs/                    # 文档
├── Cargo.toml               # Rust 配置
├── Cargo.lock               # 依赖锁定
└── build.rs                 # 构建脚本
```

## 核心模块

### 1. CLI 模块 (`src/cli/`)

CLI 模块负责命令行界面：

- **args.rs** - 命令行参数定义
- **mod.rs** - 命令分发和执行
- **repl.rs** - 交互式 REPL
- **commands/** - 子命令实现

### 2. Agent 模块 (`src/agent/`)

Agent 模块是 GQY 的核心：

- **mod.rs** - Agent 定义和生命周期
- **compact.rs** - 上下文压缩算法
- **tool_loop.rs** - 工具调用循环
- **conversation.rs** - 对话管理
- **overflow.rs** - 上下文溢出处理

### 3. LLM 模块 (`src/llm/`)

LLM 模块负责与 AI 模型通信：

- **openai_compatible.rs** - OpenAI 兼容客户端
- **pi_rpc.rs** - pi RPC 客户端

### 4. 工具模块 (`src/tools/`)

工具模块包含所有内置工具：

- **registry.rs** - 工具注册和管理
- **scripts.rs** - 脚本工具系统
- **skills.rs** - 技能系统
- 各种工具实现文件

### 5. 记忆模块 (`src/memory/`)

记忆模块管理持久化记忆：

- **mod.rs** - 记忆存储和检索

### 6. 状态模块 (`src/state/`)

状态模块管理应用状态：

- **conversation_db.rs** - 对话数据库
- **usage.rs** - 用量统计

## 构建和测试

### 编译检查

```zsh
# 快速编译检查
cargo check

# 完整编译
cargo build

# 发布版本
cargo build --release --locked
```

### 运行测试

```zsh
# 运行所有测试
cargo test

# 运行特定测试
cargo test <test-name>

# 运行单线程测试
cargo test -- --test-threads=1
```

### 代码检查

```zsh
# 运行 clippy
cargo clippy -- -W warnings

# 格式化代码
cargo fmt

# 检查格式
cargo fmt --check
```

### 开发模式

```zsh
# 使用 cargo-watch 监听变化
cargo watch -x check

# 监听并运行测试
cargo watch -x test

# 监听并运行特定测试
cargo watch -x "test <test-name>"
```

## 添加新工具

### 1. 创建工具文件

在 `src/tools/` 目录下创建新文件：

```rust
// src/tools/my_tool.rs
use crate::tools::registry::{ToolRegistry, ToolSpec};
use serde_json::{json, Value};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(ToolSpec {
        name: "my_tool".to_string(),
        description: "我的自定义工具".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "输入参数"
                }
            },
            "required": ["input"]
        }),
        permission: crate::tools::registry::ToolPermission::Auto,
    });
}

pub async fn execute(args: Value) -> anyhow::Result<String> {
    let input = args["input"].as_str().unwrap_or("");
    Ok(format!("处理结果: {}", input))
}
```

### 2. 注册工具

在 `src/tools/mod.rs` 中注册：

```rust
mod my_tool;

pub fn builtin_registry(config: &AppConfig, paths: &GqyPaths) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    
    // 注册其他工具...
    my_tool::register(&mut registry);
    
    registry
}
```

### 3. 添加工具描述

在 `src/tools/descriptions/` 目录下添加描述文件。

## 添加新命令

### 1. 定义命令参数

在 `src/cli/args.rs` 中添加：

```rust
#[derive(Subcommand)]
pub enum Command {
    // 现有命令...
    
    /// 我的自定义命令
    MyCommand(MyCommandArgs),
}

#[derive(Args)]
pub struct MyCommandArgs {
    /// 输入参数
    #[arg(short, long)]
    pub input: String,
}
```

### 2. 实现命令

在 `src/cli/commands/` 目录下创建新文件：

```rust
// src/cli/commands/my_command.rs
use crate::cli::args::MyCommandArgs;
use crate::paths::GqyPaths;
use anyhow::Result;

pub fn run_my_command(paths: &GqyPaths, args: MyCommandArgs) -> Result<()> {
    println!("执行命令: {}", args.input);
    Ok(())
}
```

### 3. 注册命令

在 `src/cli/mod.rs` 中注册：

```rust
mod commands;

pub async fn run(cli: Cli, paths: GqyPaths) -> Result<()> {
    match cli.command {
        // 现有命令...
        Some(Command::MyCommand(args)) => commands::my_command::run_my_command(&paths, args),
        // ...
    }
}
```

## 代码风格

### 命名规范

- **模块名**: 小写下划线 (`my_module`)
- **类型名**: 大驼峰 (`MyType`)
- **函数名**: 小写下划线 (`my_function`)
- **常量**: 大写下划线 (`MY_CONSTANT`)

### 注释规范

```rust
/// 函数文档注释
/// 
/// # Arguments
/// 
/// * `param1` - 参数1说明
/// * `param2` - 参数2说明
/// 
/// # Returns
/// 
/// 返回值说明
/// 
/// # Examples
/// 
/// ```rust
/// let result = my_function("value1", "value2");
/// ```
pub fn my_function(param1: &str, param2: &str) -> String {
    // 实现注释
    format!("{} {}", param1, param2)
}
```

### 错误处理

```rust
use anyhow::{Context, Result};

fn my_function() -> Result<()> {
    // 使用 context 添加错误上下文
    let file = std::fs::read_to_string("file.txt")
        .context("读取文件失败")?;
    
    // 使用 bail 返回错误
    if file.is_empty() {
        anyhow::bail!("文件为空");
    }
    
    Ok(())
}
```

## 测试

### 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_my_function() {
        let result = my_function("hello", "world");
        assert_eq!(result, "hello world");
    }
    
    #[tokio::test]
    async fn test_async_function() {
        let result = async_function().await.unwrap();
        assert!(result > 0);
    }
}
```

### 集成测试

```rust
// tests/integration_test.rs
use gqy::my_module;

#[test]
fn test_integration() {
    let result = my_module::my_function();
    assert!(result.is_ok());
}
```

### 测试环境

```rust
use tempfile::tempdir;

#[test]
fn test_with_tempdir() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    
    std::fs::write(&file_path, "test content").unwrap();
    
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "test content");
}
```

## 调试

### 日志

```rust
use tracing::{info, warn, error, debug};

fn my_function() {
    info!("函数开始执行");
    debug!("调试信息: {:?}", some_value);
    warn!("警告信息");
    error!("错误信息");
}
```

### 环境变量

```zsh
# 启用调试日志
RUST_LOG=debug cargo run

# 启用特定模块日志
RUST_LOG=gqy::agent=debug cargo run

# 启用跟踪日志
RUST_LOG=trace cargo run
```

## 贡献指南

### 1. Fork 仓库

```zsh
# Fork 仓库到你的 GitHub 账号
# 然后克隆
git clone https://github.com/your-username/GQY.git
cd GQY
```

### 2. 创建分支

```zsh
# 创建功能分支
git checkout -b feature/my-feature

# 创建修复分支
git checkout -b fix/my-fix
```

### 3. 提交更改

```zsh
# 添加更改
git add .

# 提交更改
git commit -m "feat: 添加我的功能"

# 推送更改
git push origin feature/my-feature
```

### 4. 创建 Pull Request

在 GitHub 上创建 Pull Request，描述你的更改。

### 提交信息规范

```
<type>(<scope>): <subject>

<body>

<footer>
```

类型：
- `feat`: 新功能
- `fix`: 修复
- `docs`: 文档
- `style`: 格式
- `refactor`: 重构
- `test`: 测试
- `chore`: 构建/工具

## 下一步

- [API参考](../API参考/README.md) - 工具和命令参考
- [架构说明](../架构说明/README.md) - 系统架构概述
- [故障排除](../故障排除/README.md) - 常见问题解决