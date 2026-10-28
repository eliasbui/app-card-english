(() => {
  "use strict";
  const MARKS = { "̄": 1, "́": 2, "̌": 3, "̀": 4 };
  /// The tone of each syllable of pinyin with tone marks, 5 for the neutral tone: `xuéxí` is [2, 2], `xièxie` [4, 5].
  /// A syllable is a run of vowels, which a consonant, y, w or an apostrophe always ends; the erhua r has none.
  const tonesOf = (pinyin) => (String(pinyin ?? "").normalize("NFD").toLowerCase().match(/[aeiouv̀-ͯ]+/g) || [])
    .map((run) => MARKS[[...run].find((c) => MARKS[c])] || 5);
  const toneName = (t) => (t === 5 ? "nhẹ" : String(t));
  const pattern = (tones) => tones.map(toneName).join(" + ");

  // `node ui/parts/drill-tones.js` checks the helper.
  if (typeof window === "undefined") {
    const cases = { "xuéxí": [2, 2], "xièxie": [4, 5], "Xī'ān": [1, 1], "nǚ'ér": [3, 2], "yìdiǎnr": [4, 3], "lǜsè": [4, 4], "wǒmen zǒu le.": [3, 5, 3, 5], "": [] };
    for (const [pinyin, want] of Object.entries(cases)) {
      if (String(tonesOf(pinyin)) !== String(want)) throw new Error(`tonesOf("${pinyin}") is [${tonesOf(pinyin)}], not [${want}]`);
    }
    return console.log("tonesOf: ok");
  }

  /// Luyện thanh điệu: the app reads a word and the learner picks the tones of its syllables out of four; then the
  /// pinyin shows and the learner says the word.
  function show(w, card) {
    const tones = tonesOf(w.ipa), options = [tones];
    while (options.length < 4) {
      const other = tones.map((t) => (Math.random() < 0.5 ? 1 + Math.floor(Math.random() * 4) : t));
      if (!options.some((o) => pattern(o) === pattern(other))) options.push(other);
    }
    shuffle(options);
    const right = options.indexOf(tones);
    card.innerHTML = `
      ${st("mascot-zh", "mode-st")}
      <div class="prompt-label">Nghe rồi chọn thanh điệu của từng chữ</div>
      <div class="row center" style="margin-top:12px"><button class="btn primary" id="btn-play">Nghe</button><button class="btn" id="btn-play-slow">Nghe chậm</button></div>
      <div class="options">${options.map((o, i) => `<button class="btn" data-i="${i}"><span class="opt-n">${i + 1}</span>${pattern(o)}</button>`).join("")}</div>
      <div id="fill-result"></div>
      <div class="pron hidden" id="tone-pron"></div>
      <div class="grades hidden" id="choice-next"><button class="btn primary" id="btn-next">Câu tiếp</button></div>`;
    $("btn-play").onclick = (e) => sayWith(e.currentTarget, w.word);
    $("btn-play-slow").onclick = (e) => sayWith(e.currentTarget, w.word, true);
    speak(w.word);
    card.querySelectorAll(".options button").forEach((b) => (b.onclick = async () => {
      if (b.disabled) return;
      const ok = +b.dataset.i === right;
      card.querySelectorAll(".options button").forEach((x) => {
        x.disabled = true;
        if (+x.dataset.i === right) x.classList.add("right");
      });
      if (!ok) b.classList.add("wrong");
      // Hearing a tone wrong is not forgetting the word.
      await answered(w, ok, b, { grade: false });
      $("fill-result").innerHTML = verdict(ok, pattern(tones))
        + `<div class="tone-word"><b>${esc(w.word)}</b> <span class="ipa">${esc(w.ipa)}</span></div><div class="muted">${esc(w.meaning)}</div>`;
      if (!ok) whyWrong($("fill-result"), w, `${pattern(tones)} (${w.ipa})`, pattern(options[+b.dataset.i]), "Listen to the word and pick the tone of each syllable");
      sayAloud(w);
      $("choice-next").classList.remove("hidden");
      $("btn-next").onclick = next;
      $("btn-next").focus();
    }));
  }

  /// The learner says the word, when the Chinese Whisper model is there, and sees the tone of each syllable.
  function sayAloud(w) {
    const box = $("tone-pron");
    box.classList.remove("hidden");
    if (!state.models.sttzh?.ready) {
      box.innerHTML = `<span class="muted">Tải Whisper tiếng Trung trong Cài đặt để tự đọc từ này và xem chữ nào sai thanh.</span>`;
      return;
    }
    pronWidget(box, w, { onScore: (score, button, s) => {
      if (button.dataset.a === "rec") box.querySelector(".result").insertAdjacentHTML("beforeend", toneCheck(w, s));
    } });
  }

  /// Each character with the tone it was said in, when the sounds were right; nothing when they were not, since then
  /// the score already says so.
  function toneCheck(w, s) {
    const want = tonesOf(s.target_ipa), got = tonesOf(s.heard_ipa);
    const sounds = (p) => String(p ?? "").normalize("NFD").toLowerCase().replace(/[^a-z]/g, "");
    if (!want.length || want.length !== got.length || sounds(s.target_ipa) !== sounds(s.heard_ipa)) return "";
    const chars = [...w.word].filter(hasHan);
    return `<div class="tone-syls">${want.map((t, i) => `<span class="tone-syl ${t === got[i] ? "ok" : "off"}"><b>${esc(chars[i] || "")}</b>`
      + `${t === got[i] ? `thanh ${toneName(t)}` : `cần thanh ${toneName(t)}, bạn đọc thanh ${toneName(got[i])}`}</span>`).join("")}</div>`;
  }

  exercises.tone = {
    show,
    fits: (w) => hasHan(w.word) && tonesOf(w.ipa).length > 0,
    prefer: (w) => tonesOf(w.ipa).length === 2,
    none: "Chưa có từ tiếng Trung nào có pinyin để luyện thanh điệu.",
  };
})();
