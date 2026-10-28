(() => {
  "use strict";
  // Ready-made word packs: a panel in the vocabulary page lists the packs of the course with how far the learner
  // is in each, and adds the next words one after another, each explained by the AI. The vocabulary test sets
  // where each pack starts, past what the learner already knows.
  const fmt = (n) => Number(n).toLocaleString("vi-VN");
  const pct = (x) => `${Math.round(100 * x)}%`;
  let packs = [], chosen = "", run = null, report = "";

  const opener = document.createElement("button");
  opener.className = "btn";
  opener.id = "btn-packs";
  opener.textContent = "Bộ từ có sẵn";
  $("btn-open-suggest").before(opener);
  const card = document.createElement("div");
  card.className = "card hidden";
  card.id = "packs-card";
  $("suggest-card").before(card);
  const shown = () => !card.classList.contains("hidden");

  async function refresh() {
    try {
      const s = await app.call("packs");
      packs = s.packs;
      // A run started before the page was opened again: its progress comes with its next word.
      if (s.running && !run) run = { done: 0, total: 0, word: "", name: "" };
    } catch (e) {
      packs = [];
      report = `<span class="error">${esc(e)}</span>`;
    }
    render();
  }

  function render() {
    if (!packs.some((p) => p.id === chosen)) chosen = (packs.find((p) => p.left) || packs[0])?.id || "";
    card.innerHTML = `
      <div class="row"><b class="grow">Bộ từ có sẵn</b><button class="btn small" data-pk="test">Kiểm tra vốn từ</button><button class="btn small" data-pk="close">Đóng</button></div>
      <p class="muted small pk-lead">Chọn một bộ, app thêm lần lượt những từ bạn chưa có và nhờ AI giải thích từng từ. Chưa biết bắt đầu từ đâu? Hãy làm bài kiểm tra vốn từ.</p>
      <div class="tiles pk-tiles" role="radiogroup" aria-label="Bộ từ">${packs.map(tile).join("")}</div>
      <div class="pk-action"></div>
      <div class="pk-report">${report}</div>`;
    renderAction();
  }

  const tile = (p) => `
    <input type="radio" name="pack" id="pk-${p.id}" value="${p.id}" ${p.id === chosen ? "checked" : ""}>
    <label class="tile" for="pk-${p.id}"><b>${esc(p.name)}</b><span>${esc(p.about)}</span>
      <i class="pk-bar"><i style="width:${(100 * p.have) / Math.max(1, p.size)}%"></i></i>
      <em><b class="num">${fmt(p.have)}</b> / ${fmt(p.size)} từ đã có trong danh sách</em></label>`;

  function renderAction() {
    const box = card.querySelector(".pk-action"), p = packs.find((x) => x.id === chosen);
    if (!box) return;
    if (run) {
      box.innerHTML = `
        <div class="pk-run" role="status">
          <div class="row"><span class="grow pk-run-text"></span><button class="btn small" data-pk="stop"></button></div>
          <div class="progress"><div></div></div>
          <p class="muted small">AI giải thích từng từ rồi thêm vào danh sách. Bạn vẫn có thể học hay làm việc khác trong lúc chờ.</p>
        </div>`;
      return paintRun();
    }
    if (!p) return (box.innerHTML = "");
    const start = p.pos > 0
      ? `<div class="row pk-start"><span class="muted small grow">Bắt đầu từ từ thứ ${fmt(p.pos + 1)}, sau phần bạn đã thêm hoặc đã biết.</span><button class="btn ghost small" data-pk="reset">Về đầu bộ</button></div>`
      : "";
    box.innerHTML = p.left
      ? `<div class="pk-next"><span class="muted">Tiếp theo:</span> ${p.next.map((w) => `<span class="tag plain">${esc(w)}</span>`).join(" ")}${p.left > p.next.length ? ` <span class="muted small">và ${fmt(p.left - p.next.length)} từ nữa</span>` : ""}</div>
         <div class="row pk-add"><span class="muted">Thêm từ tiếp theo:</span>${[10, 20, 50].map((n) => `<button class="btn small${n === 10 ? " primary" : ""}" data-add="${n}">Thêm ${n} từ</button>`).join("")}</div>${start}`
      : `<p class="muted pk-done">Bạn đã có hoặc đã biết hết các từ của bộ này.</p>${start}`;
  }

  // Updates the bar where it is, so that it moves smoothly from one word to the next.
  function paintRun() {
    const text = card.querySelector(".pk-run-text");
    if (!text || !run) return;
    const of = run.total ? ` (${run.done + (run.word ? 1 : 0)} / ${run.total})` : "";
    const idle = run.total && run.done >= run.total ? "Sắp xong" : "Đang chuẩn bị";
    text.innerHTML = run.word ? `Đang thêm <b>${esc(run.word)}</b>${run.name ? ` của ${esc(run.name)}` : ""}${of}` : idle + of;
    card.querySelector(".pk-run .progress div").style.width = run.total ? `${(100 * run.done) / run.total}%` : "0";
    const stop = card.querySelector('[data-pk="stop"]');
    stop.textContent = run.stopping ? "Đang dừng..." : "Dừng";
    stop.disabled = !!run.stopping;
  }

  function summary(r) {
    let html = r.added.length ? `Đã thêm ${fmt(r.added.length)} từ vào danh sách.` : "Chưa thêm được từ nào.";
    if (r.cancelled) html = `Đã dừng. ${html}`;
    if (r.failed.length) {
      html += ` <details><summary>${fmt(r.failed.length)} từ chưa thêm được, bấm Thêm lần nữa để thử lại</summary>
        <ul>${r.failed.map((f) => `<li><b>${esc(f.word)}</b>: ${esc(f.error)}</li>`).join("")}</ul></details>`;
    }
    return html;
  }

  async function add(n) {
    const p = packs.find((x) => x.id === chosen);
    if (!p || run) return;
    run = { mine: true, pack: p.id, name: p.name, done: 0, total: Math.min(n, p.left), word: "" };
    report = "";
    renderAction();
    card.querySelector(".pk-report").innerHTML = "";
    try {
      const r = await app.call("pack_add", { pack: p.id, n, day: localDay() });
      report = summary(r);
      if (!shown() || !$("page-words").classList.contains("active")) toast(r.added.length ? `Đã thêm ${r.added.length} từ của ${p.name}` : "Chưa thêm được từ nào");
    } catch (e) {
      report = `<span class="error">${esc(e)}</span>`;
    }
    finished();
  }

  function finished() {
    run = null;
    refresh();
    if ($("page-words").classList.contains("active")) loadWords(); else refreshBadges();
  }

  async function move(moves) {
    try {
      packs = (await app.call("pack_move", { moves })).packs;
    } catch (e) {
      toast(String(e));
    }
    render();
  }

  opener.onclick = () => {
    if (shown()) return card.classList.add("hidden");
    card.classList.remove("hidden");
    refresh();
    card.scrollIntoView({ behavior: "smooth", block: "nearest" });
  };
  // For the tutor's buttons in the chat.
  window.openPacks = () => {
    go("words");
    if (!shown()) opener.click();
  };
  window.openVocabTest = () => openTest();
  card.addEventListener("change", (e) => {
    if (e.target.name !== "pack") return;
    chosen = e.target.value;
    renderAction();
  });
  card.addEventListener("click", (e) => {
    const b = e.target.closest("[data-pk], [data-add]");
    if (!b) return;
    if (b.dataset.add) return add(+b.dataset.add);
    const act = b.dataset.pk;
    if (act === "close") card.classList.add("hidden");
    if (act === "test") openTest();
    if (act === "reset") move({ [chosen]: 0 });
    if (act === "stop" && run) {
      run.stopping = true;
      paintRun();
      app.call("pack_cancel").catch(() => {});
    }
  });

  hooks.event.push((name, data) => {
    if (name !== "pack_progress") return;
    if (!run) run = { done: 0, total: 0, word: "" };
    Object.assign(run, data, { name: packs.find((p) => p.id === data.pack)?.name || run.name || "" });
    if (data.done >= data.total) {
      run.word = "";
      // A run this page did not start has no answer to wait for.
      if (!run.mine) return finished();
    }
    if (!card.querySelector(".pk-run")) renderAction(); else paintRun();
  });
  hooks.course.push(() => {
    report = "";
    if (shown()) refresh();
  });
  hooks.page.push((page) => {
    if (page === "words" && shown()) refresh();
  });

  // The vocabulary test: yes or no to words sampled from the bands of the packs, with a few made-up words.
  const modal = document.createElement("div");
  modal.className = "modal hidden";
  modal.id = "vt-modal";
  modal.innerHTML = `<div class="dialog vt-dialog" role="dialog" aria-modal="true" aria-labelledby="vt-title"></div>`;
  document.body.append(modal);
  const dialog = modal.firstElementChild;
  let test = null;

  function openTest() {
    test = null;
    const zh = courseOf() === "zh";
    dialog.innerHTML = `
      <h3 id="vt-title">Kiểm tra vốn từ</h3>
      <p>App đưa ra khoảng 55 từ ${zh ? "của các cấp HSK, từ cấp 1 đến cấp 9" : "từ dễ đến khó"}. Với mỗi từ, chọn <b>Biết</b> nếu bạn hiểu nghĩa của nó, chọn <b>Không biết</b> nếu không chắc.</p>
      <p class="muted">Có vài từ không có thật để xem bạn có đoán hay không, nên cứ trả lời thật lòng. Mất khoảng 3 phút.</p>
      <div class="row actions"><button class="btn" data-vt="close">Để sau</button><button class="btn primary" data-vt="start">Bắt đầu</button></div>`;
    modal.classList.remove("hidden");
    dialog.querySelector('[data-vt="start"]').focus();
  }
  function closeTest() {
    modal.classList.add("hidden");
    test = null;
  }

  async function startTest() {
    try {
      const t = await app.call("vocab_test");
      test = { lang: t.lang, items: shuffle(t.items), at: 0, answers: [] };
    } catch (e) {
      return toast(String(e));
    }
    dialog.innerHTML = `
      <div class="study-bar vt-bar"><div class="grow progress"><div></div></div><span class="muted num vt-count"></span></div>
      <p class="eyebrow vt-ask" id="vt-title">Bạn có biết từ này không?</p>
      <div class="vt-word" lang="${test.lang === "zh" ? "zh-CN" : "en"}"></div>
      <div class="vt-answers"><button class="btn big" data-vt="no">Không biết <kbd>1</kbd></button><button class="btn primary big" data-vt="yes">Biết <kbd>2</kbd></button></div>
      <div class="row vt-foot"><span class="muted small grow">Chỉ chọn Biết khi bạn nhớ được nghĩa của từ.</span><button class="btn ghost small" data-vt="close">Dừng</button></div>`;
    ask();
  }
  function ask() {
    dialog.querySelector(".vt-word").textContent = test.items[test.at].word;
    dialog.querySelector(".vt-count").textContent = `${test.at + 1} / ${test.items.length}`;
    dialog.querySelector(".vt-bar .progress div").style.width = `${(100 * test.at) / test.items.length}%`;
  }
  function answer(yes) {
    if (!test || test.result || test.at >= test.items.length) return;
    test.answers.push({ band: test.items[test.at].band, yes });
    test.at++;
    if (test.at < test.items.length) return ask();
    finishTest();
  }
  async function finishTest() {
    dialog.innerHTML = `<div class="thinking center">${st("mascot-think")}<span>Đang tính kết quả...</span></div>`;
    try {
      test.result = await app.call("vocab_result", { lang: test.lang, answers: test.answers });
      showResult(test.result);
    } catch (e) {
      dialog.innerHTML = `<p class="error">${esc(e)}</p><div class="row actions"><button class="btn" data-vt="close">Đóng</button></div>`;
    }
  }

  const LEVELS = {
    A1: "Bạn đang ở những bước đầu: hãy học hết 1.000 từ đầu tiên của NGSL, chúng có mặt trong phần lớn mọi câu nói.",
    A2: "Bạn dùng được những từ cơ bản nhất trong giao tiếp hằng ngày.",
    B1: "Bạn hiểu được phần lớn các câu nói và bài viết đơn giản về những chủ đề quen thuộc.",
    B2: "Bạn biết gần hết các từ thông dụng, đủ để đọc báo và trò chuyện tự nhiên.",
    C1: "Bạn biết hầu hết các từ thông dụng và nhiều từ học thuật.",
  };
  const packName = (id) => packs.find((p) => p.id === id)?.name || id.toUpperCase();
  const bandName = (b) => (b.pack === "ngsl" ? `NGSL ${fmt(b.start + 1)}-${fmt(b.end)}` : packName(b.pack));

  function showResult(r) {
    const zh = r.lang === "zh";
    const words = fmt(Math.round(r.words / 50) * 50);
    const fakes = test.answers.filter((a) => a.band == null);
    const guessed = fakes.filter((a) => a.yes).length;
    const next = zh && r.level && r.level !== "HSK 7-9" ? `, nên học tiếp HSK ${+r.level.slice(4) + 1}` : "";
    const explain = zh
      ? (r.level ? `Bạn nắm phần lớn từ vựng đến ${r.level}${next}.` : "Bạn nên bắt đầu từ HSK 1.")
      : `${LEVELS[r.level] || ""} Từ học thuật (NAWL): biết khoảng ${pct(r.academic)}. Từ TOEIC (TSL): khoảng ${pct(r.toeic)}.`;
    const guess = guessed
      ? `Bạn chọn Biết cho ${guessed} trong ${fakes.length} từ không có thật, kết quả đã trừ đi phần có thể là đoán.${r.guessed >= 0.5 ? " Kết quả có thể chưa chính xác, hãy làm lại và chỉ chọn Biết khi chắc chắn." : ""} `
      : "";
    const moves = r.moves.map((m) => {
      const p = packs.find((x) => x.id === m.pack);
      return `<li>${esc(packName(m.pack))}: ${p && m.to >= p.size ? "bỏ qua cả bộ" : `bắt đầu từ từ thứ ${fmt(m.to + 1)}`}</li>`;
    }).join("");
    dialog.innerHTML = `
      <h3 id="vt-title">Kết quả kiểm tra vốn từ</h3>
      <div class="vt-big">Khoảng ${words} ${zh ? "từ HSK" : "từ thông dụng"}</div>
      <div class="row vt-level"><span class="tag">${zh ? esc(r.level || "Chưa tới HSK 1") : `Trình độ khoảng ${esc(r.level)}`}</span></div>
      <p>${esc(explain)}</p>
      <div class="vt-bands">${r.bands.map((b) => `
        <div class="vt-band"><span>${esc(bandName(b))}</span><div class="progress"><div style="width:${pct(b.known)}"></div></div><span class="muted num">${pct(b.known)}</span></div>`).join("")}
      </div>
      <p class="muted small">${esc(guess)}Đây là ước lượng nhanh từ một bài kiểm tra ngắn.</p>
      ${moves ? `<div class="vt-move"><b>Bỏ qua phần bạn đã biết?</b><ul>${moves}</ul><p class="muted small">Thêm từ tiếp theo sẽ bắt đầu sau phần này. Những từ đã biết không được thêm vào danh sách.</p></div>` : ""}
      <div class="row actions">${moves
        ? `<button class="btn" data-vt="close">Giữ nguyên</button><button class="btn primary" data-vt="apply">Bỏ qua phần đã biết</button>`
        : `<button class="btn" data-vt="again">Làm lại</button><button class="btn primary" data-vt="close">Đóng</button>`}</div>`;
  }

  async function applyMoves() {
    await move(Object.fromEntries(test.result.moves.map((m) => [m.pack, m.to])));
    closeTest();
    toast("Các bộ từ sẽ bắt đầu sau phần bạn đã biết");
  }

  dialog.addEventListener("click", (e) => {
    const act = e.target.closest("[data-vt]")?.dataset.vt;
    if (act === "close") closeTest();
    if (act === "start" || act === "again") startTest();
    if (act === "yes" || act === "no") answer(act === "yes");
    if (act === "apply") applyMoves();
  });
  modal.addEventListener("click", (e) => { if (e.target === modal) closeTest(); });
  document.addEventListener("keydown", (e) => {
    if (modal.classList.contains("hidden")) return;
    if (e.key === "Escape") return closeTest();
    if (!test || test.result) return;
    const yes = { Digit2: true, ArrowRight: true, Digit1: false, ArrowLeft: false }[e.code];
    if (yes !== undefined) {
      e.preventDefault();
      answer(yes);
    }
  });
})();
