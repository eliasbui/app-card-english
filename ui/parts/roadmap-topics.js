// Topics the learner makes, and picking many words of the list at once to delete, set aside or give a topic.
(() => {
  "use strict";
  // The quick-quiz window loads the same page and has no word list.
  if (window.MINI) return;
  const COLORS = { blue: "Xanh dương", green: "Xanh lá", amber: "Vàng", red: "Đỏ", purple: "Tím", teal: "Xanh ngọc", gray: "Xám" };
  /// The learner's own topics shown first among the suggestion chips, the ones with the most words.
  const CHIPS = 12;
  state.topics = [];
  const picked = new Set();
  let shown = [], lastPick = null, editingTopic = null;

  /// A topic as a tag in its colour.
  window.topicTag = (name) => `<span class="tag ttag tc-${state.topics.find((t) => t.name === name)?.color || "none"}">${esc(name)}</span>`;

  async function loadTopics() {
    try {
      state.topics = (await app.call("topics")).sort((a, b) => a.name.localeCompare(b.name, "vi"));
    } catch {
      state.topics = [];
    }
    renderChips();
    renderCard();
    renderStudyTopics();
    if ($("page-words").classList.contains("active")) renderWords();
  }
  window.loadTopics = loadTopics;

  // Home: today's session on the words of one topic only.
  $("session-hint").insertAdjacentHTML("afterend", `
    <div class="row study-topic hidden"><label for="study-topic">Học theo chủ đề</label>
      <select id="study-topic"><option value="">Mọi chủ đề</option></select></div>`);
  function renderStudyTopics() {
    const select = $("study-topic"), keep = select.value;
    const names = state.topics.filter((t) => t.words > 0).map((t) => t.name);
    select.innerHTML = '<option value="">Mọi chủ đề</option>' + names.map((n) => `<option value="${esc(n)}">${esc(n)}</option>`).join("");
    select.value = names.includes(keep) ? keep : "";
    select.parentElement.classList.toggle("hidden", !names.length);
  }
  $("study-topic").onchange = () => refreshHome();
  // The numbers, the hint and the start button of Home then count only what a session on the topic would bring.
  hooks.home.push(async () => {
    const topic = $("study-topic").value;
    if (!topic) return;
    const words = await app.call("session", { day_start: dayStart(), topic });
    if ($("study-topic").value !== topic) return;
    const due = words.filter((w) => w.learned_at > 0).length, n = words.length;
    const t = state.topics.find((x) => x.name === topic);
    countUp($("st-due"), due);
    countUp($("st-new"), n - due);
    $("session-hint").textContent = n ? `Chỉ các từ của chủ đề "${topic}", khoảng ${Math.max(1, Math.round(n * 0.4))} phút là xong.`
      : t && t.words > t.learned ? `Hôm nay bạn đã học đủ số từ mới. Luyện thêm chủ đề "${topic}" ở mục Bài tập, hoặc tăng số từ mới mỗi ngày trong Cài đặt.`
      : `Chủ đề "${topic}" hôm nay không còn từ cần ôn. Luyện thêm ở mục Bài tập.`;
    $("btn-start").disabled = n === 0;
    $("btn-start").classList.toggle("pulse", n > 0);
  });

  /// The chips of the AI suggestions: the learner's topics, then the usual ones of the course.
  function renderChips() {
    const mine = state.topics.slice().sort((a, b) => b.words - a.words).slice(0, CHIPS).map((t) => t.name);
    $("topics").innerHTML = [...new Set([...mine, ...TOPICS[courseOf()]])].map((t) => `<button data-topic="${esc(t)}">${esc(t)}</button>`).join("");
  }

  // The card where topics are made, renamed and deleted.
  $("btn-open-suggest").insertAdjacentHTML("beforebegin", '<button class="btn" id="btn-topics">Chủ đề</button>');
  $("suggest-card").insertAdjacentHTML("afterend", `
    <div class="card hidden" id="topics-card">
      <div class="row"><b class="grow">Chủ đề của bạn</b><button class="btn small" id="btn-topics-close">Đóng</button></div>
      <p class="muted small">Gom từ theo chủ đề để lọc, luyện tập và lập lộ trình. Đổi tên một chủ đề thì mọi từ của nó đổi theo; xóa chủ đề thì từ vẫn còn, chỉ không còn chủ đề.</p>
      <div class="topic-form" id="topic-new"></div>
      <div class="topic-list" id="topic-rows"></div>
    </div>`);
  const formHtml = (t, key) => `
    <div class="row">
      <input class="grow" data-f="name" maxlength="40" placeholder="Tên chủ đề, ví dụ: Phỏng vấn xin việc" value="${esc(t.name)}" aria-label="Tên chủ đề">
      <input class="grow" data-f="description" maxlength="200" placeholder="Mô tả ngắn, không bắt buộc" value="${esc(t.description)}" aria-label="Mô tả chủ đề">
    </div>
    <div class="row">
      <div class="swatches" role="radiogroup" aria-label="Màu của chủ đề">${Object.entries(COLORS).map(([c, name]) => `
        <label class="swatch tc-${c}" title="${name}"><input type="radio" name="tc-${key}" value="${c}" ${c === (t.color || "blue") ? "checked" : ""}><span class="sr-only">${name}</span></label>`).join("")}</div>
      <span class="grow"></span>
      ${key === "new" ? '<button class="btn small primary" data-t="add">Thêm chủ đề</button>'
        : '<button class="btn small" data-t="cancel">Hủy</button><button class="btn small primary" data-t="save">Lưu</button>'}
    </div>
    <div class="error" data-f="error"></div>`;
  function renderCard() {
    if ($("topics-card").classList.contains("hidden")) return;
    if (!$("topic-new").innerHTML) $("topic-new").innerHTML = formHtml({ name: "", description: "", color: "" }, "new");
    $("topic-rows").innerHTML = state.topics.map((t) => editingTopic === t.name
      ? `<div class="topic-row" data-name="${esc(t.name)}"><div class="topic-form">${formHtml(t, "edit")}</div></div>`
      : `<div class="topic-row" data-name="${esc(t.name)}">
          <i class="tdot tc-${t.color || "none"}"></i>
          <div class="grow"><b>${esc(t.name)}</b> <span class="muted small">${t.words ? `${t.words} từ, ${t.learned} đã thuộc` : "chưa có từ nào"}</span>
            ${t.description ? `<div class="muted small">${esc(t.description)}</div>` : ""}</div>
          ${t.words ? '<button class="btn small" data-t="study">Học</button><button class="btn small ghost" data-t="filter">Xem từ</button>' : ""}
          <button class="btn small" data-t="edit">Sửa</button>
          <button class="btn small danger" data-t="del">Xóa</button>
        </div>`).join("") || '<p class="muted">Chưa có chủ đề nào. Đặt tên cho chủ đề đầu tiên ở trên.</p>';
  }
  function openCard() {
    $("topics-card").classList.remove("hidden");
    loadTopics();
    $("topics-card").scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
  $("btn-topics").onclick = () => ($("topics-card").classList.contains("hidden") ? openCard() : $("topics-card").classList.add("hidden"));
  $("btn-topics-close").onclick = () => $("topics-card").classList.add("hidden");
  const formOf = (box) => ({
    name: box.querySelector('[data-f="name"]').value.trim(),
    description: box.querySelector('[data-f="description"]').value.trim(),
    color: box.querySelector("input[type=radio]:checked")?.value || "",
  });
  async function saveTopic(box, topic, old) {
    try {
      state.topics = await app.call("topic_save", { topic, old });
    } catch (e) {
      box.querySelector('[data-f="error"]').textContent = e;
      return false;
    }
    // A renamed topic: its words changed, and the filter follows it.
    if (old && old !== topic.name) {
      if ($("topic-filter").value === old) $("topic-filter").value = "";
      toast(`Đã đổi "${old}" thành "${topic.name}"`);
      loadWords();
    }
    renderChips();
    return true;
  }
  $("topics-card").addEventListener("click", async (e) => {
    const b = e.target.closest("[data-t]");
    if (!b) return;
    const row = b.closest(".topic-row"), t = row && state.topics.find((x) => x.name === row.dataset.name);
    if (b.dataset.t === "add") {
      const box = $("topic-new"), form = formOf(box);
      if (!form.name) return box.querySelector('[data-f="name"]').focus();
      if (await saveTopic(box, { uid: "", ...form }, "")) {
        toast(`Đã thêm chủ đề "${form.name}"`);
        box.innerHTML = "";
        renderCard();
        renderWords();
      }
    }
    if (!t) return;
    if (b.dataset.t === "edit") { editingTopic = t.name; renderCard(); row.querySelector('[data-f="name"]')?.focus(); }
    if (b.dataset.t === "cancel") { editingTopic = null; renderCard(); }
    if (b.dataset.t === "save" && (await saveTopic(row, { uid: t.uid, ...formOf(row) }, t.name))) {
      editingTopic = null;
      renderCard();
      renderWords();
    }
    if (b.dataset.t === "study") {
      $("study-topic").value = t.name;
      go("home");
      return startSession();
    }
    if (b.dataset.t === "filter") {
      $("topic-filter").value = t.name;
      renderWords();
      document.querySelector("#page-words .list-tools").scrollIntoView({ behavior: "smooth" });
    }
    if (b.dataset.t === "del" && armed(b)) {
      try {
        state.topics = await app.call("topic_delete", { uid: t.uid, name: t.name });
        toast(t.words ? `Đã xóa chủ đề "${t.name}", ${t.words} từ của nó không còn chủ đề` : `Đã xóa chủ đề "${t.name}"`);
      } catch (err) {
        return toast(String(err));
      }
      renderChips();
      renderCard();
      loadWords();
    }
  });
  $("topics-card").addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || !e.target.matches("[data-f]")) return;
    e.target.closest(".topic-form").querySelector('[data-t="add"], [data-t="save"]')?.click();
  });

  // Picking words: a box on every row, all the rows shown from the header or the bar, a range with Shift.
  document.querySelector("#page-words .list-card").insertAdjacentHTML("beforebegin", `
    <div class="pick-bar hidden" id="pick-bar" role="toolbar" aria-label="Các từ đã chọn">
      <label class="pick"><input type="checkbox" id="pick-bar-all" aria-label="Chọn mọi từ đang hiện"></label>
      <b id="pick-count" aria-live="polite"></b>
      <button class="btn small ghost" id="pick-clear">Bỏ chọn</button>
      <div class="pick-acts">
        <button class="btn small" id="pick-topic">Gán chủ đề</button>
        <button class="btn small" id="pick-skip">Bỏ qua</button>
        <button class="btn small danger" id="pick-del">Xóa</button>
      </div>
    </div>`);
  const pickedWords = () => state.words.filter((w) => picked.has(w.id));
  function syncPicks() {
    document.querySelectorAll("#word-rows [data-pick]").forEach((box) => {
      box.checked = picked.has(+box.dataset.pick);
      box.closest("tr").classList.toggle("picked", box.checked);
    });
    const n = picked.size, all = n > 0 && n === shown.length;
    for (const box of [$("pick-all"), $("pick-bar-all")]) {
      box.checked = all;
      box.indeterminate = n > 0 && !all;
      box.disabled = !shown.length;
    }
    $("pick-bar").classList.toggle("hidden", n === 0);
    $("pick-count").textContent = `Đã chọn ${n} từ`;
    const words = pickedWords();
    $("pick-skip").textContent = words.length && words.every((w) => w.skipped) ? "Học lại" : "Bỏ qua";
  }
  // Drawn again after a search or a filter: only the rows still shown stay picked.
  (hooks.words ??= []).push((rows) => {
    shown = rows.map((w) => w.id);
    const visible = new Set(shown);
    for (const id of picked) if (!visible.has(id)) picked.delete(id);
    syncPicks();
  });
  const clearPicks = () => { picked.clear(); lastPick = null; syncPicks(); };
  // Shift keeps the browser from selecting the text between two clicks.
  $("word-rows").addEventListener("mousedown", (e) => { if (e.shiftKey && e.target.closest(".pick")) e.preventDefault(); });
  $("word-rows").addEventListener("click", (e) => {
    const box = e.target.closest("[data-pick]");
    if (!box) return;
    const id = +box.dataset.pick;
    if (e.shiftKey && lastPick !== null && shown.includes(lastPick)) {
      const [a, b] = [shown.indexOf(lastPick), shown.indexOf(id)].sort((x, y) => x - y);
      shown.slice(a, b + 1).forEach((x) => (box.checked ? picked.add(x) : picked.delete(x)));
    } else if (box.checked) picked.add(id); else picked.delete(id);
    lastPick = id;
    syncPicks();
  });
  for (const id of ["pick-all", "pick-bar-all"]) {
    $(id).addEventListener("change", (e) => {
      shown.forEach((x) => (e.target.checked ? picked.add(x) : picked.delete(x)));
      syncPicks();
    });
  }
  $("pick-clear").onclick = clearPicks;
  $("pick-del").onclick = async (e) => {
    const b = e.currentTarget, n = picked.size;
    if (!armed(b)) {
      b.textContent = n > 1 ? `Bấm lần nữa để xóa ${n} từ` : "Bấm lần nữa để xóa";
      return;
    }
    try {
      await app.call("delete_many", { ids: [...picked] });
      toast(`Đã xóa ${n} từ`);
    } catch (err) {
      return toast(String(err));
    }
    clearPicks();
    loadWords();
    loadTopics();
  };
  $("pick-skip").onclick = async (e) => {
    const words = pickedWords(), on = !words.every((w) => w.skipped);
    e.currentTarget.disabled = true;
    try {
      await app.call("skip_many", { ids: [...picked], on });
      toast(on ? `Đã bỏ qua ${words.length} từ, chúng không còn trong buổi học và bài tập` : `Đã đưa ${words.length} từ trở lại danh sách học`);
      await loadWords();
    } catch (err) {
      toast(String(err));
    }
    e.currentTarget.disabled = false;
  };

  // Giving the picked words a topic: one of the course's, a new one, or none.
  document.body.insertAdjacentHTML("beforeend", `
    <div id="pick-topic-modal" class="modal hidden">
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="ptm-title">
        <h3 id="ptm-title"></h3>
        <p class="muted">Chọn một chủ đề có sẵn, hoặc nhập tên chủ đề mới.</p>
        <div class="chips" id="ptm-chips"></div>
        <div class="row"><input id="ptm-name" class="grow" list="topic-list" maxlength="40" placeholder="Chủ đề mới" aria-label="Tên chủ đề"><button class="btn primary" id="ptm-ok">Gán</button></div>
        <div class="row actions"><button class="btn ghost" id="ptm-none">Bỏ chủ đề của các từ này</button><span class="grow"></span><button class="btn" id="ptm-cancel">Hủy</button></div>
      </div>
    </div>`);
  const closeTopicModal = () => $("pick-topic-modal").classList.add("hidden");
  $("pick-topic").onclick = () => {
    $("ptm-title").textContent = `Gán chủ đề cho ${picked.size} từ`;
    $("ptm-chips").innerHTML = state.topics.map((t) => `<button data-topic="${esc(t.name)}"><i class="tdot tc-${t.color || "none"}"></i>${esc(t.name)}</button>`).join("");
    $("ptm-name").value = "";
    $("pick-topic-modal").classList.remove("hidden");
    $("ptm-name").focus();
  };
  async function giveTopic(topic) {
    const n = picked.size;
    try {
      await app.call("set_topic_many", { ids: [...picked], topic });
      toast(topic ? `Đã gán chủ đề "${topic}" cho ${n} từ` : `Đã bỏ chủ đề của ${n} từ`);
    } catch (err) {
      return toast(String(err));
    }
    closeTopicModal();
    await loadWords();
    loadTopics();
  }
  $("ptm-chips").addEventListener("click", (e) => { const b = e.target.closest("[data-topic]"); if (b) giveTopic(b.dataset.topic); });
  $("ptm-ok").onclick = () => ($("ptm-name").value.trim() ? giveTopic($("ptm-name").value.trim()) : $("ptm-name").focus());
  $("ptm-name").addEventListener("keydown", (e) => { if (e.key === "Enter") $("ptm-ok").click(); });
  $("ptm-none").onclick = () => giveTopic("");
  $("ptm-cancel").onclick = closeTopicModal;
  $("pick-topic-modal").addEventListener("click", (e) => { if (e.target === $("pick-topic-modal")) closeTopicModal(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeTopicModal(); });

  hooks.init.push(loadTopics);
  hooks.course.push(() => { clearPicks(); editingTopic = null; state.topics = []; loadTopics(); });
  hooks.page.push((page) => { if (page === "words") loadTopics(); });
})();
