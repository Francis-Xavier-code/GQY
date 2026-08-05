<p align="center">
  <img src="pics/GQY-icon.png" alt="GQY" width="160">
</p>

<h1 align="center">GQY — 顾清影</h1>

<p align="center">
  活在终端与菜单栏里的 AI 助理
</p>

<p align="center">
  <a href="https://github.com/GQYTeam/GQY/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License"></a>
  <img src="https://img.shields.io/badge/Rust-1.97.1-orange?logo=rust" alt="Rust">
  <a href="https://github.com/GQYTeam/GQY/actions"><img src="https://img.shields.io/github/actions/workflow/status/GQYTeam/GQY/ci.yml?label=CI" alt="CI"></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple" alt="Platform">
</p>

---
## 仓库停更解释说明

过往代码由于长期使用`vibe coding`后累积了大量的屎山代码，为防止项目远离初衷，现保留关键文件为后续重构做铺垫。
如果你想查看过往代码以及架构，可以查看仓库过往git历史进行复线。
如果想了解之前的项目架构设计理念，可以阅读 [byebye_副本/README.md](/byebye_副本/README.md)以及[byebye_副本/ARCHITECTURE.md](/byebye_副本/ARCHITECTURE.md)


---
# 角色说明

GQY 是一个大模型驱动的 CLI AI 助理，运行在 macOS 终端和菜单栏中。她是 Coding Agent + 日常聊天、系统排障、娱乐互动的桌面伴侣。
##  角色设计图片展示

![](/image-and-video/顾清影1.png)
![](/image-and-video/顾清影2.png)
![](/image-and-video/顾清影3.png)


默认接入 [opencode](https://github.com/anomalyco/opencode) 公共模型服务，开箱即用（可采用自然语言让免费模型帮你添加`GQY`的模型服务商）；也支持任意 OpenAI 兼容接口、本地模型（llama.cpp / Ollama）和 [pi](https://github.com/earendil-works/pi) 底座模式。

<p align="center">
  <img src="pics/GQY-image.png" alt="GQY Demo" width="680">
</p>

## 安装

### 一键安装（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/GQYTeam/GQY/main/install.sh | bash
```

自动检测平台（macOS Intel/Apple Silicon、Linux x86_64/aarch64），下载预编译二进制并安装共享资源。


## 致谢

- [opencode](https://github.com/anomalyco/opencode) — 最好的开源 Coding Agent

## 许可

本项目使用 [GPL-3.0](LICENSE) 发布。新增代码与修改按 GPL-3.0 授权。
