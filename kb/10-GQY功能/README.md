# GQY 功能指南

本目录包含 GQY 特有功能的详细文档，帮助用户充分利用 GQY 的各种能力。

## 功能分类

### 记忆系统
- [记忆系统](记忆系统.md) - Cindy Memory 集成、记忆搜索与回忆、事实记录与遗忘

### Agent 集群
- [Agent集群](Agent集群.md) - 多 Agent 协作、Agent 创建与管理、并行任务执行

### 工具系统
- [工具系统](工具系统.md) - 可用工具概览、工具注册与加载、工具权限与层级

## 快速开始

### 记忆系统
```zsh
# 记住一个事实
gqy memory remember "用户喜欢使用 Rust 编程"

# 搜索记忆
gqy memory search "编程偏好"

# 列出所有记忆
gqy memory list
```

### Agent 集群
```zsh
# 创建新 Agent
gqy agent create --name "研究员" --role "负责技术调研"

# 与 Agent 对话
gqy agent talk 研究员 "请调研 Rust 异步编程最佳实践"

# 列出所有 Agent
gqy agent list
```

### 工具系统
```zsh
# 查看可用工具
gqy tools list

# 查看工具详情
gqy tools info web_search

# 注册自定义脚本
gqy scripts register my-script.sh --name "我的脚本"
```

## 相关文档

- [快速开始](../快速开始.md) - GQY 安装与基本使用
- [架构说明](../架构说明.md) - GQY 系统架构概述
- [常见问题](../常见问题.md) - 常见问题与解决方案