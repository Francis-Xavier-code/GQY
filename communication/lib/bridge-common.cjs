#!/usr/bin/env node
/**
 * GQY 桥接层公共模块：askGqy / 会话隔离 / 输出清理 / 回复分片 / 会话串行队列。
 * 供 napcat/bridge.cjs 与 tg/bridge.cjs 共用，避免两份代码漂移。
 *
 * 会话隔离约定：
 *  - 每个隔离会话一个独立 GQY_HOME（conversation.db / memory.db 互不串扰）；
 *  - 同一会话的消息经串行队列逐个处理，杜绝并发 gqy 进程争抢同一个数据库
 *    导致的上下文互相覆盖与回复乱序；
 *  - 会话配置复制自主配置，但 api_key/token/password 等敏感字段会被替换为
 *    `$env:GQY_BRIDGE_KEY_n` 引用（GQY 原生支持 $env 引用），真实密钥只经
 *    进程环境注入，会话目录里不留明文。
 */
'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const HOME = process.env.HOME || '';
const GQY_BIN = process.env.GQY_BIN || '/opt/homebrew/bin/gqy';
const GQY_TIMEOUT_MS = Number(process.env.GQY_TIMEOUT_MS || 120000);
const SESSIONS_DIR = process.env.GQY_SESSIONS_DIR || path.join(HOME, 'Library/Application Support/gqy/sessions');
const MAIN_CONFIG = process.env.GQY_MAIN_CONFIG || path.join(HOME, 'Library/Application Support/gqy/config.jsonc');

// 主配置中需要替换为 $env 引用的敏感字段
const KEY_FIELD_RE = /(api_?key|token|password|secret|credential)/i;

function log(logFile, ...args) {
  const line = `[${new Date().toISOString()}] ${args.join(' ')}`;
  try { fs.appendFileSync(logFile, line + '\n'); } catch (_) {}
  process.stdout.write(line + '\n');
}

// 清理 gqy ask 输出的 ANSI 转义码和状态行
function cleanOutput(raw) {
  return raw
    .replace(/\u001b\[[0-9;?]*[a-zA-Z]/g, '')
    .replace(/^思考\s*[·:：].*$/gm, '')
    .replace(/^\s*$/gm, '')
    .trim();
}

// 字符串感知地剥掉 JSONC 注释（// 与 /* */），再容忍尾逗号
function readJsonc(file) {
  try {
    const text = fs.readFileSync(file, 'utf8');
    let stripped = '';
    let state = 'normal'; // normal | lineComment | blockComment | string
    for (let i = 0; i < text.length; i++) {
      const ch = text[i];
      const next = text[i + 1];
      if (state === 'lineComment') {
        if (ch === '\n') { state = 'normal'; stripped += ch; }
        continue;
      }
      if (state === 'blockComment') {
        if (ch === '*' && next === '/') { state = 'normal'; i++; }
        continue;
      }
      if (state === 'string') {
        stripped += ch;
        if (ch === '\\') { stripped += next || ''; i++; continue; }
        if (ch === '"') state = 'normal';
        continue;
      }
      if (ch === '/' && next === '/') { state = 'lineComment'; i++; continue; }
      if (ch === '/' && next === '*') { state = 'blockComment'; i++; continue; }
      if (ch === '"') state = 'string';
      stripped += ch;
    }
    // 容忍尾逗号（JSONC 常见）
    let cleaned = stripped.replace(/,\s*}/g, '}').replace(/,\s*]/g, ']');
    return JSON.parse(cleaned);
  } catch (_) {
    return null;
  }
}

// 深遍历配置，把敏感字段的字符串值替换为 $env: 引用，收集注入用环境变量
function redactKeysToEnv(node, envMap, counter) {
  if (Array.isArray(node)) {
    for (const item of node) redactKeysToEnv(item, envMap, counter);
    return node;
  }
  if (node && typeof node === 'object') {
    for (const key of Object.keys(node)) {
      const value = node[key];
      if (KEY_FIELD_RE.test(key) && typeof value === 'string') {
        if (!value.startsWith('$env:')) {
          const envName = `GQY_BRIDGE_KEY_${counter.n++}`;
          envMap[envName] = value;
          node[key] = `$env:${envName}`;
        }
      } else {
        redactKeysToEnv(value, envMap, counter);
      }
    }
  }
  return node;
}

/**
 * 确保隔离会话目录存在（权限 700），首次创建时复制主配置但剥离密钥。
 * 返回需要注入子进程的环境变量表。
 */
