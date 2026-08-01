# GQY 工具包标准（Tool Package Standard）

> 目标：让「发个仓库给顾清影，她能 100% 兼容地转换成自己的工具，长期使用」这件事有章可循。

## 一、工具包是什么

一个**工具包**是一个目录或 Git 仓库，里面放若干可执行脚本/程序，以及（可选的）清单文件。
用一条命令导入后，里面的每个脚本都会变成 GQY 可调用的**脚本工具**，随对话长期可用、
随备份快照、换机恢复后依然在：

```zsh
gqy tools import ./my-tools            # 本地目录
gqy tools import https://github.com/xxx/gqy-tools-demo   # Git 仓库（自动克隆）
gqy tools list                        # 查看已导入的工具包
```

## 二、两种形态（推荐清单，兜底自动扫描）

### 形态 A：带清单（推荐，100% 兼容）

仓库根放 `gqy-tools.json`（`manifest.json` / `index.json` 也认），格式与 GQY 脚本工具一致：

```json
{
  "scripts": [
    {
      "id": "disk_usage",
      "display_name": "磁盘占用分析",
      "description": "扫描目录并返回占用最大的文件/目录 TOP N。",
      "path": "bin/disk-usage.sh",
      "parameters": {
        "type": "object",
        "properties": {
          "dir": { "type": "string", "description": "要扫描的目录，默认当前目录" },
          "limit": { "type": "integer", "description": "返回条数，默认 10" }
        },
        "required": ["dir"]
      },
      "timeout_seconds": 30,
      "load_policy": "group",
      "groups": ["system"]
    }
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | ✅ | 工具唯一标识（脚本文件名无关） |
| `path` | ✅ | 相对仓库根的可执行文件路径（禁止 `..` 越界） |
| `description` | ✅ | 给模型的工具说明，写清楚用途与参数 |
| `display_name` | 否 | 界面显示名 |
| `parameters` | 否 | JSON Schema 参数定义（缺省 `{}`，无参数） |
| `timeout_seconds` | 否 | 超时（缺省 120s） |
| `always_loaded` | 否 | 是否常驻模型定义（缺省 false，懒加载） |
| `load_policy` | 否 | `group`（按分组加载）/ `summary`（按需） |
| `groups` | 否 | 分组名，配合 `load_tools` 批量加载 |

### 形态 B：自动扫描（零配置兜底）

没有清单时，GQY 自动扫描仓库内**有执行权限的文件**（跳过 `.git`、`.` 开头文件），
描述取脚本头部的 `Description:` 注释（`# Description: xxx` / `; Description:` 均可）。

## 二点五、先理解再导入（推荐流程）

仓库里常常混着**与功能无关的脚本**（构建、发布、CI 辅助），无脑全转会让模型面对一堆
候选时误用。推荐两步走：

```zsh
# 1. 先看候选：列出所有可执行脚本 + 头部摘要，判断哪些是核心功能
gqy tools inspect https://github.com/xxx/tool-repo

# 2. 精准导入核心工具（构建/发布脚本不转）
gqy tools import https://github.com/xxx/tool-repo --only download.sh,install.sh
```

给顾清影的用法：把仓库链接发给她，让她先 `gqy tools inspect` + 读 README 判断核心功能，
再 `gqy tools import ... --only` 导入，最后 `gqy tools list` 确认。她全程可自主完成。

## 三、导入后的行为

- 文件复制到 `GQY_HOME/config/scripts/<包名>/`，权限 755；
- 生成 `index.json` 清单，**每轮对话自动扫描注册**，立刻可用；
- 工具按 `load_policy` 懒加载：模型需要时用 `load_tools` 加载，或对话里让 GQY 加载；
- 长期有效：随备份快照；`gqy backup restore` 换机恢复后重新 `gqy tools import <仓库>` 即可（推荐用 Git 仓库形态，天然可更新：再导入一次自动 `git pull`）。

## 四、给工具作者的要求

1. **可执行**：`chmod +x`；任意语言（shell/python/node/swift…），系统需有对应运行时；
2. **参数走命令行或 stdin**：脚本工具通过 stdin 传入 JSON 参数（`{"dir": "/tmp"}`），
   也接受位置参数；输出打印到 stdout 即工具结果；
3. **描述要写清**：`description` 决定模型会不会用、用得对不对；
4. **不依赖网络/密钥**：工具应自包含；需要凭据时通过环境变量注入，不要写死在脚本里；
5. **安全**：GQY 会做路径越界校验；工具自身的操作风险由描述与使用场景承担
   （写文件类工具请在自己的描述里声明）。

## 五、让 GQY 自己导入

对话里说「把我这个仓库转成你的工具」并把路径/URL 给她即可——她会执行
`gqy tools import <url>` 完成导入（她能跑命令）。导入后让她 `gqy tools list`
确认工具列表，之后就能长期调用了。
