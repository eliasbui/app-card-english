// Role-play: the AI plays the other person of a scene in the language of the course, the learner answers by voice or
// by typing, and at the end the AI says what went well and what to fix. Each talk is kept as a thread of its own.
(() => {
  "use strict";
  if (window.MINI) return;
  /// The scenes the app knows (src/ai.rs has whom the AI plays in each), with their names in Chinese for that course.
  const SCENES = [
    ["interview", "Phỏng vấn xin việc", "面试", "AI là nhà tuyển dụng, bạn là ứng viên."],
    ["airport", "Ở sân bay", "在机场", "AI là nhân viên quầy làm thủ tục, bạn là hành khách."],
    ["restaurant", "Gọi món ở nhà hàng", "在餐厅点菜", "AI là người phục vụ, bạn là khách."],
    ["hotel", "Nhận phòng khách sạn", "酒店入住", "AI là lễ tân, bạn là khách."],
    ["directions", "Hỏi đường", "问路", "AI là người dân ở đó, bạn là du khách bị lạc."],
    ["shopping", "Đi mua sắm", "买衣服", "AI là nhân viên cửa hàng quần áo, bạn là khách."],
    ["meeting", "Họp với đồng nghiệp", "开会", "AI là đồng nghiệp chủ trì cuộc họp, bạn là thành viên nhóm."],
  ];
  const sceneOf = (key) => SCENES.find((s) => s[0] === key) || SCENES[0];
  /// The longest a recording runs before it stops by itself, a little under what the app keeps.
  const REC_MS = 15000;

  const anchor = $("practice") || $("page-practice").lastElementChild;
  anchor.insertAdjacentHTML("afterend", `<div id="roleplay" class="hidden"></div>`);
  const root = $("roleplay");
  /// The talk on screen: its thread (empty until the first line), its scene, the learner's lines so far, whether the
  /// AI is being asked, and the recording under way.
  let rp = null;

  const zh = () => courseOf() === "zh";
  const language = () => (zh() ? "tiếng Trung" : "tiếng Anh");
  const speechReady = () => hears();
  const weakModel = () => window.chatAi?.() === "local:llm";

  function show() {
    $("practice-menu").classList.add("hidden");
    root.classList.remove("hidden");
    $("page-practice").classList.add("practicing");
  }
  function close() {
    if (rp?.rec) stopRec(true);
    rp = null;
    root.classList.add("hidden");
    root.innerHTML = "";
    $("practice-menu").classList.remove("hidden");
    $("page-practice").classList.remove("practicing");
  }

  // Picking a scene, and the talks from before.
  async function pick() {
    if (rp?.rec) stopRec(true);
    rp = null;
    show();
    root.innerHTML = `<div class="card">
      <div class="row rp-top">${st("mascot-cheer")}<div class="grow"><div class="prompt-label">Hội thoại nhập vai</div><h3>Chọn một tình huống</h3>
        <p class="muted">AI đóng vai người đối diện, nói ${language()} vừa sức bạn và lồng vào các từ bạn đã học. Bạn trả lời bằng giọng nói hoặc gõ phím, xong bấm Kết thúc để nhận nhận xét.</p></div>
        <button class="btn" data-rp="back">Quay lại</button></div>
      ${weakModel() ? `<p class="rp-note">Bạn đang dùng Qwen 0.5B trên máy, model này còn yếu cho hội thoại. Qwen 1.5B hoặc một nguồn AI trên mạng sẽ cho cuộc trò chuyện tốt hơn nhiều. Bạn vẫn có thể thử. <button class="btn small" data-rp="settings">Mở Cài đặt</button></p>` : ""}
      <div class="tiles rp-scenes">${SCENES.map(([key, name, han, who]) => `
        <button class="tile" data-scene="${key}"><b>${name}</b>${zh() ? `<span class="rp-zh">${han}</span>` : ""}<span>${who}</span></button>`).join("")}</div>
      <div id="rp-past"></div>
    </div>`;
    try {
      const past = await app.call("roleplay_sessions");
      if (rp || !past.length || !$("rp-past")) return;
      $("rp-past").innerHTML = `<h3 class="group">Các lần trước</h3><div class="rp-past">${past.map((t) => `
        <button data-thread="${esc(t.thread)}"><b>${esc(sceneOf(t.thread.split(":")[1])[1])}</b>
          <span>${shortDate(t.started)}, ${t.turns} câu của bạn${t.ended ? ", đã có nhận xét" : ""}</span><em>Mở lại</em></button>`).join("")}</div>`;
    } catch (e) {
      // The list is a convenience; the scenes work without it.
    }
  }

  // The talk.
  function talk(key) {
    const [, name, han] = sceneOf(key);
    show();
    root.innerHTML = `
      <div class="study-bar rp-bar"><b class="grow">${name}${zh() ? ` <span class="muted">${han}</span>` : ""}</b>
        <button class="btn small" data-rp="end">Kết thúc</button><button class="btn small" data-rp="pick">Thoát</button></div>
      <div class="card rp-talk">
        <div class="rp-log" id="rp-log" role="log" aria-live="polite"></div>
        <div class="rp-rec" id="rp-rec"></div>
        <div class="rp-compose">
          <textarea id="rp-input" rows="1" maxlength="500" placeholder="Trả lời bằng ${language()}" aria-label="Câu trả lời của bạn"></textarea>
          <button class="btn" data-rp="mic" id="rp-mic" ${speechReady() ? "" : "disabled"}>Nói</button>
          <button class="btn primary" data-rp="send" id="rp-send">Gửi</button>
        </div>
        <p class="muted rp-hint" id="rp-hint">${speechReady() ? ""
          : `Chưa tải model nghe giọng nói ${language()}, vào Cài đặt, mục Model trên máy để tải. Trong lúc đó bạn cứ gõ câu trả lời.`}</p>
      </div>`;
    const input = $("rp-input");
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); say(); }
    });
    input.addEventListener("input", () => { input.style.height = "auto"; input.style.height = Math.min(120, input.scrollHeight + 2) + "px"; });
  }

  function line(m, by = "") {
    if (m.role === "feedback") return notes(m, by);
    if (m.role === "user") return `<div class="chat-msg me"><div class="chat-bubble">${esc(m.text)}</div></div>`;
    return `<div class="chat-msg">
      <div class="chat-bubble"><div>${esc(m.text)}</div>${m.reading ? `<div class="rp-reading">${esc(m.reading)}</div>` : ""}${m.vi ? `<div class="rp-vi" hidden>${esc(m.vi)}</div>` : ""}</div>
      <div class="row rp-acts"><button class="btn ghost" data-say="${esc(m.text)}">Nghe</button>${m.vi ? `<button class="btn ghost" data-rp="vi" aria-pressed="false">Dịch</button>` : ""}</div>
      ${by ? `<div class="chat-by">${by}</div>` : ""}</div>`;
  }
  /// The notes that end a talk, kept as JSON: what went well, the mistakes as "wrong -> right (why)", the learner's
  /// words that came up and two suggestions.
  function notes(m, by = "") {
    let n = {};
    try { n = JSON.parse(m.text); } catch (e) { n = { good: m.text }; }
    const mistake = (text) => {
      const at = text.search(/->|→|=>/);
      if (at < 0) return `<li>${esc(text)}</li>`;
      const [, right, why] = text.slice(at).replace(/^(->|→|=>)\s*/, "").match(/^(.*?)\s*(\(.*\))?$/) || [];
      return `<li><del>${esc(text.slice(0, at).trim())}</del> <ins>${esc(right)}</ins> ${why ? `<span class="muted">${esc(why)}</span>` : ""}</li>`;
    };
    const mistakes = n.mistakes || [], used = n.used || [], tips = n.tips || [];
    return `<div class="rp-feedback">
      <div class="prompt-label">Nhận xét của AI</div>
      ${n.good ? `<h4>Bạn đã làm tốt</h4><p>${esc(n.good)}</p>` : ""}
      <h4>Lỗi cần sửa</h4>${mistakes.length ? `<ul>${mistakes.map(mistake).join("")}</ul>` : `<p>Không thấy lỗi nào, giỏi quá!</p>`}
      <h4>Từ trong danh sách bạn đã dùng</h4>${used.length ? `<div class="review">${used.map((w) => `<span class="tag ok">${esc(w)}</span>`).join("")}</div>`
        : `<p class="muted">Chưa dùng từ nào trong danh sách. Lần sau thử dùng vài từ bạn đã học nhé.</p>`}
      ${tips.length ? `<h4>Gợi ý</h4><ol>${tips.map((t) => `<li>${esc(t)}</li>`).join("")}</ol>` : ""}
      <div class="chat-by">${by || (m.source ? `Trả lời bởi ${esc(m.source)}.` : "")}</div>
      <div class="row"><button class="btn primary" data-rp="pick">Hội thoại mới</button><button class="btn" data-rp="back">Xong</button></div>
    </div>`;
  }
  function add(html) {
    const log = $("rp-log");
    log.insertAdjacentHTML("beforeend", html);
    log.scrollTop = log.scrollHeight;
    return log.lastElementChild;
  }
  const waiting = (text) => add(`<div class="chat-msg"><div class="thinking">${st("mascot-think")}<span>${text}</span></div></div>`);
  function busy(on) {
    if (!rp) return;
    rp.busy = on;
    for (const id of ["rp-send", "rp-mic"]) if ($(id)) $(id).disabled = on || (id === "rp-mic" && !speechReady());
    const end = root.querySelector('[data-rp="end"]');
    if (end) end.disabled = on;
  }

  /// A new talk of a scene: the AI says the first line.
  async function start(key) {
    rp = { thread: "", key, turns: 0, busy: false };
    const mine = rp;
    talk(key);
    busy(true);
    const wait = waiting("AI đang vào vai...");
    try {
      const r = await app.call("roleplay", { scenario: key, day: localDay() });
      if (rp !== mine) return;
      wait.remove();
      rp.thread = r.reply.thread;
      add(line(r.reply, byHtml(r.by)));
      speak(r.reply.text);
    } catch (e) {
      if (rp !== mine) return;
      wait.remove();
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>
        <div class="row"><button class="btn" data-rp="retry">Thử lại</button><button class="btn" data-rp="pick">Chọn tình huống khác</button></div>`);
    }
    busy(false);
  }

  /// A talk from before, to read again and go on with.
  async function reopen(thread) {
    const key = thread.split(":")[1];
    rp = { thread, key, turns: 0, busy: false };
    const mine = rp;
    talk(key);
    try {
      const lines = await app.call("chat_history", { thread });
      if (rp !== mine) return;
      rp.turns = lines.filter((m) => m.role === "user").length;
      $("rp-log").innerHTML = lines.map((m) => line(m)).join("");
      $("rp-log").scrollTop = $("rp-log").scrollHeight;
    } catch (e) {
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
    }
  }

  async function say() {
    const input = $("rp-input");
    const text = input.value.trim();
    if (!rp || rp.busy || !text) return input?.focus();
    if (!rp.thread) return;
    if (rp.rec) await stopRec(true);
    const mine = rp;
    const own = add(line({ role: "user", text }));
    input.value = "";
    input.style.height = "";
    if (speechReady()) $("rp-hint").textContent = "";
    busy(true);
    const wait = waiting("Đang nghe bạn nói...");
    try {
      const r = await app.call("roleplay", { thread: rp.thread, text, day: localDay() });
      if (rp !== mine) return;
      wait.remove();
      rp.turns++;
      add(line(r.reply, byHtml(r.by)));
      speak(r.reply.text);
    } catch (e) {
      if (rp !== mine) return;
      wait.remove();
      own.remove();
      input.value = text;
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
    }
    busy(false);
  }

  async function end(button) {
    if (!rp || rp.busy || !rp.thread) return;
    if (!rp.turns) return toast("Hãy trả lời ít nhất một câu rồi mới kết thúc");
    if (rp.rec) await stopRec(true);
    const mine = rp;
    busy(true);
    const wait = waiting("AI đang xem lại cuộc trò chuyện...");
    try {
      const r = await app.call("roleplay_end", { thread: rp.thread, day: localDay() });
      if (rp !== mine) return;
      wait.remove();
      add(line(r.reply, byHtml(r.by)));
      // A little for every line said, as the other exercises give for each answer.
      gainXp(Math.min(20, 2 * rp.turns), button);
    } catch (e) {
      if (rp !== mine) return;
      wait.remove();
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
    }
    busy(false);
  }

  // Speaking the answer: the microphone's live wave while it records, then Whisper writes it into the box to check.
  async function rec() {
    if (!rp || rp.busy) return;
    if (rp.rec) return stopRec(false);
    const button = $("rp-mic");
    try {
      await app.call("rec_start");
    } catch (e) {
      $("rp-rec").innerHTML = `<span class="error">${esc(e)}</span>`;
      return;
    }
    rp.rec = { poll: liveWave($("rp-rec")), timer: setTimeout(() => stopRec(false), REC_MS) };
    button.textContent = "Dừng";
    button.classList.add("recording");
  }
  async function stopRec(discard) {
    const take = rp?.rec;
    if (!take) return;
    rp.rec = null;
    clearInterval(take.poll);
    clearTimeout(take.timer);
    const button = $("rp-mic"), box = $("rp-rec"), mine = rp;
    if (button) { button.classList.remove("recording"); button.textContent = "Đang nghe..."; button.disabled = true; }
    try {
      const text = await app.call("transcribe");
      if (discard || rp !== mine) return;
      const input = $("rp-input");
      input.value = (input.value.trim() ? input.value.trim() + " " : "") + text;
      input.focus();
      box.innerHTML = "";
      $("rp-hint").textContent = "Xem lại câu máy nghe được, sửa nếu cần rồi bấm Gửi.";
    } catch (e) {
      if (!discard && rp === mine) box.innerHTML = `<span class="error">${esc(e)}</span>`;
    } finally {
      if (button && rp === mine) { button.textContent = "Nói"; button.disabled = !!rp?.busy; }
    }
  }

  $("mode-roleplay")?.addEventListener("click", pick);
  root.addEventListener("click", (e) => {
    const scene = e.target.closest("[data-scene]");
    if (scene) return start(scene.dataset.scene);
    const past = e.target.closest("[data-thread]");
    if (past) return reopen(past.dataset.thread);
    const b = e.target.closest("[data-rp]");
    if (!b) return;
    const act = b.dataset.rp;
    if (act === "back") close();
    if (act === "pick") pick();
    if (act === "settings") go("settings");
    if (act === "send") say();
    if (act === "mic") rec();
    if (act === "end") end(b);
    if (act === "retry" && rp) start(rp.key);
    if (act === "vi") {
      const vi = b.closest(".chat-msg").querySelector(".rp-vi");
      vi.hidden = !vi.hidden;
      b.setAttribute("aria-pressed", String(!vi.hidden));
      b.textContent = vi.hidden ? "Dịch" : "Ẩn bản dịch";
    }
  });
  // The talk belongs to the course it began in.
  hooks.course.push(() => { if (!root.classList.contains("hidden")) close(); });
})();
