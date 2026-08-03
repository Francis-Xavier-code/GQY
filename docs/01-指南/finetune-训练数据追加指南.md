# 自我进化：训练数据收集与 LoRA 微调

GQY 的「自我进化」功能：每一轮对话自动标注生成训练样本，日积月累形成专属数据集，
用本地 MLX 对底座（Qwen3-4B-Instruct）做 LoRA 增量微调，产出「长成顾清影样子」的专属权重。

> 状态：一期（数据收集）已完成并默认可开启；二期（MLX 微调脚本）已就绪待跑；
> 三期（权重热加载集成）规划中。**明确不做「每轮即时微调」**——原因见下文「设计约束」。

## 为什么不做每轮即时微调

微调是「批量蒸馏风格」的过程，不是「在线学习」：
- **灾难性遗忘**：单条样本连续微调会覆盖模型原有知识；
- **过拟合**：模型会复读那条对话；
- **性能/费用灾难**：训练时权重被锁、聊天中断。

正确做法：**数据每轮自动攒，微调攒够才训**（阈值建议 500–1000 条，或每周一次）。

## 数据收集

### 存储

```
$GQY_HOME/data/finetune/
├── turns.jsonl            # 训练样本（每行一条 JSON）
└── GUIDE-追加训练数据.md   # 数据追加指南（给 AI/作者追加理想样本用）
```

### 样本格式（一行一条 JSON）

```json
{"ts": 1785762490, "mode": "chat", "user": "用户输入", "assistant": "顾清影回复", "tools": ["search_meme"]}
```

| 字段 | 说明 |
|---|---|
| `ts` | unix 秒 |
| `mode` | `normal`（日常）/ `plan`（编码）/ `chat`（女友态闲聊） |
| `tools` | 回复用到的工具名，无则 `[]` |

### 开关与来源

- **自动收集**：`finetune.collect`（默认 `false`，因涉及对话内容落盘，需显式开启）：
  ```zsh
  gqy config set finetune.collect true
  ```
  每轮对话收尾时由 `src/finetune.rs::record_turn` 追加一条（best-effort，失败静默）。
- **手动追加**：按数据目录里的追加指南补理想样本（种子数据，用于纠正风格）。

## 微调流程（二期，脚本已就绪）

```zsh
bash finetune-mlx.sh
```

内置流程：环境检查 → 统计样本（不足阈值自动拦截）→ 清洗（去重/短样本/隐私关键词）→
混入 30% 通用数据防遗忘 → MLX LoRA 训练（Qwen3-4B-Instruct，rank 8，2 epoch）→
权重存档（`data/finetune/lora/<日期>/adapter.safetensors`）+ 训练报告。

费用：本地 MLX，每次约 0.1 元电费，数据不出本机。

产物两种形态：
- **LoRA 补丁**（默认，几十 MB）：`lora/<日期>/adapter.safetensors`，运行时叠加底座使用；
- **完整模型**（`GQY_MERGE=1`，~8GB）：`lora/<日期>/merged/`，LoRA 合并进底座后的完整模型目录，
  可复制、命名（如「顾清影·专属 v1」）、当普通模型加载，也可作为下一次训练的新底座。

## 三期规划（权重热加载）

- GQY provider 层支持加载最新 LoRA adapter（`mlx_lm.server --adapter-path ...`）；
- 新权重就绪后热切换继续聊天（复用切换供应商时的上下文压缩逻辑）；
- 多代权重管理：`gqy finetune list` / 回滚。

## 记忆与微调的分工（重要）

- **微调负责风格**：说话方式、称呼、节奏——「像她」；
- **记忆系统负责知识**：facts/episodes 向量检索——「记得你」；
- 两者互补，**不要把具体记忆微调进权重**（记忆持续变化，参数是静态快照，
  会过期、冲突、无法纠错）。详见记忆架构文档。

## 已知限制

- 数据收集默认关闭，需用户显式开启；
- `config set` 在旧版本（无 `finetune` 配置段的二进制）上会报
  `intermediate key is not an object`——新版本已自动创建中间段；
- brew 旧版二进制不认识 `finetune` 段，配置保存会丢该段，请使用源码编译版；
- 微调效果以说话风格为主，不增加知识量。

## 关联文档

- 数据追加指南（作者/AI 追加理想样本）：`$GQY_HOME/data/finetune/GUIDE-追加训练数据.md`
- 自我进化方案评估：工作区 `self-evolving-training.md` / `memory-in-model.md`
