#!/usr/bin/env node
/**
 * GQY OneBot 桥接层 v2 —— 接 GQY 长驻 daemon（`gqy web`）
 *
 * v1 的问题：每条消息 spawn 一个 `gqy ask` 进程，拿不到图片事件/流式输出，
 * 只能收发文本。v2 改为：
 *   NapCat OneBot WS ──> 桥（本文件）──HTTP/SSE──> GQY daemon（channel=qq）
 *   桥负责：把 QQ 消息 POST /api/turns 交给 GQY 完整 agent 循环；
 *   订阅 /api/events 事件流，把流式文字收齐、把图片资产（tool.image）下载后
 *   以 base64 图片段发回 QQ；支持追问（question.requested）回复。
 *
 * 会话模型：本平台一个 daemon 通道（GQY_CHANNEL=qq），所有 QQ 聊天共享一份
 * 对话上下文。桥内部对提交做串行化，同一时刻只跑一个 run。
 *
 * 规则：
 *  - 私聊：全部响应
 *  - 群聊：只在被 @（或 @全体）时响应
 *  - 处理中忽略重复消息（简单去抖）
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

const WS_URL = process.env.GQY_WS_URL || 'ws://127.0.0.1:3001';
const LOG_FILE = process.env.GQY_BRIDGE_LOG || path.join(process.env.HOME || '', 'napcat', 'bridge.log');
const SELF_ID = process.env.GQY_SELF_ID || '';
const PORT = Number(process.env.GQY_QQ_WEB_PORT || 4102);
const CHANNEL = process.env.GQY_QQ_CHANNEL || 'qq';
const RUN_TIMEOUT_MS = Number(process.env.GQY_RUN_TIMEOUT_MS || 600000);

if (!SELF_ID) {
  log(LOG_FILE, '警告：未设置 GQY_SELF_ID，群聊 @ 响应将不可用（私聊不受影响）');
}

let daemon = null; // { baseUrl, child, owned }
const sseState = { lastId: 0 };

/** chatKey -> { questionId }：待回答的追问（chatKey: 群/私 + id） */
const pendingQuestions = new Map();
/** runId -> { chatKey, messageType, userId, groupId, text, images: [], timer } */
const activeRuns = new Map();
const turnQueue = [];
let turnRunning = false;

// 从 OneBot array 消息段提取文本 + 记录是否有 @ 我
function parseMessage(message, selfId) {
  let text = '';
  let atMe = false;
  for (const seg of message || []) {
    if (seg.type === 'text') {
      text += seg.data?.text || '';
    } else if (seg.type === 'at') {
      const qq = String(seg.data?.qq || '');
      if (qq === 'all' || (selfId && qq === selfId)) {
        atMe = true;
        text += ' [有人@我] ';
      }
    } else if (seg.type === 'face') {
      text += `[表情${seg.data?.id || ''}]`;
    } else if (seg.type === 'image') {
      text += ' [图片] ';
    } else if (seg.type === 'reply') {
      text += ' [回复] ';
    } else if (seg.type === 'json') {
      text += ' [卡片消息] ';
    } else if (seg.type === 'forward') {
      text += ' [合并转发] ';
    }
  }
  return { text: text.trim(), atMe };
}

function chatKeyOf(messageType, userId, groupId) {
  return messageType === 'group' ? `group:${groupId}` : `private:${userId}`;
}

function sendAction(action, params, echo) {
  if (!global.ws || global.ws.readyState !== 1) return;
  global.ws.send(JSON.stringify({ action, params, echo: echo || undefined }));
}

/** 发文字（分片） */
function sendText(chatKey, messageType, userId, groupId, text) {
  const action = messageType === 'group' ? 'send_group_msg' : 'send_private_msg';
  const params = messageType === 'group' ? { group_id: groupId } : { user_id: userId };
  for (const part of splitReply(text || '(我没想出该说啥)', 4000)) {
    sendAction(action, { ...params, message: [{ type: 'text', data: { text: part } }] }, `reply-${chatKey}`);
  }
}

/** 发图片（base64:// 段，NapCat 支持） */
function sendImage(chatKey, messageType, userId, groupId, buf, mime) {
  const action = messageType === 'group' ? 'send_group_msg' : 'send_private_msg';
  const params = messageType === 'group' ? { group_id: groupId } : { user_id: userId };
  sendAction(action, {
    ...params,
    message: [{ type: 'image', data: { file: `base64://${buf.toString('base64')}` } }],
  }, `img-${chatKey}`);
}

