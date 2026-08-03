#!/usr/bin/env bash
# 顾清影 LoRA 训练助手：一键训练 + 终端实时查看进度
#
# 用法：
#   ./train-lora.sh                     前台训练（终端实时滚动看进度，Ctrl+C 可中断）
#   ./train-lora.sh --background        后台训练 + 实时进度条（每 5 秒刷新）
#   ./train-lora.sh status              查看后台训练进度（百分比/速度/剩余时间）
#   ./train-lora.sh logs                查看完整训练日志
#   ./train-lora.sh test                测试最新训练的 LoRA（直接对话试效果）
#   ./train-lora.sh stop                停止后台训练
#
# 数据来源：data/finetune/turns.jsonl（30 万条全量）或 GQY_DATA 指定
# 产物：data/finetune/lora/<时间戳>/adapter/adapters.safetensors
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="${GQY_DATA:-$SCRIPT_DIR/data/finetune}"
LOG="$DATA_DIR/train.log"
TRAIN_PID="$DATA_DIR/train.pid"
LORA_ROOT="$DATA_DIR/lora"
MODEL_BASE="${GQY_BASE_MODEL:-huihui-ai/Huihui-Qwen3-4B-Instruct-2507-abliterated}"
EPOCHS="${GQY_EPOCHS:-2}"
SAMPLE="${GQY_SAMPLE:-}"   # 取样条数，如 5000；空=全量

# 从日志解析当前 iters
current_iter() {
  grep -oE "Iter [0-9]+: Train" "$LOG" 2>/dev/null | tail -1 | grep -oE "[0-9]+" || echo "0"
}
total_iters() {
  if [ -n "${GQY_TOTAL_ITERS:-}" ]; then
    echo "$GQY_TOTAL_ITERS"
    return
  fi
  local total
  total=$(wc -l < "$DATA_DIR/turns.jsonl" 2>/dev/null | tr -d ' ')
  echo "$(( (total > 0 ? total : 0) * EPOCHS ))"
}

# 取样（可选）
prepare_data() {
  if [ -n "$SAMPLE" ]; then
    mkdir -p "$DATA_DIR"
    python3 - "$DATA_DIR/turns.jsonl" "$SAMPLE" << 'PY'
import json, random, sys
src, n = sys.argv[1], int(sys.argv[2])
random.seed(42)
lines = open(src, encoding='utf-8').readlines()
picked = []
seen = set()
for l in random.sample(lines, min(n * 20, len(lines))):
    try:
        r = json.loads(l)
        u, a = (r.get('user') or '').strip(), (r.get('assistant') or '').strip()
        if len(u) < 4 or len(a) < 10: continue
        k = (u, a)
        if k in seen: continue
        seen.add(k); picked.append(r)
        if len(picked) >= n: break
    except Exception: pass
with open(src + '.sample', 'w', encoding='utf-8') as f:
    for r in picked:
        f.write(json.dumps(r, ensure_ascii=False) + '\n')
print(f"   已取样 {len(picked)} 条 → {src}.sample")
PY
    # 用取样文件替换 turns.jsonl（训练后不保留）
    mv "$DATA_DIR/turns.jsonl.sample" "$DATA_DIR/turns.jsonl"
  fi
}

cmd_foreground() {
  prepare_data
  echo "==> 前台训练（Ctrl+C 中断；终端实时滚动）"
  echo "   底座: $MODEL_BASE | epochs: $EPOCHS | 数据: $DATA_DIR/turns.jsonl"
  exec bash "$SCRIPT_DIR/finetune-mlx.sh" "$DATA_DIR/.." 2>&1 | tee "$LOG"
}

cmd_background() {
  prepare_data
  echo "==> 后台训练启动（日志: $LOG）"
  nohup bash "$SCRIPT_DIR/finetune-mlx.sh" "$DATA_DIR/.." > "$LOG" 2>&1 &
  echo $! > "$TRAIN_PID"
  echo "   PID: $! | 实时进度: ./train-lora.sh status"
  cmd_status
}

