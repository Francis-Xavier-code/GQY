<p align="center">
  <img src="pics/GQY-icon.png" alt="顾清影" width="180">
</p>

# GQY —— 顾清影

一个活在 `MAC终端/菜单栏` 里的二次元少女。

macOS 独立主目录、菜单栏壳与私有 Git 记忆备份的当前用法见 [macOS、独立主目录与记忆备份](docs/macos-portable-home-and-backup.md)。


## 谁是 GQY？

GQY 是从我的想法中诞生出来的人格，从 [shorin-miyu](https://github.com/SHORiN-KiWATA/Miyu)`FORK` 过来的一个终端助理

![](./pics/GQY-image.png)

## 有什么功能？

`GQY` 由大模型驱动，默认接入了 [opencode](https://github.com/anomalyco/opencode) 的公共模型服务，你也可以配置自己的大模型服务。她并非专业的 Coding Agent，而是更偏向聊天日常、游戏娱乐、系统排障等日用场景。并且 `GQY` 无缝与 `zsh`（mac） 集成，终端打字直接无缝对话！

`GQY` 还自带了 TUI 方便修改配置：

```
gqy config
```

她的所有配置、记忆和对话状态都收拢在独立主目录 `GQY_HOME`（建议 `~/Library/Application Support/GQY`）中，与宿主机其他配置隔离；每一轮对话后还会自动生成 Git 快照保存记忆，绑定私有远程仓库后自动推送，换机器一键恢复。详见 [macOS、独立主目录与记忆备份](docs/macos-portable-home-and-backup.md)。

## 如何安装？

需要安装 Rust 1.96 或更新版本、C 编译工具链，图片显示功能依赖 `chafa`（`brew install chafa`）。

```
git clone https://github.com/Francis-Xavier-code/GQY.git
cd GQY
cargo build --release --locked
./target/release/gqy --version
```

macOS 依赖示例：

```
brew install rust chafa
```

首次运行（会创建 GQY_HOME 与默认配置）：

```
export GQY_HOME="$HOME/Library/Application Support/GQY"
./target/release/gqy
```

### 菜单栏

轻量 AppKit 菜单栏壳位于 `macos/GQYMenuBar`，不需要完整 Xcode：

```
zsh macos/GQYMenuBar/build.sh
open "macos/GQYMenuBar/.build/顾清影.app"
```

菜单提供终端对话、本地 Web 面板、立即备份、打开独立主目录、开机自启与退出。详细说明见 [macOS、独立主目录与记忆备份](docs/macos-portable-home-and-backup.md)。

### 界面语言

GQY 的 CLI、REPL、配置 TUI 和工具状态支持英文与简体中文。在 `gqy config` 的“全局设置 / Global Settings”中可将“界面语言 / Interface language”设为：

- `auto`：默认值，跟随系统 locale
- `en`：英文
- `zh`：简体中文

`GQY_LANG=en` 或 `GQY_LANG=zh` 可以临时覆盖配置。语言选择优先级为 `GQY_LANG`、`display.language`、系统 locale；在配置 TUI 中保存后，下次启动 GQY 时生效。

### 内置功能

<details><summary>[展开/收起] 具体介绍</summary>
<br>

- 表情包

  表情包毫无疑问是聊天时最重要的部分，在对话时，GQY 会根据情景自主发送符合情境的表情包。除了自主发送，设置里还可以设置概率、置信度和冷却时间。表情库跟随人格，你可以准备一些图片，把路径给 Ai，让其保存到表情库。GQY 默认使用 opencode 公共模型服务中的多模态模型进行识图，所以即使不配置自己的多模态模型也可以看图片。

- 玄学算命

  >心理学。

  算命就像看天气预报一般稀松平常。GQY 自带了周易六十四卦、吉凶占、塔罗牌抽取等玄学功能。

- 投骰子

  >赌！

  闲来无事可以和 AI 比比大小。

- 闹钟

  >要我说，这比系统自带时钟的闹钟好用多了

  GQY 自带了闹钟，日常泡泡面、番茄钟学习、计时任务什么的都很实用。内置了闹钟音频，你还可以通过路径传入你想要在到点后播放的“闹钟”。

- 知识库

  你可以通过 `gqy kb` 命令，或者通过跟 AI 的自然语言交互管理属于你自己的知识库。回答问题时 GQY 会优先查询知识库里的可信内容。

- 网络搜索

  即使不配置网络搜索 API，GQY 也仍然拥有基础的网络搜索和网页读取能力。可以在插件配置中设置 Tavily、Firecrawl、AnySearch、SearXNG 等网络搜索 API 以获得更佳的搜索效果。

- 搜图

  GQY 还能帮你找图片喔！搜图会根据网络环境并行使用多个来源，并通过视觉模型筛选相关且安全的结果。图片会默认保存至 GQY 的图片目录。

  >NSFW 禁止！

- 生图

  支持 OpenAI 的画图服务喔。图片会默认保存至 GQY 的图片目录。

  >这个功能默认用不了，要自己在插件设置里开启并配置 API

- 天气查询

  查询天气是每天的必做活动，当然少不了。

- 汇率查询

  国际社会，查个汇率也很合理吧？

- Man 手册查询

  >Man！

  专门的手册查询工具，虽然网络搜索也能做到，但这值得做成单独的插件。

- 文件操作

  >自不必说。

  GQY 支持读写文件、搜索内容、查找文件、删除文件等。

- 计算器和哈希编解码

  为了计算结果的准确性，GQY 自带了科学计算器和哈希编解码的能力。

- 记忆系统

  GQY 的记忆由两部分组成，其一是“曾经发生的事”，其二是“信息中的知识点”。对话时会根据用户消息自动召回条目，这是联想功能。每一轮对话结束后，记忆会落盘并由独立 Git 备份自动快照保存。

- 深度研究

  >Token 燃烧警告

  重量级插件。对于一个命题，GQY 可以引经据典，有理有据地进行深度研究并写出研究报告。

</details>

## 做出贡献

<details><summary>[展开/收起] 如果你想要一同开发 GQY 请先阅读下面的内容</summary>
<br>

### 设计理念

GQY 的定位是桌面助手，不是 Coding Agent，她更注重拟真、系统集成度、实用、日常排障等方面。GQY 应该开箱即用，并且足够轻量，不开发超重的 3D 桌宠，不使用 GUI 框架，也不设计需要学习成本的 CLI 选项，尽量通过自然语言和无缝无感的触发方式进行所有的操作。

以下是一些可能的方向：

- 提升系统日常排障能力、系统维护能力

  作为桌面助手，尤其是 macOS 桌面端助手，对日常问题的排障能力是重中之重。她应当能够解决日用系统会遇到的问题，如软件崩溃、磁盘空间、网络代理、启动项异常等。

- 知识和信息

  扩充她自己的知识库。增加对软件推荐、时事新闻、学习辅助等非开发场景下会出现的情景的处理能力。增加知识和信息检索的时效性和可靠性也是关键点。

- 提升角色扮演能力，提高对话娱乐性和拟真度

  需要更多像“发送表情包”、“玄学算命”那样提升对话时的趣味性或拟真度的功能。TTS、语音对话等重要功能也在日程上。

- 提高和系统的无缝集成

  不使用任何命令作为触发器，能够直接使用自然语言开启对话。目前是通过 Command Not Found 内容交给 GQY 的方式做到和终端的无缝集成，但是逐行解释命令的特点导致提示词包含多行内容时每一行都会调用一次，如何支持多行无缝对话是一个需要研究的点。

  终端以外的集成也值得研究，例如做成守护进程，拥有持续运行的能力，监听系统事件，在特定事件发生时做出特定反应等。

- 优化功能和修复 BUG

  在不变更设计语义，不影响现有功能效果的前提下优化运行表现，修复 BUG。已知目前流式输出兼容和工具调用兼容有点问题，不是所有模型都正常。

### 如何 PR

PR时必须提供功能的设计理念，作用场景和实际意义。一个 PR 必须仅包含一个功能，若包含多个功能，应当拆分后提交多个 PR。

</details>


## 致谢

- [opencode](https://github.com/anomalyco/opencode) 最好的开源 Coding Agent。

## 许可

GQY 使用 MIT License 发布，见 `LICENSE`。
