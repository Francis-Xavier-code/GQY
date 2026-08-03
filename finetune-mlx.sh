#!/usr/bin/env bash
# 顾清影自我进化 · 二期：MLX LoRA 批量微调（Apple Silicon 本地，免费可预测）
# 底座：cognitivecomputations/Dolphin-2.9.2-qwen2.5-7b（无审查沉浸式人设）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOME_DIR="${1:-$SCRIPT_DIR}"
# 数据目录：GQY_DATA_DIR 优先（train-lora.sh 直接指定取样目录），
# 否则按 <参数或脚本目录>/data/finetune（向后兼容）
DATA_DIR="${GQY_DATA_DIR:-$HOME_DIR/data/finetune}"
TURNS="$DATA_DIR/turns.jsonl"
LORA_ROOT="$DATA_DIR/lora"
BASE_MODEL="${GQY_BASE_MODEL:-huihui-ai/Huihui-Qwen3-4B-Instruct-2507-abliterated}"
GENERIC_FILE="${GQY_GENERIC_FILE:-}"
MERGE="${GQY_MERGE:-0}"
# LoRA 命名：GQY_LORA_NAME 指定，否则自动 gqy-lover-v<日期>
LORA_NAME="${GQY_LORA_NAME:-gqy-lover-v$(date +%Y%m%d-%H%M)}"
MIN_SAMPLES="${GQY_MIN_SAMPLES:-500}"
EPOCHS="${GQY_EPOCHS:-2}"
LR="${GQY_LR:-2e-5}"

# 激活本地虚拟环境（如果存在）
VENV_DIR="$SCRIPT_DIR/venv"
if [ -d "$VENV_DIR" ]; then
  source "$VENV_DIR/bin/activate"
fi

# 底座模型缓存放项目目录（默认），保证自包含、可迁移；可用 GQY_HF_HOME 覆盖
export HF_HOME="${GQY_HF_HOME:-$SCRIPT_DIR/.hf-cache}"

echo "==> 0/6 环境检查"
if [ ! -f "$TURNS" ]; then
  echo "未找到训练数据：$TURNS"
  exit 1
fi
echo "   数据：$TURNS"

