#!/usr/bin/env node
/**
 * GQY Telegram 桥接层 v2 —— 接 GQY 长驻 daemon（`gqy web`）
 *
 * v1 的问题：每条消息 spawn 一个 `gqy ask` 进程，拿不到图片事件/流式输出，
 * 只能收发文本。v2 改为：
 *   TG 长轮询 ──> 桥（本文件）──HTTP/SSE──> GQY daemon（channel=tg）
 *   桥负责：把 TG 消息 POST /api/turns 交给 GQY 完整 agent 循环；
 *   订阅 /api/events 事件流，把流式文字（assistant.delta）收齐、把图片资产
 *   （tool.image）下载后经 sendPhoto 发回 TG；支持追问（question.requested）回复。
 *
 * 会话模型：本平台一个 daemon 通道（GQY_CHANNEL=tg），所有 TG 聊天共享一份
 * 对话上下文（平台间隔离：QQ/WebUI 各自独立）。桥内部对提交做串行化，
 * 同一时刻只跑一个 run，避免排队导致的 run_id 匹配歧义。
 *
 * 规则：
 *  - 私聊：主人（GQY_TG_OWNER_ID）与其他人均走同一 TG 通道上下文
 *  - 群聊：只在被 @bot、回复 bot 消息、或 /command@bot 时响应
 *  - edited_message 不触发新问答；处理中忽略重复消息（简单去抖）
 */
'use strict';

const path = require('node:path');
const { log, splitReply } = require('../lib/bridge-common.cjs');
const {
  ensureDaemon,
  stopDaemon,
  createTurn,
  answerQuestion,
  downloadAsset,
  subscribeEvents,
} = require('../lib/daemon-client.cjs');

const TOKEN = process.env.GQY_TG_TOKEN || '';
const LOG_FILE = process.env.GQY_BRIDGE_LOG || path.join(process.env.HOME || '', 'napcat', 'tg-bridge.log');
const OWNER_ID = process.env.GQY_TG_OWNER_ID || '';
const PORT = Number(process.env.GQY_TG_WEB_PORT || 4101);
const CHANNEL = process.env.GQY_TG_CHANNEL || 'tg';
const RUN_TIMEOUT_MS = Number(process.env.GQY_RUN_TIMEOUT_MS || 600000);

const API = `https://api.telegram.org/bot${TOKEN}`;

let BOT_USERNAME = '';
let BOT_ID = '';
let daemon = null; // { baseUrl, child, owned }
const sseState = { lastId: 0 };

/** chatId -> { questionId, questions }：待回答的追问 */
const pendingQuestions = new Map();
/** runId -> { chatId, text, images: [], timer }：进行中的 run */
const activeRuns = new Map();
/** 串行提交队列 */
const turnQueue = [];
let turnRunning = false;

