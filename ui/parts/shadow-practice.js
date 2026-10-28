// Shadowing ("Nói đuổi"): the app reads the word's example sentence, the learner repeats it at once, and the take is
// scored word by word, with the pitch of their voice drawn against the reading's.
(() => {
  "use strict";
  // The mode rides on the practice runner: startPractice sets up a run of this kind like any other, and showPractice
  // puts its cards here while keeping the progress, the results screen, the retry and the skipping of words.
  exercises.shadow = { show };

  // ponytail: fixed levels; measure the room's noise first if a loud fan keeps takes from ending.
  /// Louder than this (root mean square) is the learner speaking; a take ends QUIET_MS after they stop, or after
  /// WAIT_MS if they never start.
  const VOICE = 0.02, QUIET_MS = 1000, WAIT_MS = 5000;
  /// The stop of the last recording, which the next take waits for, since the microphone opens once at a time.
  let stopped = Promise.resolve();

  function show(w, card, p) {
    const zh = (w.lang || "en") === "zh";
    const short = w.example && (zh ? w.example.length <= 30 : w.example.split(/\s+/).length <= 20);
    const sentence = short ? w.example : w.word;
    const reading = short ? (zh ? w.example_reading : w.example_vi) : w.ipa;
    card.innerHTML = `
      ${st("mode-speak", "mode-st shadow-st")}
      <div class="prompt-label">Nghe câu mẫu, thấy chữ Nói ngay thì nói theo</div>
      <div class="shadow-line">${esc(sentence)}</div>
      ${reading ? `<div class="${short && !zh ? "muted" : "ipa"}">${esc(reading)}</div>` : ""}
      <div class="muted small">${esc(w.word)}: ${esc(w.meaning)}</div>
      <div class="pron shadow-box"></div>
      <div class="grades">
        <button class="btn hidden" data-a="again">Nói lại</button>
        <button class="btn hidden" data-a="listen">Nghe lại</button>
        <button class="btn" data-a="skip">Bỏ qua</button>
        <button class="btn primary hidden" data-a="next">Câu tiếp</button>
      </div>`;
    const box = card.querySelector(".shadow-box"), line = card.querySelector(".shadow-line");
    const button = (a) => card.querySelector(`[data-a="${a}"]`);
    const alive = () => practice === p && box.isConnected;
    const kokoro = !!state.voices?.[zh ? "zh" : "en"]?.ready;
    const longest = Math.min(15000, 2500 + (zh ? 400 * sentence.length : 500 * sentence.split(/\s+/).length));
    let first = true, busy = false;

    // The reading: with the Kokoro voice, whose pitch the take is held against, or else the system's voice.
    const read = async () => {
      if (kokoro) {
        const r = await app.call("shadow_say", { text: sentence, rate: +state.settings.speech_rate || 0.95 }).catch(() => null);
        if (r) return r.reference;
      }
      await speak(sentence);
      return null;
    };
    const settle = () => {
      busy = false;
      button("again").classList.remove("hidden");
      button("listen").classList.remove("hidden");
      button("skip").classList.toggle("hidden", !first);
      button("next").classList.toggle("hidden", first);
      if (!first) button("next").focus();
    };
    const fail = (e) => {
      if (!alive()) return;
      box.innerHTML = `<span class="error">${esc(e)}</span>`;
      settle();
    };
    const take = async () => {
      if (busy) return;
      busy = true;
      button("again").classList.add("hidden");
      button("listen").classList.add("hidden");
      box.innerHTML = `<div class="shadow-cue muted">Đang đọc câu mẫu...</div>`;
      const reference = await read();
      await stopped;
      if (!alive()) return;
      try {
        await app.call("rec_start");
      } catch (e) {
        return fail(e);
      }
      box.innerHTML = `<div class="shadow-cue">Nói ngay!</div><div class="shadow-wave"></div>
        <div class="row center"><button class="btn small recording" data-a="stop">Dừng và chấm điểm</button></div>`;
      const wave = liveWave(box.querySelector(".shadow-wave"));
      await spoken(box.querySelector('[data-a="stop"]'), longest, alive);
      clearInterval(wave);
      const scoring = app.call("shadow_stop", { target: sentence, reference });
      stopped = scoring.catch(() => {});
      if (!alive()) return;
      box.innerHTML = `<div class="shadow-cue muted">Đang chấm điểm...</div>`;
      let r;
      try {
        r = await scoring;
      } catch (e) {
        return fail(e);
      }
      if (!alive()) return;
      result(box, line, r, zh);
      // Only the first take counts; repeating it is still good practice.
      if (first) {
        first = false;
        await answered(w, r.score.score >= 75, box, { grade: false });
      }
      settle();
    };
    button("again").onclick = take;
    button("listen").onclick = async () => {
      if (busy) return;
      busy = button("listen").disabled = true;
      await read();
      busy = button("listen").disabled = false;
    };
    button("skip").onclick = async () => {
      if (first) { first = false; await answered(w, false, null, { grade: false }); }
      next();
    };
    button("next").onclick = next;
    take();
  }

  /// Waits until the learner has spoken and gone quiet, presses stop, runs out of time, or leaves the card.
  function spoken(stop, longest, alive) {
    return new Promise((resolve) => {
      const started = Date.now();
      let voice = 0, asking = false;
      const end = () => { clearInterval(timer); resolve(); };
      const timer = setInterval(async () => {
        if (!alive()) return end();
        if (asking) return;
        asking = true;
        const level = await app.call("rec_level").catch(() => 0);
        asking = false;
        const now = Date.now();
        if (level > VOICE) voice = now;
        if (now - started > longest || (voice && now - voice > QUIET_MS) || (!voice && now - started > WAIT_MS)) end();
      }, 100);
      stop.onclick = end;
    });
  }

  /// The score of the words, each marked heard or not in the sentence, and the two tunes with what to change.
  function result(box, line, r, zh) {
    const s = r.score, color = s.score >= 75 ? "var(--green)" : s.score >= 50 ? "var(--orange)" : "var(--red)";
    line.innerHTML = r.words.map((x) => (x.missed ? `<mark class="miss">${esc(x.text)}</mark>` : `<span class="shadow-heard">${esc(x.text)}</span>`)).join(zh ? "" : " ");
    const tunes = r.reference || r.learner;
    box.innerHTML = `
      <div class="row"><span class="score" style="color:${color}"><span class="num">0</span>/100</span><span>${esc(s.message)}</span></div>
      <div class="meter"><div style="background:${color}"></div></div>
      <div class="muted small">Máy nghe được: <b>${esc(s.heard || "(không rõ)")}</b></div>
      ${tunes ? `<div class="shadow-head"><b>${zh ? "Thanh điệu" : "Ngữ điệu"}</b>
        ${r.reference ? '<span class="shadow-key ref">Giọng mẫu</span>' : ""}${r.learner ? '<span class="shadow-key">Giọng bạn</span>' : ""}
        ${r.similar != null ? `<span class="tag plain">Giống mẫu ${r.similar}%</span>` : ""}</div>
      <div class="shadow-chart"><canvas role="img" aria-label="Đường lên xuống giọng của câu"></canvas></div>` : ""}
      <p class="shadow-feedback">${esc(r.feedback)}</p>
      ${r.reference ? "" : '<button class="btn small" data-a="settings">Mở Cài đặt</button>'}`;
    countUp(box.querySelector(".num"), s.score);
    requestAnimationFrame(() => (box.querySelector(".meter div").style.width = s.score + "%"));
    box.querySelector('[data-a="settings"]')?.addEventListener("click", () => go("settings"));
    const canvas = box.querySelector("canvas");
    // Drawn again whenever the window, and so the card, changes width.
    if (canvas) new ResizeObserver(() => draw(canvas, r, zh)).observe(canvas.parentElement);
  }

  /// The tunes in semitones around each voice's middle, over the words of the sentence: the reading's dashed, the
  /// learner's solid. Chinese shows each character's pinyin under it, for its tone.
  function draw(canvas, r, zh) {
    const width = canvas.parentElement.clientWidth, height = 170, ratio = devicePixelRatio || 1;
    if (!width) return;
    canvas.width = width * ratio;
    canvas.height = height * ratio;
    canvas.style.height = height + "px";
    const g = canvas.getContext("2d");
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    const css = getComputedStyle(canvas), color = (name) => css.getPropertyValue(name).trim(), font = css.fontFamily;
    const left = 30, right = 6, top = 8, bottom = zh ? 42 : 24;
    const tunes = [r.reference, r.learner].filter(Boolean);
    // At least half an octave up and down, more when a voice goes further.
    const span = 3 * Math.ceil(Math.max(6, ...tunes.flat().filter((v) => v != null).map(Math.abs)) / 3);
    const x = (t) => left + t * (width - left - right);
    const y = (v) => top + ((span - Math.max(-span, Math.min(span, v))) / (2 * span)) * (height - top - bottom);
    const floor = y(-span);
    g.lineWidth = 1;
    g.strokeStyle = color("--line");
    for (const w of r.words.slice(1)) {
      g.beginPath();
      g.moveTo(x(w.from), top);
      g.lineTo(x(w.from), floor);
      g.stroke();
    }
    // The middle of each voice.
    g.strokeStyle = color("--line-strong");
    g.setLineDash([2, 4]);
    g.beginPath();
    g.moveTo(left, y(0));
    g.lineTo(width - right, y(0));
    g.stroke();
    g.fillStyle = color("--faint");
    g.font = `11px ${font}`;
    g.textAlign = "left";
    g.textBaseline = "top";
    g.fillText("cao", 0, top);
    g.textBaseline = "bottom";
    g.fillText("thấp", 0, floor);
    // The words under their stretch of time, missed ones in red; one that would overlap the last is left out.
    g.textAlign = "center";
    g.textBaseline = "top";
    let end = -Infinity;
    for (const w of r.words) {
      const mid = x((w.from + w.to) / 2);
      g.font = zh ? `600 15px ${font}` : `500 12px ${font}`;
      const half = g.measureText(w.text).width / 2;
      if (mid - half < end + 4) continue;
      end = mid + half;
      g.fillStyle = color(w.missed ? "--red" : "--text-2");
      g.fillText(w.text, mid, floor + 6);
      g.font = `11px ${font}`;
      if (w.reading && g.measureText(w.reading).width <= x(w.to) - x(w.from) + 2) {
        g.fillStyle = color("--muted");
        g.fillText(w.reading, mid, floor + 25);
      }
    }
    g.lineWidth = 2.5;
    g.lineJoin = g.lineCap = "round";
    const tune = (c, stroke, dash) => {
      g.strokeStyle = stroke;
      g.setLineDash(dash);
      g.beginPath();
      let pen = false;
      c.forEach((v, i) => {
        if (v == null) return (pen = false);
        g[pen ? "lineTo" : "moveTo"](x(i / (c.length - 1)), y(v));
        pen = true;
      });
      g.stroke();
    };
    if (r.reference) tune(r.reference, color("--shadow-ref"), [7, 5]);
    if (r.learner) tune(r.learner, color("--shadow-you"), []);
  }
})();
