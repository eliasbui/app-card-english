(() => {
  "use strict";
  /// The tiles of a word's example: English by its spaces, Chinese by its characters but with every occurrence of the
  /// word kept whole. Punctuation goes with the tile before it, or with the first one when it opens the sentence.
  function tilesOf(w) {
    const zh = (w.lang || "en") === "zh";
    const parts = zh ? w.example.split(w.word).flatMap((s, i) => (i ? [w.word, ...s] : [...s])) : w.example.split(/\s+/);
    const tiles = [];
    let lead = "";
    for (const t of parts.filter((t) => t.trim())) {
      if (/[\p{L}\p{N}]/u.test(t)) { tiles.push(lead + t); lead = ""; }
      else if (tiles.length) tiles[tiles.length - 1] += t;
      else lead += t;
    }
    return tiles;
  }
  const joined = (w, tiles) => tiles.join((w.lang || "en") === "zh" ? "" : " ");

  function show(w, card) {
    const tiles = tilesOf(w), right = joined(w, tiles);
    const order = tiles.map((_, i) => i);
    // Shuffled, but never left in the right order when another one exists.
    for (let n = 0; n < 8 && joined(w, order.map((i) => tiles[i])) === right; n++) shuffle(order);
    card.innerHTML = `
      ${st("nav-words", "mode-st")}
      <div class="prompt-label">Chạm vào các mảnh để xếp thành câu đúng</div>
      <div class="muted">Câu ví dụ của <b>${esc(w.word)}</b>: ${esc(w.meaning)}</div>
      <div class="drill-line" id="order-line"></div>
      <div class="drill-tiles" id="order-pool">${order.map((i) => `<button class="drill-tile" data-i="${i}">${esc(tiles[i])}</button>`).join("")}</div>
      <div class="grades"><button class="btn primary" id="btn-check" disabled>Kiểm tra</button></div>
      <div id="fill-result"></div>`;
    const line = $("order-line"), pool = $("order-pool"), check = $("btn-check");
    card.querySelectorAll(".drill-tile").forEach((b) => (b.onclick = () => {
      if (b.disabled) return;
      (b.parentNode === pool ? line : pool).append(b);
      check.disabled = pool.children.length > 0;
    }));
    let done = false;
    check.onclick = async () => {
      if (done) return next();
      done = true;
      const placed = [...line.children];
      const given = joined(w, placed.map((b) => tiles[b.dataset.i]));
      const ok = given === right;
      placed.forEach((b, i) => { b.disabled = true; b.classList.add(tiles[b.dataset.i] === tiles[i] ? "right" : "wrong"); });
      // Word order is grammar, not forgetting the word.
      await answered(w, ok, check, { grade: false });
      const zh = (w.lang || "en") === "zh";
      $("fill-result").innerHTML = verdict(ok, esc(w.example))
        + (zh && w.example_reading ? `<div class="muted">${esc(w.example_reading)}</div>` : "")
        + (w.example_vi ? `<div class="muted">Dịch: ${esc(w.example_vi)}</div>` : "")
        + `<div class="row center" style="margin-top:8px"><button class="btn small" data-say="${esc(w.example)}">Nghe câu</button></div>`;
      if (!ok) whyWrong($("fill-result"), w, w.example, given, "Put the shuffled words in order to make the sentence");
      check.textContent = "Câu tiếp";
      check.focus();
      speak(w.example);
    };
  }

  exercises.order = {
    show,
    fits: (w) => !!w.example && tilesOf(w).length >= 3 && tilesOf(w).length <= 16,
    none: "Chưa có từ nào có câu ví dụ vừa độ dài để xếp. Hãy thêm câu ví dụ cho vài từ trước.",
  };
})();
