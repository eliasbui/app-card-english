// The quick quiz: the same page in a small window from the tray (window.MINI), one due word at a time.
(() => {
  "use strict";
  if (!window.MINI) {
    // The main page: words answered in the quick quiz change the counts, the goal and the level shown here.
    let timer;
    hooks.event.push((name) => {
      if (name !== "mini_answered") return;
      clearTimeout(timer);
      timer = setTimeout(() => refreshHome().catch(() => {}), 400);
    });
    return;
  }

  document.title = "Đố nhanh";
  const box = document.createElement("div");
  box.id = "mini";
  box.innerHTML = `
    <header class="mini-head"><b>Đố nhanh</b><span class="muted num" id="mini-count"></span><span class="grow"></span>
      <button class="btn small ghost" id="mini-close">Đóng <kbd>Esc</kbd></button></header>
    <div class="mini-card" id="mini-card"><div class="thinking center">${st("mascot-think")}<span>Đang tìm từ cần ôn...</span></div></div>`;
  document.body.append(box);
  const quiz = { queue: [], done: 0, xp: 0, flipped: false, busy: false };
  const close = () => app.call("close_mini");
  $("mini-close").onclick = close;

  hooks.init.push(async () => {
    try {
      // Reviews only: a new word needs the whole card of the session to be learned.
      quiz.queue = (await app.call("session", { day_start: dayStart() })).filter((w) => w.learned_at > 0);
    } catch (e) {
      toast(String(e));
    }
    show();
  });

  function show() {
    const w = quiz.queue[0], card = $("mini-card");
    quiz.flipped = false;
    $("mini-count").textContent = w ? `${quiz.done + 1}/${quiz.done + quiz.queue.length}` : "";
    if (!w) {
      card.innerHTML = `<div class="mini-done">${st(quiz.done ? "mascot-cheer" : "mascot-sleep")}<b>Hết từ cần ôn</b>
        <p class="muted">${quiz.done ? `Bạn vừa ôn ${quiz.done} từ, được ${quiz.xp} XP. Giỏi lắm!` : "Chưa có từ nào đến hạn. Lúc khác quay lại nhé."}</p>
        <button class="btn primary" id="mini-end">Đóng</button></div>`;
      $("mini-end").onclick = close;
      $("mini-end").focus();
      return;
    }
    card.innerHTML = `
      <span class="tag due">Ôn tập</span>
      <div class="mini-word">${esc(w.word)}</div>
      <button class="btn small" data-say="${esc(w.word)}">Nghe</button>
      <div class="mini-back hidden" id="mini-back">
        <div class="ipa">${esc(w.ipa)}${w.pos ? ` <span class="tag">${esc(w.pos)}</span>` : ""}</div>
        ${w.lang === "zh" && w.hanviet ? `<div class="muted small">Hán Việt: ${esc(w.hanviet)}</div>` : ""}
        <div class="mini-meaning">${esc(w.meaning)}</div>
        ${w.example ? `<div class="mini-example">${esc(w.example)}</div>` : ""}
      </div>
      <div class="mini-actions" id="mini-flip"><button class="btn primary" id="mini-show">Hiện đáp án <kbd>Space</kbd></button></div>
      <div class="mini-actions mini-grades hidden" id="mini-grades">
        <button class="btn g-again" data-q="1">Quên <kbd>1</kbd></button>
        <button class="btn g-hard" data-q="2">Khó <kbd>2</kbd></button>
        <button class="btn g-good" data-q="3">Được <kbd>3</kbd></button>
        <button class="btn g-easy" data-q="4">Dễ <kbd>4</kbd></button>
      </div>`;
    slideIn(card);
    $("mini-show").onclick = flip;
    $("mini-grades").querySelectorAll("[data-q]").forEach((b) => (b.onclick = () => answer(+b.dataset.q, b)));
  }

  function flip() {
    if (quiz.flipped || !quiz.queue[0]) return;
    quiz.flipped = true;
    flipCard($("mini-card"), () => {
      $("mini-back").classList.remove("hidden");
      $("mini-flip").classList.add("hidden");
      $("mini-grades").classList.remove("hidden");
    });
  }

  /// Through the same `grade` as the session, so that the schedule, the experience and the streak stay right.
  async function answer(q, button) {
    if (!quiz.flipped || quiz.busy) return;
    quiz.busy = true;
    const w = quiz.queue[0];
    try {
      const updated = await app.call("grade", { id: w.id, q });
      quiz.queue.shift();
      gainXp(GRADE_XP[q], button, q >= 2);
      quiz.xp += GRADE_XP[q];
      // A forgotten word comes back at the end, as in the session.
      if (q === 1) quiz.queue.push(updated);
      else quiz.done++;
    } catch (e) {
      toast(String(e));
    }
    quiz.busy = false;
    show();
  }

  document.addEventListener("keydown", (e) => {
    if (e.target.matches("input, textarea, select")) return;
    if (e.key === "Escape") return close();
    if (e.code === "Space") { e.preventDefault(); flip(); }
    const q = { Digit1: 1, Digit2: 2, Digit3: 3, Digit4: 4 }[e.code];
    if (q && quiz.flipped) answer(q, document.querySelector(`#mini-grades [data-q="${q}"]`));
  });
})();
