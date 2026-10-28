// The cards under the tutor's answers (src/assistant.rs builds them): words to add, a quiz, buttons into the app's
// features, numbers. Each renderer fills the element it is given; chat-tutor.js calls them by the card's type.
(() => {
  "use strict";
  if (window.MINI) return;
  const cards = (window.chatCards ??= {});
  const zh = () => courseOf() === "zh";
  /// Answers compared as typed: case, spacing, punctuation and marks aside.
  const norm = (s) => String(s).toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/[^\p{L}\p{N}]+/gu, " ").trim();

  // Words: a box each, "Thêm" for the chosen ones, "Gợi ý thêm" while the request wants more.
  cards.words = (card, el) => {
    const rows = card.words.slice();
    let more = card.more;
    el.innerHTML = `<div class="ccard">
      <div class="ccard-head"><b>${esc(card.title)}</b><label class="ccard-all"><input type="checkbox" checked> Chọn tất cả</label></div>
      <div class="cw-list"></div>
      <div class="ccard-foot"><button class="btn small primary" data-cw="add">Thêm từ đã chọn</button>
        <button class="btn small" data-cw="more" hidden></button><span class="muted small" data-cw="status"></span></div></div>`;
    const list = el.querySelector(".cw-list"), status = el.querySelector('[data-cw="status"]'), moreButton = el.querySelector('[data-cw="more"]');
    const draw = () => {
      list.innerHTML = rows.map((w, i) => `<label class="cw${w.in_list ? " have" : ""}" data-i="${i}">
        <input type="checkbox" ${w.in_list ? "disabled" : w.off ? "" : "checked"} aria-label="Chọn ${esc(w.word)}">
        <span class="cw-main"><b>${esc(w.word)}</b>${w.reading ? ` <span class="muted">${esc(w.reading)}</span>` : ""}
          <span class="cw-meaning">${esc(w.meaning || "AI sẽ giải thích khi thêm")}</span></span>
        ${w.in_list ? `<span class="tag plain">${w.added ? "Đã thêm" : "Đã có"}</span>` : ""}
        <button type="button" class="btn small" data-say="${esc(w.word)}">Nghe</button></label>`).join("");
      moreButton.hidden = !more;
      if (more) moreButton.textContent = `Gợi ý thêm ${Math.min(more.count, 10)} từ`;
    };
    draw();
    list.addEventListener("change", (e) => {
      const row = e.target.closest(".cw");
      if (row) rows[+row.dataset.i].off = !e.target.checked;
    });
    el.querySelector(".ccard-all input").onchange = (e) => {
      rows.forEach((w) => (w.off = !e.target.checked));
      draw();
    };
    el.querySelector('[data-cw="add"]').onclick = async (e) => {
      const button = e.currentTarget, chosen = rows.filter((w) => !w.in_list && !w.off);
      if (!chosen.length) return (status.textContent = "Chưa chọn từ mới nào.");
      button.disabled = true;
      let added = 0;
      for (const w of chosen) {
        status.textContent = `Đang thêm ${added + 1}/${chosen.length}: ${w.word}`;
        try {
          const saved = await app.call("save", { word: { id: 0, word: w.word, meaning: w.meaning || "", ipa: zh() ? "" : w.reading || "", topic: card.topic || "" } });
          w.in_list = w.added = true;
          added++;
          // The rest of the explanation follows in the background, as for the other suggestions; a meaning the card
          // already had stays.
          app.call("explain", { word: w.word, day: localDay() })
            .then((x) => app.call("save", { word: { ...saved, ...Object.fromEntries(EXPLAINED.filter((f) => f !== "meaning" || !saved.meaning).map((f) => [f, x[f] || ""])), topic: saved.topic } }))
            .catch(() => {});
        } catch {
          w.in_list = true;
        }
      }
      draw();
      button.disabled = false;
      status.textContent = `Đã thêm ${added} từ vào danh sách.`;
      if (added) toast(`Đã thêm ${added} từ, AI đang giải thích từng từ`);
    };
    moreButton.onclick = async () => {
      moreButton.disabled = true;
      status.textContent = "Đang gợi ý thêm...";
      try {
        const r = await app.call("assistant_suggest", { topic: more.topic, count: Math.min(more.count, 10), shown: rows.map((w) => w.word) });
        const next = r.cards[0]?.words || [];
        rows.push(...next);
        const left = more.count - next.length;
        more = next.length && left > 0 ? { topic: more.topic, count: left } : null;
        status.textContent = `Thêm ${next.length} từ.`;
        draw();
      } catch (err) {
        status.textContent = String(err);
      }
      moreButton.disabled = false;
    };
  };

  // A quiz of the learner's own words, one question at a time, with XP and the schedule as the exercises have them.
  cards.quiz = (card, el) => {
    const items = card.items.slice(), wrong = [];
    let at = 0, right = 0;
    const show = () => {
      if (at >= items.length) {
        el.innerHTML = `<div class="ccard"><div class="ccard-head"><b>Đúng ${right}/${items.length}</b></div>
          <p class="muted small">${wrong.length ? `Cần xem lại: ${wrong.map(esc).join(", ")}` : "Giỏi quá, đúng hết!"}</p>
          <div class="ccard-foot"><button class="btn small" data-cq="again">Làm lại</button></div></div>`;
        el.querySelector('[data-cq="again"]').onclick = () => {
          at = right = 0;
          wrong.length = 0;
          shuffle(items);
          show();
        };
        return;
      }
      const it = items[at];
      el.innerHTML = `<div class="ccard"><div class="ccard-head"><b>${esc(card.title)}</b><span class="muted small">Câu ${at + 1}/${items.length}</span></div>
        <p class="cq-prompt">${esc(it.prompt)}</p>${it.hint ? `<p class="muted small">Gợi ý: ${esc(it.hint)}</p>` : ""}
        ${it.kind === "choice" ? `<div class="cq-options">${it.options.map((o, i) => `<button class="btn" data-o="${i}">${esc(o)}</button>`).join("")}</div>`
          : `<form class="cq-fill"><input autocomplete="off" aria-label="Từ còn thiếu"><button class="btn primary">Kiểm tra</button></form>`}
        <div class="cq-after"></div></div>`;
      const after = el.querySelector(".cq-after");
      const judge = (given, ok, from) => {
        el.querySelectorAll(".cq-options button, .cq-fill input, .cq-fill button").forEach((x) => (x.disabled = true));
        if (ok) {
          right++;
          gainXp(5, from, true);
        } else {
          wrong.push(it.word);
          gainXp(0, null, false);
          // A learned word answered wrong comes back into review soon, as in the exercises.
          if (it.learned) app.call("grade", { id: it.id, q: 1 }).catch(() => {});
        }
        after.innerHTML = `<p class="${ok ? "ok" : "error"}">${ok ? "Chính xác!" : `Chưa đúng. Đáp án: <b>${esc(it.answer)}</b>`}</p>
          <div class="ccard-foot">${ok ? "" : `<button class="btn small" data-cq="why">Vì sao sai?</button>`}
            <button class="btn small" data-say="${esc(it.word)}">Nghe</button>
            <button class="btn small primary" data-cq="next">${at + 1 < items.length ? "Câu tiếp" : "Xem kết quả"}</button></div>
          <p class="cq-why muted small"></p>`;
        after.querySelector('[data-cq="next"]').onclick = () => { at++; show(); };
        const why = after.querySelector('[data-cq="why"]');
        if (why) why.onclick = async () => {
          why.disabled = true;
          why.classList.add("loading");
          try {
            const m = await app.call("explain_mistake", { word: it.word, meaning: it.meaning, expected: it.answer, given, context: it.prompt, day: localDay() });
            after.querySelector(".cq-why").textContent = m.explain;
          } catch (err) {
            after.querySelector(".cq-why").textContent = String(err);
          }
          why.classList.remove("loading");
        };
      };
      if (it.kind === "choice") {
        el.querySelectorAll("[data-o]").forEach((b) => (b.onclick = () => {
          const given = it.options[+b.dataset.o], ok = given === it.answer;
          el.querySelectorAll("[data-o]").forEach((x) => it.options[+x.dataset.o] === it.answer && x.classList.add("right"));
          if (!ok) b.classList.add("wrong");
          judge(given, ok, b);
        }));
      } else {
        const form = el.querySelector(".cq-fill");
        form.querySelector("input").focus({ preventScroll: true });
        form.onsubmit = (e) => {
          e.preventDefault();
          const given = form.querySelector("input").value.trim();
          if (given) judge(given, norm(given) === norm(it.answer), form.querySelector("button"));
        };
      }
    };
    show();
  };

  /// Opens what a button of an `actions` card names.
  async function run(r) {
    window.chatClose?.();
    if (r.kind === "page") return go(r.page);
    if (r.kind === "packs") return window.openPacks?.();
    if (r.kind === "vocab_test") return window.openVocabTest?.();
    if (r.kind === "exam") return window.openExam?.(r.roadmap, r.stage ?? -1);
    if (r.kind !== "practice") return;
    go("practice");
    await state.sources;
    const select = $("practice-source"), source = r.source === "recent" || !r.source ? "all" : r.source;
    if (![...select.options].some((o) => o.value === source)) {
      select.insertAdjacentHTML("beforeend", `<option value="${esc(source)}">${esc(source.replace(/^topic:/, "Chủ đề: "))}</option>`);
    }
    select.value = source;
    startPractice(r.exercise);
  }
  cards.actions = (card, el) => {
    el.innerHTML = `<div class="ccard-actions">${card.buttons.map((b, i) => `<button class="btn small${i ? "" : " primary"}" data-run="${i}">${esc(b.label)}</button>`).join("")}</div>`;
    el.querySelectorAll("[data-run]").forEach((b) => (b.onclick = () => run(card.buttons[+b.dataset.run].run)));
  };

  cards.stats = (card, el) => {
    el.innerHTML = `<div class="ccard"><div class="cs-grid">${card.items.map((x) => `<div><b>${esc(x.value)}</b><span>${esc(x.label)}</span></div>`).join("")}</div>
      ${card.note ? `<p class="muted small">${esc(card.note)}</p>` : ""}</div>`;
  };
})();
