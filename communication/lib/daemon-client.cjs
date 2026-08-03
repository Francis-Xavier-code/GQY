#!/usr/bin/env node
/**
 * GQY 长驻 daemon（`gqy web`）客户端：生命周期管理 + HTTP + SSE 事件流。
 *
 * 为什么桥接层要连 daemon 而不是 spawn `gqy ask`：
 *  - `gqy ask` 是一次性无状态调用，图片事件（tool.image）、流式输出、工具进度
 *    全部拿不到——这就是旧桥只能收发文本的根本原因；
 *  - `gqy web` 是 GQY 的完整 agent 循环：POST /api/turns 提交对话，
 *    GET /api/events 订阅 SSE 事件流（含图片资产、思考、工具调用），
 *    GET /api/assets/{id} 下载图片资产。
 *
 * 生命周期策略（与菜单栏一致）：探测端口上的 /api/health，已有 daemon 就复用；
 * 没有就自己 spawn 一个（--host 127.0.0.1 --no-open，不弹浏览器、不对外网卡），
 * 退出时只杀自己拉起的（owned），复用的不碰。
 */
'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const HOME = process.env.HOME || '';
const GQY_BIN = process.env.GQY_BIN || '/opt/homebrew/bin/gqy';
const GQY_HOME = process.env.GQY_HOME || path.join(HOME, 'Library/Application Support/gqy');

function log(logFile, ...args) {
  const line = `[${new Date().toISOString()}] ${args.join(' ')}`;
  try { fs.appendFileSync(logFile, line + '\n'); } catch (_) {}
  process.stdout.write(line + '\n');
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** 带超时与错误描述的 JSON 请求。无 Origin 头（非浏览器）时后端放行。 */
async function httpJson(baseUrl, method, pathname, body, timeoutMs = 15000) {
  const res = await fetch(baseUrl + pathname, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(timeoutMs),
  });
  const text = await res.text();
  let json = null;
  try { json = JSON.parse(text); } catch (_) {}
  if (!res.ok) {
    throw new Error(`${method} ${pathname} -> ${res.status}: ${(json?.error || text || '').slice(0, 200)}`);
  }
  return json;
}

/**
 * 确保指定端口的 GQY daemon 在跑。
 * @returns {{ baseUrl: string, child: import('node:child_process').ChildProcess|null, owned: boolean }}
 */
