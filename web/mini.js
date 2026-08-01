// 顾清影 · 迷你对话（独立逻辑，非面板缩小版）
// 输入框发送 → queue API → SSE 流式接收 → 展开回复区
(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);
  const conversation = $("conversation");
  const scroll = $("scroll");
  const messages = $("messages");
  const thinking = $("thinking");
  const input = $("input");
  const form = $("form");
  const send = $("send");
  const expand = $("expand");

  let currentAssistant = null;   // 当前流式回复元素
  let currentRunId = null;
  let es = null;                 // SSE 连接
  let busy = false;

  // ── 工具 ──
  function escapeHtml(text) {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // 极简行级 markdown（迷你窗口够用：粗体/斜体/行内代码/链接/代码块/列表）
  function renderMarkdown(text) {
    let html = escapeHtml(text);
    html = html.replace(/```([\s\S]*?)```/g, (_, code) => `<pre><code>${code.trim()}</code></pre>`);
    html = html
      .replace(/`([^`\n]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
      .replace(/\*([^*\n]+)\*/g, "<em>$1</em>")
      .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank">$1</a>')
      .replace(/(https?:\/\/[^\s<]+)/g, '<a href="$1" target="_blank">$1</a>')
      .replace(/^### (.+)$/gm, "<h3>$1</h3>")
      .replace(/^## (.+)$/gm, "<h2>$1</h2>")
      .replace(/^# (.+)$/gm, "<h2>$1</h2>")
      .replace(/^- (.+)$/gm, "<li>$1</li>")
      .replace(/(<li>[\s\S]*?<\/li>)(?![\s\S]*<li>)/g, "<ul>$1</ul>")
      .replace(/^&gt; (.+)$/gm, "<blockquote>$1</blockquote>");
    return html.replace(/\n/g, "<br>");
  }

  function addMessage(role, text) {
    const el = document.createElement("div");
    el.className = `msg ${role}`;
    if (role === "user") {
      el.textContent = text;
    } else {
      el.innerHTML = renderMarkdown(text);
    }
    messages.appendChild(el);
    return el;
  }

  function scrollToBottom() {
    scroll.scrollTop = scroll.scrollHeight;
  }

  // ── 思考动画 ──
  function showThinking() { thinking.hidden = false; scrollToBottom(); }
  function hideThinking() { thinking.hidden = true; }

  // ── 输入条 ──
  function autoResize() {
    input.style.height = "auto";
    input.style.height = Math.min(input.scrollHeight, 100) + "px";
  }
  input.addEventListener("input", () => {
    autoResize();
    send.disabled = busy || input.value.trim().length === 0;
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      form.requestSubmit();
    }
  });

  // ── 发送：queue API ──
  async function submit() {
    const text = input.value.trim();
    if (!text || busy) return;
    busy = true;
    send.disabled = true;
    send.classList.add("busy");
    input.value = "";
    autoResize();

    // 展开回复区
    conversation.hidden = false;
    addMessage("user", text);
    showThinking();
    scrollToBottom();

    try {
      // /api/turns：无运行轮次时发起新对话；运行中则排队（与面板同 API）
      const res = await fetch("/api/turns", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ content: text, mode: "normal" }),
      });
      if (!res.ok) throw new Error(`send ${res.status}`);
      const data = await res.json().catch(() => ({}));
      currentRunId = data?.run_id || null;
      connectSSE();
    } catch (err) {
      hideThinking();
      addMessage("assistant", `发送失败：${err.message}`);
      finishTurn();
    }
  }
  form.addEventListener("submit", (e) => { e.preventDefault(); submit(); });

  // ── SSE 流式接收 ──
  function connectSSE() {
    if (es) es.close();
    es = new EventSource("/api/events");
    es.addEventListener("assistant.delta", (e) => {
      const data = JSON.parse(e.data);
      hideThinking();
      if (!currentAssistant || data.run_id !== currentRunId) {
        currentAssistant = addMessage("assistant", "");
        currentAssistant.classList.add("streaming");
        currentRunId = data.run_id;
      }
      currentAssistant.innerHTML = renderMarkdown(data.delta);
      currentAssistant.classList.add("streaming");
      scrollToBottom();
    });
    es.addEventListener("reasoning.start", () => { showThinking(); });
    es.addEventListener("reasoning.delta", () => { hideThinking(); });
    es.addEventListener("run.completed", () => { finishTurn(); });
    es.addEventListener("run.cancelled", () => { finishTurn(); });
    es.addEventListener("run.failed", () => { finishTurn(); });
    es.onerror = () => { /* 重连由 EventSource 自动处理 */ };
  }

  function finishTurn() {
    hideThinking();
    if (currentAssistant) currentAssistant.classList.remove("streaming");
    currentAssistant = null;
    busy = false;
    send.classList.remove("busy");
    send.disabled = input.value.trim().length === 0;
    if (es) { es.close(); es = null; }
    scrollToBottom();
  }

  // ── 放大按钮 ──
  expand.addEventListener("click", () => {
    try {
      window.webkit.messageHandlers.gqyExpand.postMessage("expand");
    } catch (_) {
      window.open("/", "_blank");
    }
  });

  input.focus();
})();
