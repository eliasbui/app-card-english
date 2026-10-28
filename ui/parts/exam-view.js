// Mock exams written by the AI: a full-screen view over the app (the test's plan, the paper being written, each
// section with its timer, the review of a submitted section, the result and the history), the test of each stage of
// a hard roadmap on its card, and the optional mock on the goal card.
(() => {
  "use strict";
  // The quick-quiz window loads the same page and has no exams.
  if (window.MINI) return;
  const LETTERS = "ABCDE";
  /// A long spoken answer is heard in pieces of this length, which Whisper takes in one go.
  const CLIP_MS = 12000;

  /// Where the view was opened from: the roadmap's uid and the stage's index, -1 for the optional mock.
  let target = null;
  /// What `exam_plan` answered, shown before a paper is written.
  let plan = null;
  /// The exam under way or shown, as `exam_view` gives it: keys only for the sections already submitted.
  let ex = null;
  /// The section on screen and the answers given in it, one per item in order.
  let at = 0, answers = [];
  let screen = "", busy = false, error = "";
  let ticker = 0, deadline = 0;
  /// Plays used of each recording, and whether practice allows one more.
  let plays = {}, replay = false;
  /// The recording under way, or the preparation before it.
  let rec = null, prep = null;
  let seq = 0;

  document.body.insertAdjacentHTML("beforeend", `
    <div id="exam" class="exam hidden" role="dialog" aria-modal="true" aria-labelledby="ex-title">
      <div class="exam-bar"><div class="grow"><b id="ex-title"></b><div class="muted small" id="ex-where"></div></div>
        <span class="exam-timer hidden" id="ex-timer" role="timer" aria-live="off"></span>
        <button class="btn small ghost" data-x="close">Thoát</button></div>
      <div class="exam-body" id="ex-body"></div>
    </div>`);
  const body = $("ex-body");
  const zh = () => courseOf() === "zh";
  const fmt = (sec) => `${Math.floor(Math.max(0, sec) / 60)}:${String(Math.max(0, sec) % 60).padStart(2, "0")}`;
  const when = (sec) => new Date(sec * 1000).toLocaleDateString("vi-VN", { day: "numeric", month: "numeric", year: "numeric" });
  const count = (text) => zh() ? [...String(text || "")].filter((c) => hasHan(c)).length : String(text || "").trim().split(/\s+/).filter(Boolean).length;
  const unit = () => zh() ? "chữ" : "từ";
  const who = (w) => zh() ? (w === "M" ? "男：" : w === "W" ? "女：" : "") : (w === "M" ? "Nam: " : w === "W" ? "Nữ: " : "");
  const whisperReady = () => !!state.models?.[zh() ? "sttzh" : "stt"]?.ready;
  const section = () => ex.paper.sections[at];
  /// Asks for a second press within a few seconds before doing something that loses work.
  function twice(button, text) {
    if (button.dataset.armed) return true;
    button.dataset.armed = "1";
    const label = button.textContent;
    button.textContent = text;
    setTimeout(() => { delete button.dataset.armed; button.textContent = label; }, 3500);
    return false;
  }
  /// The items of a section in order, with where each sits.
  function items(s) {
    const out = [];
    s.parts.forEach((p, pi) => p.sets.forEach((set, si) => set.items.forEach((it, ii) => out.push({ p, pi, set, si, it, ii, qi: out.length }))));
    return out;
  }

  // Opening and closing.
  async function open(roadmap, stage) {
    const mine = ++seq;
    target = { roadmap, stage };
    ex = null;
    error = "";
    show();
    screen = "intro";
    plan = null;
    body.innerHTML = `<div class="exam-inner"><div class="skeleton"></div><div class="skeleton"></div></div>`;
    setBar("Bài thi thử", "");
    try {
      plan = await app.call("exam_plan", { roadmap, stage });
    } catch (e) {
      if (mine === seq) body.innerHTML = `<div class="exam-inner"><div class="card"><p class="error">${esc(e)}</p></div></div>`;
      return;
    }
    if (mine === seq) renderIntro();
  }
  async function openExam(uid) {
    const mine = ++seq;
    show();
    try {
      const v = await app.call("exam_get", { uid });
      if (mine !== seq) return;
      begin(v);
    } catch (e) {
      toast(String(e));
    }
  }
  function show() {
    $("exam").classList.remove("hidden");
  }
  function close() {
    stopAll();
    seq++;
    $("exam").classList.add("hidden");
    ex = plan = null;
    screen = "";
    // The stage's card and Home show the test's new state.
    const page = document.querySelector("main section.active")?.id.slice(5);
    if (page) reloadPage(page);
  }
  function stopAll() {
    clearInterval(ticker);
    ticker = 0;
    $("ex-timer").classList.add("hidden");
    if (prep) { clearInterval(prep.timer); prep = null; }
    if (rec) {
      const r = rec;
      rec = null;
      clearInterval(r.poll); clearInterval(r.chunk); clearTimeout(r.limit);
      app.call("exam_clip", { more: false }).catch(() => {});
    }
    app.call("exam_stop").catch(() => {});
  }
  function setBar(title, where) {
    $("ex-title").textContent = title;
    $("ex-where").textContent = where;
  }

  // Before a test: what it holds, the pass mark, the AI that writes it.
  function renderIntro() {
    const p = plan;
    setBar(p.title, p.name);
    const minutes = p.sections.reduce((n, s) => n + s.minutes, 0);
    const blocked = !!p.ai.error;
    body.innerHTML = `
      <div class="exam-inner narrow">
        <div class="card ex-intro">
          <div class="eyebrow">${p.full ? "Thi thử đủ bốn kỹ năng" : "Bài kiểm tra chặng"}, theo dạng đề ${esc(p.name)}</div>
          <h2>${esc(p.title)}</h2>
          <div class="ex-pass">Cần đạt <b>${esc(p.pass_text)}</b> để qua${p.full ? "" : " chặng"}.</div>
          <div class="ex-sections">${p.sections.map((s, i) => `
            <div class="ex-sec-row"><span class="ex-n">${i + 1}</span><div class="grow"><b>${esc(s.vi)}</b> <span class="muted">(${esc(s.name)})</span>
              <div class="muted small">${s.parts.map(esc).join("; ")}</div></div>
              <span class="small muted num">${s.items} ${s.skill === "writing" ? "bài" : "câu"}, ${s.minutes} phút</span></div>`).join("")}</div>
          <p class="muted small">Khoảng ${minutes} phút. Mỗi phần có đồng hồ riêng, đáp án chỉ hiện sau khi nộp phần đó.
            ${p.plays === 1 ? "Bài nghe chỉ được nghe một lần như đề thật." : `Bài nghe được nghe ${p.plays} lần như đề thật.`}
            ${p.topics.length ? `Chủ đề: ${p.topics.map(esc).join(", ")}.` : ""}</p>
          <label class="switch"><input type="checkbox" id="ex-replay" ${replay ? "checked" : ""}><i></i>Chế độ luyện tập: cho nghe lại thêm một lần</label>
          ${p.sections.some((s) => s.skill === "speaking") && !whisperReady()
            ? `<p class="ex-note">Phần Nói cần model nhận giọng nói của khóa này (Cài đặt, mục Phát âm). Chưa có model thì bạn vẫn làm được các phần khác và bỏ qua phần Nói.</p>` : ""}
          ${p.ai.error ? `<p class="ex-note bad">${esc(p.ai.error)}</p>` : p.ai.warning ? `<p class="ex-note">${esc(p.ai.warning)}</p>` : ""}
          <div class="row actions">
            ${p.open ? `<button class="btn primary" data-x="resume" data-uid="${esc(p.open.uid)}">Làm tiếp bài đang làm</button>`
              : `<button class="btn primary" data-x="make" ${blocked ? "disabled" : ""}>Tạo đề và bắt đầu</button>`}
            ${p.last ? `<button class="btn" data-x="see" data-uid="${esc(p.last.uid)}">Xem kết quả lần trước</button>` : ""}
            <button class="btn ghost" data-x="history">Bài thi đã làm</button>
          </div>
          <p class="error">${esc(error)}</p>
        </div>
      </div>`;
  }

  async function make() {
    const mine = seq;
    error = "";
    screen = "making";
    renderMaking({ done: 0, total: 1, label: "" });
    try {
      const v = await app.call("exam_make", { roadmap: target.roadmap, stage: target.stage, day: localDay() });
      if (mine !== seq) return;
      begin(v);
    } catch (e) {
      if (mine !== seq) return;
      error = String(e);
      screen = "intro";
      renderIntro();
    }
  }
  function renderMaking(d) {
    const pct = d.total ? Math.round((100 * d.done) / d.total) : 0;
    body.innerHTML = `
      <div class="exam-inner narrow"><div class="card">
        <div class="thinking">${st("mascot-think")}<span>AI đang viết đề theo dạng ${esc(plan?.name || "")}: bài đọc, bài nghe, câu hỏi và đáp án. Đề được lưu lại, mở lại không phải viết lại.</span></div>
        <div class="progress ex-making"><div style="width:${pct}%"></div></div>
        <div class="muted small num">${d.done}/${d.total} ${d.label ? "phần: " + esc(d.label) : ""}</div>
        <div class="row actions"><button class="btn ghost" data-x="cancel">Dừng</button></div>
      </div></div>`;
  }
  hooks.event.push((name, data) => { if (name === "exam_progress" && screen === "making") renderMaking(data); });

  /// An exam to take or to look at: the first section not submitted, or the result.
  function begin(v) {
    ex = v;
    plays = {};
    if (v.result && v.finished) return renderResult();
    at = v.next ?? 0;
    startSection();
  }

  // A section: its parts in order, a timer, the answers kept as they are given.
  function startSection() {
    stopAll();
    screen = "section";
    error = "";
    const s = section();
    answers = items(s).map(() => "");
    deadline = Date.now() + s.seconds * 1000;
    renderSection();
    // Speaking keeps time per answer instead.
    if (s.skill !== "speaking") {
      $("ex-timer").classList.remove("hidden");
      tick();
      ticker = setInterval(tick, 1000);
    }
  }
  function tick() {
    const left = Math.round((deadline - Date.now()) / 1000);
    const t = $("ex-timer");
    t.textContent = `Còn ${fmt(left)}`;
    t.classList.toggle("low", left <= 60);
    if (left <= 0 && screen === "section" && !busy) {
      toast("Hết giờ, phần này được nộp");
      submit(true);
    }
  }

  function renderSection() {
    const s = section();
    setBar(ex.paper.title, `Phần ${at + 1}/${ex.paper.sections.length}: ${s.vi} (${s.name})`);
    const list = items(s);
    let n = 0;
    const parts = s.parts.map((p, pi) => `
      <div class="card ex-part">
        <div class="ex-part-head"><b>${esc(p.label)}</b><div class="muted small">${esc(p.hint)}${p.words ? ` Tối thiểu ${p.words} ${unit()}.` : ""}</div></div>
        ${p.sets.map((set, si) => setHtml(s, p, pi, set, si, list.filter((x) => x.pi === pi && x.si === si).map((x) => ({ ...x, n: ++n })))).join("")}
      </div>`).join("");
    const speakingBlocked = s.skill === "speaking" && !whisperReady();
    body.innerHTML = `
      <div class="exam-inner">
        ${speakingBlocked ? `<div class="card ex-note">Chưa tải model nhận giọng nói của khóa này nên chưa làm được phần Nói. Tải trong Cài đặt rồi mở lại bài, hoặc bỏ qua phần này.
          <div class="row actions"><button class="btn" data-x="skip">Bỏ qua phần Nói</button></div></div>` : ""}
        ${parts}
        <div class="ex-foot"><span class="muted small" id="ex-answered"></span><span class="grow"></span>
          <p class="error" id="ex-error">${esc(error)}</p>
          <button class="btn primary big" data-x="submit">Nộp phần ${esc(s.vi)}</button></div>
      </div>`;
    body.scrollTop = 0;
    answered();
  }

  function setHtml(s, p, pi, set, si, list) {
    const key = `${at}-${pi}-${si}`;
    const heardSet = p.heard && (s.skill === "listening" || p.kind === "s_retell") || p.heard_options;
    const audio = heardSet ? audioHtml(key, `data-p="${pi}" data-s="${si}"`) : "";
    const passage = !p.heard && set.text ? `<div class="ex-passage">${passageHtml(set.text)}${listHtml(set.list)}</div>` : "";
    const bank = !passage && set.list.length ? listHtml(set.list) : "";
    const questions = `<div class="ex-items">${list.map((x) => itemHtml(p, set, x)).join("")}</div>`;
    return `<div class="ex-set ${passage ? "ex-split" : ""}">${audio}${bank}${passage}${questions}</div>`;
  }
  const passageHtml = (text) => text.split("\n").map((l) => l.includes(" | ") ? `<div class="ex-row">${esc(l)}</div>` : `<p>${esc(l)}</p>`).join("");
  const listHtml = (list) => list.length ? `<div class="ex-list">${list.map((e) => `<div>${esc(e)}</div>`).join("")}</div>` : "";
  function audioHtml(key, where) {
    const allowed = (ex.paper.plays || 1) + (replay ? 1 : 0), used = plays[key] || 0;
    return `<div class="ex-audio"><button class="btn small primary" data-x="play" data-key="${key}" ${where} ${used >= allowed ? "disabled" : ""}>Nghe</button>
      <span class="muted small" id="pl-${key}">${used >= allowed ? "Đã hết lượt nghe" : `Còn ${allowed - used} lần nghe`}</span></div>`;
  }

  function itemHtml(p, set, x) {
    const { it, qi, n } = x;
    const q = it.q ? `<div class="ex-qtext">${esc(it.q).replace(/\n/g, "<br>")}</div>` : `<div class="ex-qtext muted">Nghe rồi chọn câu đáp đúng.</div>`;
    const head = (extra = "") => `<div class="ex-q"><span class="ex-n">${n}</span><div class="grow">${q}${extra}</div></div>`;
    switch (p.mode) {
      case "choice":
        return `<div class="ex-item">${head()}<div class="ex-opts" role="radiogroup" aria-label="Câu ${n}">${it.options.map((o, j) => `
          <label><input type="radio" name="x${qi}" value="${LETTERS[j]}" data-qi="${qi}"><span><b>${LETTERS[j]}.</b> ${esc(o)}</span></label>`).join("")}</div></div>`;
      case "match":
        return `<div class="ex-item">${head()}<select data-qi="${qi}" aria-label="Câu ${n}"><option value="">Chọn</option>${set.list.map((e) => {
          const label = e.trim().match(/^\(?([A-Za-z]+)/)?.[1] || e;
          return `<option value="${esc(label)}">${esc(e)}</option>`;
        }).join("")}</select></div>`;
      case "text": {
        const pieces = p.kind === "w_order" ? `<div class="ex-pieces">${it.q.split(/[/／]/).map((w) => `<span class="tag plain">${esc(w.trim())}</span>`).join("")}</div>` : "";
        const order = p.kind === "r_order" ? `<div class="ex-list">${it.options.map((o, j) => `<div><b>${LETTERS[j]}.</b> ${esc(o)}</div>`).join("")}</div>` : "";
        const hint = { r_order: "Ví dụ: BAC", w_order: "Gõ lại cả câu", w_hanzi: "Chữ Hán" }[p.kind] || (zh() ? "Đáp án" : "Không quá hai từ");
        return `<div class="ex-item">${p.kind === "w_order" ? `<div class="ex-q"><span class="ex-n">${n}</span><div class="grow">${pieces}</div></div>` : head(order)}
          <input data-qi="${qi}" maxlength="80" placeholder="${hint}" aria-label="Câu ${n}" autocomplete="off" spellcheck="false"></div>`;
      }
      case "write":
        return `<div class="ex-item">${head(set.text ? `<div class="ex-passage inline">${passageHtml(set.text)}</div>` : "")}
          <textarea data-qi="${qi}" rows="12" aria-label="Bài viết câu ${n}" spellcheck="false"></textarea>
          <div class="ex-count muted small num" id="wc-${qi}">0 ${unit()}${p.words ? `, tối thiểu ${p.words}` : ""}</div></div>`;
      default:
        return speakHtml(p, set, x, head);
    }
  }

  // Speaking: listen when the item is heard, prepare when the exam gives time, then record; a long answer is heard
  // in pieces, joined in order.
  function speakHtml(p, set, x, head) {
    const { qi, si } = x;
    const heardItem = p.heard && p.kind !== "s_retell";
    const retell = p.kind === "s_retell" ? audioHtml(`${at}-${x.pi}-${si}`, `data-p="${x.pi}" data-s="${si}"`) : "";
    const listen = heardItem ? audioHtml(`${at}-${x.pi}-${si}-${x.ii}`, `data-p="${x.pi}" data-s="${si}" data-i="${x.ii}"`) : "";
    const what = heardItem ? `<div class="ex-qtext muted">${p.kind === "s_repeat" ? "Nghe câu rồi nói lại đúng câu đó." : "Nghe câu hỏi rồi trả lời."}</div>` : "";
    return `<div class="ex-item ex-speak">${x.ii === 0 ? retell : ""}${heardItem ? `<div class="ex-q"><span class="ex-n">${x.n}</span><div class="grow">${what}</div></div>` : head()}
      ${listen}
      <div class="row"><button class="btn small ${p.prep ? "" : "primary"}" data-x="${p.prep ? "prep" : "rec"}" data-qi="${qi}" data-prep="${p.prep}" data-max="${p.secs}">
        ${p.prep ? `Chuẩn bị ${p.prep} giây rồi ghi âm` : "Ghi âm câu trả lời"}</button>
        <span class="muted small">Nói tối đa ${p.secs >= 60 ? fmt(p.secs).replace(/:00$/, "") + " phút" : p.secs + " giây"}</span></div>
      <div class="ex-live" id="live-${qi}"></div><div class="ex-heard" id="heard-${qi}"></div></div>`;
  }
  async function play(button) {
    const key = button.dataset.key, allowed = (ex.paper.plays || 1) + (replay ? 1 : 0);
    if ((plays[key] || 0) >= allowed || button.classList.contains("loading")) return;
    plays[key] = (plays[key] || 0) + 1;
    const left = allowed - plays[key];
    $("pl-" + key).textContent = left ? `Còn ${left} lần nghe` : "Đã hết lượt nghe";
    button.classList.add("loading");
    button.disabled = true;
    const args = { uid: ex.uid, section: at, part: +button.dataset.p, set: +button.dataset.s, rate: +state.settings.speech_rate || 0.95 };
    if (button.dataset.i !== undefined) args.item = +button.dataset.i;
    try {
      await app.call("exam_play", args);
    } catch (e) {
      toast(String(e));
      plays[key]--;
    }
    button.classList.remove("loading");
    button.disabled = (plays[key] || 0) >= allowed;
    if ($("pl-" + key)) $("pl-" + key).textContent = (plays[key] || 0) >= allowed ? "Đã hết lượt nghe" : `Còn ${allowed - plays[key]} lần nghe`;
  }
  function startPrep(button) {
    const qi = +button.dataset.qi, total = +button.dataset.prep, box = $("live-" + qi);
    if (rec || prep) return;
    const ends = Date.now() + total * 1000;
    button.disabled = true;
    const draw = () => {
      const left = Math.ceil((ends - Date.now()) / 1000);
      box.innerHTML = `<div class="ex-prep"><b class="num">${fmt(left)}</b> chuẩn bị, ghi vài ý chính ra giấy. <button class="btn small" data-x="prep-done">Nói ngay</button></div>`;
      if (left <= 0) done();
    };
    const done = () => {
      clearInterval(prep?.timer);
      prep = null;
      button.dataset.x = "rec";
      button.disabled = false;
      record(button);
    };
    prep = { timer: setInterval(draw, 500), done };
    draw();
  }
  async function record(button) {
    const qi = +button.dataset.qi;
    if (rec) return rec.qi === qi ? finishRec() : undefined;
    const box = $("live-" + qi);
    try {
      await app.call("rec_start");
    } catch (e) {
      box.innerHTML = `<span class="error">${esc(e)}</span>`;
      return;
    }
    rec = { qi, button, jobs: [], parts: [], n: 0, poll: liveWave(box) };
    rec.chunk = setInterval(() => clip(rec, true), CLIP_MS);
    rec.limit = setTimeout(() => finishRec(), +button.dataset.max * 1000);
    button.textContent = "Dừng ghi âm";
    button.classList.add("recording", "primary");
    $("heard-" + qi).textContent = "";
  }
  /// One piece of the answer: the recording stops, starts again at once when there is more, and the piece is heard.
  function clip(r, more) {
    const i = r.n++;
    const job = app.call("exam_clip", { more }).then((c) => {
      r.parts[i] = c;
      if (more && !c.recording && rec === r) finishRec();
    }, (e) => { r.error = String(e); });
    r.jobs.push(job);
    return job;
  }
  async function finishRec() {
    const r = rec;
    if (!r) return;
    rec = null;
    clearInterval(r.poll); clearInterval(r.chunk); clearTimeout(r.limit);
    const box = $("live-" + r.qi);
    box.innerHTML = `<div class="thinking small">${st("mascot-think")}<span>Đang nghe lại câu trả lời...</span></div>`;
    r.button.disabled = true;
    clip(r, false);
    await Promise.all(r.jobs);
    const said = r.parts.filter((c) => c && c.text);
    const text = said.map((c) => c.text).join(zh() ? "" : " ").trim();
    const confidence = said.length ? said.reduce((a, c) => a + c.confidence, 0) / said.length : 0;
    answers[r.qi] = text ? { text, confidence } : "";
    if (!box.isConnected) return;
    box.innerHTML = r.error && !text ? `<span class="error">${esc(r.error)}</span>` : "";
    $("heard-" + r.qi).innerHTML = text ? `<span class="muted small">Máy nghe được:</span> ${esc(text)}` : `<span class="error">Chưa nghe được gì, hãy ghi âm lại và nói gần micro hơn.</span>`;
    r.button.classList.remove("recording", "primary");
    r.button.disabled = false;
    r.button.textContent = "Ghi âm lại";
    answered();
  }

  function answered() {
    const done = answers.filter((a) => a && (typeof a === "string" ? a.trim() : a.text)).length;
    const el = $("ex-answered");
    if (el) el.textContent = `Đã làm ${done}/${answers.length} câu`;
    return done;
  }

  async function submit(auto, button, skip = false) {
    if (busy) return;
    const left = answers.length - answered();
    if (!auto && !skip && left && button && !twice(button, `Còn ${left} câu chưa làm, bấm lần nữa để nộp`)) return;
    if (rec) await finishRec();
    busy = true;
    stopAll();
    const s = section();
    const graded = !skip && (s.skill === "writing" || s.skill === "speaking");
    const foot = body.querySelector(".ex-foot");
    if (foot) foot.innerHTML = `<div class="thinking">${st("mascot-think")}<span>${graded ? "AI đang chấm theo tiêu chí của đề thi, có thể mất một phút..." : "Đang chấm..."}</span></div>`;
    try {
      ex = await app.call("exam_submit", { uid: ex.uid, section: at, answers, skip, day: localDay() });
      busy = false;
      if (ex.result && ex.finished) {
        confettiIf(ex.result.passed);
        return renderResult();
      }
      renderReview(at);
    } catch (e) {
      busy = false;
      error = String(e);
      // The answers stay; the timer goes on where it was.
      if (foot) foot.innerHTML = `<p class="error">${esc(e)}</p><span class="grow"></span><button class="btn primary big" data-x="submit">Nộp lại</button>`;
      if (s.skill !== "speaking") { $("ex-timer").classList.remove("hidden"); ticker = setInterval(tick, 1000); }
    }
  }
  const confettiIf = (yes) => { if (yes) confetti(160); };

  // A submitted section: every item with the right answer and why, the transcript of what was heard, the
  // examiner's grade for writing and speaking.
  function renderReview(i, fromResult = false) {
    screen = "review";
    at = i;
    const s = section(), r = s.result || {};
    setBar(ex.paper.title, `Phần ${i + 1}/${ex.paper.sections.length}: ${s.vi}, đã nộp`);
    const next = ex.paper.sections.findIndex((x, j) => j > i && !x.submitted);
    const score = r.skipped ? "Đã bỏ qua phần này" : [r.text, r.total ? `đúng ${r.right}/${r.total} câu` : ""].filter(Boolean).join(", ");
    let n = 0;
    body.innerHTML = `
      <div class="exam-inner">
        <div class="card ex-review-head"><div class="grow"><div class="eyebrow">Kết quả phần ${esc(s.vi)}</div><div class="ex-big">${esc(score)}</div></div>
          ${fromResult ? '<button class="btn" data-x="result">Về kết quả</button>' : next >= 0
            ? `<button class="btn primary big" data-x="next" data-i="${next}">Phần tiếp theo: ${esc(ex.paper.sections[next].vi)}</button>`
            : '<button class="btn primary big" data-x="result">Xem kết quả</button>'}</div>
        ${r.feedback ? gradeHtml(r.feedback, (r.criteria || []).map((c, j, all) => j === all.length - 1 ? `${c} (máy đo độ rõ khi nghe)` : c), "Nhận xét của giám khảo AI về phần Nói") : ""}
        ${s.parts.map((p) => `
          <div class="card ex-part"><div class="ex-part-head"><b>${esc(p.label)}</b></div>
            ${p.sets.map((set) => `<div class="ex-set">${transcriptHtml(p, set)}${set.text && !p.heard && s.skill === "reading" ? `<details class="ex-more"><summary>Xem lại bài đọc</summary><div class="ex-passage inline">${passageHtml(set.text)}${listHtml(set.list)}</div></details>` : ""}
              <div class="ex-items">${set.items.map(() => reviewItem(p, s, set, r.items?.[n], ++n)).join("")}</div></div>`).join("")}
          </div>`).join("")}
      </div>`;
    body.scrollTop = 0;
  }
  function transcriptHtml(p, set) {
    if (!p.heard && !p.heard_options) return "";
    const lines = set.lines.length ? set.lines.map((l) => `<p><b>${esc(who(l.who))}</b>${esc(l.text)}</p>`).join("") : set.text ? passageHtml(set.text) : "";
    return lines ? `<details class="ex-more" open><summary>Bản ghi bài nghe</summary><div class="ex-transcript">${lines}</div></details>` : "";
  }
  function reviewItem(p, s, set, v, n) {
    if (!v) return "";
    const said = v.say ? `<div class="ex-said">${esc(v.say).replace(/(男：|女：|M:|W:)/g, "<br><b>$1</b>").replace(/^<br>/, "")}</div>` : "";
    const q = v.q ? `<div class="ex-qtext">${esc(v.q).replace(/\n/g, "<br>")}</div>` : "";
    if (p.mode === "write") {
      return `<div class="ex-item"><div class="ex-q"><span class="ex-n">${n}</span><div class="grow">${q}</div></div>
        <div class="ex-answer"><div class="muted small">Bài của bạn, ${count(v.given)} ${unit()}${p.words ? ` (tối thiểu ${p.words})` : ""}:</div>${esc(v.given || "(chưa viết)").replace(/\n/g, "<br>")}</div>
        ${v.grade ? gradeHtml(v.grade, s.result.criteria, "Chấm theo tiêu chí của đề") : ""}</div>`;
    }
    if (p.mode === "speak") {
      const pron = v.pron !== undefined ? `<div class="small">Phát âm (máy chấm theo từng âm): <b>${v.pron}/100</b></div>` : "";
      return `<div class="ex-item"><div class="ex-q"><span class="ex-n">${n}</span><div class="grow">${q}${said}</div></div>
        <div class="ex-answer"><span class="muted small">Máy nghe được:</span> ${esc(v.given || "(không có câu trả lời)")}</div>${pron}</div>`;
    }
    const mark = v.right ? '<span class="tag ok">Đúng</span>' : '<span class="tag due">Sai</span>';
    const opts = p.mode === "choice" ? `<div class="ex-opts done">${v.options.map((o, j) => {
      const l = LETTERS[j], cls = l === v.key ? "right" : l === v.given ? "wrong" : "";
      return `<div class="${cls}"><b>${l}.</b> ${esc(o)}</div>`;
    }).join("")}</div>` : `<div class="small">Bạn trả lời: <b>${esc(entry(set, v.given) || "(bỏ trống)")}</b>. Đáp án: <b>${esc(entry(set, v.key))}</b></div>`;
    return `<div class="ex-item ${v.right ? "" : "missed"}"><div class="ex-q"><span class="ex-n">${n}</span><div class="grow">${q}${said}</div>${mark}</div>
      ${opts}${v.why ? `<div class="ex-why">${esc(v.why)}</div>` : ""}</div>`;
  }
  /// A match answer as the entry of the list it names, "B 打算" for "B".
  const entry = (set, label) => (label && set.list.find((e) => (e.trim().match(/^\(?([A-Za-z]+)/)?.[1] || "").toLowerCase() === String(label).toLowerCase())) || label;
  function gradeHtml(g, criteria, title) {
    const rate = { ielts: 9, toeic: 5 }[ex.format] || 10;
    return `<div class="ex-grade"><div class="eyebrow">${esc(title)}</div>
      <div class="ex-criteria">${(criteria || []).map((c, i) => `<div><span>${esc(c)}</span><b class="num">${g.scores?.[i] ?? "-"}/${rate}</b></div>`).join("")}</div>
      ${g.fixes?.length ? `<div class="ex-fixes"><div class="muted small">Lỗi và cách sửa</div><ul>${g.fixes.map((f) => `<li>${esc(f)}</li>`).join("")}</ul></div>` : ""}
      ${g.better ? `<details class="ex-more"><summary>Bản viết lại tốt hơn</summary><div class="ex-better">${esc(g.better).replace(/\n/g, "<br>")}</div></details>` : ""}
      ${g.note ? `<p class="ex-why">${esc(g.note)}</p>` : ""}</div>`;
  }

  // The result: each skill on the exam's scale, the estimate overall, what went well, what did not, what next.
  function renderResult() {
    stopAll();
    screen = "result";
    const r = ex.result;
    setBar(ex.paper.title, `Kết quả, ${when(ex.finished)}`);
    body.innerHTML = `
      <div class="exam-inner narrow">
        <div class="card ex-result ${r.passed ? "pass" : "fail"}">
          <div class="eyebrow">${esc(ex.paper.name)}, ước tính trên thang điểm của đề thật</div>
          <div class="ex-score ${r.text.length > 16 ? "long" : ""}">${esc(r.text)}</div>
          <div class="row">${r.passed ? '<span class="tag ok">Đạt</span>' : '<span class="tag due">Chưa đạt</span>'}<span class="muted">Cần đạt ${esc(r.pass_text)}</span></div>
          <div class="ex-skills">${r.skills.map((k, i) => `
            <button class="ex-skill" data-x="review" data-i="${i}" ${k.skipped ? "disabled" : ""}><span class="grow"><b>${esc(k.vi)}</b> <span class="muted small">${esc(k.name)}</span></span>
              ${k.skipped ? '<span class="muted small">Bỏ qua</span>' : `<span class="progress"><span style="width:${Math.round(100 * k.fraction)}%"></span></span><b class="num">${esc(k.text)}</b>`}</button>`).join("")}</div>
          <p class="muted small">Bấm vào một kỹ năng để xem lại từng câu.</p>
        </div>
        ${listCard("Điểm mạnh", r.strengths, "ok")}${listCard("Cần cải thiện", r.weaknesses, "due")}${listCard("Nên học tiếp", r.next, "")}
        ${r.wrong?.length ? `<div class="card"><details class="ex-more"><summary>${r.wrong.length} câu sai và lời giải thích</summary>
          ${r.wrong.map((w) => `<div class="ex-item missed"><div class="muted small">${esc(w.vi)}, ${esc(w.part)}</div>
            ${w.q ? `<div class="ex-qtext">${esc(w.q).replace(/\n/g, "<br>")}</div>` : ""}${w.say ? `<div class="ex-said">${esc(w.say)}</div>` : ""}
            <div class="small">Bạn trả lời: <b>${esc(answerText(w, w.given) || "(bỏ trống)")}</b>. Đáp án: <b>${esc(answerText(w, w.key))}</b></div>
            ${w.why ? `<div class="ex-why">${esc(w.why)}</div>` : ""}</div>`).join("")}</details></div>` : ""}
        <div class="row actions center">
          ${r.passed ? "" : '<button class="btn primary" data-x="again">Làm đề mới</button>'}
          <button class="btn" data-x="history">Bài thi đã làm</button><button class="btn ghost" data-x="close">Đóng</button>
        </div>
      </div>`;
    body.scrollTop = 0;
  }
  const answerText = (w, letter) => {
    const j = LETTERS.indexOf(letter);
    return w.options?.length && j >= 0 && w.options[j] ? `${letter}. ${w.options[j]}` : letter;
  };
  const listCard = (title, list, cls) => list?.length ? `<div class="card ex-list-card ${cls}"><div class="card-title">${esc(title)}</div><ul>${list.map((x) => `<li>${esc(x)}</li>`).join("")}</ul></div>` : "";

  // The exams of the roadmap.
  async function renderHistory() {
    stopAll();
    screen = "history";
    setBar("Bài thi đã làm", plan?.title || "");
    const roadmap = target?.roadmap || ex?.roadmap || "";
    let list = [];
    try {
      list = await app.call("exam_list", { roadmap });
    } catch (e) {
      toast(String(e));
    }
    body.innerHTML = `
      <div class="exam-inner narrow"><div class="card">
        <div class="card-title"><span class="grow">Bài thi của lộ trình</span>${plan ? '<button class="btn small" data-x="back">Quay lại</button>' : ""}</div>
        ${list.length ? list.map((e) => `
          <div class="ex-hist"><div class="grow"><b>${esc(e.title)}</b><div class="muted small">${esc(e.name)}, ${when(e.created)}${e.finished ? "" : `, đã nộp ${e.submitted}/${e.sections} phần`}</div></div>
            ${e.finished ? `<span class="num">${esc(e.text)}</span>${e.passed ? '<span class="tag ok">Đạt</span>' : '<span class="tag due">Chưa đạt</span>'}` : '<span class="tag new">Đang làm</span>'}
            <button class="btn small" data-x="see" data-uid="${esc(e.uid)}">${e.finished ? "Xem" : "Làm tiếp"}</button>
            <button class="btn small danger" data-x="delete" data-uid="${esc(e.uid)}">Xóa</button></div>`).join("")
          : `<div class="empty-note muted">Chưa có bài thi nào. Bài kiểm tra chặng nằm trên thẻ của mỗi chặng.</div>`}
      </div></div>`;
  }

  // Answers as they are given.
  body.addEventListener("input", (e) => {
    const qi = e.target.dataset.qi;
    if (qi === undefined || screen !== "section") return;
    answers[+qi] = e.target.value;
    const wc = $("wc-" + qi);
    if (wc) {
      const p = section().parts.find((x) => x.mode === "write"), words = count(e.target.value), min = +(wc.textContent.match(/tối thiểu (\d+)/)?.[1] || p?.words || 0);
      wc.textContent = `${words} ${unit()}${min ? `, tối thiểu ${min}` : ""}`;
      wc.classList.toggle("short", !!min && words < min);
    }
    answered();
  });
  body.addEventListener("change", (e) => {
    if (e.target.id === "ex-replay") { replay = e.target.checked; return; }
    const qi = e.target.dataset.qi;
    if (qi === undefined || screen !== "section") return;
    answers[+qi] = e.target.value;
    answered();
  });
  $("exam").addEventListener("click", async (e) => {
    const b = e.target.closest("[data-x]");
    if (!b || b.disabled) return;
    switch (b.dataset.x) {
      case "close":
        if (screen === "section" && answered() && !twice(b, "Phần này chưa nộp sẽ mất, bấm lần nữa")) return;
        return close();
      case "make": return make();
      case "cancel": app.call("exam_cancel").catch(() => {}); b.disabled = true; b.textContent = "Đang dừng..."; return;
      case "resume": case "see": return openExam(b.dataset.uid);
      case "history": return renderHistory();
      case "back": return plan ? renderIntro() : close();
      case "delete":
        if (!armed(b)) return;
        await app.call("exam_delete", { uid: b.dataset.uid, roadmap: target?.roadmap || "" }).catch((err) => toast(String(err)));
        if (plan) plan = await app.call("exam_plan", target).catch(() => plan);
        return renderHistory();
      case "play": return play(b);
      case "prep": return startPrep(b);
      case "prep-done": return prep?.done();
      case "rec": return record(b);
      case "submit": return submit(false, b);
      case "skip": return submit(false, b, true);
      case "next": at = +b.dataset.i; return startSection();
      case "result": return renderResult();
      case "review": return renderReview(+b.dataset.i, true);
      case "again":
        if (!target) target = { roadmap: ex.roadmap, stage: -1 };
        return open(target.roadmap, target.stage);
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !$("exam").classList.contains("hidden") && screen !== "section" && screen !== "making") close();
  });

  // The roadmap: a stage's test on its card, the mock and the history on the goal card, Home's "Học tiếp".
  window.openExam = (roadmap, stage) => open(roadmap, stage);
  window.examStage = (s, i, r) => {
    const t = s.test;
    if (!t || !r) return "";
    const tag = { none: '<span class="tag plain">Chưa làm</span>', open: '<span class="tag new">Đang làm dở</span>',
                  failed: `<span class="tag due">Chưa qua: ${esc(t.score)}</span>`, passed: `<span class="tag ok">Đã qua: ${esc(t.best)}</span>` }[t.state];
    const label = t.state === "open" ? "Làm tiếp" : t.state === "passed" ? "Xem bài kiểm tra" : t.full ? "Thi thử" : t.state === "failed" ? "Làm lại bài kiểm tra" : "Làm bài kiểm tra chặng";
    return `<div class="ex-stage ${t.pending ? "pending" : ""}"><div class="grow"><b>${t.full ? `Thi thử ${esc(t.name)}` : "Bài kiểm tra chặng"}</b>
        <div class="muted small">${t.pending ? "Bạn đã thuộc đủ từ, qua bài này là xong chặng. " : ""}Cần đạt ${esc(t.pass_text)}.</div></div>
      ${tag}<button class="btn small ${t.pending ? "primary" : ""}" data-exam="stage" data-r="${esc(r.uid)}" data-i="${i}">${label}</button></div>`;
  };
  window.examGoal = (r) => (r.hard ? "" : `<button class="btn" data-exam="stage" data-r="${esc(r.uid)}" data-i="-1">Thi thử</button>`)
    + `<button class="btn" data-exam="history" data-r="${esc(r.uid)}">Bài thi đã làm</button>`;
  document.addEventListener("click", (e) => {
    const b = e.target.closest("[data-exam]");
    if (!b) return;
    if (b.dataset.exam === "history") {
      target = { roadmap: b.dataset.r, stage: -1 };
      plan = null;
      show();
      return renderHistory();
    }
    open(b.dataset.r, +b.dataset.i);
  });
  hooks.course.push(() => { if (!$("exam").classList.contains("hidden")) close(); });
})();
