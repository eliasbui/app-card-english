// The speaking tutor (beta): a talk by voice with an AI teacher. The page listens until the learner stops speaking,
// the app writes down what it heard (Vietnamese or the language of the course), the AI corrects and answers, and its
// answer is read aloud while it is still being written. Then the page listens again, until the learner ends the talk.
(() => {
  "use strict";
  if (window.MINI) return;
  /// How the page tells speech from silence by the microphone's loudness (RMS): a voice starts above `START` (or
  /// three times the room's own noise) for `VOICED` readings, and the turn ends after `PAUSE_MS` below two thirds of
  /// that. START is low enough for a microphone that is not built in, such as a webcam's on a Mac mini. After
  /// `EARLY_MS` of the pause the app starts writing down what was said, so that the text is ready when it ends.
  const START = 0.01, VOICED = 3, PAUSE_MS = 600, EARLY_MS = 250, POLL_MS = 60;
  /// The longest a turn listens, a little under what the app records.
  const LISTEN_MS = 15000;
  /// Topics a talk can start with, sent as the learner's first line.
  const TOPICS = ["Giới thiệu bản thân", "Công việc của tôi", "Sở thích", "Du lịch", "Gọi món ở nhà hàng", "Phỏng vấn xin việc"];

  document.querySelector('nav [data-page="practice"]').insertAdjacentHTML("afterend",
    `<button class="nav" data-page="talk">${st("mode-speak")}Gia sư nói<span class="beta">Beta</span></button>`);
  document.querySelector('nav [data-page="talk"]').addEventListener("click", () => go("talk"));
  document.querySelector("main").insertAdjacentHTML("beforeend", `
    <section id="page-talk">
      <header class="page-head"><div class="grow"><h2>Gia sư nói <span class="beta">Beta</span></h2>
        <p>Gia sư AI nghe bạn nói, sửa lỗi và trả lời bằng giọng đọc.</p></div>
        <button class="btn" id="vt-new">Cuộc mới</button></header>
      <div class="card vt" id="vt" data-phase="idle">
        <div class="vt-head">
          <div class="vt-who">${st("mascot")}<div><b>Gia sư AI</b><span id="vt-course"></span></div></div>
          <div class="vt-pick"><label for="vt-ai">AI</label><select id="vt-ai"></select><span id="vt-model-box"></span></div>
        </div>
        <div class="vt-note" id="vt-note" hidden></div>
        <div class="vt-log" id="vt-log" role="log" aria-live="polite"></div>
        <div class="vt-dock">
          <div class="vt-state">
            <span class="st vt-face" id="vt-face" aria-hidden="true"></span>
            <div class="vt-side">
              <div class="vt-status" id="vt-status" aria-live="polite"></div>
              <div class="vt-foot"><span id="vt-mic-name"></span><a href="#" data-vt="delete" id="vt-delete" hidden>Xóa model nghe giọng</a></div>
            </div>
            <button class="btn small" id="vt-end" hidden>Kết thúc</button>
          </div>
          <form class="vt-type" id="vt-type"><input id="vt-input" maxlength="500" placeholder="Hoặc gõ câu của bạn" aria-label="Gõ câu của bạn"><button class="btn">Gửi</button>
            <button type="button" class="vt-mic" id="vt-mic"></button></form>
        </div>
      </div>
    </section>`);
  const card = $("vt"), log = $("vt-log"), status = $("vt-status"), mic = $("vt-mic"), endButton = $("vt-end"), input = $("vt-input");

  /// The talk: its lines for the AI, the sentence the tutor asked the learner to say, what the page is doing
  /// ("idle", "starting", "listening", "thinking" or "speaking"), the timer of the listening, the latest turn and the
  /// tutor's bubble of each turn still being answered (one cut short goes on filling in while the next one runs).
  const talk = { history: [], target: "", phase: "idle", on: false, poll: 0, turn: 0, bubbles: new Map() };
  const zh = () => courseOf() === "zh";
  /// The model that hears the learner (Whisper small, Vietnamese and the language of the course): { ready, size }.
  let hearing = { ready: false, size: 0 };
  /// The microphone the last recording used, named in the advice when it hears nothing.
  let micName = "";

  /// The round button's word and what it does, the line beside the tutor's face and the face, for each phase.
  const PHASES = {
    idle: ["Nói", "Bắt đầu nói", "Bấm Nói rồi nói tự nhiên, ngừng một chút là gia sư trả lời.", "tutor-teach"],
    starting: ["Chờ", "Đang mở micro", "Đang mở micro...", "tutor-listen"],
    listening: ["Xong", "Gửi câu vừa nói", "Gia sư đang nghe. Ngừng một chút là tự gửi, hoặc bấm Xong.", "tutor-listen"],
    thinking: ["Chờ", "Gia sư đang nghĩ", "Gia sư đang suy nghĩ...", "mascot-think"],
    speaking: ["Ngắt", "Ngắt lời gia sư", "Gia sư đang nói. Bấm Ngắt để nói luôn.", "tutor-speak"],
  };
  function setPhase(phase, note = "", error = false) {
    const [word, action, line, face] = PHASES[phase];
    talk.phase = card.dataset.phase = phase;
    $("vt-face").className = `st vt-face st-${face}`;
    mic.textContent = word;
    mic.setAttribute("aria-label", action);
    mic.disabled = phase === "thinking" || phase === "starting";
    card.style.setProperty("--lv", 0);
    endButton.hidden = !talk.on;
    tell(note || (phase === "idle" && talk.on ? "" : line), error);
  }
  const tell = (text, error = false) => (status.innerHTML = text ? `<span class="${error ? "error" : ""}">${esc(text)}</span>` : "");

  // Which AI answers: the same choice as the study assistant's, and for a provider on the internet, its model.
  async function models() {
    $("vt-ai").innerHTML = window.chatChoices?.() || "";
    $("vt-course").textContent = zh() ? "Khóa tiếng Trung" : "Khóa tiếng Anh";
    const source = window.chatAi?.() || "local:llm", box = $("vt-model-box");
    hearing = await app.call("talk_model").catch(() => hearing);
    const notes = [];
    if (!hearing.ready) notes.push(`<div class="grow"><b>Tải model nghe giọng nói</b>
      <span>Whisper small, ${Math.round(hearing.size / 1e6)} MB, nghe tiếng Việt và tiếng ${zh() ? "Trung" : "Anh"}, chạy trên máy. Trong lúc chờ, bạn vẫn gõ câu được.</span>
      <progress id="vt-dl" max="1" value="0" hidden></progress><span class="error" id="vt-dl-error"></span></div>
      <button class="btn small primary" data-vt="download">Tải ngay</button>`);
    if (source.startsWith("local:")) notes.push(`<div class="grow"><b>Nên chọn ChatGPT ở trên</b>
      <span>Model trên máy trả lời chậm và chưa đủ giỏi để làm gia sư.</span></div>`);
    if (!packsFor(courseOf()).length) notes.push(`<div class="grow"><b>Chưa có giọng đọc tự nhiên</b>
      <span>Gia sư đang đọc bằng giọng của máy.</span></div><button class="btn small" data-vt="settings">Tải trong Cài đặt</button>`);
    const note = $("vt-note");
    note.hidden = !notes.length;
    note.innerHTML = notes.map((n) => `<div class="vt-note-row">${n}</div>`).join("");
    $("vt-delete").hidden = !hearing.ready;
    box.innerHTML = "";
    if (source.startsWith("local:")) return;
    try {
      const list = await app.call("ai_models", { provider: source });
      if (window.chatAi?.() !== source || !list.models.length) return;
      box.innerHTML = `<label for="vt-model">Model</label><select id="vt-model" data-provider="${source}">${list.models
        .map((m) => `<option value="${esc(m.id)}" ${m.id === list.selected ? "selected" : ""}>${esc(m.name)}</option>`).join("")}</select>`;
    } catch (e) {
      box.innerHTML = `<span class="error">${esc(e)}</span>`;
    }
  }
  $("page-talk").addEventListener("change", async (e) => {
    try {
      if (e.target.id === "vt-ai") state.settings = await app.call("set_setting", { key: "tutor_ai", value: e.target.value });
      if (e.target.id === "vt-model") state.settings = await app.call("set_setting", { key: e.target.dataset.provider + "_model", value: e.target.value });
    } catch (err) {
      toast(String(err));
    }
    models();
  });

  function add(html) {
    log.querySelector(".vt-empty")?.remove();
    log.insertAdjacentHTML("beforeend", html);
    log.scrollTop = log.scrollHeight;
    return log.lastElementChild;
  }
  function empty() {
    log.innerHTML = `<div class="vt-empty">${st("tutor-teach")}<b>Sẵn sàng luyện nói?</b>
      <p>Chọn một chủ đề để gia sư mở lời, hoặc bấm Nói và nói bất cứ điều gì.</p>
      <div class="chips">${TOPICS.map((t) => `<button type="button" data-topic="${esc(t)}">${esc(t)}</button>`).join("")}</div></div>`;
  }
  /// The learner's line, with how well the sentence to repeat went.
  function mine(text, check) {
    const how = check ? `<div class="vt-check ${check.score >= 75 ? "ok" : "low"}">Nói theo: ${check.score}/100${check.missed?.length
      ? `, chưa nghe rõ: ${check.missed.map((w) => `<b>${esc(w)}</b>`).join(", ")}` : ""}</div>` : "";
    return `<div class="chat-msg me"><div class="chat-bubble">${esc(text)}</div>${how}</div>`;
  }
  /// The parts of an answer as it streams: SAY aloud, TRY to repeat, FIX notes in Vietnamese.
  function parts(text) {
    const out = { say: [], try: [], fix: [] };
    let part = "say";
    for (const raw of text.split("\n")) {
      const line = raw.trim().replace(/^[*#\- ]+/, "");
      const m = line.match(/^(SAY|TRY|FIX):\**\s*(.*)$/);
      if (m) part = m[1].toLowerCase();
      const rest = m ? m[2] : line;
      if (rest) out[part].push(rest);
    }
    return { say: out.say.join(" "), try: out.try.join(" "), fix: out.fix.join("\n") };
  }
  /// A note of FIX, its `"wrong" -> "right": why` shown as the wrong words struck out and the right ones in green.
  function fixLine(line) {
    const m = line.match(/^"(.+?)"\s*->\s*"(.+?)"\s*:?\s*(.*)$/);
    if (!m) return `<li>${esc(line)}</li>`;
    return `<li><span class="vt-wrong">${esc(m[1])}</span> thành <span class="vt-right">${esc(m[2])}</span>${m[3] ? `<div>${esc(m[3])}</div>` : ""}</li>`;
  }
  /// The inside of a tutor's message: what it said, the sentence to say after it and the corrections; its face talks
  /// while the answer comes.
  function tutorHtml(p, by = "", face = "mascot") {
    const fixes = p.fix ? p.fix.split("\n").filter(Boolean) : [];
    return `${st(face)}<div class="vt-body"><div class="chat-bubble">${esc(p.say)}</div>
      ${p.try ? `<div class="vt-try"><span class="vt-label">Nói theo câu này</span><b>${esc(p.try)}</b>
        <button class="btn small" data-say="${esc(p.try)}">Nghe mẫu</button></div>` : ""}
      ${fixes.length ? `<div class="vt-fix"><span class="vt-label">${st("tutor-teach")}Sửa lỗi</span><ul>${fixes.map(fixLine).join("")}</ul></div>` : ""}
      ${by ? `<div class="chat-by">${by}</div>` : ""}</div>`;
  }

  const thinking = () => add(`<div class="chat-msg vt-tutor">${st("mascot-think")}<div class="vt-body"><div class="chat-bubble vt-dots" aria-label="Gia sư đang nghĩ"><i></i><i></i><i></i></div></div></div>`);
  hooks.event.push((name, data) => {
    if (name !== "talk" || !talk.bubbles.has(data.turn)) return;
    // What was heard, or the line typed: the learner's bubble and the tutor's, waiting for its answer.
    if (data.kind === "heard") {
      add(mine(data.text, data.check));
      talk.bubbles.set(data.turn, thinking());
    }
    const bubble = talk.bubbles.get(data.turn);
    if (data.kind === "text" && bubble) {
      bubble.innerHTML = tutorHtml(parts(data.text), "", "tutor-speak");
      log.scrollTop = log.scrollHeight;
      if (data.turn === talk.turn && talk.phase === "thinking") setPhase("speaking");
    }
  });

  // Listening: the microphone's loudness decides when the learner has finished.
  async function listen() {
    if (!talk.on) return;
    setPhase("starting");
    try {
      micName = (await app.call("rec_start")) || "";
      $("vt-mic-name").textContent = micName ? `Micro: ${micName}` : "";
    } catch (e) {
      talk.on = false;
      setPhase("idle", String(e), true);
      return;
    }
    // The talk ended while the microphone opened.
    if (!talk.on || talk.phase !== "starting") return app.call("rec_cancel").catch(() => {});
    setPhase("listening");
    const started = Date.now();
    let floor = 1, peak = 0, voiced = 0, quietSince = 0, early = false, asking = false;
    talk.poll = setInterval(async () => {
      if (asking) return;
      asking = true;
      const level = await app.call("rec_level").catch(() => 0);
      asking = false;
      if (talk.phase !== "listening") return;
      // Speech is around 0.02 to 0.2 RMS; the curve lifts quiet voices so that they still show.
      card.style.setProperty("--lv", Math.min(1, Math.pow(level / 0.15, 0.6)).toFixed(2));
      const elapsed = Date.now() - started;
      // The room's own noise, learned in the first moments and whenever it is quieter.
      floor = Math.min(floor, Math.max(level, 0.002));
      peak = Math.max(peak, level);
      const loud = Math.max(START, floor * 3);
      if (level > loud) { voiced++; quietSince = 0; early = false; }
      else if (voiced >= VOICED && level < loud * 0.66 && !quietSince) quietSince = Date.now();
      if (quietSince && !early && Date.now() - quietSince > EARLY_MS) {
        early = true;
        app.call("talk_hear_early").catch(() => {});
      }
      if ((voiced >= VOICED && quietSince && Date.now() - quietSince > PAUSE_MS) || elapsed > LISTEN_MS) {
        if (voiced >= VOICED) return send();
        setPhase("starting");
        stopListening(true).then(async () => {
          if (peak > 0) {
            await listen();
            if (talk.phase === "listening") tell("Chưa nghe thấy giọng bạn. Hãy nói to hơn và gần micro hơn.");
            return;
          }
          // Not a sound at all, not even the room: the system keeps the microphone from the app, or it is not a real one.
          talk.on = false;
          setPhase("idle", `Micro${micName ? ` "${micName}"` : ""} không gửi âm thanh nào. Hãy chọn đúng micro trong phần âm thanh của máy, cho phép Card English dùng micro trong phần quyền riêng tư, rồi thử lại.`, true);
        });
      }
    }, POLL_MS);
  }
  async function stopListening(discard) {
    clearInterval(talk.poll);
    if (discard) await app.call("rec_cancel").catch(() => {});
  }

  /// A turn: what was said (or `typed`), then the tutor's answer, read aloud; then the page listens again. The
  /// learner's bubble comes with the "heard" event, for a typed line too.
  async function send(typed = "") {
    const turn = ++talk.turn;
    const listening = talk.phase === "listening";
    // At once, so that the listening's own timer cannot send the same turn again.
    setPhase("thinking", typed ? "" : "Đang nghe lại câu bạn vừa nói...");
    talk.bubbles.set(turn, null);
    if (listening) await stopListening(!!typed);
    try {
      const r = await app.call("talk_turn", { history: talk.history, target: talk.target, text: typed, day: localDay(), turn });
      // The talk was started over meanwhile.
      if (!talk.bubbles.has(turn)) return;
      talk.history.push({ role: "user", text: r.heard }, { role: "ai", text: r.say });
      talk.target = r.try || "";
      const bubble = talk.bubbles.get(turn);
      if (bubble) bubble.innerHTML = tutorHtml({ say: r.say, try: r.try, fix: r.fix }, byHtml(r.by));
      if (r.spoken && turn === talk.turn) toast(r.spoken);
      gainXp(2, mic);
    } catch (e) {
      const heard = talk.bubbles.get(turn);
      if (turn !== talk.turn) return heard?.remove();
      // Nothing made out of the sound: say so and listen again. Any other failure ends the talk.
      if (!heard && !typed && talk.on) {
        await listen();
        if (talk.phase === "listening") tell(String(e));
        return;
      }
      heard?.remove();
      add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
      talk.on = false;
      setPhase("idle");
      return;
    } finally {
      talk.bubbles.delete(turn);
    }
    // A turn cut short has already handed over to the next one.
    if (turn !== talk.turn) return;
    setPhase("idle");
    listen();
  }

  async function end() {
    talk.on = false;
    talk.turn++;
    if (talk.phase === "listening") await stopListening(true);
    app.call("talk_stop").catch(() => {});
    setPhase("idle");
  }

  mic.addEventListener("click", async () => {
    if (talk.phase === "listening") return send();
    if (talk.phase === "speaking") {
      // Cut the tutor short and listen at once; its answer still goes on screen.
      talk.turn++;
      await app.call("talk_stop").catch(() => {});
      setPhase("idle");
      return listen();
    }
    if (talk.phase !== "idle") return;
    if (!hearing.ready) return tell("Hãy tải model nghe giọng nói ở trên trước, hoặc gõ câu bên dưới.", true);
    talk.on = true;
    listen();
  });
  endButton.addEventListener("click", end);
  /// A new talk: the one under way ends, and an answer still coming is dropped.
  function reset() {
    end();
    talk.bubbles.clear();
    talk.history = [];
    talk.target = "";
    empty();
  }
  $("vt-new").addEventListener("click", reset);
  $("vt-type").addEventListener("submit", (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text || !["idle", "listening"].includes(talk.phase)) return;
    input.value = "";
    send(text);
  });
  $("page-talk").addEventListener("click", async (e) => {
    const topic = e.target.closest("[data-topic]");
    if (topic) {
      if (talk.phase !== "idle") return;
      // The tutor opens the topic, then listens if it can hear.
      talk.on = hearing.ready;
      return send(`Mình muốn luyện nói về chủ đề: ${topic.dataset.topic}`);
    }
    const b = e.target.closest("[data-vt]");
    if (!b) return;
    e.preventDefault();
    if (b.dataset.vt === "settings") return go("settings");
    if (b.dataset.vt === "delete") {
      if (talk.on) await end();
      hearing = await app.call("talk_delete").catch((err) => (toast(String(err)), hearing));
      return models();
    }
    b.disabled = true;
    b.classList.add("loading");
    $("vt-dl-error").textContent = "";
    try {
      hearing = await app.call("talk_download");
      models();
    } catch (err) {
      $("vt-dl-error").textContent = String(err);
      b.disabled = false;
      b.classList.remove("loading");
    }
  });
  hooks.event.push((name, data) => {
    const bar = $("vt-dl");
    if (name !== "progress" || data.group !== "talk" || !bar) return;
    bar.hidden = false;
    bar.value = data.done / data.total;
  });

  // The drawing of a line, for the chat bubble's voice mode (chat-voice.js).
  window.talkKit = { parts, tutorHtml, mine };
  setPhase("idle");
  hooks.page.push((page) => {
    if (page === "talk") { models(); if (!log.children.length) empty(); }
    else if (talk.on || talk.phase !== "idle") end();
  });
  hooks.course.push(reset);
})();