async function ensureDaemon({ channel, port, logFile }) {
  const baseUrl = `http://127.0.0.1:${port}`;
  // 1) 复用已有 daemon（可能是上一轮桥进程留下的，或用户手动开的）
  try {
    const h = await httpJson(baseUrl, 'GET', '/api/health', undefined, 2000);
    if (h && h.status === 'ready') {
      log(logFile, `复用已有 GQY daemon ${baseUrl}（channel=${channel}）`);
      return { baseUrl, child: null, owned: false };
    }
  } catch (_) { /* 端口没服务，继续拉起 */ }

  // 2) 自己拉起
  const args = ['web', '--host', '127.0.0.1', '--port', String(port), '--no-open'];
  log(logFile, `启动 GQY daemon: ${GQY_BIN} ${args.join(' ')}（GQY_CHANNEL=${channel}, GQY_HOME=${GQY_HOME}）`);
  const child = spawn(GQY_BIN, args, {
    env: { ...process.env, GQY_CHANNEL: channel, GQY_HOME },
    // detached + 进程组：退出时连子进程一起清，不留孤儿
    detached: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (d) => process.stdout.write(`[gqy-daemon] ${d}`));
  child.stderr.on('data', (d) => process.stdout.write(`[gqy-daemon:err] ${d}`));
  child.on('exit', (code, signal) => log(logFile, `GQY daemon 退出 code=${code} signal=${signal}`));

  for (let i = 0; i < 90; i++) {
    try {
      const h = await httpJson(baseUrl, 'GET', '/api/health', undefined, 1000);
      if (h && h.status === 'ready') {
        log(logFile, `GQY daemon 就绪 ${baseUrl}`);
        return { baseUrl, child, owned: true };
      }
    } catch (_) {}
    await sleep(1000);
  }
  stopDaemon({ child, owned: true });
  throw new Error(`GQY daemon 启动超时（90s），检查 ${GQY_BIN} 与 GQY_HOME=${GQY_HOME}`);
}

/** 退出时清理自己拉起的 daemon（连进程组一起杀）。复用的不碰。 */
function stopDaemon(daemon) {
  if (!daemon || !daemon.owned || !daemon.child || daemon.child.exitCode != null) return;
  try { process.kill(-daemon.child.pid, 'SIGTERM'); } catch (_) { try { daemon.child.kill('SIGTERM'); } catch (_) {} }
}

/** 提交一轮对话。返回 { run_id }（忙时 { queued: true }）。 */
async function createTurn(baseUrl, content) {
  return httpJson(baseUrl, 'POST', '/api/turns', { content, mode: 'auto', images: [] }, 20000);
}

/** 回答 GQY 的追问（ask_question 工具）。answers: 每个问题一个字符串数组。 */
async function answerQuestion(baseUrl, questionId, answers) {
  return httpJson(baseUrl, 'POST', `/api/questions/${encodeURIComponent(questionId)}/answer`, { answers }, 20000);
}

/** 下载图片资产（tool.image 里的 asset.id）。 */
async function downloadAsset(baseUrl, assetId) {
  const res = await fetch(`${baseUrl}/api/assets/${encodeURIComponent(assetId)}`);
  if (!res.ok) throw new Error(`asset ${assetId} -> ${res.status}`);
  const buf = Buffer.from(await res.arrayBuffer());
  return { buf, mime: res.headers.get('content-type') || 'image/png' };
}

/** 重置当前通道会话（清空对话上下文）。 */
async function resetConversation(baseUrl) {
  return httpJson(baseUrl, 'POST', '/api/conversation/reset', {}, 20000);
}

/**
 * 订阅 SSE 事件流，断线自动用最后一条事件 id 续传（不丢事件）。
 * @param {object} opts
 * @param {string} opts.baseUrl
 * @param {{ lastId: number }} opts.sseState  共享 lastId，断线续传用；外部可读
 * @param {(kind: string, data: object, id: number|null) => void} opts.onEvent
 * @param {string} opts.logFile
 */
async function subscribeEvents({ baseUrl, sseState, onEvent, logFile }) {
  while (true) {
    const after = sseState.lastId || 0;
    const url = `${baseUrl}/api/events` + (after ? `?after=${after}` : '');
    try {
      const res = await fetch(url);
      if (!res.ok || !res.body) throw new Error(`events -> ${res.status}`);
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = '';
      let evt = null, data = [], id = null;

      const flush = () => {
        if (evt && data.length) {
          let obj = null;
          try { obj = JSON.parse(data.join('\n')); } catch (_) {}
          if (obj) {
            if (id != null && Number(id) > (sseState.lastId || 0)) sseState.lastId = Number(id);
            try { onEvent(evt, obj, id); } catch (e) { log(logFile, `事件处理异常 ${evt}: ${e.message}`); }
          }
        }
        evt = null; data = []; id = null;
      };

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let nl;
        while ((nl = buf.indexOf('\n')) >= 0) {
          const line = buf.slice(0, nl).replace(/\r$/, '');
          buf = buf.slice(nl + 1);
          if (line === '') { flush(); }
          else if (line.startsWith(':')) { /* keepalive 注释，忽略 */ }
          else if (line.startsWith('event:')) { evt = line.slice(6).trim(); }
          else if (line.startsWith('data:')) { data.push(line.slice(5).replace(/^ /, '')); }
          else if (line.startsWith('id:')) { id = line.slice(3).trim(); }
        }
      }
      log(logFile, 'SSE 流结束，3 秒后重连');
    } catch (e) {
      log(logFile, `SSE 断开: ${e.message}，3 秒后重连`);
    }
    await sleep(3000);
  }
}

module.exports = {
  GQY_BIN,
  GQY_HOME,
  log,
  sleep,
  httpJson,
  ensureDaemon,
  stopDaemon,
  createTurn,
  answerQuestion,
  downloadAsset,
  resetConversation,
  subscribeEvents,
};
