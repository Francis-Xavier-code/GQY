/**
 * GQY 用量分析 v2 —— 自包含增强面板（零改动接入）
 *
 * 不修改 app.js / index.html：本脚本自注入一个浮动按钮 + 全屏遮罩面板，
 * 直接读取现有 API（/api/usage/stats、/api/usage/details）渲染：
 *   1. GitHub 式贡献热力图（52 周 × 7 天，月/周标签 + 色阶图例 + 悬浮详情）
 *   2. 费用估算（元）：按 provider 单价表折算，可配置、存 localStorage
 *   3. token 构成：近 30 天 prompt / completion / cache_read 堆叠柱状
 *   4. 模型维度表：token、费用、缓存命中率、点击下钻
 *   5. 调用级明细：每次调用的 cache 命中与费用（可直接看到「烧钱」的那次）
 *
 * 加载方式：在 index.html 里 <script defer src="/usage-viz.js"></script>，
 * 或直接在浏览器控制台 eval 本文件内容。
 */
(function () {
  'use strict';

  if (window.__GQY_USAGE_VIZ__) return;
  window.__GQY_USAGE_VIZ__ = true;

  const NS = 'gqyuv';
  const $ = (sel, root) => (root || document).querySelector(sel);
  const el = (tag, cls, text) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  };

  // ─────────────────────────── 单价表（元 / 百万 token） ───────────────────────────
  // 可点击面板里的「单价」修改，存 localStorage。DeepSeek V3.2 官方价：
  // 输入 ¥2 / 输出 ¥8 / 缓存命中 ¥0.2。其他供应商先按同价估算，可自行改。
  const DEFAULT_PRICES = {
    deepseek: { input: 2, output: 8, cache: 0.2 },
    opencode: { input: 2, output: 8, cache: 0.2 },
    default: { input: 2, output: 8, cache: 0.2 },
  };
  function loadPrices() {
    try { return Object.assign({}, DEFAULT_PRICES, JSON.parse(localStorage.getItem(NS + '-prices') || '{}')); }
    catch (_) { return DEFAULT_PRICES; }
  }
  function savePrices(prices) {
    try { localStorage.setItem(NS + '-prices', JSON.stringify(prices)); } catch (_) {}
  }
  const priceOf = (prices, provider) =>
    prices[provider] || prices.default || DEFAULT_PRICES.default;

  function costOf(prices, provider, prompt, completion, cacheRead) {
    const p = priceOf(prices, provider);
    return (prompt * p.input + completion * p.output + (cacheRead || 0) * p.cache) / 1e6;
  }

  // ─────────────────────────── 格式化 ───────────────────────────

  function fmtTokens(n) {
    if (n == null) return '0';
    if (n >= 1e9) return (n / 1e9).toFixed(2) + 'B';
    if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
    if (n >= 1e3) return (n / 1e3).toFixed(1) + 'k';
    return String(n);
  }
  function fmtYuan(n) {
    if (n == null) return '¥0';
    if (Math.abs(n) < 0.01) return '¥' + n.toFixed(4);
    return '¥' + n.toFixed(2);
  }
  function fmtDate(ts) {
    const d = new Date(ts * 1000);
    const pad = (x) => String(x).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }
  function fmtNum(n) {
    return (n == null ? 0 : n).toLocaleString('en-US');
  }

  // ─────────────────────────── API ───────────────────────────

  async function apiJson(path) {
    const res = await fetch(path);
    if (!res.ok) throw new Error(`${path} -> ${res.status}`);
    return res.json();
  }

  // ─────────────────────────── 热力图（GitHub 式） ───────────────────────────

  function renderHeatmap(container, daily, prices) {
    // daily: [{date:"YYYY-MM-DD", tokens, requests}]
    const byDate = new Map(daily.map((d) => [d.date, d]));
    const today = new Date();
    // 定位到本周日（GitHub 风格：列=周，行=周一..周日）
    const end = new Date(today);
    end.setDate(end.getDate() + (7 - ((end.getDay() + 6) % 7))); // 本周日
    const start = new Date(end);
    start.setDate(start.getDate() - 52 * 7 + 1); // 52 周前周一
    const maxTokens = Math.max(1, ...daily.map((d) => d.tokens));

    const grid = el('div', NS + '-heatmap');
    const inner = el('div', NS + '-heatmap-inner');
    grid.appendChild(inner);

    // 月份标签行
    const months = el('div', NS + '-hm-months');
    const weekRow = el('div', NS + '-hm-weeks');
    const cells = el('div', NS + '-hm-cells');

    // 生成 53 列（周）
    const weeks = [];
    for (let w = 0; w < 53; w++) {
      const col = el('div', NS + '-hm-week');
      const colDate = new Date(start);
      colDate.setDate(start.getDate() + w * 7);
      if (w === 0 || colDate.getMonth() !== new Date(start.getDate() + (w - 1) * 7 + '' ).getMonth?.() || !weeks.length) {
        // 月初第一列打月份标签
      }
      for (let day = 0; day < 7; day++) {
        const d = new Date(colDate);
        d.setDate(colDate.getDate() + day);
        const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
        const rec = byDate.get(key);
        const tokens = rec ? rec.tokens : 0;
        const cell = el('div', NS + '-hm-cell' + (tokens === 0 ? ' l0' : tokens < maxTokens / 4 ? ' l1' : tokens < maxTokens / 2 ? ' l2' : tokens < maxTokens * 0.8 ? ' l3' : ' l4'));
        cell.title = '';
        cell.addEventListener('mouseenter', () => {
          tooltip.show(cell, rec
            ? `${key}\n${fmtTokens(tokens)} token · ${rec.requests} 次\n≈ ${fmtYuan(costOf(prices, 'deepseek', tokens, 0, 0))}（按默认单价粗估）`
            : `${key}\n无记录`);
        });
        col.appendChild(cell);
      }
      weeks.push(col);
    }

    // 月标签（每列顶部）
    const monthPos = new Map();
    for (let w = 0; w < weeks.length; w++) {
      const d = new Date(start); d.setDate(start.getDate() + w * 7 + 3);
      const mkey = `${d.getFullYear()}-${d.getMonth()}`;
      if (!monthPos.has(mkey)) monthPos.set(mkey, w);
    }
    const monthNames = ['1月', '2月', '3月', '4月', '5月', '6月', '7月', '8月', '9月', '10月', '11月', '12月'];
    for (const [mkey, w] of monthPos) {
      const [y, m] = mkey.split('-').map(Number);
      const label = el('span', NS + '-hm-month', `${y}-${monthNames[m]}`);
      label.style.gridColumnStart = String(w + 1);
      months.appendChild(label);
    }
    // 周标签（左列 周一/三/五）
    for (const [i, name] of [[1, '一'], [3, '三'], [5, '五']]) {
      const l = el('span', NS + '-hm-day', name);
      l.style.gridRowStart = String(i + 1);
      weekRow.appendChild(l);
    }

    inner.appendChild(months);
    inner.appendChild(weekRow);
    for (const c of weeks) cells.appendChild(c);
    inner.appendChild(cells);

    // 图例
    const legend = el('div', NS + '-legend');
    legend.append(
      el('span', '', '少'),
      ...['l0', 'l1', 'l2', 'l3', 'l4'].map((l) => el('span', NS + '-legend-cell ' + l)),
      el('span', '', '多'),
    );

    container.replaceChildren(grid, legend);
  }

  // 简单 tooltip（面板内绝对定位）
  const tooltip = {
    node: null,
    show(anchor, text) {
      if (!this.node) {
        this.node = el('div', NS + '-tooltip');
        document.body.appendChild(this.node);
      }
      this.node.textContent = text;
      this.node.style.display = 'block';
      const r = anchor.getBoundingClientRect();
      this.node.style.left = Math.min(r.left, window.innerWidth - 220) + 'px';
      this.node.style.top = (r.top - 8) + 'px';
    },
    hide() { if (this.node) this.node.style.display = 'none'; },
  };
  document.addEventListener('mouseover', (e) => {
    if (!e.target.closest('.' + NS + '-hm-cell')) tooltip.hide();
  });

  // ─────────────────────────── 近 30 天堆叠柱状 ───────────────────────────

  function renderBars(container, records, prices) {
    // records: 新→旧，含 {ts, provider, prompt, completion, cache_read}
    const days = new Map();
    for (const r of records) {
      const d = new Date(r.ts * 1000);
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
      const agg = days.get(key) || { prompt: 0, completion: 0, cache: 0, cost: 0 };
      agg.prompt += r.prompt || 0;
      agg.completion += r.completion || 0;
      agg.cache += r.cache_read || 0;
      agg.cost += costOf(prices, r.provider, r.prompt || 0, r.completion || 0, r.cache_read || 0);
      days.set(key, agg);
    }
    const keys = [...days.keys()].sort().slice(-30);
    const maxV = Math.max(1, ...keys.map((k) => days.get(k).prompt + days.get(k).completion));

    const wrap = el('div', NS + '-bars-wrap');
    const chart = el('div', NS + '-bars');
    for (const key of keys) {
      const agg = days.get(key);
      const col = el('div', NS + '-bar-col');
      const bar = el('div', NS + '-bar');
      const segP = el('div', NS + '-seg seg-prompt', '');
      const segC = el('div', NS + '-seg seg-completion', '');
      const segK = el('div', NS + '-seg seg-cache', '');
      const hP = (agg.prompt / maxV) * 100;
      const hC = (agg.completion / maxV) * 100;
      const hK = (agg.cache / maxV) * 100;
      segP.style.height = Math.max(hP, 0.5) + '%';
      segC.style.height = Math.max(hC, 0.5) + '%';
      segK.style.height = Math.max(hK, 0.5) + '%';
      bar.append(segK, segP, segC);
      col.appendChild(bar);
      const label = el('div', NS + '-bar-label', key.slice(5));
      col.appendChild(label);
      col.addEventListener('mouseenter', () => tooltip.show(col,
        `${key}\n输入 ${fmtTokens(agg.prompt)} · 输出 ${fmtTokens(agg.completion)} · 缓存 ${fmtTokens(agg.cache)}\n≈ ${fmtYuan(agg.cost)}`));
      chart.appendChild(col);
    }
    const legend = el('div', NS + '-legend');
    legend.append(
      el('span', NS + '-legend-chip seg-prompt', '输入'),
      el('span', NS + '-legend-chip seg-completion', '输出'),
      el('span', NS + '-legend-chip seg-cache', '缓存命中'),
    );
    wrap.append(chart, legend);
    container.replaceChildren(wrap);
  }

  // ─────────────────────────── 模型表 + 明细表 ───────────────────────────

  function renderModelTable(container, models, prices) {
    if (!models || !models.length) {
      container.replaceChildren(el('div', NS + '-empty', '暂无用量数据'));
      return;
    }
    const table = el('table', NS + '-table');
    const head = el('thead');
    head.appendChild(el('tr', '', ''))
      .append(
        el('th', '', '供应商 / 模型'), el('th', '', '请求'), el('th', '', '输入'),
        el('th', '', '输出'), el('th', '', '总计'), el('th', '', '估算费用'),
      );
    const body = el('tbody');
    for (const m of models) {
      const cost = costOf(prices, m.provider_id, m.prompt_tokens, m.completion_tokens, 0);
      const tr = el('tr');
      const td = (txt) => { const t = el('td', '', txt); tr.appendChild(t); return t; };
      td(`${m.provider_id} · ${m.model}`);
      td(fmtNum(m.requests));
      td(fmtTokens(m.prompt_tokens));
      td(fmtTokens(m.completion_tokens));
      td(fmtTokens(m.total_tokens));
      td(fmtYuan(cost));
      body.appendChild(tr);
    }
    table.append(head, body);
    container.replaceChildren(table);
  }

  function renderDetailTable(container, records, prices) {
    if (!records || !records.length) {
      container.replaceChildren(el('div', NS + '-empty', '暂无调用明细'));
      return;
    }
    const table = el('table', NS + '-table detail');
    const head = el('thead');
    head.appendChild(el('tr', '', ''))
      .append(
        el('th', '', '时间'), el('th', '', '供应商 / 模型'), el('th', '', '输入'),
        el('th', '', '输出'), el('th', '', '缓存命中'), el('th', '', '费用'),
      );
    const body = el('tbody');
    for (const r of records.slice(0, 300)) {
      const cost = costOf(prices, r.provider, r.prompt, r.completion, r.cache_read);
      const cachePct = r.prompt ? (((r.cache_read || 0) / r.prompt) * 100).toFixed(0) : '–';
      const tr = el('tr');
      const td = (txt, cls) => { const t = el('td', cls || '', txt); tr.appendChild(t); return t; };
      td(fmtDate(r.ts));
      td(`${r.provider} · ${r.model}`);
      td(fmtTokens(r.prompt));
      td(fmtTokens(r.completion));
      td(`${fmtTokens(r.cache_read)} (${cachePct}%)`);
      td(fmtYuan(cost), cost > 1 ? NS + '-hot' : '');
      body.appendChild(tr);
    }
    table.append(head, body);
    container.replaceChildren(table);
  }

  // ─────────────────────────── 面板组装 ───────────────────────────

  function buildPanel() {
    const overlay = el('div', NS + '-overlay');
    overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });

    const panel = el('div', NS + '-panel');
    const header = el('div', NS + '-header');
    header.append(
      el('strong', '', '📊 用量分析 v2'),
      el('button', NS + '-close', '✕'),
    );
    header.querySelector('button').addEventListener('click', close);

    const summary = el('div', NS + '-summary');
    const heat = el('section', NS + '-card');
    heat.append(el('h3', '', 'Token 贡献热力图（近 52 周）'));
    const heatBody = el('div', '', '载入中…');
    heat.appendChild(heatBody);

    const bars = el('section', NS + '-card');
    bars.append(el('h3', '', '近 30 天 Token 构成'));
    const barsBody = el('div', '', '载入中…');
    bars.appendChild(barsBody);

    const models = el('section', NS + '-card');
    models.append(el('h3', '', '按模型汇总'));
    const modelsBody = el('div', '', '载入中…');
    models.appendChild(modelsBody);

    const detail = el('section', NS + '-card');
    detail.append(el('h3', '', '最近调用明细（红色 = 单次费用 > ¥1）'));
    const detailBody = el('div', '', '载入中…');
    detail.appendChild(detailBody);

    panel.append(header, summary, heat, bars, models, detail);
    overlay.appendChild(panel);
    document.body.appendChild(overlay);

    // 单价编辑
    const pricesEditor = el('div', NS + '-prices');
    const prices = loadPrices();
    const mkPriceInput = (provider, key, label) => {
      const wrap = el('label', NS + '-price-field');
      wrap.append(el('span', '', `${label}`));
      const input = el('input', '');
      input.type = 'number';
      input.step = '0.1';
      input.min = '0';
      input.value = String((prices[provider] || DEFAULT_PRICES.default)[key]);
      input.addEventListener('change', () => {
        const v = parseFloat(input.value);
        if (Number.isFinite(v) && v >= 0) {
          prices[provider] = Object.assign({}, prices[provider] || DEFAULT_PRICES.default, { [key]: v });
          savePrices(prices);
          refresh();
        }
      });
      wrap.appendChild(input);
      return wrap;
    };
    const priceRow = el('div', NS + '-price-row');
    priceRow.append(
      el('span', NS + '-price-title', '单价（元/M token）：'),
      mkPriceInput('deepseek', 'input', '输入'),
      mkPriceInput('deepseek', 'output', '输出'),
      mkPriceInput('deepseek', 'cache', '缓存'),
    );
    summary.appendChild(priceRow);

    async function refresh() {
      try {
        const [statsRes, detailRes] = await Promise.all([
          apiJson('/api/usage/stats'),
          apiJson('/api/usage/details'),
        ]);
        const stats = statsRes.stats;
        const records = Array.isArray(detailRes.records) ? detailRes.records : [];
        const p = loadPrices();

        // 汇总卡片
        summary.replaceChildren(priceRow);
        const cards = el('div', NS + '-cards');
        for (const [label, agg] of [['累计', stats.total], ['今日', stats.today], ['本周', stats.this_week], ['本月', stats.this_month]]) {
          const cost = costOf(p, 'deepseek', agg.prompt_tokens, agg.completion_tokens, 0);
          const card = el('div', NS + '-card-mini');
          card.append(
            el('div', NS + '-mini-label', label),
            el('div', NS + '-mini-tokens', fmtTokens(agg.total_tokens)),
            el('div', NS + '-mini-sub', `${fmtNum(agg.requests)} 次 · ≈ ${fmtYuan(cost)}`),
          );
          cards.appendChild(card);
        }
        summary.appendChild(cards);

        renderHeatmap(heatBody, stats.daily || [], p);
        renderBars(barsBody, records, p);
        renderModelTable(modelsBody, stats.models || [], p);
        renderDetailTable(detailBody, records, p);
      } catch (e) {
        summary.replaceChildren(el('div', NS + '-empty', '加载失败：' + e.message));
      }
    }

    refresh();
    const refreshBtn = el('button', NS + '-refresh', '↻ 刷新');
    refreshBtn.addEventListener('click', refresh);
    header.appendChild(refreshBtn);

    function close() {
      overlay.remove();
    }
  }

  // 浮动按钮（不依赖 app.js 的 DOM 结构）
  function injectButton() {
    if (document.getElementById(NS + '-fab')) return;
    const fab = el('button', NS + '-fab', '📊');
    fab.title = '用量分析 v2';
    fab.id = NS + '-fab';
    fab.addEventListener('click', buildPanel);
    document.body.appendChild(fab);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', injectButton);
  } else {
    injectButton();
  }

  // ─────────────────────────── 样式 ───────────────────────────

  const css = `
.${NS}-fab{position:fixed;right:20px;bottom:20px;z-index:99999;width:48px;height:48px;border-radius:24px;border:none;background:#0a84ff;color:#fff;font-size:20px;cursor:pointer;box-shadow:0 4px 14px rgba(0,0,0,.35);}
.${NS}-fab:hover{background:#409cff;}
.${NS}-overlay{position:fixed;inset:0;z-index:99998;background:rgba(0,0,0,.55);backdrop-filter:blur(6px);display:flex;align-items:center;justify-content:center;padding:20px;}
.${NS}-panel{background:#1c1c1e;color:#e5e5ea;border-radius:14px;width:min(1100px,96vw);max-height:92vh;overflow:auto;padding:18px 22px;box-shadow:0 20px 60px rgba(0,0,0,.5);font-size:13px;line-height:1.5;}
.${NS}-header{display:flex;align-items:center;gap:10px;margin-bottom:12px;}
.${NS}-header strong{font-size:16px;}
.${NS}-close,.${NS}-refresh{margin-left:auto;background:#3a3a3c;color:#e5e5ea;border:none;border-radius:6px;padding:5px 12px;cursor:pointer;font-size:12px;}
.${NS}-close{margin-left:auto;}
.${NS}-refresh{margin-left:0;}
.${NS}-card{background:#2c2c2e;border-radius:10px;padding:14px 16px;margin-bottom:14px;}
.${NS}-card h3{margin:0 0 10px;font-size:13px;color:#98989d;font-weight:600;}
.${NS}-cards{display:flex;gap:10px;margin-top:10px;flex-wrap:wrap;}
.${NS}-card-mini{background:#2c2c2e;border-radius:10px;padding:12px 14px;flex:1;min-width:130px;}
.${NS}-mini-label{color:#98989d;font-size:12px;}
.${NS}-mini-tokens{font-size:18px;font-weight:700;margin:4px 0;}
.${NS}-mini-sub{color:#98989d;font-size:12px;}
.${NS}-price-row{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:4px;}
.${NS}-price-title{color:#98989d;font-size:12px;}
.${NS}-price-field{display:flex;align-items:center;gap:4px;color:#98989d;font-size:12px;}
.${NS}-price-field input{width:64px;background:#1c1c1e;border:1px solid #3a3a3c;color:#e5e5ea;border-radius:6px;padding:3px 6px;font-size:12px;}
.${NS}-heatmap{overflow-x:auto;}
.${NS}-heatmap-inner{display:grid;grid-template-columns:30px 1fr;grid-template-rows:20px 1fr;gap:4px;min-width:820px;}
.${NS}-hm-months{grid-column:2;display:grid;grid-auto-flow:column;grid-auto-columns:1fr;font-size:10px;color:#98989d;white-space:nowrap;}
.${NS}-hm-weeks{grid-row:2;display:grid;grid-template-rows:repeat(7,1fr);font-size:10px;color:#98989d;}
.${NS}-hm-cells{grid-column:2;grid-row:2;display:grid;grid-auto-flow:column;grid-auto-columns:1fr;gap:3px;}
.${NS}-hm-week{display:grid;grid-template-rows:repeat(7,1fr);gap:3px;}
.${NS}-hm-cell{width:12px;height:12px;border-radius:3px;background:#2c2c2e;}
.${NS}-hm-cell.l1{background:#0e4429;}
.${NS}-hm-cell.l2{background:#006d32;}
.${NS}-hm-cell.l3{background:#26a641;}
.${NS}-hm-cell.l4{background:#39d353;}
.${NS}-legend{display:flex;align-items:center;gap:4px;margin-top:8px;color:#98989d;font-size:11px;}
.${NS}-legend-cell{width:10px;height:10px;border-radius:2px;}
.${NS}-legend-cell.l0{background:#2c2c2e;}.${NS}-legend-cell.l1{background:#0e4429;}.${NS}-legend-cell.l2{background:#006d32;}.${NS}-legend-cell.l3{background:#26a641;}.${NS}-legend-cell.l4{background:#39d353;}
.${NS}-bars-wrap{overflow-x:auto;}
.${NS}-bars{display:flex;align-items:flex-end;gap:4px;min-width:560px;height:140px;}
.${NS}-bar-col{flex:1;display:flex;flex-direction:column;align-items:center;height:100%;justify-content:flex-end;gap:2px;}
.${NS}-bar{width:100%;max-width:18px;height:100%;display:flex;flex-direction:column-reverse;border-radius:2px;overflow:hidden;background:#2c2c2e;}
.${NS}-seg{width:100%;}
.${NS}-seg-prompt{background:#0a84ff;}
.${NS}-seg-completion{background:#ff9f0a;}
.${NS}-seg-cache{background:#30d158;}
.${NS}-bar-label{font-size:9px;color:#98989d;transform:rotate(-45deg);white-space:nowrap;}
.${NS}-legend-chip{display:inline-block;width:10px;height:10px;border-radius:2px;margin:0 4px 0 10px;}
.${NS}-table{width:100%;border-collapse:collapse;font-size:12px;}
.${NS}-table th,.${NS}-table td{padding:6px 8px;border-bottom:1px solid #3a3a3c;text-align:left;white-space:nowrap;}
.${NS}-table th{color:#98989d;font-weight:600;}
.${NS}-table.detail td:last-child{text-align:right;}
.${NS}-hot{color:#ff453a;font-weight:700;}
.${NS}-empty{color:#98989d;padding:20px;text-align:center;}
.${NS}-tooltip{position:fixed;z-index:100000;background:#000;color:#fff;font-size:12px;padding:8px 10px;border-radius:6px;white-space:pre-line;pointer-events:none;box-shadow:0 4px 12px rgba(0,0,0,.4);display:none;max-width:260px;}
`;
  const style = document.createElement('style');
  style.textContent = css;
  document.head.appendChild(style);
})();
