(() => {
  "use strict";
  // Reading: a text pasted or opened from a file, every word coloured by where it stands in the learner's list of the
  // course. A click on a word tells what is known of it at once, reads it aloud, and adds it to the list with the
  // sentence it was in as its example; the AI explains the rest in the background.

  /// The most of a text the reader keeps, in UTF-16 units: at most three bytes each, under the app's limit.
  const MAX_TEXT = 60000;
  /// The longest phrase of the list looked for in the text, in words (English) or pieces (Chinese).
  const MAX_PHRASE = 4;
  const STATES = { unknown: "Chưa có trong danh sách", learning: "Đang học", known: "Đã thuộc", skipped: "Đã bỏ qua" };
  /// What the AI's explanation brings to a word added here; the example is the sentence of the text, and the backend
  /// gives the readings.
  const FROM_AI = ["pos", "meaning", "explain", "synonyms", "antonyms", "collocations", "level"];
  // English: letters with the apostrophes and hyphens inside a word, numbers, and everything else.
  const TOKEN = /([\p{L}\p{M}]+(?:['’-][\p{L}\p{M}]+)*)|(\p{N}+(?:[.,]\p{N}+)*)|([^\p{L}\p{M}\p{N}]+)/gu;

  const reader = { course: null, text: "", tokens: [], words: new Map(), pop: null };
  const zh = () => courseOf() === "zh";
  const keyOf = (w) => (zh() ? w : w.toLowerCase().replace(/’/g, "'"));

  // The mode in the exercises, in a group of its own before the note under them.
  const menu = $("practice-menu");
  const note = [...menu.children].find((c) => c.tagName === "P");
  (note || menu.lastElementChild).insertAdjacentHTML(note ? "beforebegin" : "afterend", `
    <h3 class="group">Học từ ngữ cảnh</h3>
    <div class="modes">
      <button class="mode" id="mode-reader">${st("nav-words")}<b>Đọc hiểu</b><span>Đọc văn bản hay phụ đề, từ được tô màu theo danh sách. Bấm vào từ để xem nghĩa và thêm.</span><em>Mở trình đọc</em></button>
    </div>`);
  $("mode-reader").onclick = () => go("reader");

  document.querySelector("main").insertAdjacentHTML("beforeend", `
    <section id="page-reader">
      <header class="page-head">
        <div class="grow"><h2>Đọc hiểu</h2><p>Bấm vào một từ để xem nghĩa, nghe đọc và thêm vào danh sách học.</p></div>
        <button class="btn" id="rd-back">Về Bài tập</button>
      </header>
      <div class="card" id="rd-edit">
        <textarea id="rd-input" rows="9" maxlength="${MAX_TEXT}" aria-label="Văn bản để đọc"></textarea>
        <div class="row actions">
          <button class="btn primary" id="rd-read">Đọc</button>
          <button class="btn" id="rd-open">Mở file .txt, .srt, .vtt</button>
          <button class="btn ghost hidden" id="rd-cancel">Hủy</button>
          <span class="muted small" id="rd-count"></span>
        </div>
        <p class="error" id="rd-error"></p>
      </div>
      <div id="rd-view" class="hidden">
        <div class="rd-bar">
          <div class="grow"><b id="rd-summary"></b><div class="rd-legend" id="rd-legend"></div></div>
          <button class="btn small" id="rd-change">Sửa văn bản</button>
          <button class="btn small" id="rd-open-2">Mở file</button>
        </div>
        <div class="card rd-text" id="rd-text" tabindex="0" aria-label="Văn bản. Dùng phím mũi tên để chuyển giữa các từ, Enter để xem từ."></div>
      </div>
      <input type="file" id="rd-file" accept=".txt,.srt,.vtt,text/plain,text/vtt" hidden>
    </section>`);
  document.body.insertAdjacentHTML("beforeend", `<div class="rd-pop hidden" id="rd-pop" role="dialog" aria-label="Từ trong văn bản"></div>`);

  /// The words of a subtitle file without the numbers and times: the lines of a cue together, a cue to a line.
  function subtitles(raw) {
    const cues = [];
    for (const block of raw.replace(/\r\n?/g, "\n").split(/\n[ \t]*\n/)) {
      const lines = block.split("\n").map((l) => l.trim()).filter(Boolean);
      const at = lines.findIndex((l) => l.includes("-->"));
      if (at < 0) continue;
      const text = lines.slice(at + 1).join(" ").replace(/<[^>]*>/g, "").replace(/\{\\[^}]*\}/g, "")
        .replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
      // Captions made as they are spoken repeat a line from one cue to the next.
      if (text && text !== cues[cues.length - 1]) cues.push(text);
    }
    return cues.join("\n");
  }

  /// The forms a word of the text may have in the list: itself, or without a plural, past or -ing ending.
  function forms(w) {
    const out = [w];
    const add = (s) => s.length > 1 && !out.includes(s) && out.push(s);
    if (w.endsWith("'s")) add(w.slice(0, -2));
    if (w.endsWith("ies") || w.endsWith("ied")) add(w.slice(0, -3) + "y");
    if (w.endsWith("es")) add(w.slice(0, -2));
    if (w.endsWith("s") && !w.endsWith("ss")) add(w.slice(0, -1));
    for (const end of ["ed", "ing"]) {
      if (!w.endsWith(end)) continue;
      const stem = w.slice(0, -end.length);
      add(stem);
      add(stem + "e");
      if (/([^aeiou])\1$/.test(stem)) add(stem.slice(0, -1));
    }
    return out;
  }
  /// The word of the list a word or phrase of the text is, if any.
  function entryOf(parts) {
    const words = reader.words;
    if (zh()) return words.get(parts.join(""));
    const low = parts.map(keyOf);
    const tries = low.length === 1 ? forms(low[0])
      : [...forms(low[0]).map((f) => [f, ...low.slice(1)].join(" ")), ...forms(low[low.length - 1]).map((f) => [...low.slice(0, -1), f].join(" "))];
    for (const t of tries) if (words.has(t)) return words.get(t);
    return null;
  }
  const stateOf = (w) => (!w ? "unknown" : w.skipped ? "skipped" : w.learned_at === 0 || w.due <= nowSec() ? "learning" : "known");

  /// The text in pieces, each with its place in the text and whether it is a word to click.
  async function tokenize(text) {
    const pieces = zh() ? await app.call("segment", { text }) : [...text.matchAll(TOKEN)].map((m) => [m[0], !!m[1]]);
    let at = 0;
    return pieces.map(([t, word]) => { const piece = { t, word, at }; at += t.length; return piece; });
  }

  /// Puts the pieces of a phrase of the list together, then finds every word in the list.
  function classify() {
    const src = reader.tokens, tokens = [], step = zh() ? 1 : 2;
    // No longer than the longest of the list, in words, or in characters for Chinese, whose pieces have one at least.
    const longest = Math.min(MAX_PHRASE, Math.max(1, ...[...reader.words.keys()].map((k) => (zh() ? k.length : k.split(" ").length))));
    for (let i = 0; i < src.length; i++) {
      const tk = src[i];
      if (!tk.word) { tokens.push(tk); continue; }
      // The longest phrase of the list that starts here: words with single spaces between them in English, pieces
      // side by side in Chinese.
      let phrase = null;
      for (let n = longest; n >= 2 && !phrase; n--) {
        const run = src.slice(i, i + (n - 1) * step + 1);
        const fits = run.length === (n - 1) * step + 1 && run.every((p, j) => (j % step === 0 ? p.word : /^[ \t]+$/.test(p.t)));
        const entry = fits && entryOf(run.filter((_, j) => j % step === 0).map((p) => p.t));
        if (entry) phrase = { t: run.map((p) => p.t).join(""), word: true, at: tk.at, entry, merged: run };
      }
      if (phrase) i += phrase.merged.length - 1;
      tokens.push(phrase || { ...tk, entry: entryOf([tk.t]) });
    }
    for (const tk of tokens) if (tk.word) tk.state = stateOf(tk.entry);
    reader.tokens = tokens;
  }
  // A phrase is split again before the words are found once more, since the list may have changed.
  const unmerge = () => (reader.tokens = reader.tokens.flatMap((t) => t.merged || [t]).map(({ t, word, at }) => ({ t, word, at })));

  function render() {
    const view = $("rd-text");
    view.innerHTML = reader.tokens.map((tk, i) => (tk.word ? `<span class="rd-w rd-${tk.state}" data-i="${i}" tabindex="-1">${esc(tk.t)}</span>` : esc(tk.t))).join("");
    summary();
  }
  /// Colours the words again after the list changed, leaving the text where it is.
  function repaint() {
    unmerge();
    classify();
    render();
  }
  function summary() {
    const counts = { unknown: new Set(), learning: new Set(), known: new Set(), skipped: new Set() };
    let total = 0;
    for (const tk of reader.tokens) {
      if (!tk.word) continue;
      total++;
      counts[tk.state].add(tk.entry ? tk.entry.id : keyOf(tk.t));
    }
    $("rd-summary").textContent = `${total} từ, ${counts.unknown.size} từ chưa có trong danh sách của bạn`;
    $("rd-legend").innerHTML = Object.entries(STATES).map(([k, label]) =>
      `<span><span class="rd-sw rd-${k}">${zh() ? "字" : "Aa"}</span>${label} <b class="num">${counts[k].size}</b></span>`).join("");
  }

  async function loadWords() {
    const list = await listCourse();
    reader.words = new Map(list.map((w) => [keyOf(w.word), w]));
  }

  function showEdit(editing) {
    $("rd-edit").classList.toggle("hidden", !editing);
    $("rd-view").classList.toggle("hidden", editing);
    $("rd-cancel").classList.toggle("hidden", !reader.text);
    $("rd-input").placeholder = zh() ? "Dán một đoạn văn, bài báo hay lời phụ đề tiếng Trung (chữ Hán)" : "Dán một đoạn văn, bài báo hay lời phụ đề tiếng Anh";
    $("rd-count").textContent = "";
    $("rd-error").textContent = "";
    if (editing) $("rd-input").value = reader.text;
  }

  /// Shows the text of the course, its words coloured by the list as it is now.
  async function show() {
    closePop();
    if (reader.course !== courseOf()) {
      reader.course = courseOf();
      reader.text = await app.call("reader_text").catch(() => "");
    }
    const course = reader.course;
    await loadWords();
    if (!reader.text) return showEdit(true);
    let tokens;
    try {
      tokens = await tokenize(reader.text);
    } catch (e) {
      showEdit(true);
      $("rd-error").textContent = String(e);
      return;
    }
    // The learner may have switched the course meanwhile.
    if (course !== courseOf()) return;
    reader.tokens = tokens;
    classify();
    render();
    showEdit(false);
  }

  async function read(text) {
    text = text.trim();
    if (!text) return $("rd-input").focus();
    if (text.length > MAX_TEXT) {
      text = text.slice(0, MAX_TEXT);
      toast(`Văn bản dài quá, app chỉ giữ ${MAX_TEXT.toLocaleString("vi-VN")} ký tự đầu`);
    }
    const course = hasHan(text) ? "zh" : "en";
    // The text decides the course. The switch shows the page again, which then finds this text already in place.
    reader.course = course;
    reader.text = text;
    if (course !== courseOf()) await switchCourse(course);
    else await show();
    app.call("reader_text", { text }).catch((e) => toast(String(e)));
  }

  $("rd-back").onclick = () => go("practice");
  $("rd-read").onclick = () => read($("rd-input").value);
  $("rd-cancel").onclick = () => showEdit(false);
  $("rd-change").onclick = () => { closePop(); showEdit(true); $("rd-input").focus(); };
  $("rd-input").addEventListener("input", () => {
    const n = $("rd-input").value.length;
    $("rd-count").textContent = n ? `${n.toLocaleString("vi-VN")} / ${MAX_TEXT.toLocaleString("vi-VN")} ký tự` : "";
  });
  $("rd-open").onclick = $("rd-open-2").onclick = () => $("rd-file").click();
  $("rd-file").addEventListener("change", () => {
    const file = $("rd-file").files[0];
    $("rd-file").value = "";
    if (!file) return;
    const fr = new FileReader();
    fr.onload = () => {
      const raw = String(fr.result).replace(/^\uFEFF/, "");
      const text = /\.(srt|vtt)$/i.test(file.name) || raw.startsWith("WEBVTT") ? subtitles(raw) : raw;
      if (!text.trim()) return toast("File này không có chữ nào để đọc");
      read(text);
    };
    fr.onerror = () => toast("Không đọc được file này, hãy thử một file .txt, .srt hoặc .vtt khác");
    fr.readAsText(file);
  });

  /// The sentence a piece of the text is in, out to the marks that end one but never far from the word.
  function sentenceOf(tk) {
    const text = reader.text, end = /[.!?。！？…\n]/;
    let from = tk.at, to = tk.at + tk.t.length;
    while (from > Math.max(0, tk.at - 200) && !end.test(text[from - 1])) from--;
    while (to < Math.min(text.length, tk.at + tk.t.length + 200) && !end.test(text[to])) to++;
    if (to < text.length && end.test(text[to]) && text[to] !== "\n") to++;
    return text.slice(from, to).replace(/\s+/g, " ").trim();
  }
  /// The word as it goes into the list: English in lower case without a possessive, unless it is a name inside a
  /// sentence.
  function formOf(tk) {
    if (zh()) return tk.t;
    const w = tk.t.replace(/’/g, "'").replace(/'s$/i, "");
    const name = /^\p{Lu}/u.test(w) && sentenceOf(tk).indexOf(tk.t) > 0;
    return name ? w : w.toLowerCase();
  }

  // The card of a word, beside it in the text.
  function closePop() {
    if (!reader.pop) return;
    $("rd-pop").classList.add("hidden");
    reader.pop.el?.classList.remove("on");
    reader.pop = null;
  }
  async function openPop(i, focus = false) {
    const tk = reader.tokens[i], el = $("rd-text").querySelector(`[data-i="${i}"]`);
    if (!tk || !el) return;
    closePop();
    el.classList.add("on");
    const pop = $("rd-pop"), w = tk.entry, word = w ? w.word : formOf(tk), sentence = sentenceOf(tk);
    reader.pop = { i, el };
    const mine = reader.pop;
    pop.innerHTML = `
      <div class="rd-pop-head"><b class="rd-pop-word">${esc(word)}</b>${w ? status(w) : `<span class="tag plain">${STATES.unknown}</span>`}</div>
      <div class="ipa" id="rd-pop-ipa">${esc(w?.ipa || "")}</div>
      ${zh() ? `<div class="muted small" id="rd-pop-hv">${w?.hanviet ? `Hán Việt: ${esc(w.hanviet)}` : ""}</div><div class="muted small" id="rd-pop-en"></div>` : ""}
      ${w ? `<div class="rd-pop-mean">${w.meaning ? esc(w.meaning) : '<span class="muted">Chưa có nghĩa</span>'}</div>` : ""}
      <div class="row rd-pop-actions">
        ${w ? "" : '<button class="btn small primary" id="rd-add">Thêm vào danh sách</button>'}
        <button class="btn small" data-say="${esc(word)}">Nghe</button>
        ${sentence && sentence !== tk.t ? `<button class="btn small" data-say="${esc(sentence)}">Nghe câu</button>` : ""}
      </div>`;
    pop.classList.remove("hidden");
    place(pop, el);
    $("rd-add")?.addEventListener("click", (e) => add(tk, word, sentence, e.currentTarget));
    if (focus) pop.querySelector(".btn")?.focus({ preventScroll: true });
    // What the dictionaries know of a word the list does not have.
    if (!w?.ipa) app.call("ipa", { word }).then((r) => { if (reader.pop === mine) $("rd-pop-ipa").textContent = r || (zh() ? "" : "Không có phiên âm trong từ điển"); }).catch(() => {});
    if (zh() && !w?.hanviet) app.call("hanviet", { word }).then((r) => { if (reader.pop === mine && r) $("rd-pop-hv").textContent = `Hán Việt: ${r}`; }).catch(() => {});
    // CC-CEDICT's English, which may make the card taller: it is placed again.
    if (zh()) app.call("gloss", { word }).then((r) => { if (reader.pop === mine && r) { $("rd-pop-en").textContent = `Tiếng Anh: ${r}`; place(pop, el); } }).catch(() => {});
  }
  /// Under the word, or over it when there is no room below, and inside the window.
  function place(pop, el) {
    const r = el.getBoundingClientRect(), w = pop.offsetWidth, h = pop.offsetHeight;
    const below = r.bottom + 8 + h <= innerHeight - 8 || r.top - 8 - h < 8;
    pop.style.top = `${Math.max(8, below ? r.bottom + 8 : r.top - 8 - h)}px`;
    pop.style.left = `${Math.min(Math.max(8, r.left + r.width / 2 - w / 2), innerWidth - w - 8)}px`;
  }

  async function add(tk, word, example, button) {
    button.disabled = true;
    let saved;
    try {
      saved = await app.call("save", { word: { id: 0, word, example } });
    } catch (e) {
      button.disabled = false;
      return toast(String(e));
    }
    gainXp(3, button);
    toast(`Đã thêm "${saved.word}". AI đang giải thích nghĩa...`);
    const course = courseOf();
    const learnt = (w) => {
      if (courseOf() !== course) return;
      reader.words.set(keyOf(w.word), w);
      const open = reader.pop && reader.tokens[reader.pop.i];
      repaint();
      // The card stays open on the same word, which now has the list's colour.
      if (open) {
        const i = reader.tokens.findIndex((t) => t.at === open.at);
        if (i >= 0) openPop(i);
      }
    };
    learnt(saved);
    try {
      const e = await app.call("explain", { word: saved.word, day: localDay() });
      const more = Object.fromEntries(FROM_AI.map((f) => [f, e[f] || ""]));
      learnt(await app.call("save", { word: { ...saved, ...more, topic: saved.topic || e.topic || "" } }));
    } catch (e) {
      toast(`Đã thêm "${saved.word}" nhưng AI chưa giải thích được: ${e}`);
    }
  }

  $("rd-text").addEventListener("click", (e) => {
    const el = e.target.closest("[data-i]");
    if (el) openPop(+el.dataset.i);
  });
  // The keyboard: arrows move between the words, Enter opens the one in focus.
  $("rd-text").addEventListener("keydown", (e) => {
    const words = [...$("rd-text").querySelectorAll("[data-i]")];
    const at = words.indexOf(document.activeElement);
    if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      e.preventDefault();
      const next = words[at < 0 ? 0 : Math.min(words.length - 1, Math.max(0, at + (e.key === "ArrowRight" ? 1 : -1)))];
      next?.focus();
    } else if ((e.key === "Enter" || e.key === " ") && at >= 0) {
      e.preventDefault();
      openPop(+words[at].dataset.i, true);
    }
  });
  document.addEventListener("click", (e) => {
    if (reader.pop && !e.target.closest("#rd-pop, #rd-text [data-i]")) closePop();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !reader.pop) return;
    const el = reader.pop.el;
    closePop();
    el.focus();
  });
  // The card follows its word while the text scrolls, and goes once the word is out of sight.
  document.querySelector("main").addEventListener("scroll", () => {
    if (!reader.pop) return;
    const r = reader.pop.el.getBoundingClientRect();
    if (r.bottom < 0 || r.top > innerHeight) closePop();
    else place($("rd-pop"), reader.pop.el);
  }, { passive: true });
  addEventListener("resize", closePop);

  hooks.page.push((page) => {
    if (page !== "reader") return closePop();
    // The reader belongs to the exercises.
    const nav = document.querySelector('nav [data-page="practice"]');
    nav.classList.add("active");
    nav.setAttribute("aria-current", "page");
    show();
  });
})();
