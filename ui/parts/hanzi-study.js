// Chinese characters: the order of their strokes and what they are made of, in the details of a word, and an
// exercise that has them written stroke by stroke. Drawing and checking is Hanzi Writer's (hanzi-0-writer.js); the
// strokes and the parts come from Rust, which keeps the strokes in the data dir after the first download.
(() => {
  "use strict";
  const isHan = (c) => /\p{Script=Han}/u.test(c);
  const unique = (word) => [...new Set([...word].filter(isHan))];

  const partsCache = new Map();
  /// The radical and components of each character of a word, asked once per word.
  function partsOf(word) {
    if (!partsCache.has(word)) partsCache.set(word, app.call("hanzi_parts", { word }).catch((e) => { partsCache.delete(word); throw e; }));
    return partsCache.get(word);
  }
  /// The stroke data of the characters of a word that have some, by character; null for one the data lacks.
  const strokesCache = new Map();
  async function strokesOf(word) {
    const missing = unique(word).filter((c) => !strokesCache.has(c));
    if (missing.length) {
      const got = await app.call("hanzi_strokes", { chars: missing.join("") });
      for (const c of missing) strokesCache.set(c, got[c] || null);
    }
    return Object.fromEntries(unique(word).map((c) => [c, strokesCache.get(c)]));
  }

  /// Hanzi Writer parses colours itself, so the theme's are read as values rather than given as var().
  function colors() {
    const css = getComputedStyle(document.documentElement), v = (name) => css.getPropertyValue(name).trim();
    return { strokeColor: v("--text"), outlineColor: v("--line-strong"), highlightColor: v("--accent"), drawingColor: v("--accent") };
  }
  function makeWriter(el, char, data, size, options) {
    el.textContent = "";
    return HanziWriter.create(el, char, { width: size, height: size, padding: 4, charDataLoader: (c, done) => done(data[c]), ...colors(), ...options });
  }
  const GRID = `<svg class="hz-grid" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true"><path d="M50 0V100M0 50H100M0 0L100 100M100 0L0 100"/></svg>`;

  const part = (r) => `<span class="hz-part"><b>${esc(r.char)}</b>${r.pinyin ? ` <span class="hz-py">${esc(r.pinyin)}</span>` : ""}${r.hanviet ? ` <span class="muted">(${esc(r.hanviet)})</span>` : ""}</span>`;
  /// One character in a few words: 好 = 女 (nữ) + 子 (tử), or its radical when its parts are not all known.
  function shortLine(c) {
    const read = (r) => `${esc(r.char)}${r.hanviet ? ` (${esc(r.hanviet)})` : ""}`;
    if (c.parts.length >= 2 && !c.partial) return `${esc(c.char)} = ${c.parts.map(read).join(" + ")}`;
    if (c.radical && c.radical.char !== c.char) return `${esc(c.char)}: bộ ${read(c.radical)}`;
    return c.radical ? `${esc(c.char)} là bộ thủ` : "";
  }

  // Each word's details end with its components on one quiet line, and, in the word's own dialog, with a box for
  // the order of the strokes and the parts in full. Both are filled once on the page.
  hooks.detail.push((w) => {
    if (w.lang !== "zh" || !unique(w.word).length) return "";
    queueMicrotask(fill);
    return `<div class="hz-line" data-hz="${esc(w.word)}"></div><div class="pron hz-box" data-hz="${esc(w.word)}"></div>`;
  });
  function fill() {
    for (const el of document.querySelectorAll("[data-hz]:not([data-hz-done])")) {
      el.dataset.hzDone = "1";
      if (el.classList.contains("hz-line")) {
        partsOf(el.dataset.hz).then((chars) => {
          el.innerHTML = chars.map(shortLine).filter(Boolean).join("; ");
          el.title = chars.flatMap((c) => c.parts).map((p) => `${p.char} ${p.pinyin}`.trim()).join(", ");
        }).catch(() => {});
      } else if (el.closest(".word-dialog")) {
        strokeBox(el, el.dataset.hz);
      }
    }
  }

  async function strokeBox(box, word) {
    let chars;
    try {
      chars = await partsOf(word);
    } catch (e) {
      return;
    }
    const byChar = new Map(chars.map((c) => [c.char, c]));
    const list = unique(word);
    box.innerHTML = `
      <div class="pron-head"><b>Thứ tự nét</b><span class="muted small">Bấm vào một chữ để xem lại cách viết.</span></div>
      <div class="hz-tiles">${list.map((c, i) => {
        const r = byChar.get(c);
        return `<button class="hz-tile" data-i="${i}" aria-label="Xem cách viết chữ ${esc(c)}"><span class="hz-draw">${GRID}<span class="hz-glyph">${esc(c)}</span></span>
          <span class="hz-cap">${r ? esc([r.pinyin, r.hanviet].filter(Boolean).join(" · ")) : ""}</span></button>`;
      }).join("")}</div>
      <div class="row hz-again"><button class="btn small" data-hz-replay>Xem lại</button><span class="muted small" data-hz-status></span></div>
      <div class="pron-head hz-sub"><b>Bộ thủ và thành phần</b></div>
      <div class="hz-rows">${chars.filter((c, i) => chars.findIndex((x) => x.char === c.char) === i).map((c) => `
        <div class="hz-row"><span class="hz-char">${esc(c.char)}</span><div class="grow">
          <div>${c.parts.length ? `${c.parts.map(part).join(" + ")}${c.partial ? ` <span class="muted">+ phần khác</span>` : ""}`
            : `<span class="muted">${c.radical && c.radical.char === c.char ? "Chữ này là một bộ thủ" : "Chưa có dữ liệu thành phần"}</span>`}</div>
          ${c.radical ? `<div class="muted small">Bộ thủ: ${part(c.radical)}</div>` : ""}</div></div>`).join("")}</div>`;
    const status = box.querySelector("[data-hz-status]");
    let data;
    try {
      data = await strokesOf(word);
    } catch (e) {
      status.textContent = String(e);
      box.querySelector("[data-hz-replay]").classList.add("hidden");
      return;
    }
    if (!box.isConnected) return;
    const lacking = list.filter((c) => !data[c]);
    if (lacking.length) status.textContent = `Chưa có nét viết của chữ ${lacking.join(", ")}.`;
    // Drawn once at once without motion; otherwise each character draws itself in turn.
    const still = reducedMotion;
    const writers = list.map((c, i) => data[c] && makeWriter(box.querySelectorAll(".hz-glyph")[i], c, data, 88,
      { showCharacter: still, radicalColor: colors().highlightColor, strokeAnimationSpeed: 1.3, delayBetweenStrokes: 250 }));
    let run = 0;
    const play = async (only) => {
      const mine = ++run;
      for (const writer of only ? [only] : writers) {
        if (!writer) continue;
        if (mine !== run || !box.isConnected) return;
        await new Promise((done) => writer.animateCharacter({ onComplete: done }));
      }
    };
    box.querySelector("[data-hz-replay]").onclick = () => play();
    box.querySelectorAll(".hz-tile").forEach((t) => (t.onclick = () => writers[+t.dataset.i] && play(writers[+t.dataset.i])));
    if (!still) play();
  }

  // The exercise: the meaning and pinyin are shown, and each character is written stroke by stroke. A word is right
  // when no character took more than two wrong strokes and no hint was needed.
  const menu = document.querySelector("#practice-menu .modes");
  menu.insertAdjacentHTML("beforeend", `<button class="mode hz-mode">${st("mascot-zh")}<b>Viết chữ Hán</b>
    <span>Xem nghĩa và pinyin rồi viết từng nét chữ Hán bằng chuột hoặc cảm ứng.</span><em>Bắt đầu</em></button>`);
  menu.lastElementChild.onclick = () => startPractice("hanzi");
  exercises.hanzi = {
    show: (w, card, p) => writeWord(p, w, card),
    fits: (w) => [...w.word].some(isHan),
    none: "Chưa có từ tiếng Trung nào có nghĩa để luyện viết.",
  };

  async function writeWord(p, w, card) {
    const chars = [...w.word].filter(isHan);
    card.innerHTML = `
      ${st("mascot-zh", "mode-st hz-st")}
      <div class="prompt-label">Viết chữ Hán có nghĩa</div>
      <div class="question">${esc(w.meaning)}</div>
      <div class="ipa">${esc(w.ipa)}${w.hanviet ? ` <span class="muted">(${esc(w.hanviet)})</span>` : ""}</div>
      <div class="hz-slots">${chars.map(() => `<span class="hz-slot"></span>`).join("")}</div>
      <div class="hz-pad">${GRID}<div class="hz-quiz"></div></div>
      <div class="muted small hz-status" aria-live="polite">Đang tải nét chữ...</div>
      <div id="hz-result"></div>
      <div class="grades">
        <button class="btn" id="hz-hint">Gợi ý</button><button class="btn" id="hz-show">Không nhớ</button>
        <button class="btn hidden" id="hz-retry">Thử lại</button><button class="btn primary hidden" id="hz-next">Câu tiếp</button>
      </div>`;
    const $$ = (s) => card.querySelector(s), status = $$(".hz-status"), slots = card.querySelectorAll(".hz-slot");
    $$("#hz-next").onclick = next;
    const after = p.items[p.at + 1];
    let data;
    try {
      data = await strokesOf(w.word);
    } catch (e) {
      if (practice !== p) return;
      status.textContent = String(e);
      status.classList.replace("muted", "error");
      ["#hz-hint", "#hz-show", ".hz-pad", ".hz-slots"].forEach((s) => $$(s).classList.add("hidden"));
      $$("#hz-retry").classList.remove("hidden");
      $$("#hz-retry").onclick = () => writeWord(p, w, card);
      return;
    }
    if (practice !== p || p.items[p.at] !== w) return;
    if (after) strokesOf(after.word).catch(() => {});
    if (!chars.some((c) => data[c])) {
      // Nothing to write: the word leaves this run without counting.
      p.items.splice(p.at, 1);
      return p.items.length ? showPractice() : quitPractice();
    }
    // As big as the card and the window leave room for, with the buttons still in sight.
    const size = Math.max(160, Math.min(260, card.clientWidth - 48, innerHeight - 420));
    const pad = $$(".hz-pad");
    // The border of the pad goes around the drawing.
    pad.style.width = pad.style.height = size + 2 + "px";
    const writer = makeWriter($$(".hz-quiz"), chars.find((c) => data[c]), data, size,
      { padding: 10, showCharacter: false, showOutline: false, drawingWidth: 6, strokeAnimationSpeed: 1.5, delayBetweenStrokes: 200 });
    // `helped`: the character being written needed a hint; `hinted`: some character of the word did.
    let at = -1, helped = false, hinted = false, worst = 0, total = 0;
    const done = (i, mistakes) => {
      slots[i].textContent = chars[i];
      slots[i].classList.add(mistakes > 2 ? "bad" : "good");
    };
    const quiz = (traced) => {
      if (traced) writer.showOutline(); else writer.hideOutline();
      status.textContent = `Chữ ${at + 1}/${chars.length}: ${traced ? "viết lại theo nét mờ." : "viết từng nét theo đúng thứ tự."}`;
      writer.quiz({
        onMistake: (s) => {
          if (s.mistakesOnStroke >= 3) helped = hinted = true;
          status.textContent = s.mistakesOnStroke >= 3 ? "Nét cần viết đang sáng lên, hãy viết theo." : `Chưa đúng nét này (sai ${s.totalMistakes} lần), thử lại nhé.`;
        },
        onCorrectStroke: (s) => { if (s.strokesRemaining) status.textContent = `Đúng rồi, còn ${s.strokesRemaining} nét.`; },
        onComplete: (s) => {
          worst = Math.max(worst, s.totalMistakes);
          total += s.totalMistakes;
          done(at, helped ? 3 : s.totalMistakes);
          setTimeout(nextChar, 500);
        },
      });
    };
    const nextChar = async () => {
      if (practice !== p) return;
      at++;
      // A character the data lacks is shown as it is.
      while (at < chars.length && !data[chars[at]]) done(at++, 0);
      if (at >= chars.length) return finish();
      slots.forEach((s, i) => s.classList.toggle("now", i === at));
      $$("#hz-hint").disabled = false;
      helped = false;
      await writer.setCharacter(chars[at]);
      quiz(false);
    };
    $$("#hz-hint").onclick = () => {
      hinted = helped = true;
      writer.showOutline();
      $$("#hz-hint").disabled = true;
    };
    // The character draws itself, then is written again over its outline.
    $$("#hz-show").onclick = () => {
      hinted = helped = true;
      writer.cancelQuiz();
      writer.animateCharacter({ onComplete: () => { if (practice === p) { writer.hideCharacter(); quiz(true); } } });
    };
    const finish = async () => {
      slots.forEach((s) => s.classList.remove("now"));
      const ok = !hinted && worst <= 2;
      await answered(w, ok, pad, { grade: false });
      $$("#hz-result").innerHTML = (ok ? verdict(true, "")
        : `<div class="error">${st("fb-wrong", "fb")}<span>Viết xong rồi, nhưng cần luyện thêm: sai ${total} nét${hinted ? ", có xem gợi ý" : ""}.</span></div>`)
        + `<div class="muted">${esc(w.word)} ${esc(w.ipa)}: ${esc(w.meaning)}</div>`;
      status.classList.add("hidden");
      ["#hz-hint", "#hz-show"].forEach((s) => $$(s).classList.add("hidden"));
      $$("#hz-next").classList.remove("hidden");
      $$("#hz-next").focus();
      speak(w.word);
    };
    nextChar();
  }
})();