async function sendFinal(run) {
  if (run.text && run.text.trim()) sendText(run.chatKey, run.messageType, run.userId, run.groupId, run.text.trim());
  for (const img of run.images) {
    try {
      const { buf, mime } = await downloadAsset(daemon.baseUrl, img.assetId);
      sendImage(run.chatKey, run.messageType, run.userId, run.groupId, buf, mime);
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

async function runTurn(chatKey, messageType, userId, groupId, text) {
  let turn;
  try {
    turn = await createTurn(daemon.baseUrl, text);
  } catch (e) {
    log(LOG_FILE, `createTurn 失败: ${e.message}`);
    sendText(chatKey, messageType, userId, groupId, `出错了：${e.message.slice(0, 120)}`);
    releaseQueue();
    return;
  }
  if (turn.queued) {
    sendText(chatKey, messageType, userId, groupId, '我在忙上一件事，稍等一下再问我哦～');
    releaseQueue();
    return;
  }
  const run = { chatKey, messageType, userId, groupId, text: '', images: [], timer: null };
  activeRuns.set(turn.run_id, run);
  run.timer = setTimeout(() => {
    log(LOG_FILE, `run ${turn.run_id} 超时（${RUN_TIMEOUT_MS / 1000}s）`);
    const r = activeRuns.get(turn.run_id);
    if (r) { sendFinal(r); finishRun(turn.run_id); }
  }, RUN_TIMEOUT_MS);
}

// ─────────────────────────── 队列（串行提交） ───────────────────────────

function enqueueTurn(chatKey, messageType, userId, groupId, text) {
  turnQueue.push({ chatKey, messageType, userId, groupId, text });
  pumpQueue();
}

function pumpQueue() {
  if (turnRunning) return;
  const next = turnQueue.shift();
  if (!next) return;
  turnRunning = true;
  runTurn(next.chatKey, next.messageType, next.userId, next.groupId, next.text).catch((e) => {
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
      if (run) { sendFinal(run); finishRun(runId); }
      break;
    }
    case 'run.failed': {
      const run = activeRuns.get(runId);
      if (run) {
        sendText(run.chatKey, run.messageType, run.userId, run.groupId, `出错了：${(data.message || 'GQY 处理失败').slice(0, 200)}`);
        finishRun(runId);
      }
      break;
    }
    case 'run.cancelled': {
      const run = activeRuns.get(runId);
      if (run) { sendText(run.chatKey, run.messageType, run.userId, run.groupId, '（已中断）'); finishRun(runId); }
      break;
    }
    case 'question.requested': {
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
        sendText(run.chatKey, run.messageType, run.userId, run.groupId, `我确认一下：\n${q}`);
        pendingQuestions.set(run.chatKey, { questionId: data.question_id });
        // 等回答，run 不结束；回答到达后 run 会继续并以原 run_id 收尾（run.completed 触发 finishRun）
        if (run.timer) clearTimeout(run.timer);
        run.timer = setTimeout(() => {
          const r = activeRuns.get(runId);
          if (r) {
            sendText(r.chatKey, r.messageType, r.userId, r.groupId, '（追问等待超时，先这样吧）');
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

// ─────────────────────────── QQ 消息处理 ───────────────────────────

const processing = new Map();

async function handleMessage(event) {
  const { post_type, message_type, message_id, user_id, group_id, message } = event;
  if (post_type !== 'message') return;

  const { text, atMe } = parseMessage(message, SELF_ID);
  if (!text) return;

  if (message_type === 'group' && !atMe) {
    log(LOG_FILE, `群 ${group_id} 消息忽略（未@我）: ${text.slice(0, 60)}`);
    return;
  }

  const key = `${message_type}:${message_id}`;
  if (processing.has(key)) return;
  processing.set(key, Date.now());

  const chatKey = chatKeyOf(message_type, user_id, group_id);

  try {
    log(LOG_FILE, `收到 ${message_type} 来自 ${user_id}${group_id ? ' 群 ' + group_id : ''}: ${text.slice(0, 120)}`);

    // 有未回答的追问：先回追问，不新建 run
    const pending = pendingQuestions.get(chatKey);
    if (pending) {
      pendingQuestions.delete(chatKey);
      try {
        await answerQuestion(daemon.baseUrl, pending.questionId, [[text.slice(0, 2000)]]);
        log(LOG_FILE, `已回答追问 ${pending.questionId}`);
      } catch (e) {
        log(LOG_FILE, `回答追问失败: ${e.message}`);
        sendText(chatKey, message_type, user_id, group_id, `追问回答失败了：${e.message.slice(0, 120)}`);
      }
      return;
    }

    enqueueTurn(chatKey, message_type, user_id, group_id, text.slice(0, 2000));
  } catch (e) {
    log(LOG_FILE, `处理失败: ${e.message}`);
  } finally {
    setTimeout(() => processing.delete(key), 5000);
  }
}

function connect() {
  log(LOG_FILE, `连接 ${WS_URL} ...`);
  const socket = new WebSocket(WS_URL);

  socket.onopen = () => log(LOG_FILE, '已连接 OneBot WebSocket');
  socket.onmessage = (ev) => {
    try {
      const event = JSON.parse(String(ev.data));
      if (event.post_type) {
        handleMessage(event).catch((e) => log(LOG_FILE, 'handleMessage error: ' + e.message));
      }
    } catch (e) {
      log(LOG_FILE, '解析消息失败: ' + e.message);
    }
  };
  socket.onerror = (e) => log(LOG_FILE, 'WS 错误: ' + (e.message || e.type || 'unknown'));
  socket.onclose = () => {
    log(LOG_FILE, '连接断开，3 秒后重连...');
    setTimeout(connect, 3000);
  };
  global.ws = socket;
}

function shutdown() {
  log(LOG_FILE, '桥退出，清理 daemon');
  stopDaemon(daemon);
  process.exit(0);
}

async function main() {
  daemon = await ensureDaemon({ channel: CHANNEL, port: PORT, logFile: LOG_FILE });
  subscribeEvents({ baseUrl: daemon.baseUrl, sseState, onEvent: onSseEvent, logFile: LOG_FILE })
    .catch((e) => log(LOG_FILE, 'subscribeEvents 退出: ' + e.message));
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  connect();
}

main().catch((e) => { log(LOG_FILE, '启动失败: ' + e.message); process.exit(1); });
