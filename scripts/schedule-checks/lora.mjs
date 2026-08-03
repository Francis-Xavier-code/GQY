import { existsSync, readFileSync } from 'node:fs';
const log = '/Users/mac/Desktop/GQY/data/finetune-sample/train.log';
if (!existsSync(log)) process.exit(2); // 无日志：跳过
const tail = readFileSync(log, 'utf8').slice(-4000);
// 完成标记：脚本打印的「训练完成」或 mlx_lm 的 Saved final weights
if (tail.includes('训练完成') || tail.includes('Saved final weights')) process.exit(0); // 放行：让 agent 汇报
process.exit(2); // 仍在训练：跳过本轮，零 token
