// The speaking tutor (beta): a talk by voice with an AI teacher. The page listens until the learner stops speaking,
// the app writes down what it heard (Vietnamese or the language of the course), the AI corrects and answers, and its
// answer is read aloud while it is still being written. Then the page listens again, until the learner ends the talk.
(() => {
  "use strict";
  if (window.MINI) return;
  /// How the page tells speech from silence by the microphone's loudness (RMS): a voice starts above `START` (or
  /// three times the room's own noise), and the turn ends after `PAUSE_MS` below two thirds of that.
  const START = 0.02, PAUSE_MS = 750, POLL_MS = 60;
  /// The longest a turn listens, a little under what the app records.
  const LISTEN_MS = 15000;

  document.querySelector('nav [data-page="practice"]').insertAdjacentHTML("afterend",
    `<button class="nav" data-page="talk">${st("mode-speak")}Gia sư nói<span class="beta">Beta</span></button>`);
  document.querySelector('nav [data-page="talk"]').addEventListener("click", () => go("talk"));
  document.querySelector("main").insertAdjacentHTML("beforeend", `
    <section id="page-talk">
      <header class="page-head"><div class="grow"><h2>Gia sư nói <span class="beta">Beta</span></h2>
        <p>Nói chuyện 1 kèm 1 với gia sư AI. Bạn cứ nói, hỏi bằng tiếng Việt cũng được; gia sư sửa phát âm, ngữ pháp và trả lời ngay bằng giọng đọc.</p></div></header>
      <div class="card vt">
        <div class="vt-models row">
          <label for="vt-ai">AI</label><select id="vt-ai"></select>
          <span id="vt-model-box"></span>
          <span class="grow"></span><a href="#" class="muted" data-vt="delete" id="vt-delete" hidden>Xóa model nghe giọng</a>
        </div>
        <p class="vt-note" id="vt-note" hidden></p>
        <div class="vt-log" id="vt-log" role="log" aria-live="polite"></div>
        <div class="vt-state" id="vt-state" aria-live="polite"></div>
        <div class="vt-controls">
          <button class="btn primary" id="vt-main">Bắt đầu nói</button>
          <button class="btn" id="vt-end" hidden>Kết thúc</button>
          <form class="vt-type" id="vt-type"><input id="vt-input" maxlength="500" placeholder="Hoặc gõ câu của bạn" aria-label="Gõ câu của bạn"><button class="btn">Gửi</button></form>
        </div>
      </div>
    </section>`);
  const log = $("vt-log"), stateBox = $("vt-state"), main = $("vt-main"), endButton = $("vt-end"), input = $("vt-input");

  /// The talk: its lines for the AI, the sentence the tutor asked the learner to say, what the page is doing
  /// ("idle", "listening", "thinking" or "speaking"), the timers of the listening, the latest turn and the tutor's
  /// bubble of each turn still being answered (one cut short goes on filling in while the next one runs).
  const talk = { history: [], target: "", phase: "idle", on: false, poll: 0, wave: 0, turn: 0, bubbles: new Map() };
  const zh = () => courseOf() === "zh";
  /// The model that hears the learner (Whisper small, Vietnamese and the language of the course): { ready, size }.
  let hearing = { ready: false, size: 0 };

  function setPhase(phase, text = "") {
    talk.phase = phase;
    main.textContent = { idle: talk.on ? "Nói tiếp" : "Bắt đầu nói", listening: "Xong, gửi đi", thinking: "Đang nghĩ...", speaking: "Ngắt lời" }[phase];
    main.disabled = phase === "thinking";
    endButton.hidden = !talk.on;
    if (phase !== "listening") {
      clearInterval(talk.wave);
      stateBox.innerHTML = text ? `<span class="muted">${esc(text)}</span>` : "";
    }
  }

  // Which AI answers: the same choice as the study assistant's, and for a provider on the internet, its model.
  async function models() {
    $("vt-ai").innerHTML = window.chatChoices?.() || "";
    const source = window.chatAi?.() || "local:llm", box = $("vt-model-box");
    hearing = await app.call("talk_model").catch(() => hearing);
    const note = $("vt-note");
    const notes = [];
    if (!hearing.ready) notes.push(`Cần tải model nghe giọng nói tiếng Việt và tiếng ${zh() ? "Trung" : "Anh"} (Whisper small, ${Math.round(hearing.size / 1e6)} MB, chạy trên máy). <button class="btn small" data-vt="download">Tải ngay</button><span id="vt-dl"></span> Trong lúc đó bạn có thể gõ câu.`);
    if (source.startsWith("local:")) notes.push("Model trên máy trả lời chậm và chưa đủ giỏi để làm gia sư. Chọn ChatGPT ở trên để nhanh nhất.");
    if (!state.voices?.[courseOf()]?.ready) notes.push(`Chưa tải giọng Kokoro nên gia sư đọc bằng giọng của máy. <a href="#" data-vt="settings">Tải trong Cài đặt</a> để giọng tự nhiên hơn.`);
    note.hidden = !notes.length;
    $("vt-delete").hidden = !hearing.ready;
    note.innerHTML = notes.map((n) => `<span>${n}</span>`).join("");
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
    log.innerHTML = `<div class="vt-empty">${st(zh() ? "mascot-zh" : "mascot-cheer")}<b>Sẵn sàng luyện nói?</b>
      <p>Bấm Bắt đầu nói rồi cứ nói tự nhiên, ngừng một chút là gia sư hiểu bạn đã nói xong. Gia sư nghe được cả tiếng Việt, nên muốn hỏi "câu này nói thế nào" thì cứ hỏi.</p></div>`;
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
  function tutorHtml(p, by = "") {
    return `<div class="chat-bubble"><div class="vt-say">${esc(p.say)}</div>
      ${p.try ? `<div class="vt-try"><span class="prompt-label">Nói theo</span><b>${esc(p.try)}</b><button class="btn small" data-say="${esc(p.try)}">Nghe</button></div>` : ""}
      ${p.fix ? `<div class="vt-fix">${p.fix.split("\n").map((l) => `<div>${esc(l)}</div>`).join("")}</div>` : ""}</div>
      ${by ? `<div class="chat-by">${by}</div>` : ""}`;
  }

  const thinking = () => add(`<div class="chat-msg"><div class="thinking">${st("mascot-think")}<span>Gia sư đang nghĩ...</span></div></div>`);
  hooks.event.push((name, data) => {
    if (name !== "talk" || !talk.bubbles.has(data.turn)) return;
    if (data.kind === "heard") {
      add(mine(data.text, data.check));
      talk.bubbles.set(data.turn, thinking());
    }
    const bubble = talk.bubbles.get(data.turn);
    if (data.kind === "text" && bubble) {
      bubble.innerHTML = tutorHtml(parts(data.text));
      log.scrollTop = log.scrollHeight;
      if (data.turn === talk.turn && talk.phase === "thinking") setPhase("speaking", "Gia sư đang nói. Bấm Ngắt lời để nói luôn.");
    }
  });

  // Listening: the microphone's loudness decides when the learner has finished.
  async function listen() {
    if (!talk.on) return;
    try {
      await app.call("rec_start");
    } catch (e) {
      setPhase("idle");
      stateBox.innerHTML = `<span class="error">${esc(e)}</span>`;
      return;
    }
    setPhase("listening");
    talk.wave = liveWave(stateBox);
    const started = Date.now();
    let floor = 1, peak = 0, voiced = 0, quietSince = 0, asking = false;
    talk.poll = setInterval(async () => {
      if (asking) return;
      asking = true;
      const level = await app.call("rec_level").catch(() => 0);
      asking = false;
      if (talk.phase !== "listening") return;
      const elapsed = Date.now() - started;
      // The room's own noise, learned in the first moments and whenever it is quieter.
      floor = Math.min(floor, Math.max(level, 0.002));
      peak = Math.max(peak, level);
      const loud = Math.max(START, floor * 3);
      if (level > loud) { voiced++; quietSince = 0; }
      else if (voiced >= 2 && level < loud * 0.66 && !quietSince) quietSince = Date.now();
      if ((voiced >= 2 && quietSince && Date.now() - quietSince > PAUSE_MS) || elapsed > LISTEN_MS) {
        if (voiced >= 2) return send();
        stopListening(true).then(() => {
          if (peak > 0) return listen();
          // Not a sound at all, not even the room: the system keeps the microphone from the app.
          talk.on = false;
          setPhase("idle");
          stateBox.innerHTML = `<span class="error">Micro không gửi âm thanh nào. Hãy cho phép Card English dùng micro trong phần quyền riêng tư của máy rồi thử lại.</span>`;
        });
      }
    }, POLL_MS);
  }
  async function stopListening(discard) {
    clearInterval(talk.poll);
    clearInterval(talk.wave);
    if (discard) await app.call("rec_cancel").catch(() => {});
  }

  /// A turn: what was said (or `typed`), then the tutor's answer, read aloud; then the page listens again.
  async function send(typed = "") {
    const turn = ++talk.turn;
    if (talk.phase === "listening") await stopListening(!!typed);
    setPhase("thinking", typed ? "" : "Đang nghe lại câu bạn vừa nói...");
    talk.bubbles.set(turn, null);
    if (typed) {
      add(mine(typed));
      talk.bubbles.set(turn, thinking());
    }
    try {
      const r = await app.call("talk_turn", { history: talk.history, target: talk.target, text: typed, day: localDay(), turn });
      talk.history.push({ role: "user", text: r.heard }, { role: "ai", text: r.say });
      talk.target = r.try || "";
      const bubble = talk.bubbles.get(turn);
      if (bubble) bubble.innerHTML = tutorHtml({ say: r.say, try: r.try, fix: r.fix }, byHtml(r.by));
      if (r.spoken && turn === talk.turn) toast(r.spoken);
      gainXp(2, main);
    } catch (e) {
      const heard = talk.bubbles.get(turn);
      if (turn !== talk.turn) return heard?.remove();
      // Nothing made out of the sound: say so and listen again. Any other failure ends the talk.
      if (!heard && !typed && talk.on) {
        setPhase("idle");
        await listen();
        stateBox.insertAdjacentHTML("afterbegin", `<div class="muted">${esc(e)}</div>`);
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

  main.addEventListener("click", async () => {
    if (talk.phase === "listening") return send();
    if (talk.phase === "speaking") {
      // Cut the tutor short and listen at once; its answer still goes on screen.
      talk.turn++;
      await app.call("talk_stop").catch(() => {});
      setPhase("idle");
      return listen();
    }
    if (talk.phase !== "idle") return;
    if (!hearing.ready) return toast("Hãy tải model nghe giọng nói trước, hoặc gõ câu bên cạnh");
    talk.on = true;
    listen();
  });
  endButton.addEventListener("click", end);
  $("vt-type").addEventListener("submit", (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text || talk.phase === "thinking" || talk.phase === "speaking") return;
    input.value = "";
    send(text);
  });
  $("page-talk").addEventListener("click", async (e) => {
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
    try {
      hearing = await app.call("talk_download");
      models();
    } catch (err) {
      $("vt-dl").innerHTML = ` <span class="error">${esc(err)}</span>`;
      b.disabled = false;
    }
  });
  hooks.event.push((name, data) => {
    if (name === "progress" && data.group === "talk" && $("vt-dl")) $("vt-dl").textContent = ` ${Math.round((100 * data.done) / data.total)}%`;
  });

  hooks.page.push((page) => {
    if (page === "talk") { models(); if (!log.children.length) empty(); }
    else if (talk.on) end();
  });
  hooks.course.push(() => { if (talk.on) end(); talk.history = []; talk.target = ""; empty(); });
})();
