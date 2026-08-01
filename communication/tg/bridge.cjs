#!/usr/bin/env node
/**
 * GQY Telegram 桥接层
 * 使用 Telegram Bot API 长轮询，把 TG 消息路由到 gqy ask，回复再吐回 TG
 *
 * 规则：
 *  - 私聊：主人（GQY_TG_OWNER_ID）走全局上下文；其他用户按 user_id 隔离会话
 *  - 群聊：只在被 @bot、回复 bot 消息、或 /command@bot 时响应，按群隔离会话
 *  - 同一会话的消息串行处理（见 lib/bridge-common.cjs）
 *  - 处理中忽略重复消息（简单去抖）；edited_message 不触发新问答
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

const TOKEN = process.env.GQY_TG_TOKEN || '';
const LOG_FILE = process.env.GQY_BRIDGE_LOG || path.join(process.env.HOME || '', 'napcat', 'tg-bridge.log');
// 主人 QQ/TG 数字 ID：私聊时走主 GQY_HOME（全局上下文）
const OWNER_ID = process.env.GQY_TG_OWNER_ID || '';

const API = `https://api.telegram.org/bot${TOKEN}`;

let BOT_USERNAME = '';
let BOT_ID = '';

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

// 回复附带的交互按钮
function buildReplyMarkup() {
  return {
    inline_keyboard: [
      [
        { text: '👍 有用', callback_data: 'gqy:thumbs_up' },
        { text: '👎 没用', callback_data: 'gqy:thumbs_down' },
      ],
      [
        { text: '↻ 换个角度再说说', callback_data: 'gqy:again' },
      ],
    ],
  };
}

// 处理按钮点击
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
      const chatType = cq.message?.chat?.type;
      const userId = String(cq.from?.id || '');
      const { sessionHome, sessionKey } = sessionFor(chatType, chatId, userId);
      const reply = await enqueueSession(sessionKey, async () => {
        const extraEnv = ensureSession(sessionHome, LOG_FILE);
        return askGqy('继续我们刚才的话题，换个角度再说说，说点新的。', sessionHome, extraEnv, LOG_FILE);
      });
      await sendChunks(chatId, reply, null);
    }
  } catch (e) {
    log(LOG_FILE, 'callback 处理失败: ' + e.message);
  }
}

function isBotMentioned(text, replyTo) {
  const t = (text || '').trim();
  if (!BOT_USERNAME) return true; // 拿不到 username 时保守响应
  if (new RegExp(`^@${BOT_USERNAME}\\b`, 'i').test(t)) return true;
  if (new RegExp(`^/[^\\s]+@${BOT_USERNAME}\\b`, 'i').test(t)) return true;
  if (replyTo && String(replyTo.from?.id) === BOT_ID) return true;
  return false;
}

/**
 * 会话隔离：
 *  - 私聊：主人（OWNER_ID）→ 主 GQY_HOME（全局上下文）；其他用户 → tg-private-<id>
 *  - 群聊：按群隔离 tg-group-<chatId>
 * sessionHome 为 null 表示用主上下文（不隔离）。
 */
function sessionFor(chatType, chatId, userId) {
  if (chatType === 'private') {
    if (OWNER_ID && userId === OWNER_ID) {
      return { sessionHome: null, sessionKey: 'tg-owner' };
    }
    const key = `tg-private-${userId}`;
    return { sessionHome: path.join(SESSIONS_DIR, key), sessionKey: key };
  }
  const key = `tg-group-${chatId}`;
  return { sessionHome: path.join(SESSIONS_DIR, key), sessionKey: key };
}

// 分片发送：Telegram 单条消息上限 4096 字符
async function sendChunks(chatId, reply, replyToMessageId) {
  const base = { chat_id: chatId, reply_markup: buildReplyMarkup() };
  if (replyToMessageId) base.reply_to_message_id = replyToMessageId;
  const parts = splitReply(reply, 4096);
  for (let i = 0; i < parts.length; i++) {
    await api('sendMessage', { ...base, text: parts[i] });
  }
}

const processing = new Map();

async function handleUpdate(update) {
  const msg = update.message;
  if (!msg || !msg.text) return; // 只处理文本消息；edited_message 不触发问答
  const chat = msg.chat;
  const chatType = chat.type; // private / group / supergroup / channel
  const text = String(msg.text).slice(0, 2000);

  if (chatType === 'channel') return;

  const isPrivate = chatType === 'private';
  if (!isPrivate && !isBotMentioned(text, msg.reply_to_message)) {
    log(LOG_FILE, `群 ${chat.id} 消息忽略（未@bot）: ${text.slice(0, 60)}`);
    return;
  }

  // 去掉 @bot / /cmd@bot 前缀再问
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

  const userId = String(msg.from?.id || '');
  const { sessionHome, sessionKey } = sessionFor(chatType, chat.id, userId);

  try {
    log(LOG_FILE, `收到 ${chatType} 来自 ${userId}${chatType !== 'private' ? ' 群 ' + chat.id : ''}: ${question.slice(0, 120)}`);
    // 处理前发 typing 提示（仅私聊/群聊可见，低成本反馈）
    api('sendChatAction', { chat_id: chat.id, action: 'typing' }).catch(() => {});
    const reply = await enqueueSession(sessionKey, async () => {
      const extraEnv = ensureSession(sessionHome, LOG_FILE);
      return askGqy(question, sessionHome, extraEnv, LOG_FILE);
    });
    await sendChunks(chat.id, reply, msg.message_id);
    log(LOG_FILE, `回复 ${chat.id}: ${reply.slice(0, 120)}`);
  } catch (e) {
    log(LOG_FILE, `处理失败: ${e.message}`);
  } finally {
    setTimeout(() => processing.delete(key), 5000);
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
  setTimeout(poll, 0); // 长轮询返回后立即下一轮
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
  if (!OWNER_ID) {
    log(LOG_FILE, '提示：未设置 GQY_TG_OWNER_ID，私聊将按用户隔离（主人也看不到主上下文）');
  }
  poll();
}

main();