cmd_status() {
  local pid=""
  if [ -f "$TRAIN_PID" ] && kill -0 "$(cat "$TRAIN_PID")" 2>/dev/null; then
    pid="$(cat "$TRAIN_PID")"
  else
    pid="$(pgrep -f 'mlx_lm.*lora' 2>/dev/null | head -1 || true)"
  fi
  if [ -n "$pid" ]; then
    local log="$LOG"
    if [ ! -f "$log" ]; then
      log="$(ls -t "$LORA_ROOT"/*/train.log 2>/dev/null | head -1 || true)"
    fi
    if [ ! -f "$log" ]; then
      log="$(ls -t /tmp/*train*.log 2>/dev/null | head -1 || true)"
    fi
    [ -f "$log" ] && echo "   日志: $log"
    local iter total pct it_sec remain
    iter=$(grep -oE "Iter [0-9]+: Train" "$log" 2>/dev/null | tail -1 | grep -oE "[0-9]+" || echo "0")
    total=$(total_iters)
    [ "$total" -eq 0 ] && total=1
    pct=$(( iter * 100 / total ))
    it_sec=$(grep -oE "It/sec [0-9.]+" "$log" 2>/dev/null | tail -1 | grep -oE "[0-9.]+" || echo "?")
    remain="?"
    if [ "$it_sec" != "?" ] && [ "${it_sec%.*}" -gt 0 ] 2>/dev/null && [ "$iter" -gt 0 ]; then
      local secs=$(( (total - iter) / ${it_sec%.*} ))
      remain="$(( secs / 60 ))m$(( secs % 60 ))s"
    fi
    local loss
    loss=$(grep -oE "Train loss [0-9.]+" "$log" 2>/dev/null | tail -1 | awk '{print $3}' || echo "-")
    echo "────────────────────────────────────────────"
    echo " ⏳ 训练中  PID=$pid"
    echo "    进度: $iter / $total  ($pct%)  剩余约 $remain"
    echo "    Loss: $loss  速度: ${it_sec:-?} it/sec"
    echo "    内存: $(ps -o rss= -p "$pid" 2>/dev/null | awk '{printf "%.1fGB", $1/1048576}')"
    echo "    （实时刷新：再次运行 ./train-lora.sh status）"
    echo "────────────────────────────────────────────"
  else
    if [ -f "$LOG" ] && grep -q "Saved final weights" "$LOG" 2>/dev/null; then
      local saved
      saved=$(grep -oE "data/finetune/lora/[0-9-]+/adapter" "$LOG" | tail -1)
      echo "✅ 训练已完成！"
      echo "   权重: $saved"
      echo "   测试: ./train-lora.sh test"
    else
      echo "⏸️  没有正在运行的训练。"
      [ -f "$LOG" ] && echo "   上次日志: $LOG（tail 查看）"
    fi
  fi
}

cmd_logs() {
  if [ -f "$LOG" ]; then
    tail -n "${1:-20}" "$LOG" | sed 's/\r/\n/g' | grep -vE "Downloading|%\|" | tail -"${1:-20}"
  else
    echo "暂无日志"
  fi
}

cmd_test() {
  local adapter
  adapter=$(ls -td "$LORA_ROOT"/*/adapter 2>/dev/null | head -1)
  if [ -z "$adapter" ]; then
    echo "❌ 没有训练产物（先跑 ./train-lora.sh）"
    exit 1
  fi
  echo "✅ 使用最新 LoRA: $adapter"
  echo "   输入内容直接对话，Ctrl+C 退出（venv python）"
  "$SCRIPT_DIR/venv/bin/python" - "$MODEL_BASE" "$adapter" << 'PY'
import sys
from mlx_lm import load, generate
model, tok = load(sys.argv[1], adapter_path=sys.argv[2])
print("顾清影 LoRA 已加载。直接输入聊天（Ctrl+C 退出）")
while True:
    try:
        u = input("\n你> ")
    except (EOFError, KeyboardInterrupt):
        break
    if not u.strip(): continue
    msgs = [{"role": "user", "content": u}]
    tpl = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
    out = generate(model, tok, prompt=tpl, max_tokens=200)
    print("顾清影>", out.strip())
PY
}

cmd_stop() {
  if [ -f "$TRAIN_PID" ]; then
    local pid
    pid=$(cat "$TRAIN_PID")
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" && echo "✅ 已停止训练 (PID $pid)"
    else
      echo "进程已不在"
    fi
    rm -f "$TRAIN_PID"
  else
    echo "没有后台训练 PID 记录"
  fi
}

case "${1:-}" in
  status) cmd_status ;;
  logs) cmd_logs "${2:-20}" ;;
  test) cmd_test ;;
  stop) cmd_stop ;;
  --background|-b) cmd_background ;;
  "" ) cmd_foreground ;;
  *) echo "用法: $0 [status|logs|test|stop|--background]"; exit 1 ;;
esac
