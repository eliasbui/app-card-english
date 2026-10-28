(() => {
  "use strict";
  const PAIRS = 6;
  const seconds = (ms) => (ms / 1000).toFixed(1).replace(".", ",");

  /// Nối từ: six words and their meanings as tiles, against the clock. A tap picks a tile, a tap on the other side
  /// matches it; the best time of each course is kept.
  function start() {
    const p = practice, card = $("practice-card");
    // The source's words first, then others, with meanings that differ so that every tile has a single pair.
    const items = [];
    for (const w of [...p.items, ...shuffle(p.all.slice())]) {
      if (items.length < PAIRS && !items.some((x) => x.id === w.id || x.meaning === w.meaning)) items.push(w);
    }
    p.items = items;
    const best = +state.settings.match_best || 0;
    const tile = (w, side) => `<button class="drill-tile" data-id="${w.id}" data-side="${side}">${esc(side === "word" ? w.word : w.meaning)}</button>`;
    card.innerHTML = `
      <div class="row drill-match-head"><span class="prompt-label grow">Chạm một từ rồi chạm nghĩa của nó</span><b class="num" id="match-time">0,0 s</b></div>
      <div class="drill-match">
        <div class="drill-col">${shuffle(items.slice()).map((w) => tile(w, "word")).join("")}</div>
        <div class="drill-col">${shuffle(items.slice()).map((w) => tile(w, "meaning")).join("")}</div>
      </div>
      ${best ? `<div class="muted small">Kỷ lục: ${seconds(best)} giây</div>` : ""}`;
    const progress = (n) => {
      $("practice-bar").style.width = (100 * n) / items.length + "%";
      $("practice-count").textContent = `${n}/${items.length}`;
    };
    progress(0);
    const started = performance.now();
    const timer = setInterval(() => {
      if (practice !== p) return clearInterval(timer);
      $("match-time").textContent = seconds(performance.now() - started) + " s";
    }, 100);
    let picked = null, matched = 0;
    card.querySelectorAll(".drill-tile").forEach((b) => (b.onclick = () => {
      if (b.disabled) return;
      picked?.classList.remove("picked");
      if (!picked || picked === b || picked.dataset.side === b.dataset.side) {
        picked = picked === b ? null : b;
        picked?.classList.add("picked");
        return;
      }
      const a = picked;
      picked = null;
      const w = items.find((x) => x.id === +(a.dataset.side === "word" ? a : b).dataset.id);
      if (a.dataset.id === b.dataset.id) {
        a.disabled = b.disabled = true;
        a.classList.add("right");
        b.classList.add("right");
        progress(++matched);
        answered(w, true, b, { still: true });
        if (matched === items.length) finish(p, performance.now() - started, best, timer);
      } else {
        for (const t of [a, b]) { t.classList.remove("wrong"); void t.offsetWidth; t.classList.add("wrong"); }
        setTimeout(() => [a, b].forEach((t) => t.classList.remove("wrong")), 500);
        answered(w, false, null, { grade: false, still: true });
      }
    }));
  }

  function finish(p, ms, best, timer) {
    clearInterval(timer);
    // Only a full board makes a record, so that records stay comparable.
    const record = p.items.length === PAIRS && (!best || ms < best);
    if (record) app.call("set_setting", { key: "match_best", value: String(Math.max(1, Math.round(ms))) }).then((s) => (state.settings = s)).catch(() => {});
    const missed = [...new Map(p.wrong.map((w) => [w.id, w])).values()];
    const card = $("practice-card");
    slideIn(card);
    card.innerHTML = `<div class="done">
      ${st(record ? "mascot-cheer" : "mascot")}
      <div class="prompt-label">${record ? "Kỷ lục mới!" : "Thời gian của bạn"}</div>
      <div class="done-score">${seconds(ms)}<span> giây</span></div>
      <p>${best ? `${record ? "Kỷ lục cũ" : "Kỷ lục"}: <b>${seconds(best)} giây</b>. ` : ""}Nhận <b>${p.xp} XP</b>, nối sai <b>${p.wrong.length}</b> lần.</p>
      ${missed.length ? `<div class="prompt-label">Cần xem lại</div><div class="review">${missed.map((x) => `<span class="tag plain">${esc(x.word)}</span>`).join("")}</div>` : ""}
      <div class="row"><button class="btn primary" id="btn-again">Chơi lại</button><button class="btn" id="btn-back">Chọn bài khác</button></div></div>`;
    $("btn-again").onclick = () => startPractice("match");
    $("btn-back").onclick = quitPractice;
    if (record) confetti(150);
  }

  exercises.match = { start, size: PAIRS, min: PAIRS, none: `Cần ít nhất ${PAIRS} từ có nghĩa để chơi Nối từ.` };
})();