async function api(method, params = {}) {
  const res = await fetch(`${API}/${method}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });
  const json = await res.json().catch(() => ({}));
  if (!json.ok) throw new Error(`${method} 失败: ${JSON.stringify(json.description || json)}`);
  return json.result;
}

/** 发图片：multipart 上传，caption 可选 */
async function sendPhoto(chatId, buf, mime, caption) {
  const form = new FormData();
  form.append('chat_id', String(chatId));
  const ext = (mime.split('/')[1] || 'png').replace('jpeg', 'jpg');
  form.append('photo', new Blob([buf], { type: mime }), `gqy.${ext}`);
  if (caption) form.append('caption', String(caption).slice(0, 1024));
  const res = await fetch(`${API}/sendPhoto`, { method: 'POST', body: form });
  const json = await res.json().catch(() => ({}));
  if (!json.ok) throw new Error(`sendPhoto 失败: ${JSON.stringify(json.description || json)}`);
  return json.result;
}

function buildReplyMarkup() {
  return {
    inline_keyboard: [
      [{ text: '👍 有用', callback_data: 'gqy:thumbs_up' }],
      [{ text: '👎 没用', callback_data: 'gqy:thumbs_down' }],
      [{ text: '↻ 换个角度再说说', callback_data: 'gqy:again' }],
    ],
  };
}

async function sendText(chatId, text, replyToMessageId) {
  const base = { chat_id: chatId, reply_markup: buildReplyMarkup() };
  if (replyToMessageId) base.reply_to_message_id = replyToMessageId;
  for (const part of splitReply(text || '(我没想出该说啥)', 4096)) {
    await api('sendMessage', { ...base, text: part });
  }
}

/** run 结束后统一发送：先文字后图片（图片逐张 sendPhoto） */
async function sendFinal(chatId, run, replyToMessageId) {
  if (run.text && run.text.trim()) await sendText(chatId, run.text.trim(), replyToMessageId);
  for (const img of run.images) {
    try {
      const { buf, mime } = await downloadAsset(daemon.baseUrl, img.assetId);
      await sendPhoto(chatId, buf, mime, img.alt);
    } catch (e) {
      log(LOG_FILE, `发图失败 ${img.assetId}: ${e.message}`);
    }
  }
}

function finishRun(runId) {
  const run = activeRuns.get(runId);
  if (!run) return;
  if (run.timer) clearTimeout(run.timer);
  activeRuns.delete(runId);
  releaseQueue();
}

/** 提交一轮对话（串行：同一时刻只有一个 run） */
async function runTurn(chatId, text, replyToMessageId) {
  api('sendChatAction', { chat_id: chatId, action: 'typing' }).catch(() => {});
  let turn;
  try {
    turn = await createTurn(daemon.baseUrl, text);
  } catch (e) {
    log(LOG_FILE, `createTurn 失败: ${e.message}`);
    await sendText(chatId, `出错了：${e.message.slice(0, 120)}`, replyToMessageId);
    releaseQueue();
    return;
  }
  if (turn.queued) {
    // 极端情况：daemon 正忙（理论上串行化后不会发生）。不追踪 run_id（排队后 run_id 会变）。
    await sendText(chatId, '我在忙上一件事，稍等一下再问我哦～', replyToMessageId);
    releaseQueue();
    return;
  }
  const run = { chatId, replyToMessageId, text: '', images: [], timer: null };
  activeRuns.set(turn.run_id, run);
  run.timer = setTimeout(() => {
    log(LOG_FILE, `run ${turn.run_id} 超时（${RUN_TIMEOUT_MS / 1000}s）`);
    const r = activeRuns.get(turn.run_id);
    if (r) {
      sendFinal(r.chatId, r, r.replyToMessageId).catch(() => {});
      finishRun(turn.run_id);
    }
  }, RUN_TIMEOUT_MS);
}

// ─────────────────────────── 队列（串行提交） ───────────────────────────

function enqueueTurn(chatId, text, replyToMessageId) {
  turnQueue.push({ chatId, text, replyToMessageId });
  pumpQueue();
}

function pumpQueue() {
  if (turnRunning) return;
  const next = turnQueue.shift();
  if (!next) return;
  turnRunning = true;
  runTurn(next.chatId, next.text, next.replyToMessageId).catch((e) => {
    log(LOG_FILE, `runTurn 异常: ${e.message}`);
    turnRunning = false;
    pumpQueue();
  });
}

/** 只有 run 真正结束（completed/failed/cancelled/超时）才放行队列 */
function releaseQueue() {
  turnRunning = false;
  pumpQueue();
}

// ─────────────────────────── SSE 事件分发 ───────────────────────────

function onSseEvent(kind, data) {
  const runId = data?.run_id;
  switch (kind) {
    case 'assistant.delta': {
      const run = activeRuns.get(runId);
      if (run && data.delta) run.text += data.delta;
      break;
    }
    case 'tool.image': {
      const run = activeRuns.get(runId);
      if (!run || !data.asset) break;
      run.images.push({ assetId: data.asset.id, alt: data.asset.alt || '' });
      break;
    }
    case 'run.completed': {
      const run = activeRuns.get(runId);
      if (run) { sendFinal(run.chatId, run, run.replyToMessageId).catch(() => {}); finishRun(runId); }
      break;
    }
    case 'run.failed': {
      const run = activeRuns.get(runId);
      if (run) { sendText(run.chatId, `出错了：${(data.message || 'GQY 处理失败').slice(0, 200)}`, run.replyToMessageId).catch(() => {}); finishRun(runId); }
      break;
    }
    case 'run.cancelled': {
      const run = activeRuns.get(runId);
      if (run) { sendText(run.chatId, '（已中断）', run.replyToMessageId).catch(() => {}); finishRun(runId); }
      break;
    }
    case 'question.requested': {
      // GQY 需要追问：把问题转发给发起这个 run 的聊天，记住待答
      const run = activeRuns.get(runId);
      if (run && Array.isArray(data.questions) && data.questions.length) {
        const q = data.questions
          .map((item, i) => {
            const text = item?.text || item?.question || JSON.stringify(item);
            const choices = Array.isArray(item?.choices) && item.choices.length
              ? '\n' + item.choices.map((c, j) => `${j + 1}. ${c}`).join('\n')
              : '';
            return `${i + 1}. ${text}${choices}`;
          })
          .join('\n');
        sendText(run.chatId, `我确认一下：\n${q}`).catch(() => {});
        pendingQuestions.set(String(run.chatId), {
          questionId: data.question_id,
          questions: data.questions,
        });
        // 等回答，run 不结束；回答到达后 run 会继续并以原 run_id 收尾（run.completed 触发 finishRun）
        if (run.timer) clearTimeout(run.timer);
        run.timer = setTimeout(() => {
          const r = activeRuns.get(runId);
          if (r) {
            sendText(r.chatId, '（追问等待超时，先这样吧）', r.replyToMessageId).catch(() => {});
            finishRun(runId);
          }
        }, RUN_TIMEOUT_MS);
      }
      break;
    }
    default:
      break;
  }
}

// ─────────────────────────── TG 消息处理 ───────────────────────────

function isBotMentioned(text, replyTo) {
  const t = (text || '').trim();
  if (!BOT_USERNAME) return true;
  if (new RegExp(`^@${BOT_USERNAME}\\b`, 'i').test(t)) return true;
  if (new RegExp(`^/[^\\s]+@${BOT_USERNAME}\\b`, 'i').test(t)) return true;
  if (replyTo && String(replyTo.from?.id) === BOT_ID) return true;
  return false;
}

const processing = new Map();

async function handleUpdate(update) {
  const msg = update.message;
  if (!msg || !msg.text) return; // 只处理文本；edited_message 不触发
  const chat = msg.chat;
  const chatType = chat.type;
  const text = String(msg.text).slice(0, 2000);
  if (chatType === 'channel') return;

  const isPrivate = chatType === 'private';
  if (!isPrivate && !isBotMentioned(text, msg.reply_to_message)) {
    log(LOG_FILE, `群 ${chat.id} 消息忽略（未@bot）: ${text.slice(0, 60)}`);
    return;
  }

  let question = text;
  if (!isPrivate && BOT_USERNAME) {
    question = question
      .replace(new RegExp(`^@${BOT_USERNAME}\\b\\s*`, 'i'), '')
      .replace(new RegExp(`^/[^\\s]+@${BOT_USERNAME}\\b\\s*`, 'i'), '')
      .trim();
  }
  if (!question) return;

  const key = `${chat.id}:${msg.message_id}`;
  if (processing.has(key)) return;
  processing.set(key, Date.now());

  try {
    log(LOG_FILE, `收到 ${chatType} 来自 ${msg.from?.id}${chatType !== 'private' ? ' 群 ' + chat.id : ''}: ${question.slice(0, 120)}`);

    // 有未回答的追问：先回追问，不新建 run
    const pending = pendingQuestions.get(String(chat.id));
    if (pending) {
      pendingQuestions.delete(String(chat.id));
      try {
        await answerQuestion(daemon.baseUrl, pending.questionId, [[question]]);
        log(LOG_FILE, `已回答追问 ${pending.questionId}`);
      } catch (e) {
        log(LOG_FILE, `回答追问失败: ${e.message}`);
        await sendText(chat.id, `追问回答失败了：${e.message.slice(0, 120)}`);
      }
      return;
    }

    enqueueTurn(chat.id, question, msg.message_id);
  } catch (e) {
    log(LOG_FILE, `处理失败: ${e.message}`);
  } finally {
    setTimeout(() => processing.delete(key), 5000);
  }
}

async function handleCallbackQuery(update) {
  const cq = update.callback_query;
  if (!cq) return;
  const data = String(cq.data || '');
  const chatId = cq.message?.chat?.id;
  log(LOG_FILE, `收到按钮点击 ${data} 来自 ${cq.from?.id} 群/聊 ${chatId}`);
  try {
    if (data === 'gqy:thumbs_up') {
      await api('answerCallbackQuery', { callback_query_id: cq.id, text: '谢谢反馈，记下了！', show_alert: false });
    } else if (data === 'gqy:thumbs_down') {
      await api('answerCallbackQuery', { callback_query_id: cq.id, text: '收到，我会改进的', show_alert: false });
    } else if (data === 'gqy:again') {
      await api('answerCallbackQuery', { callback_query_id: cq.id, text: '好，我再说点', show_alert: false });
      enqueueTurn(chatId, '继续我们刚才的话题，换个角度再说说，说点新的。', cq.message?.message_id);
    }
  } catch (e) {
    log(LOG_FILE, 'callback 处理失败: ' + e.message);
  }
}

let offset = 0;
async function poll() {
  try {
    const updates = await api('getUpdates', { offset, timeout: 30, allowed_updates: ['message', 'callback_query'] });
    for (const u of updates) {
      offset = Math.max(offset, u.update_id + 1);
      if (u.callback_query) {
        handleCallbackQuery(u).catch((e) => log(LOG_FILE, 'handleCallbackQuery error: ' + e.message));
        continue;
      }
      handleUpdate(u).catch((e) => log(LOG_FILE, 'handleUpdate error: ' + e.message));
    }
  } catch (e) {
    log(LOG_FILE, 'poll 错误: ' + e.message);
    await new Promise((r) => setTimeout(r, 3000));
  }
  setTimeout(poll, 0);
}

function shutdown() {
  log(LOG_FILE, '桥退出，清理 daemon');
  stopDaemon(daemon);
  process.exit(0);
}

async function main() {
  if (!TOKEN) { log(LOG_FILE, '未设置 GQY_TG_TOKEN，退出'); process.exit(1); }
  try {
    const me = await api('getMe');
    BOT_USERNAME = me.username || '';
    BOT_ID = String(me.id);
    log(LOG_FILE, `已连接 Telegram Bot: @${BOT_USERNAME} (id ${BOT_ID})`);
  } catch (e) {
    log(LOG_FILE, 'getMe 失败: ' + e.message);
    process.exit(1);
  }

  daemon = await ensureDaemon({ channel: CHANNEL, port: PORT, logFile: LOG_FILE });
  // 先订阅事件流再开始收消息，避免错过 run 事件
  subscribeEvents({ baseUrl: daemon.baseUrl, sseState, onEvent: onSseEvent, logFile: LOG_FILE })
    .catch((e) => log(LOG_FILE, 'subscribeEvents 退出: ' + e.message));

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  poll();
}

main().catch((e) => { log(LOG_FILE, '启动失败: ' + e.message); process.exit(1); });