function ensureSession(sessionHome, logFile) {
  const envMap = {};
  if (!sessionHome) return envMap;
  try {
    fs.mkdirSync(sessionHome, { recursive: true });
    fs.chmodSync(sessionHome, 0o700);
  } catch (e) {
    log(logFile, `创建会话目录失败: ${e.message}`);
    return envMap;
  }
  const cfgDir = path.join(sessionHome, 'config');
  fs.mkdirSync(cfgDir, { recursive: true });
  const cfgPath = path.join(cfgDir, 'config.jsonc');
  if (fs.existsSync(cfgPath)) return envMap; // 已初始化：主配置更新由 $env 引用自动跟随

  const main = readJsonc(MAIN_CONFIG);
  if (!main) return envMap; // 无主配置：会话用 GQY 默认配置即可
  const counter = { n: 0 };
  redactKeysToEnv(main, envMap, counter);
  try {
    fs.writeFileSync(cfgPath, JSON.stringify(main, null, 2));
    log(logFile, `会话 ${path.basename(sessionHome)} 初始化配置（${counter.n} 个密钥改为 $env 引用）`);
  } catch (e) {
    log(logFile, `写入会话配置失败: ${e.message}`);
  }
  return envMap;
}

/** 调用 gqy ask；会话密钥经 extraEnv 注入，避免子进程环境里出现明文 */
function askGqy(question, sessionHome, extraEnv, logFile, channel) {
  return new Promise((resolve) => {
    const started = Date.now();
    log(logFile, `ask 开始: ${question.slice(0, 60)}`);
    const child = spawn(GQY_BIN, ['--stdout', 'ask', question], {
      env: {
        ...process.env,
        NO_COLOR: '1',
        ...(sessionHome ? { GQY_HOME: sessionHome } : {}),
        // 每个通信通道独立上下文（QQ/Telegram 各有各的对话记忆）
        ...(channel ? { GQY_CHANNEL: channel } : {}),
        ...(extraEnv || {}),
      },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    child.stdin.end(); // 立即关闭 stdin，避免挂起

    let out = '', err = '', timedOut = false;
    child.stdout.on('data', (d) => { out += d; });
    child.stderr.on('data', (d) => { err += d; });

    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
      log(logFile, `ask 超时 (${GQY_TIMEOUT_MS / 1000}s) 已终止`);
      resolve(`处理超时了（${GQY_TIMEOUT_MS / 1000}s），换个问法试试？`);
    }, GQY_TIMEOUT_MS);

    child.on('error', (e) => {
      clearTimeout(timer);
      log(logFile, `ask 启动失败: ${e.message}`);
      resolve(`调用出错：${e.message}`);
    });
    child.on('close', (code) => {
      if (timedOut) return;
      clearTimeout(timer);
      const elapsed = ((Date.now() - started) / 1000).toFixed(1);
      if (code === 0) {
        const reply = cleanOutput(out);
        log(logFile, `ask 完成 (${elapsed}s): ${reply.slice(0, 60)}`);
        resolve(reply || '(我没想出该说啥)');
      } else {
        const detail = cleanOutput(err || out) || `退出码 ${code}`;
        log(logFile, `ask 失败 (${elapsed}s, code=${code}): ${detail.slice(0, 120)}`);
        resolve(`出错了：${detail.slice(0, 200)}`);
      }
    });
  });
}

/**
 * 长回复按平台限制分片（emoji/中文按码元切分，避免切断代理对）。
 * Telegram 单条上限 4096 字符；QQ 群/私聊更宽松，统一用 4000 保守切分。
 */
function splitReply(text, limit = 4000) {
  if (!text) return [];
  const parts = [];
  let rest = text;
  while (rest.length > limit) {
    let cut = rest.lastIndexOf('\n', limit);
    if (cut < limit * 0.5) cut = rest.lastIndexOf('。', limit);
    if (cut < limit * 0.5) cut = limit;
    const hardCut = cut === limit;
    // 避免切断代理对（emoji 等）
    while (cut > 0 && cut < rest.length) {
      const code = rest.charCodeAt(cut);
      if (code >= 0xdc00 && code <= 0xdfff) { cut--; continue; }
      break;
    }
    const end = hardCut ? cut : cut + 1; // 软切点带上分隔符（\n / 。）
    parts.push(rest.slice(0, end));
    rest = rest.slice(end);
  }
  if (rest) parts.push(rest);
  return parts;
}

/**
 * 会话串行队列：同一会话的消息逐个处理。
 * 避免同一 GQY_HOME 下并发 gqy 进程读写 conversation.db 的竞态与回复乱序。
 */
const sessionQueues = new Map();
function enqueueSession(sessionKey, task) {
  const prev = sessionQueues.get(sessionKey) || Promise.resolve();
  const next = prev.then(task);
  const guard = next.catch(() => {});
  sessionQueues.set(sessionKey, guard);
  guard.finally(() => {
    if (sessionQueues.get(sessionKey) === guard) sessionQueues.delete(sessionKey);
  });
  return next;
}

module.exports = {
  SESSIONS_DIR,
  MAIN_CONFIG,
  log,
  cleanOutput,
  readJsonc,
  redactKeysToEnv,
  ensureSession,
  askGqy,
  splitReply,
  enqueueSession,
};