echo "==> 1/6 统计样本"
TOTAL=$(wc -l < "$TURNS" | tr -d ' ')
echo "   已收集 $TOTAL 条"
if [ "$TOTAL" -lt "$MIN_SAMPLES" ]; then
  echo "   不足 $MIN_SAMPLES 条，暂不训练"
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
    if len(u) < 2 or len(a) < 10: continue
    key = (u, a)
    if key in seen: continue
    seen.add(key)
    if any(w in (u + a) for w in ('password', 'api_key', 'token=', '私钥', 'secret')):
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
sampled = random.choices(generic, k=max(1, len(rows) // 3))
mixed = rows + sampled
random.shuffle(mixed)
with open(dst, 'w', encoding='utf-8') as f:
    for r in mixed: f.write(json.dumps(r, ensure_ascii=False) + '\n')
print(f"   混入后 {len(mixed)} 条（专属:通用 ≈ 7:3）")
PY

echo "==> 4/6 转 MLX 训练格式"
TS=$(date +%Y%m%d-%H%M%S)
# 续训（GQY_RESUME=1）：沿用最近 checkpoint 的原训练目录，保证产物连续
if [ "${GQY_RESUME:-0}" = "1" ]; then
  RESUME_CKPT="$(ls -t "$LORA_ROOT"/*/adapter/0000*_adapters.safetensors 2>/dev/null | head -1)"
  if [ -n "$RESUME_CKPT" ]; then
    OUT="$(dirname "$(dirname "$RESUME_CKPT")")"
    echo "==> 续训：从 $RESUME_CKPT 继续（目录 $OUT）"
  fi
fi
if [ -z "${OUT:-}" ]; then
  OUT="$LORA_ROOT/$LORA_NAME"
  mkdir -p "$OUT"
fi
RESUME_FLAG=""
if [ -n "${RESUME_CKPT:-}" ]; then
  RESUME_FLAG="--resume-adapter-file=$RESUME_CKPT"
fi
mkdir -p "$OUT"

TRAIN_DIR="$DATA_DIR/train"
mkdir -p "$TRAIN_DIR"
python3 - "$DATA_DIR/train.mixed.jsonl" "$TRAIN_DIR" << 'PY'
import json, sys, os
src, outdir = sys.argv[1], sys.argv[2]
rows = []
with open(src, encoding='utf-8') as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        text = "<|im_start|>user\n" + r['user'] + "\n<|im_end|>\n<|im_start|>assistant\n" + r['assistant'] + "\n<|im_end|>"
        rows.append({"text": text})
# mlx_lm.lora 的 --data 需要目录：train.jsonl / valid.jsonl / test.jsonl
n = len(rows)
v = max(1, n // 10)          # 10% 验证
te = max(1, n // 20)         # 5% 测试
tr = max(1, n - v - te)
def dump(name, items):
    # mlx_lm：文件不存在 → 空列表（跳过）；文件存在但为空 → IndexError 报错。
    # 所以空数据集不创建文件；test 数据不足时用 valid 兜底保证非空。
    if not items:
        return
    with open(os.path.join(outdir, name), 'w', encoding='utf-8') as o:
        for it in items:
            o.write(json.dumps(it, ensure_ascii=False) + '\n')
dump("train.jsonl", rows[:tr])
dump("valid.jsonl", rows[tr:tr + v])
test_rows = rows[tr + v:]
if not test_rows:
    test_rows = rows[tr:tr + v]   # test 兜底 = valid
dump("test.jsonl", test_rows)
print(f"   mlx 数据集就绪：train={tr} valid={v} test={te}（{outdir}）")
PY

echo "==> 5/6 LoRA 训练（底座 ${BASE_MODEL}，epochs=${EPOCHS}，lr=${LR}）"

python3 -m mlx_lm.lora \
  --model "$BASE_MODEL" \
  --train \
  --data "$TRAIN_DIR" \
  --iters "$((TOTAL * EPOCHS))" \
  --num-layers 8 \
  --batch-size 1 \
  --learning-rate "$LR" \
  --steps-per-report 20 \
  --adapter-path "$OUT/adapter" \
  ${RESUME_FLAG}

echo "==> 6/6 自动合并（LoRA → 完整模型）"
if command -v mlx_lm >/dev/null 2>&1 || true; then
  FUSED="$DATA_DIR/lora-merged/$LORA_NAME"
  mkdir -p "$FUSED"
  if "$SCRIPT_DIR/venv/bin/python" -m mlx_lm fuse \
      --model "$BASE_MODEL" \
      --adapter-path "$OUT/adapter" \
      --save-path "$FUSED" > /tmp/fuse.log 2>&1; then
    # 元数据标注：模型身份/底座/训练信息写入 config.json（模型本身仍由 system prompt 注入身份）
    "$SCRIPT_DIR/venv/bin/python" - "$FUSED/config.json" << PY2
import json, sys
cfg_path = sys.argv[1]
cfg = json.load(open(cfg_path))
cfg["_gqy"] = {
    "model_id": "$LORA_NAME",
    "base_model": "$BASE_MODEL",
    "trained_at": "$(date '+%Y-%m-%d %H:%M')",
    "samples": $(wc -l < "$DATA_DIR/turns.jsonl" | tr -d ' '),
    "epochs": $EPOCHS,
    "note": "顾清影 LoRA 微调产物；身份由 GQY system prompt (lover.md) 注入，本模型提供对话风格"
}
json.dump(cfg, open(cfg_path, 'w'), indent=2, ensure_ascii=False)
PY2
    echo "   ✅ 已合并：$FUSED（模型 ID: $LORA_NAME，底座: $BASE_MODEL）"
  else
    echo "   ⚠️ 合并失败（adapter 仍可用，见 $OUT/adapter）"
    tail -3 /tmp/fuse.log
  fi
fi

# 完成时弹 macOS 系统通知（通知中心可见）
osascript -e 'display notification "LoRA 训练完成！产物在 '"$DATA_DIR"'/lora 与 lora-merged" with title "顾清影 · 训练完成"' >/dev/null 2>&1 || true

echo "==> 6/6 存档与报告"
cp "$OUT/adapter/adapter.safetensors" "$OUT/adapter.safetensors" 2>/dev/null || true
cat > "$OUT/README.md" << MD
# 顾清影 LoRA $TS
- 底座：$BASE_MODEL
- 样本：$TOTAL 条（清洗后），epochs=${EPOCHS}，lr=$LR
- 合并完整模型：${MERGE:+是（merged/）}
- 训练日期：$TS
MD

if [ "$MERGE" = "1" ]; then
  echo "==> 7/7 合并 LoRA 进底座（产出完整模型）"
  python3 -m mlx_lm.fuse \
    --model "$BASE_MODEL" \
    --adapter-path "$OUT/adapter" \
    --save-path "$OUT/merged"
  echo "✅ 合并完成：$OUT/merged"
fi

echo ""
echo "✅ 训练完成。"
echo "  权重保存位置：$OUT/adapter"
echo "  完整模型目录：$OUT/merged（仅 GQY_MERGE=1 时生成）"
