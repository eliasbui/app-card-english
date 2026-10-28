// The tutor: a chat with the AI about the learner's own words of the course, kept per course in the database.
(() => {
  "use strict";
  if (window.MINI) return;
  const THREAD = "tutor";
  /// Sent as they are, but the last, whose blanks the learner fills in.
  const STARTERS = ["Đố tôi 5 từ đã học", "Giải thích từ tôi hay quên", "Đặt câu với từ mới học", "Khác nhau giữa ... và ..."];
  /// The most messages kept on screen; older ones stay in the database.
  const SHOWN = 80;

  document.body.insertAdjacentHTML("beforeend", `
    <button class="chat-fab" id="chat-fab" title="Trợ lý học" aria-label="Mở Trợ lý học" aria-expanded="false" aria-controls="chat-panel">${st("ai")}</button>
    <aside class="chat-panel" id="chat-panel" role="dialog" aria-labelledby="chat-title" hidden>
      <header class="chat-head">
        ${st("ai")}
        <div class="grow"><b id="chat-title">Trợ lý học</b><span id="chat-sub"></span></div>
        <button class="btn small ghost" id="chat-clear">Xóa lịch sử</button>
        <button class="btn small" id="chat-close">Đóng</button>
      </header>
      <div class="chat-log" id="chat-log" role="log" aria-live="polite"></div>
      <div class="chips chat-starters" id="chat-starters">${STARTERS.map((s) => `<button type="button">${esc(s)}</button>`).join("")}</div>
      <form class="chat-compose" id="chat-form">
        <textarea id="chat-input" rows="1" maxlength="1000" aria-label="Câu hỏi cho trợ lý"></textarea>
        <button class="btn primary" type="submit" id="chat-send">Gửi</button>
      </form>
    </aside>`);
  const fab = $("chat-fab"), panel = $("chat-panel"), log = $("chat-log"), input = $("chat-input"), send = $("chat-send");
  /// The course whose history is on screen, and whether a question is on its way.
  let shown = "", busy = false;

  const zh = () => courseOf() === "zh";
  function labels() {
    $("chat-sub").textContent = zh() ? "Khóa tiếng Trung" : "Khóa tiếng Anh";
    input.placeholder = "Hỏi về từ, câu hay ngữ pháp";
  }

  /// An answer as a bubble shows it: bold where the model marked it, headings as bold lines, the rest as written.
  const rich = (text) => esc(text).replace(/^#{1,4}\s*(.+)$/gm, "<b>$1</b>").replace(/\*\*(.+?)\*\*/g, "<b>$1</b>");
  function bubble(m, by = "") {
    if (m.role === "user") return `<div class="chat-msg me"><div class="chat-bubble">${esc(m.text)}</div></div>`;
    const who = by || (m.source ? `Trả lời bởi ${esc(m.source)}.` : "");
    return `<div class="chat-msg"><div class="chat-bubble">${rich(m.text)}</div>${who ? `<div class="chat-by">${who}</div>` : ""}</div>`;
  }
  function empty() {
    log.innerHTML = `<div class="chat-empty">${st(zh() ? "mascot-zh" : "mascot-think")}<b>Chào bạn!</b>
      <p>Mình giúp bạn học tiếng ${zh() ? "Trung" : "Anh"} với chính các từ trong danh sách của bạn: đố từ, giải thích nghĩa và ngữ pháp, đặt câu ví dụ. Chọn một gợi ý bên dưới hoặc tự hỏi nhé.</p></div>`;
  }
  function add(html) {
    log.querySelector(".chat-empty")?.remove();
    log.insertAdjacentHTML("beforeend", html);
    while (log.children.length > SHOWN) log.firstElementChild.remove();
    log.scrollTop = log.scrollHeight;
    return log.lastElementChild;
  }

  async function load() {
    shown = courseOf();
    labels();
    log.innerHTML = `<div class="skeleton"></div>`;
    try {
      const messages = await app.call("chat_history", { thread: THREAD });
      if (shown !== courseOf()) return;
      if (!messages.length) return empty();
      log.innerHTML = messages.slice(-SHOWN).map((m) => bubble(m)).join("");
      log.scrollTop = log.scrollHeight;
    } catch (e) {
      log.innerHTML = `<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`;
    }
  }

  function openChat() {
    panel.hidden = false;
    document.documentElement.classList.add("chat-open");
    fab.setAttribute("aria-expanded", "true");
    if (shown !== courseOf()) load();
    input.focus({ preventScroll: true });
  }
  function closeChat() {
    if (panel.hidden) return;
    panel.hidden = true;
    document.documentElement.classList.remove("chat-open");
    fab.setAttribute("aria-expanded", "false");
    fab.focus({ preventScroll: true });
  }

  function fit() {
    input.style.height = "auto";
    input.style.height = Math.min(140, input.scrollHeight + 2) + "px";
  }

  async function ask(text) {
    text = text.trim();
    if (!text || busy) return;
    busy = true;
    send.disabled = true;
    const course = courseOf();
    const mine = add(bubble({ role: "user", text }));
    const wait = add(`<div class="chat-msg"><div class="thinking">${st("mascot-think")}<span>Trợ lý đang trả lời...</span></div></div>`);
    input.value = "";
    fit();
    try {
      const r = await app.call("tutor", { text, day: localDay() });
      if (course !== courseOf()) return;
      wait.remove();
      add(bubble(r.reply, r.answer.source ? byHtml(r.answer) : ""));
    } catch (e) {
      if (course !== courseOf()) return;
      // The question goes back into the box, to be asked again.
      wait.remove();
      mine.remove();
      if (!input.value) input.value = text;
      fit();
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
    } finally {
      busy = false;
      send.disabled = false;
    }
  }

  fab.onclick = openChat;
  $("chat-close").onclick = closeChat;
  $("chat-form").onsubmit = (e) => { e.preventDefault(); ask(input.value); };
  input.addEventListener("input", fit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); ask(input.value); }
  });
  $("chat-starters").onclick = (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    const text = b.textContent;
    if (!text.includes("...")) return ask(text);
    input.value = text;
    fit();
    input.focus();
    const at = text.indexOf("...");
    input.setSelectionRange(at, at + 3);
  };
  $("chat-clear").onclick = async (e) => {
    const b = e.currentTarget;
    if (!armed(b)) return;
    try {
      await app.call("chat_clear", { thread: THREAD });
      empty();
      toast("Đã xóa lịch sử trò chuyện");
    } catch (err) {
      toast(String(err));
    }
  };
  document.addEventListener("keydown", (e) => {
    // A dialog over the page closes first.
    if (e.key === "Escape" && !document.querySelector(".modal:not(.hidden)")) closeChat();
  });
  hooks.course.push(() => {
    labels();
    if (!panel.hidden) load(); else shown = "";
  });
})();
