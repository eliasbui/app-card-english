// The tutor: a chat with the AI about the learner's own words of the course, kept per course in the database. Each
// talk is a thread of its own, "tutor" for the first one and "tutor:<uid>" for each new one, listed in the history.
(() => {
  "use strict";
  if (window.MINI) return;
  const FIRST = "tutor";
  /// Sent as they are, but the last, whose blanks the learner fills in.
  const STARTERS = ["Gợi ý 10 từ chủ đề công nghệ", "Đố tôi 5 từ", "Tạo bài tập nối từ", "Tiến độ của tôi", "Giải thích từ tôi hay quên",
                    "Khác nhau giữa ... và ..."];
  /// The most messages kept on screen; older ones stay in the database.
  const SHOWN = 80;

  document.body.insertAdjacentHTML("beforeend", `
    <button class="chat-fab" id="chat-fab" title="Trợ lý học" aria-label="Mở Trợ lý học" aria-expanded="false" aria-controls="chat-panel">${st("ai")}</button>
    <aside class="chat-panel" id="chat-panel" role="dialog" aria-labelledby="chat-title" hidden>
      <header class="chat-head">
        ${st("ai")}
        <div class="grow"><b id="chat-title">Trợ lý học</b><span id="chat-sub"></span></div>
        <button class="btn small ghost" id="chat-new">Cuộc mới</button>
        <button class="btn small" id="chat-close">Đóng</button>
      </header>
      <div class="chat-tabs"><div class="seg" role="radiogroup" aria-label="Trợ lý học">
        <input type="radio" name="chat-tab" id="chat-tab-talk" value="talk" checked><label for="chat-tab-talk">Trò chuyện</label>
        <input type="radio" name="chat-tab" id="chat-tab-history" value="history"><label for="chat-tab-history">Lịch sử</label>
      </div></div>
      <div class="chat-view" id="chat-talk">
        <div class="chat-model"><label for="chat-model">Model</label><select id="chat-model"></select></div>
        <p class="chat-hint" id="chat-hint" hidden></p>
        <div class="chat-log" id="chat-log" role="log" aria-live="polite"></div>
        <div class="chips chat-starters" id="chat-starters">${STARTERS.map((s) => `<button type="button">${esc(s)}</button>`).join("")}</div>
        <form class="chat-compose" id="chat-form">
          <textarea id="chat-input" rows="1" maxlength="2000" aria-label="Câu hỏi cho trợ lý"></textarea>
          <button class="btn primary" type="submit" id="chat-send">Gửi</button>
        </form>
      </div>
      <div class="chat-view chat-history" id="chat-history" hidden></div>
    </aside>`);
  const fab = $("chat-fab"), panel = $("chat-panel"), log = $("chat-log"), input = $("chat-input"), send = $("chat-send");
  const history = $("chat-history"), select = $("chat-model"), hint = $("chat-hint");
  /// The course whose talk is on screen, the talk, and whether a question is on its way.
  let shown = "", thread = FIRST, busy = false;
  const fresh = () => "tutor:" + [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
  const when = (sec) => new Date(sec * 1000).toLocaleString("vi-VN", { day: "numeric", month: "numeric", hour: "2-digit", minute: "2-digit" });

  const zh = () => courseOf() === "zh";
  function labels() {
    $("chat-sub").textContent = zh() ? "Khóa tiếng Trung" : "Khóa tiếng Anh";
    input.placeholder = "Hỏi về từ, câu hay ngữ pháp";
  }

  /// Text in the language of the course: Chinese characters, or Latin letters without the marks of Vietnamese.
  const foreign = (t) => (zh() ? hasHan(t) : /[a-z]/i.test(t) && !/[àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđ]/i.test(t));
  /// An answer as a bubble shows it, in markdown, with the words of the course in bold read aloud when clicked.
  const rich = (text) => window.chatMarkdown(text, (t) => (foreign(t) ? `<b class="chat-say" data-say="${t}" title="Bấm để nghe">${t}</b>` : `<b>${t}</b>`));
  /// What "Nghe" reads of an answer: its Chinese, or its English words in bold.
  function spoken(text) {
    const parts = zh() ? text.match(/[\u4e00-\u9fff，。？！、]+/g) || [] : [...text.matchAll(/\*\*(.+?)\*\*/g)].map((m) => m[1]).filter(foreign);
    return parts.join(zh() ? "，" : ". ");
  }
  /// The lines on screen by uid, for copying, changing and asking again.
  const lines = new Map();
  const tool = (act, label) => `<button class="chat-tool" data-act="${act}">${label}</button>`;
  function bubble(m, by = "") {
    if (m.uid) lines.set(m.uid, m);
    const uid = m.uid ? ` data-uid="${esc(m.uid)}"` : "";
    if (m.role === "user") {
      return `<div class="chat-msg me"${uid}><div class="chat-bubble">${esc(m.text)}</div>
        ${m.uid ? `<div class="chat-tools">${tool("copy", "Sao chép")}${tool("edit", "Sửa")}</div>` : ""}</div>`;
    }
    const who = by || (m.source ? `Trả lời bởi ${esc(m.source)}.` : "");
    const say = spoken(m.text);
    return `<div class="chat-msg ai"${uid}><div class="chat-bubble md">${rich(m.text)}</div>
      ${m.cards?.length ? `<div class="chat-cards"></div>` : ""}
      ${say || who ? `<div class="chat-by">${say ? `<button class="btn small chat-listen" data-say="${esc(say)}">Nghe</button>` : ""}${who}</div>` : ""}
      ${m.uid ? `<div class="chat-tools">${tool("copy", "Sao chép")}${tool("again", "Hỏi lại")}</div>` : ""}</div>`;
  }
  /// The cards of an answer under its bubble, each drawn by the renderer of its type (chat-cards.js).
  function drawCards(el, m) {
    const box = el?.querySelector(".chat-cards");
    if (!box) return;
    for (const card of m.cards || []) {
      const holder = document.createElement("div");
      box.append(holder);
      window.chatCards?.[card.type]?.(card, holder, { message: m, course: courseOf() });
    }
  }
  /// Only the last answer can be asked again.
  function markLast() {
    log.querySelectorAll(".chat-msg.ai.last").forEach((el) => el.classList.remove("last"));
    [...log.querySelectorAll(".chat-msg.ai[data-uid]")].pop()?.classList.add("last");
  }
  /// The clipboard of the web view may be closed to the page, so a selected hidden text area is the way round it.
  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      const area = Object.assign(document.createElement("textarea"), { value: text });
      area.style.cssText = "position:fixed;top:0;opacity:0";
      document.body.append(area);
      area.select();
      const ok = document.execCommand("copy");
      area.remove();
      return ok;
    }
  }
  function empty() {
    log.innerHTML = `<div class="chat-empty">${st(zh() ? "mascot-zh" : "mascot-think")}<b>Chào bạn!</b>
      <p>Mình giúp bạn học và dùng tiếng ${zh() ? "Trung" : "Anh"}: gợi ý và thêm từ theo chủ đề, đố các từ của bạn, mở bài tập và bài thi, nhập danh sách từ, xem tiến độ, giải thích từ và ngữ pháp, dịch, viết và sửa email. Chọn một gợi ý bên dưới hoặc tự hỏi nhé.</p></div>`;
  }
  function add(html) {
    log.querySelector(".chat-empty")?.remove();
    log.insertAdjacentHTML("beforeend", html);
    while (log.children.length > SHOWN) log.firstElementChild.remove();
    log.scrollTop = log.scrollHeight;
    return log.lastElementChild;
  }

  /// The talk of the course talked in last, when the course is not the one on screen.
  async function load() {
    if (shown !== courseOf()) {
      shown = courseOf();
      const talks = await app.call("chat_threads").catch(() => []);
      thread = talks[0]?.thread || FIRST;
    }
    labels();
    log.innerHTML = `<div class="skeleton"></div>`;
    const course = courseOf(), asked = thread;
    try {
      const messages = await app.call("chat_history", { thread });
      if (course !== courseOf() || asked !== thread) return;
      if (!messages.length) return empty();
      const kept = messages.slice(-SHOWN);
      log.innerHTML = kept.map((m) => bubble(m)).join("");
      kept.filter((m) => m.cards?.length).forEach((m) => drawCards(log.querySelector(`[data-uid="${CSS.escape(m.uid)}"]`), m));
      markLast();
      log.scrollTop = log.scrollHeight;
    } catch (e) {
      log.innerHTML = `<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`;
    }
  }

  const QWEN = { llm: "Qwen 0.5B trên máy", llm15: "Qwen 1.5B trên máy" };
  /// The AI the tutor and the role-play answer with: the one chosen here, or else the app's; "local:llm" is Qwen 0.5B.
  function chosen() {
    if (state.settings.tutor_ai) return state.settings.tutor_ai;
    const p = state.settings.ai_provider || "local";
    return p === "local" ? "local:" + (state.settings.local_model || "llm") : p;
  }
  window.chatAi = chosen;
  /// The sources ready to answer, and the one chosen even when it is not, since the app falls back from it.
  function models() {
    const t = state.settings.tutor_ai || "", own = state.settings.ai_provider || "local";
    const options = [["", `Theo nguồn AI chung (${own === "local" ? QWEN[state.settings.local_model] || QWEN.llm : PROVIDER_NAME[own]})`]];
    for (const g of ["llm", "llm15"]) if (state.models[g]?.ready || t === "local:" + g) options.push(["local:" + g, QWEN[g]]);
    for (const p of ["chatgpt", "codex", "gemini", "deepseek"]) if (ready(p) || t === p) options.push([p, PROVIDER_NAME[p]]);
    select.innerHTML = options.map(([v, name]) => `<option value="${v}"${v === t ? " selected" : ""}>${esc(name)}</option>`).join("");
    hint.hidden = chosen() !== "local:llm";
    hint.innerHTML = `Qwen 0.5B rất nhỏ nên trả lời còn ngô nghê. ${state.models.llm15?.ready ? "Chọn Qwen 1.5B ở trên"
      : `<a href="#" data-settings>Tải Qwen 1.5B</a>`} hoặc dùng Gemini (API key miễn phí) để trợ lý thông minh hơn.`;
  }
  select.onchange = async () => {
    try {
      state.settings = await app.call("set_setting", { key: "tutor_ai", value: select.value });
    } catch (e) {
      toast(String(e));
    }
    models();
  };
  hint.onclick = (e) => {
    if (!e.target.closest("[data-settings]")) return;
    e.preventDefault();
    closeChat();
    go("settings");
  };

  function openChat() {
    panel.hidden = false;
    document.documentElement.classList.add("chat-open");
    fab.setAttribute("aria-expanded", "true");
    models();
    if (shown !== courseOf()) load();
    input.focus({ preventScroll: true });
  }
  window.chatClose = () => closeChat();
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

  /// Asks `text`; with `redo`, the uid of a question on screen, it replaces that question and what followed, which
  /// leave the database only once the new answer is there.
  async function ask(text, redo = "") {
    text = text.trim();
    if (!text || busy) return;
    busy = true;
    send.disabled = true;
    panel.classList.add("busy");
    const course = courseOf();
    const gone = [];
    for (let el = redo ? log.querySelector(`[data-uid="${CSS.escape(redo)}"]`) : null; el; el = el.nextElementSibling) gone.push(el);
    gone.forEach((el) => el.remove());
    const mine = add(bubble({ role: "user", text }));
    const wait = add(`<div class="chat-msg"><div class="thinking">${st("mascot-think")}<span>Trợ lý đang trả lời...</span></div></div>`);
    input.value = "";
    fit();
    try {
      const r = await app.call("tutor", { text, thread, day: localDay(), redo });
      if (course !== courseOf()) return;
      wait.remove();
      mine.insertAdjacentHTML("afterend", bubble(r.user));
      mine.remove();
      drawCards(add(bubble(r.reply, r.answer.source ? byHtml(r.answer) : "")), r.reply);
      markLast();
    } catch (e) {
      if (course !== courseOf()) return;
      // The question goes back into the box, to be asked again, and a replaced talk comes back as it was.
      wait.remove();
      mine.remove();
      gone.forEach((el) => log.append(el));
      if (!input.value) input.value = text;
      fit();
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
    } finally {
      busy = false;
      send.disabled = false;
      panel.classList.remove("busy");
    }
  }

  /// The question as an editor in its place; sending it asks it instead of the old one.
  function edit(el, m) {
    const parts = el.querySelectorAll(".chat-bubble, .chat-tools");
    parts.forEach((x) => (x.hidden = true));
    el.insertAdjacentHTML("beforeend", `<form class="chat-edit"><textarea rows="3" maxlength="2000" aria-label="Sửa câu hỏi"></textarea>
      <div class="row"><button class="btn small" type="button">Hủy</button><button class="btn small primary" type="submit">Gửi</button></div></form>`);
    const form = el.querySelector(".chat-edit"), area = form.querySelector("textarea");
    const close = () => { form.remove(); parts.forEach((x) => (x.hidden = false)); };
    area.value = m.text;
    area.focus();
    area.setSelectionRange(area.value.length, area.value.length);
    form.querySelector('button[type="button"]').onclick = close;
    area.addEventListener("keydown", (e) => {
      if (e.key === "Escape") { e.stopPropagation(); close(); }
      if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); form.requestSubmit(); }
    });
    form.onsubmit = (e) => {
      e.preventDefault();
      const text = area.value.trim();
      if (!text || text === m.text.trim()) return close();
      ask(text, m.uid);
    };
  }
  log.addEventListener("click", async (e) => {
    const b = e.target.closest("[data-act]"), el = b?.closest(".chat-msg"), m = lines.get(el?.dataset.uid);
    if (!m) return;
    if (b.dataset.act === "copy") return toast((await copyText(m.text)) ? "Đã sao chép" : "Chưa sao chép được, hãy chọn chữ rồi sao chép");
    if (busy) return toast("Đợi trợ lý trả lời xong đã nhé");
    if (b.dataset.act === "edit") return edit(el, m);
    // Asking again is asking the question before this answer once more.
    const question = lines.get(el.previousElementSibling?.dataset.uid);
    if (b.dataset.act === "again" && question?.role === "user") ask(question.text, question.uid);
  });

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
  function tab(name) {
    $("chat-tab-" + name).checked = true;
    $("chat-talk").hidden = name !== "talk";
    history.hidden = name !== "history";
    if (name === "history") listTalks();
    else input.focus({ preventScroll: true });
  }
  document.querySelectorAll('[name="chat-tab"]').forEach((r) => (r.onchange = () => tab(r.value)));
  $("chat-new").onclick = () => {
    if (busy) return;
    thread = fresh();
    empty();
    tab("talk");
  };

  /// The past talks of the course, the one talked in last first, each named by its first question.
  async function listTalks() {
    history.innerHTML = `<div class="skeleton"></div>`;
    let talks;
    try {
      talks = await app.call("chat_threads");
    } catch (e) {
      history.innerHTML = `<p class="error">${esc(e)}</p>`;
      return;
    }
    if (!talks.length) {
      history.innerHTML = `<p class="muted chat-none">Chưa có cuộc trò chuyện nào. Hỏi trợ lý một câu, cuộc trò chuyện sẽ được lưu ở đây.</p>`;
      return;
    }
    history.innerHTML = talks.map((t) => `<div class="chat-talk${t.thread === thread ? " current" : ""}">
        <button class="chat-pick" data-t="${esc(t.thread)}"><b>${esc(t.title || "Cuộc trò chuyện")}</b>
          <span>${when(t.last)}, ${t.turns} câu hỏi${t.thread === thread ? ", đang mở" : ""}</span></button>
        <button class="btn small ghost danger" data-del="${esc(t.thread)}">Xóa</button>
      </div>`).join("");
  }
  history.onclick = async (e) => {
    const open = e.target.closest("[data-t]"), del = e.target.closest("[data-del]");
    if (open && !busy) {
      thread = open.dataset.t;
      tab("talk");
      return load();
    }
    if (!del || !armed(del)) return;
    try {
      await app.call("chat_clear", { thread: del.dataset.del });
      if (del.dataset.del === thread) {
        thread = fresh();
        empty();
      }
      toast("Đã xóa cuộc trò chuyện");
      listTalks();
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
    if (panel.hidden) return void (shown = "");
    load();
    if (!history.hidden) listTalks();
  });
})();
