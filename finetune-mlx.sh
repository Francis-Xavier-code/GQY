#!/usr/bin/env bash
# 顾清影自我进化 · 二期：MLX LoRA 批量微调（Apple Silicon 本地，免费可预测）
# 底座：Qwen3-4B-Instruct（16GB M2 完美）；微调+推理全走 MLX 生态。
#
# 流程：检查数据 → 清洗/混入通用数据 → LoRA 训练 → 权重存档 →（可选）合并 → 报告
# 触发：攒够阈值（默认 500 条）或每周一次，绝不做每轮训练。
#
# 用法：
#   bash finetune-mlx.sh [GQY_HOME]                # 基本训练（只产 LoRA 补丁，几十 MB）
#   GQY_MERGE=1 bash finetune-mlx.sh               # 训练后合并 → 完整可命名模型（~8GB）
#
# 环境变量（可选）：
#   GQY_HOME          数据目录（默认 ~/Library/Application Support/gqy），也可作第 1 个参数传入
#   GQY_BASE_MODEL    底座模型（默认 Qwen/Qwen3-4B-Instruct；合并模型可作下一代底座）
#   GQY_MIN_SAMPLES   训练门槛条数（默认 500，不足则拦截提示）
#   GQY_EPOCHS        训练轮数（默认 2）
#   GQY_LR            学习率（默认 2e-5）
#   GQY_GENERIC_FILE  通用数据文件（JSONL），防灾难性遗忘；默认内置少量占位
#   GQY_MERGE         设 1 时训练完把 LoRA 合并进底座，产出完整模型 lora/<日期>/merged/
set -euo pipefail

HOME_DIR="${1:-$HOME/Library/Application Support/gqy}"
DATA_DIR="$HOME_DIR/data/finetune"
TURNS="$DATA_DIR/turns.jsonl"
LORA_ROOT="$DATA_DIR/lora"
BASE_MODEL="${GQY_BASE_MODEL:-cognitivecomputations/Dolphin-2.9.2-qwen2.5-7b}"
# 通用数据文件（JSONL，{user,assistant} 每行一条），防止灾难性遗忘。
# 默认用内置少量占位；建议放一份通用中文指令集（如 alpaca-zh），混入效果更好。
GENERIC_FILE="${GQY_GENERIC_FILE:-}"
# 合并开关：GQY_MERGE=1 时训练完把 LoRA 合并进底座，产出完整可命名模型（~8GB）
MERGE="${GQY_MERGE:-0}"
MIN_SAMPLES="${GQY_MIN_SAMPLES:-500}"
EPOCHS="${GQY_EPOCHS:-2}"
LR="${GQY_LR:-2e-5}"

echo "==> 0/6 环境检查"
if ! command -v uv >/dev/null 2>&1; then
  echo "需要 uv：brew install uv"; exit 1
fi
if [ ! -f "$TURNS" ]; then
  echo "还没有训练数据。先开启收集：gqy config set finetune.collect true 再聊几天。"
  exit 1
fi
echo "   数据：$TURNS"

echo "==> 1/6 统计样本"
TOTAL=$(wc -l < "$TURNS" | tr -d ' ')
echo "   已收集 $TOTAL 条"
if [ "$TOTAL" -lt "$MIN_SAMPLES" ]; then
  echo "   不足 $MIN_SAMPLES 条（还差 $((MIN_SAMPLES - TOTAL)) 条），暂不训练——"
  echo "   这就是「攒够才训」：微调需要成百上千条同分布数据，单条训练只会灾难性遗忘。"
  exit 0
fi

echo "==> 2/6 清洗（去重/过滤短样本/隐私关键词）"
CLEAN="$DATA_DIR/train.clean.jsonl"
python3 - "$TURNS" "$CLEAN" << 'PY'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
seen = set()
out = []
for line in open(src, encoding='utf-8'):
    line = line.strip()
    if not line: continue
    try: r = json.loads(line)
    except: continue
    u, a = (r.get('user') or '').strip(), (r.get('assistant') or '').strip()
    if len(u) < 2 or len(a) < 20: continue           # 短样本
    key = (u, a)
    if key in seen: continue                          # 去重
    seen.add(key)
    if any(w in (u + a) for w in ('password', 'api_key', 'token=', '私钥', 'secret')):  # 隐私
        continue
    out.append(r)
with open(dst, 'w', encoding='utf-8') as f:
    for r in out: f.write(json.dumps(r, ensure_ascii=False) + '\n')
print(f"   清洗后 {len(out)} 条")
PY

