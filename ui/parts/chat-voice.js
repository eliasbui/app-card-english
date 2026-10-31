// Voice in the chat bubble: a mic button beside the text box starts a spoken mode. What the learner says is written down
// (the speaking tutor's hearing), sent through the same routing as a typed message, and then either carried out (a `do`
// card: safe calls at once, the others after the learner says yes) or answered as a talk with the speaking tutor.
(() => {
  "use strict";
  if (window.MINI) return;
  const host = window.chatVoiceHost;
  if (!host) return;
  /// Loudness thresholds and timings of the listening, as the speaking tutor's page (voice-tutor.js) uses them.
  const START = 0.01, VOICED = 3, PAUSE_MS = 600, EARLY_MS = 250, POLL_MS = 60, LISTEN_MS = 15000;
  /// The turns of this mode count from here, so that their events never match the speaking tutor page's turns.
  const BASE = 1e6;
  const MIC_LABEL = "Nói chuyện với trợ lý";

  host.form.insertAdjacentHTML("beforebegin", `
    <div class="cv-bar" id="cv-bar" data-phase="idle" hidden>
      <div class="cv-note" id="cv-note" hidden></div>
      <div class="cv-row">
        <span class="cv-meter" aria-hidden="true"></span>
        <div class="cv-status" id="cv-status" aria-live="polite"></div>
        <button type="button" class="btn small" id="cv-main"></button>
        <button type="button" class="btn small ghost" id="cv-end">Kết thúc</button>
      </div>
    </div>`);
  const micButton = Object.assign(document.createElement("button"), { type: "button", id: "cv-mic", className: "btn cv-mic", textContent: "Nói" });
  micButton.setAttribute("aria-label", MIC_LABEL);
  micButton.title = MIC_LABEL;
  micButton.setAttribute("aria-pressed", "false");
  host.form.insertBefore(micButton, $("chat-send"));
  const bar = $("cv-bar"), status = $("cv-status"), main = $("cv-main"), endButton = $("cv-end"), noteBox = $("cv-note");

  /// The talk: its lines for the tutor AI, the sentence it asked to repeat, the phase ("idle", "starting", "listening",
  /// "thinking" or "speaking"), the timer, the latest turn, the tutor bubbles still waiting for their answer and the
  /// calls waiting for a yes.
  const v = { on: false, phase: "idle", poll: 0, turn: BASE, history: [], target: "", bubbles: new Map(), pending: null, retries: 0 };
  let hearing = { ready: false, size: 0 };
  let micName = "";
  const zh = () => courseOf() === "zh";

  const PHASES = {
    idle: ["Nói", "Bắt đầu nói", "Bấm Nói rồi nói tự nhiên, ngừng một chút là trợ lý xử lý."],
    starting: ["Chờ", "Đang mở micro", "Đang mở micro..."],
    listening: ["Xong", "Gửi câu vừa nói", "Đang nghe. Ngừng một chút là tự gửi, hoặc bấm Xong."],
    thinking: ["Chờ", "Trợ lý đang nghĩ", "Đang xử lý..."],
    speaking: ["Ngắt", "Ngắt lời trợ lý", "Trợ lý đang nói. Bấm Ngắt để nói luôn."],
  };
  const tell = (text, error = false) => (status.innerHTML = text ? `<span class="${error ? "error" : ""}">${esc(text)}</span>` : "");
  function setPhase(phase, note = "", error = false) {
    const [word, action, line] = PHASES[phase];
    v.phase = bar.dataset.phase = phase;
    main.textContent = word;
    main.setAttribute("aria-label", action);
    main.disabled = phase === "thinking" || phase === "starting";
    bar.style.setProperty("--lv", 0);
    bar.hidden = !v.on && !noteBox.innerHTML;
    micButton.setAttribute("aria-pressed", String(v.on));
    micButton.classList.toggle("on", v.on);
    tell(note || line, error);
  }

  /// Without the hearing model the mode cannot start: the same offer as the speaking tutor page makes.
  function offerDownload() {
    noteBox.hidden = false;
    noteBox.innerHTML = `<div class="grow"><b>Tải model nghe giọng nói</b>
      <span>Whisper small, ${Math.round(hearing.size / 1e6)} MB, nghe tiếng Việt và tiếng ${zh() ? "Trung" : "Anh"}, chạy trên máy. Trong lúc chờ, bạn vẫn gõ câu được.</span>
      <progress id="cv-dl" max="1" value="0" hidden></progress><span class="error" id="cv-dl-error"></span></div>
      <button type="button" class="btn small primary" id="cv-download">Tải ngay</button>`;
    bar.hidden = false;
    setPhase("idle", "Cần tải model nghe giọng nói trước.");
  }
  function clearOffer() {
    noteBox.hidden = true;
    noteBox.innerHTML = "";
  }
  noteBox.addEventListener("click", async (e) => {
    const b = e.target.closest("#cv-download");
    if (!b) return;
    b.disabled = true;
    b.classList.add("loading");
    $("cv-dl-error").textContent = "";
    try {
      hearing = await app.call("talk_download");
      clearOffer();
      setPhase("idle");
      start();
    } catch (err) {
      $("cv-dl-error").textContent = String(err);
      b.disabled = false;
      b.classList.remove("loading");
    }
  });
  hooks.event.push((name, data) => {
    const dl = $("cv-dl");
    if (name === "progress" && data.group === "talk" && dl) {
      dl.hidden = false;
      dl.value = data.done / data.total;
    }
    // The speaking tutor's events for the turns of this mode.
    if (name !== "talk" || !v.bubbles.has(data.turn)) return;
    const slot = v.bubbles.get(data.turn);
    if (!slot) return;
    if (data.kind === "text") {
      slot.tutor.innerHTML = window.talkKit.tutorHtml(window.talkKit.parts(data.text), "", "tutor-speak");
      host.log.scrollTop = host.log.scrollHeight;
      if (data.turn === v.turn && v.phase === "thinking") setPhase("speaking");
    }
  });

  /// A line of the mode in the log (what was understood, what was run), kept apart from the AI's bubbles.
  function note(text, kind = "") {
    return host.add(`<div class="cv-line ${kind}">${esc(text)}</div>`);
  }
  /// Reads `text` aloud in Vietnamese and answers when it ends; the learner's Ngắt ends the wait through `turn`.
  async function say(text, turn) {
    const plain = text.replace(/[*#`_>]/g, "").replace(/\s+/g, " ").trim().slice(0, 400);
    if (!plain || turn !== v.turn) return;
    setPhase("speaking");
    try {
      await speak(plain, false, "vi");
    } catch { /* a voice that fails is no reason to stop the talk */ }
  }

  // Listening: the loudness of the microphone decides when the learner has finished.
  async function listen() {
    if (!v.on) return;
    setPhase("starting");
    try {
      micName = (await app.call("rec_start")) || "";
    } catch (e) {
      v.on = false;
      setPhase("idle", String(e), true);
      return;
    }
    // The talk ended while the microphone opened.
    if (!v.on || v.phase !== "starting") return app.call("rec_cancel").catch(() => {});
    setPhase("listening");
    const started = Date.now();
    let floor = 1, peak = 0, voiced = 0, quietSince = 0, early = false, asking = false;
    v.poll = setInterval(async () => {
      if (asking) return;
      asking = true;
      const level = await app.call("rec_level").catch(() => 0);
      asking = false;
      if (v.phase !== "listening") return;
      bar.style.setProperty("--lv", Math.min(1, Math.pow(level / 0.15, 0.6)).toFixed(2));
      const elapsed = Date.now() - started;
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
        if (voiced >= VOICED) return utterance();
        setPhase("starting");
        stopListening(true).then(async () => {
          if (peak > 0) {
            await listen();
            if (v.phase === "listening") tell("Chưa nghe thấy giọng bạn. Hãy nói to hơn và gần micro hơn.");
            return;
          }
          v.on = false;
          setPhase("idle", `Micro${micName ? ` "${micName}"` : ""} không gửi âm thanh nào. Hãy chọn đúng micro, cho phép Card English dùng micro trong phần quyền riêng tư, rồi thử lại.`, true);
        });
      }
    }, POLL_MS);
  }
  async function stopListening(discard) {
    clearInterval(v.poll);
    if (discard) await app.call("rec_cancel").catch(() => {});
  }
  /// Back to listening after a turn, unless it was cut short or the mode ended.
  function again(turn) {
    if (turn !== v.turn || !v.on) return;
    setPhase("idle");
    listen();
  }

  /// The learner stopped: write down the sound, then route the words.
  async function utterance() {
    const turn = ++v.turn;
    setPhase("thinking", "Đang nghe lại câu bạn vừa nói...");
    clearInterval(v.poll);
    let heard;
    try {
      heard = await app.call("talk_hear");
    } catch (e) {
      if (turn !== v.turn) return;
      // Nothing made out of the sound: say so and listen again.
      await listen();
      if (v.phase === "listening") tell(String(e));
      return;
    }
    if (turn !== v.turn) return;
    route(heard, turn);
  }

  const norm = (t) => t.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/đ/g, "d").replace(/[^a-z ]/g, " ").replace(/\s+/g, " ").trim();
  const YES = /^(dong y|co|ok|okay|oke|uh|u|vang|duoc|yes|chac chan|lam di|chay di)\b/;
  const NO = /^(khong|thoi|huy|no|dung|dung lai|bo qua|thoi khong)\b/;

  /// Routes what was said: an answer to a question waiting for a yes, else the chat's own routing.
  async function route(heard, turn) {
    if (v.pending) return confirmed(heard, turn);
    const mine = host.add(window.talkKit.mine(heard));
    let r;
    try {
      // `voice` tells the app the line was spoken: when no feature of the app takes it, the talk is the speaking tutor's.
      r = await app.call("tutor", { text: heard, thread: host.thread(), day: localDay(), voice: true });
    } catch (e) {
      if (turn !== v.turn) return;
      mine.remove();
      host.add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
      return again(turn);
    }
    if (turn !== v.turn) return;
    const cards = r?.cards || r?.reply?.cards || [];
    const doCard = cards.find((c) => c.type === "do" && c.calls?.length);
    if (!cards.length || !r.reply) return tutoring(heard, turn, mine);
    // A feature of the app answered: its lines go in the chat as a typed message's would.
    mine.insertAdjacentHTML("afterend", host.bubble(r.user));
    mine.remove();
    const shownCards = cards.filter((c) => c.type !== "do");
    const el = host.add(host.bubble({ ...r.reply, cards: shownCards }, r.answer?.source ? byHtml(r.answer) : ""));
    host.drawCards(el, { ...r.reply, cards: shownCards });
    host.markLast();
    if (doCard) return doCalls(r.reply.text, doCard, turn);
    await say(r.reply.text, turn);
    again(turn);
  }

  /// Safe calls run now; the others are read back and wait for a yes.
  async function doCalls(text, card, turn) {
    const calls = card.calls.slice(0, 6);
    const safe = calls.filter((c) => !c.confirm), risky = calls.filter((c) => c.confirm);
    for (const c of safe) await run(c);
    if (!risky.length) {
      await say(text, turn);
      return again(turn);
    }
    v.pending = risky;
    v.retries = 0;
    const labels = risky.map((c) => c.label || c.tool);
    note(`Cần bạn xác nhận: ${labels.join("; ")}. Nói đồng ý hoặc không.`, "ask");
    await say(`${labels.join(", ")}. Bạn có đồng ý không?`, turn);
    again(turn);
  }
  /// Runs one call through the chat's tools; their answer, if a line, is shown.
  async function run(call) {
    const label = call.label || call.tool;
    const tool = window.chatTools?.[call.tool];
    if (typeof tool !== "function") return note(`Chưa làm được: ${label}`, "fail");
    try {
      const out = await tool(call.args || {}, { voice: true });
      note(typeof out === "string" && out ? out : `Đã xong: ${label}`, "done");
    } catch (e) {
      note(`Không làm được ${label}: ${e}`, "fail");
    }
  }
  /// The learner's answer to a read-back.
  async function confirmed(heard, turn) {
    const said = norm(heard);
    host.add(window.talkKit.mine(heard));
    const calls = v.pending;
    if (YES.test(said)) {
      v.pending = null;
      for (const c of calls) await run(c);
      await say("Xong rồi.", turn);
      return again(turn);
    }
    if (NO.test(said)) {
      v.pending = null;
      note("Đã hủy, không thay đổi gì.", "done");
      await say("Mình đã hủy.", turn);
      return again(turn);
    }
    // Neither: ask once more, then drop it.
    if (++v.retries > 1) {
      v.pending = null;
      note("Chưa rõ ý bạn, mình đã hủy.", "done");
      await say("Mình chưa nghe rõ nên đã hủy.", turn);
      return again(turn);
    }
    await say("Bạn nói đồng ý để làm, hoặc không để hủy nhé.", turn);
    again(turn);
  }

  /// No feature took the line: a turn of the speaking tutor, 1 to 1, read aloud as it is written.
  async function tutoring(heard, turn, mine) {
    const thinking = host.add(`<div class="chat-msg vt-tutor">${st("mascot-think")}<div class="vt-body"><div class="chat-bubble vt-dots" aria-label="Gia sư đang nghĩ"><i></i><i></i><i></i></div></div></div>`);
    v.bubbles.set(turn, { mine, tutor: thinking });
    setPhase("thinking");
    try {
      const r = await app.call("talk_turn", { history: v.history, text: heard, day: localDay(), turn });
      if (!v.bubbles.has(turn)) return;
      v.history.push({ role: "user", text: r.heard }, { role: "ai", text: [r.vi, r.say].filter(Boolean).join(" ") });
      v.bubbles.get(turn).tutor.innerHTML = window.talkKit.tutorHtml({ say: r.say, vi: r.vi, note: r.note }, byHtml(r.by));
      if (r.spoken && turn === v.turn) toast(r.spoken);
      gainXp(2, main);
    } catch (e) {
      thinking.remove();
      if (turn !== v.turn) return;
      host.add(`<div class="chat-msg failed"><div class="chat-bubble">${esc(e)}</div></div>`);
      v.on = false;
      return setPhase("idle");
    } finally {
      v.bubbles.delete(turn);
    }
    again(turn);
  }

  /// Ends the mode: the microphone and the voice stop, and what was waiting for a yes is dropped.
  async function end() {
    const was = v.on || v.phase !== "idle";
    v.on = false;
    v.turn++;
    v.pending = null;
    if (v.phase === "listening" || v.phase === "starting") await stopListening(true);
    v.bubbles.clear();
    if (was) {
      app.call("talk_stop").catch(() => {});
      try { speechSynthesis.cancel(); } catch { /* no web voice */ }
    }
    setPhase("idle");
    bar.hidden = noteBox.hidden;
  }
  /// The mode with a fresh talk: lines for the tutor start over.
  function reset() {
    end();
    v.history = [];
    v.target = "";
    clearOffer();
    bar.hidden = true;
  }

  async function start() {
    if (v.on) return;
    if (host.busy()) return toast("Đợi trợ lý trả lời xong đã nhé");
    hearing = await app.call("talk_model").catch(() => hearing);
    if (!hearing.ready) return offerDownload();
    clearOffer();
    v.on = true;
    v.retries = 0;
    bar.hidden = false;
    listen();
  }
  micButton.addEventListener("click", () => (v.on ? end() : start()));
  endButton.addEventListener("click", end);
  main.addEventListener("click", async () => {
    if (v.phase === "listening") return utterance();
    if (v.phase === "speaking") {
      // Cut the voice short and listen at once.
      v.turn++;
      await app.call("talk_stop").catch(() => {});
      try { speechSynthesis.cancel(); } catch { /* no web voice */ }
      setPhase("idle");
      return listen();
    }
    if (v.phase === "idle" && !v.on) start();
  });

  // The panel closing or the course changing ends the mode.
  new MutationObserver(() => { if (host.panel.hidden) reset(); }).observe(host.panel, { attributes: true, attributeFilter: ["hidden"] });
  hooks.course.push(reset);
  setPhase("idle");
  bar.hidden = true;
})();
