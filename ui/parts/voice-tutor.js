// The speaking tutor (beta): a talk by voice with an AI teacher, as with a person. The page listens until the learner
// stops speaking, the app writes down what it heard (Vietnamese or the language of the course), the AI answers as a
// teacher in a conversation would, and its answer is read aloud while it is still being written. The microphone stays
// open meanwhile, so that the learner can talk over the tutor to cut it short, and the page listens again as soon as
// it is done. Mistakes are not dwelt on during the talk: the tutor notes them, and the report at the end gathers them
// with a last exercise of sentences to say.
(() => {
  "use strict";
  if (window.MINI) return;
  /// How the page tells speech from silence by the microphone's loudness (RMS): a voice starts above `START` (or
  /// three times the room's own noise) for `VOICED` readings, and the turn ends after `PAUSE_MS` below two thirds of
  /// that. START is low enough for a microphone that is not built in, such as a webcam's on a Mac mini. After
  /// `EARLY_MS` of the pause the app starts writing down what was said, so that the text is ready when it ends.
  const START = 0.01, VOICED = 3, PAUSE_MS = 600, EARLY_MS = 250, POLL_MS = 60;
  /// The longest a turn listens.
  const LISTEN_MS = 20000;
  /// The learner talking over the tutor: louder than `BARGE` and than the tutor's own voice coming back from the
  /// speakers (by `OVER_ECHO`), for `BARGE_READS` readings in a row. What was recorded before then is dropped but
  /// for `BARGE_BACK_MS`, the start of the learner's words; when the tutor finishes on its own, `AFTER_MS`.
  /// The first `LEARN_MS` only learn how loud the tutor's voice comes back.
  const BARGE = 0.05, OVER_ECHO = 1.8, BARGE_READS = 5, BARGE_BACK_MS = 600, AFTER_MS = 300, LEARN_MS = 800;
  /// Background talk: once the learner has spoken, a level under this share of their voice counts as their pause.
  const ROOM = 0.2;
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
            <button class="btn small" id="vt-end" hidden>Kết thúc và tổng kết</button>
          </div>
          <form class="vt-type" id="vt-type"><input id="vt-input" maxlength="500" placeholder="Hoặc gõ câu của bạn" aria-label="Gõ câu của bạn"><button class="btn">Gửi</button>
            <button type="button" class="vt-mic" id="vt-mic"></button></form>
        </div>
      </div>
    </section>`);
  const card = $("vt"), log = $("vt-log"), status = $("vt-status"), mic = $("vt-mic"), endButton = $("vt-end"), input = $("vt-input");

  /// The talk: its lines for the AI, the tutor's notes for the report, what the page is doing ("idle", "starting",
  /// "listening", "thinking" or "speaking"), whether the microphone is recording, the timer that watches it, the
  /// latest turn and the tutor's bubble of each turn still being answered (one cut short goes on filling in while the
  /// next one runs), and the sentence of the report being tried aloud.
  const talk = { history: [], notes: [], phase: "idle", on: false, rec: false, poll: 0, turn: 0, bubbles: new Map(), trying: null };
  const zh = () => courseOf() === "zh";
  /// The model that hears the learner (Whisper small, Vietnamese and the language of the course): { ready, size }.
  let hearing = { ready: false, size: 0 };
  /// The microphone the last recording used, named in the advice when it hears nothing.
  let micName = "";

  /// The round button's word and what it does, the line beside the tutor's face and the face, for each phase.
  const PHASES = {
    idle: ["Nói", "Bắt đầu nói", "Bấm Nói rồi trò chuyện tự nhiên, bằng tiếng Việt cũng được. Ngừng một chút là gia sư trả lời.", "tutor-teach"],
    starting: ["Chờ", "Đang mở micro", "Đang mở micro...", "tutor-listen"],
    listening: ["Xong", "Gửi câu vừa nói", "Gia sư đang nghe. Ngừng một chút là tự gửi, hoặc bấm Xong.", "tutor-listen"],
    thinking: ["Chờ", "Gia sư đang nghĩ", "Gia sư đang suy nghĩ...", "mascot-think"],
    speaking: ["Ngắt", "Ngắt lời gia sư", "Gia sư đang nói. Cứ nói chen vào hoặc bấm Ngắt để nói luôn.", "tutor-speak"],
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
  /// The learner's line.
  const mine = (text) => `<div class="chat-msg me"><div class="chat-bubble">${esc(text)}</div></div>`;
  /// The parts of an answer as it streams: VI said in Vietnamese, SAY said in the language of the course, NOTE kept
  /// for the report (FIX, TRY and TIP of a model used to the earlier format too).
  function parts(text) {
    const out = { vi: [], say: [], note: [] };
    let part = "say";
    for (const raw of text.split("\n")) {
      const line = raw.trim().replace(/^[*#\- ]+/, "");
      const m = line.match(/^(SAY|VI|NOTE|FIX|TRY|TIP):\**\s*(.*)$/);
      if (m) part = { SAY: "say", VI: "vi" }[m[1]] || "note";
      const rest = m ? m[2] : line;
      if (rest) out[part].push(rest);
    }
    return { vi: out.vi.join(" "), say: out.say.join(" "), note: out.note.join("\n") };
  }
  /// A note, its `"wrong" -> "right": why` (or `wrong -> right (why)` in the report) shown as the wrong words struck
  /// out and the right ones in green.
  function fixLine(line) {
    const m = line.match(/^"(.+?)"\s*->\s*"(.+?)"\s*:?\s*(.*)$/) || line.match(/^(.+?)\s*->\s*(.+?)\s*(?:\((.*)\))?$/);
    if (!m) return `<li>${esc(line)}</li>`;
    return `<li><span class="vt-wrong">${esc(m[1])}</span> thành <span class="vt-right">${esc(m[2])}</span>${m[3] ? `<div>${esc(m[3])}</div>` : ""}</li>`;
  }
  /// The inside of a tutor's message: what it said in Vietnamese and in the language of the course, and its notes
  /// folded away for later; its face talks while the answer comes.
  function tutorHtml(p, by = "", face = "mascot") {
    const notes = p.note ? p.note.split("\n").filter(Boolean) : [];
    return `${st(face)}<div class="vt-body">
      ${p.vi ? `<div class="chat-bubble vt-vi">${esc(p.vi)}</div>` : ""}
      ${p.say || !p.vi ? `<div class="chat-bubble">${esc(p.say)}</div>` : ""}
      ${notes.length ? `<details class="vt-notes"><summary>Gia sư ghi lại ${notes.length} điểm cho phần tổng kết</summary><ul>${notes.map(fixLine).join("")}</ul></details>` : ""}
      ${by ? `<div class="chat-by">${by}</div>` : ""}</div>`;
  }

  const thinking = () => add(`<div class="chat-msg vt-tutor">${st("mascot-think")}<div class="vt-body"><div class="chat-bubble vt-dots" aria-label="Gia sư đang nghĩ"><i></i><i></i><i></i></div></div></div>`);
  hooks.event.push((name, data) => {
    if (name !== "talk" || !talk.bubbles.has(data.turn)) return;
    // What was heard, or the line typed: the learner's bubble and the tutor's, waiting for its answer.
    if (data.kind === "heard") {
      add(mine(data.text));
      talk.bubbles.set(data.turn, thinking());
    }
    const bubble = talk.bubbles.get(data.turn);
    if (data.kind === "text" && bubble) {
      bubble.innerHTML = tutorHtml(parts(data.text), "", "tutor-speak");
      log.scrollTop = log.scrollHeight;
      if (data.turn === talk.turn && talk.phase === "thinking") {
        setPhase("speaking");
        overhear(data.turn);
      }
    }
  });

  /// One reading of the microphone's loudness at a time.
  let asking = false;
  async function loudness() {
    if (asking) return null;
    asking = true;
    const level = await app.call("rec_level").catch(() => 0);
    asking = false;
    return level;
  }
  /// Opens the microphone, once for whoever asks while it opens.
  let opening = null;
  function openMic() {
    opening ||= app.call("rec_start").then((name) => {
      micName = name || "";
      talk.rec = true;
      $("vt-mic-name").textContent = micName ? `Micro: ${micName}` : "";
    }).finally(() => (opening = null));
    return opening;
  }

  /// While the tutor talks, the microphone listens for the learner talking over it, the way one cuts a person short.
  async function overhear(turn) {
    if (!talk.on || !hearing.ready || talk.rec) return;
    try {
      await openMic();
    } catch {
      return;
    }
    if (!talk.on) return stopListening(true);
    // The tutor already finished, and the listening goes on with this recording.
    if (talk.phase !== "speaking" || turn !== talk.turn) return;
    let echo = 0, over = 0;
    const opened = Date.now();
    clearInterval(talk.poll);
    talk.poll = setInterval(async () => {
      const level = await loudness();
      if (level === null || talk.phase !== "speaking" || turn !== talk.turn) return;
      if (Date.now() - opened < LEARN_MS) echo = Math.max(echo, level);
      else if (level > Math.max(BARGE, echo * OVER_ECHO)) over++;
      else {
        over = 0;
        // The tutor's voice as the microphone hears it, slowly forgotten.
        echo = Math.max(echo * 0.995, level);
      }
      if (over < BARGE_READS) return;
      clearInterval(talk.poll);
      talk.turn++;
      app.call("talk_stop").catch(() => {});
      await app.call("rec_mark", { back_ms: BARGE_BACK_MS }).catch(() => {});
      listen(BARGE_READS);
    }, POLL_MS);
  }

  // Listening: the microphone's loudness decides when the learner has finished. A recording opened while the tutor
  // talked goes on, without the tutor's voice in it; `voiced` counts what the learner already said over it.
  async function listen(voiced = 0) {
    if (!talk.on) return;
    clearInterval(talk.poll);
    if (opening) await opening.catch(() => {});
    if (talk.rec) {
      if (!voiced) await app.call("rec_mark", { back_ms: AFTER_MS }).catch(() => {});
    } else {
      setPhase("starting");
      try {
        await openMic();
      } catch (e) {
        talk.on = false;
        setPhase("idle", String(e), true);
        return;
      }
      // The talk ended while the microphone opened.
      if (!talk.on || talk.phase !== "starting") return stopListening(true);
    }
    setPhase("listening");
    const started = Date.now();
    let floor = 1, peak = 0, voice = 0, quietSince = 0, early = false;
    talk.poll = setInterval(async () => {
      const level = await loudness();
      if (level === null || talk.phase !== "listening") return;
      // Speech is around 0.02 to 0.2 RMS; the curve lifts quiet voices so that they still show.
      card.style.setProperty("--lv", Math.min(1, Math.pow(level / 0.15, 0.6)).toFixed(2));
      const elapsed = Date.now() - started;
      // The room's own noise, learned in the first moments and whenever it is quieter.
      floor = Math.min(floor, Math.max(level, 0.002));
      peak = Math.max(peak, level);
      const loud = Math.max(START, floor * 3);
      // Once the learner has spoken, others talking further away are quieter than them, and are their pause.
      const quiet = Math.max(loud * 0.66, voice * ROOM);
      if (level > loud && level >= quiet) {
        voiced++;
        voice = Math.max(voice * 0.98, level);
        quietSince = 0;
        early = false;
      } else if (voiced >= VOICED && level < quiet && !quietSince) quietSince = Date.now();
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
    if (discard && talk.rec) await app.call("rec_cancel").catch(() => {});
    talk.rec = false;
  }

  /// A turn: what was said (or `typed`), then the tutor's answer, read aloud; then the page listens again. The
  /// learner's bubble comes with the "heard" event, for a typed line too.
  async function send(typed = "") {
    const turn = ++talk.turn;
    // At once, so that the listening's own timer cannot send the same turn again. The app stops a recording it hears.
    setPhase("thinking", typed ? "" : "Đang nghe lại câu bạn vừa nói...");
    talk.bubbles.set(turn, null);
    await stopListening(!!typed);
    try {
      const r = await app.call("talk_turn", { history: talk.history, text: typed, day: localDay(), turn });
      // The talk was started over meanwhile.
      if (!talk.bubbles.has(turn)) return;
      talk.history.push({ role: "user", text: r.heard }, { role: "ai", text: [r.vi, r.say].filter(Boolean).join(" ") });
      if (r.note) talk.notes.push(...r.note.split("\n").filter(Boolean));
      const bubble = talk.bubbles.get(turn);
      if (bubble) bubble.innerHTML = tutorHtml({ say: r.say, vi: r.vi, note: r.note }, byHtml(r.by));
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
    if (opening) await opening.catch(() => {});
    await stopListening(true);
    app.call("talk_stop").catch(() => {});
    setPhase("idle");
  }

  /// The end of the talk and the report on it: what went well, what to say instead, words worth keeping, sounds to
  /// work on, and a last exercise of sentences to say, each with its model and a try that is scored.
  async function finish() {
    await end();
    if (!talk.history.some((m) => m.role === "user")) return;
    const turns = talk.history.filter((m) => m.role === "user").length;
    const box = add(`<div class="vt-report card"><b>Tổng kết buổi nói</b><p class="muted">Gia sư đang viết tổng kết...</p></div>`);
    endButton.hidden = true;
    try {
      const r = await app.call("talk_summary", { history: talk.history, notes: talk.notes, day: localDay() });
      const list = (title, items, line = (x) => `<li>${esc(x)}</li>`) =>
        items?.length ? `<section><span class="vt-label">${title}</span><ul>${items.map(line).join("")}</ul></section>` : "";
      box.innerHTML = `<header><b>Tổng kết buổi nói</b><span>${r.topic ? `${esc(r.topic)}, ` : ""}${turns} lượt nói</span></header>
        ${r.good ? `<section class="vt-good"><span class="vt-label">Làm tốt</span><p>${esc(r.good)}</p></section>` : ""}
        ${list("Nên nói thế này", r.mistakes, fixLine)}
        ${list("Từ và cụm từ đáng nhớ", r.phrases)}
        ${list("Phát âm cần luyện", r.sounds)}
        ${r.practice?.length ? `<section class="vt-final"><span class="vt-label">Bài luyện cuối buổi</span>
          <p class="muted">Nghe mẫu rồi bấm Nói thử nếu muốn, không bắt buộc.</p>
          <ol>${r.practice.map((x) => `<li><b>${esc(x)}</b><span class="vt-final-do">
            <button class="btn small" data-say="${esc(x)}">Nghe mẫu</button>
            <button class="btn small" data-try="${esc(x)}">Nói thử</button><span class="vt-result"></span></span></li>`).join("")}</ol></section>` : ""}
        ${r.next ? `<section><span class="vt-label">Lần sau</span><p>${esc(r.next)}</p></section>` : ""}
        <div class="chat-by">${byHtml(r.by)}</div>`;
      gainXp(5, box);
    } catch (e) {
      box.innerHTML = `<b>Tổng kết buổi nói</b><p class="error">${esc(e)}</p><button class="btn small" data-vt="summary">Thử lại</button>`;
    }
    log.scrollTop = log.scrollHeight;
  }

  /// A sentence of the last exercise said aloud: the button records until pressed again, or for eight seconds.
  async function tryOut(button) {
    const result = button.parentElement.querySelector(".vt-result");
    if (talk.trying) {
      if (talk.trying.button !== button) return;
      clearTimeout(talk.trying.timer);
      talk.trying = null;
      button.textContent = "Nói thử";
      button.disabled = true;
      result.textContent = "Đang nghe...";
      try {
        const r = await app.call("talk_try", { target: button.dataset.try });
        talk.rec = false;
        result.className = `vt-result ${r.score >= 75 ? "ok" : "low"}`;
        result.innerHTML = `${r.score}/100${r.missed?.length ? `, chưa nghe rõ: ${r.missed.map((w) => `<b>${esc(w)}</b>`).join(", ")}` : ""}`;
      } catch (e) {
        talk.rec = false;
        result.className = "vt-result low";
        result.textContent = String(e);
      }
      button.disabled = false;
      return;
    }
    if (!hearing.ready) return toast("Hãy tải model nghe giọng nói ở trên trước");
    if (talk.on || talk.phase !== "idle") return;
    try {
      await openMic();
    } catch (e) {
      return toast(String(e));
    }
    talk.trying = { button, timer: setTimeout(() => tryOut(button), 8000) };
    button.textContent = "Xong";
    result.className = "vt-result";
    result.textContent = "Đang ghi âm, nói xong bấm Xong";
  }

  mic.addEventListener("click", async () => {
    if (talk.phase === "listening") return send();
    if (talk.phase === "speaking") {
      // Cut the tutor short and listen at once; its answer still goes on screen.
      talk.turn++;
      await app.call("talk_stop").catch(() => {});
      return listen();
    }
    if (talk.phase !== "idle" || talk.trying) return;
    if (!hearing.ready) return tell("Hãy tải model nghe giọng nói ở trên trước, hoặc gõ câu bên dưới.", true);
    talk.on = true;
    listen();
  });
  endButton.addEventListener("click", finish);
  /// A new talk: the one under way ends, and an answer still coming is dropped.
  function reset() {
    if (talk.trying) {
      clearTimeout(talk.trying.timer);
      talk.trying = null;
    }
    end();
    talk.bubbles.clear();
    talk.history = [];
    talk.notes = [];
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
      if (talk.phase !== "idle" || talk.trying) return;
      // The tutor opens the topic, then listens if it can hear.
      talk.on = hearing.ready;
      return send(`Mình muốn luyện nói về chủ đề: ${topic.dataset.topic}`);
    }
    const attempt = e.target.closest("[data-try]");
    if (attempt) return tryOut(attempt);
    const b = e.target.closest("[data-vt]");
    if (!b) return;
    e.preventDefault();
    if (b.dataset.vt === "settings") return go("settings");
    if (b.dataset.vt === "summary") {
      b.closest(".vt-report")?.remove();
      return finish();
    }
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