echo "==> 3/6 混入通用数据（7:3 防灾难性遗忘）"
python3 - "$CLEAN" "$DATA_DIR/train.mixed.jsonl" "$GENERIC_FILE" << 'PY'
import json, sys, random
clean, dst, generic_file = sys.argv[1], sys.argv[2], (sys.argv[3] if len(sys.argv) > 3 else '')
GENERIC_FALLBACK = [
    {"user": "请介绍一下你自己。", "assistant": "我是顾清影，住在你的终端和菜单栏里的助手。"},
    {"user": "今天天气怎么样？", "assistant": "我看一下天气再告诉你，稍等。"},
    {"user": "帮我把这个想法整理成计划。", "assistant": "可以，我先梳理成几个步骤。"},
]
rows = [json.loads(l) for l in open(clean, encoding='utf-8')]
if generic_file:
    try:
        generic = [json.loads(l) for l in open(generic_file, encoding='utf-8')]
        print(f"   使用外部通用数据：{generic_file}（{len(generic)} 条）")
    except Exception as e:
        print(f"   外部通用数据读取失败（{e}），退回内置占位")
        generic = GENERIC_FALLBACK
else:
    generic = GENERIC_FALLBACK
random.seed(42)
sampled = random.choices(generic, k=len(rows) // 3)   # 30% 通用
mixed = rows + sampled
random.shuffle(mixed)
with open(dst, 'w', encoding='utf-8') as f:
    for r in mixed: f.write(json.dumps(r, ensure_ascii=False) + '\n')
print(f"   混入后 {len(mixed)} 条（专属:通用 ≈ 7:3）")
PY

echo "==> 4/6 转 MLX 训练格式 + 安装 mlx-lm"
uv tool install --force mlx-lm 2>/dev/null || uv pip install --system mlx-lm 2>/dev/null || true
TS=$(date +%Y%m%d-%H%M%S)
OUT="$LORA_ROOT/$TS"
mkdir -p "$OUT"

echo "==> 5/6 LoRA 训练（底座 $BASE_MODEL，epochs=$EPOCHS，lr=$LR）"
# 单轮样本模板：<|im_start|>user ... <|im_start|>assistant ...
python3 - "$DATA_DIR/train.mixed.jsonl" "$DATA_DIR/train.chat.jsonl" << 'PY'
import json, sys
with open(sys.argv[1], encoding='utf-8') as f, open(sys.argv[2], 'w', encoding='utf-8') as o:
    for line in f:
        r = json.loads(line)
        text = ("<|im_start|>user\n" + r['user'] + "\n<|im_end|>\n"
                "<|im_start|>assistant\n" + r['assistant'] + "\n<|im_end|>")
        o.write(json.dumps({"text": text}, ensure_ascii=False) + '\n')
print("   chat 格式就绪")
PY

uv run mlx_lm.lora \
  --model "$BASE_MODEL" \
  --train \
  --data "$DATA_DIR/train.chat.jsonl" \
  --iters "$((TOTAL * EPOCHS))" \
  --num-layers 8 \
  --batch-size 1 \
  --learning-rate "$LR" \
  --steps-per-report 20 \
  --adapter-path "$OUT/adapter"

echo "==> 6/6 存档与报告"
cp "$OUT/adapter/adapter.safetensors" "$OUT/adapter.safetensors" 2>/dev/null || true
cat > "$OUT/README.md" << MD
# 顾清影 LoRA $TS
- 底座：$BASE_MODEL
- 样本：$TOTAL 条（清洗后），epochs=$EPOCHS，lr=$LR
- 合并完整模型：${MERGE:+是（merged/）}
- 训练日期：$TS
MD
if [ "$MERGE" = "1" ]; then
  echo "==> 7/7 合并 LoRA 进底座（产出完整模型）"
  uv run mlx_lm.fuse \
    --model "$BASE_MODEL" \
    --adapter-path "$OUT/adapter" \
    --save-path "$OUT/merged"
  echo "✅ 合并完成：$OUT/merged"
  echo "  这是一个完整模型目录，可复制、命名、当普通模型加载，"
  echo "  也可作为下一次训练的新底座（GQY_BASE_MODEL=$OUT/merged）。"
fi

echo ""
echo "✅ 训练完成。"
echo "  权重：$OUT/adapter"
echo "  完整模型：$OUT/merged（仅 GQY_MERGE=1 时生成）"
echo ""
echo "下一步（三期集成，待做）：GQY 推理侧加载 adapter："
echo "  mlx_lm.server --model $BASE_MODEL --adapter-path $OUT/adapter --port 8080"
echo "  然后 gqy config 把 provider base_url 指到 http://127.0.0.1:8080/v1"
echo ""
echo "📊 费用：本地 MLX 只有电费（约 ¥0.1/次训练），数据不出本机。"
