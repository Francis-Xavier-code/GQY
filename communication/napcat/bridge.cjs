#!/usr/bin/env node
/**
 * GQY OneBot 桥接层
 * 连接 NapCat WebSocket 3001，把 QQ 消息路由到 gqy ask，回复再吐回 QQ
 *
 * 规则：
 *  - 私聊：全部响应，按 QQ 号隔离会话
 *  - 群聊：只在被 @（或 @全体）时响应，按群号隔离会话
 *  - 同一会话的消息串行处理（见 lib/bridge-common.cjs）
 *  - 处理中忽略重复消息（简单去抖）
 */
'use strict';

const path = require('node:path');
const {
  SESSIONS_DIR,
  log,
  ensureSession,
  askGqy,
  splitReply,
  enqueueSession,
} = require('../lib/bridge-common.cjs');

const WS_URL = process.env.GQY_WS_URL || 'ws://127.0.0.1:3001';
const LOG_FILE = process.env.GQY_BRIDGE_LOG || path.join(process.env.HOME || '', 'napcat', 'bridge.log');
// 我的 QQ 号：未设置时群聊 @ 响应不可用（启动时给出警告）
const SELF_ID = process.env.GQY_SELF_ID || '';

if (!SELF_ID) {
  log(LOG_FILE, '警告：未设置 GQY_SELF_ID，群聊 @ 响应将不可用（私聊不受影响）');
}

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
      // 其他人/未知 @ 不进文本，避免 @qq 噪音污染提问
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

// 单条消息处理 + 去抖
const processing = new Map();
async function handleMessage(event) {
  const { post_type, message_type, message_id, user_id, group_id, message } = event;
  if (post_type !== 'message') return;

  const { text, atMe } = parseMessage(message, SELF_ID);
  if (!text) return;

  // 群聊：只在被 @ 时响应
  if (message_type === 'group' && !atMe) {
    log(LOG_FILE, `群 ${group_id} 消息忽略（未@我）: ${text.slice(0, 60)}`);
    return;
  }

  const key = `${message_type}:${message_id}`;
  if (processing.has(key)) return;
  processing.set(key, Date.now());

  // 会话键：私聊按用户隔离，群聊按群隔离
  const sessionKey = message_type === 'group'
    ? `qq-group-${group_id}`
    : `qq-private-${user_id}`;
  const sessionHome = path.join(SESSIONS_DIR, sessionKey);

  try {
    log(LOG_FILE, `收到 ${message_type} 来自 ${user_id}${group_id ? ' 群 ' + group_id : ''}: ${text.slice(0, 120)}`);
    // 串行处理：同一会话的消息排队，避免并发 gqy 进程竞态与回复乱序
    const reply = await enqueueSession(sessionKey, async () => {
      const extraEnv = ensureSession(sessionHome, LOG_FILE);
      return askGqy(text.slice(0, 2000), sessionHome, extraEnv, LOG_FILE, 'qq');
    });
    const send = {
      action: message_type === 'group' ? 'send_group_msg' : 'send_private_msg',
      params: message_type === 'group' ? { group_id } : { user_id },
      echo: `reply-${message_id}`,
    };
    // 长回复分片发送（QQ 单条消息有长度限制）
    for (const part of splitReply(reply, 4000)) {
      send.params.message = part;
      ws.send(JSON.stringify(send));
    }
    log(LOG_FILE, `回复 ${user_id}: ${reply.slice(0, 120)}`);
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

connect();
