// A goal and the roadmap towards it, inside the current course: its page, and the current stage on Home.
(() => {
  "use strict";
  // The quick-quiz window loads the same page and has no roadmap to show.
  if (window.MINI) return;
  const EXERCISE = { flash: "Flashcard", fill: "Điền từ", choice: "Chọn đáp án", dict: "Nghe chép", speak: "Luyện nói", write: "Đặt câu", story: "Truyện ngắn" };
  const GOALS = {
    en: ["IELTS 6.5", "TOEIC 750", "Giao tiếp công việc", "Du lịch", "Phỏng vấn xin việc"],
    zh: ["HSK 3", "HSK 4", "Giao tiếp hằng ngày", "Du lịch", "Làm việc với đối tác Trung Quốc"],
  };
  const LEVELS = {
    en: [["", "Chưa rõ"], ["A1", "A1, mới bắt đầu"], ["A2", "A2, sơ cấp"], ["B1", "B1, trung cấp"], ["B2", "B2, khá"], ["C1", "C1, cao cấp"], ["C2", "C2, thành thạo"]],
    zh: [["", "Chưa học"], ...[1, 2, 3, 4, 5, 6].map((n) => [`HSK${n}`, `HSK ${n}`])],
  };
  /// What `roadmaps` answered: the active roadmap with the progress of its stages, the pace, and the list.
  let view = null;
  /// A plan the AI made and the learner has not saved yet, with who made it.
  let draft = null;
  /// The roadmap being edited, a copy.
  let editing = null;
  /// What the goal form holds while it is shown over an active roadmap; null when it is not.
  let goal = null;
  let planning = false, formError = "", pendingSuggest = null;
  let suggested = {};
  /// Bumped on a switch of course, so that the answer to an older request is dropped.
  let seq = 0;

  const dayOf = (day) => new Date(day + "T00:00:00");
  const addDays = (day, n) => { const d = dayOf(day); d.setDate(d.getDate() + n); return localDay(d); };
  const daysBetween = (a, b) => Math.round((dayOf(b) - dayOf(a)) / 864e5);
  const dateText = (day) => dayOf(day).toLocaleDateString("vi-VN", { day: "numeric", month: "numeric", year: "numeric" });
  const dayText = (day) => dayOf(day).toLocaleDateString("vi-VN", { day: "numeric", month: "numeric" });
  const levelOptions = (level) => LEVELS[courseOf()].map(([v, name]) => `<option value="${v}" ${v === (level || "") ? "selected" : ""}>${name}</option>`).join("");
  const decimal = (x) => x.toLocaleString("vi-VN", { maximumFractionDigits: 1 });
  /// The first and last day of each stage: they follow one another from the start of the roadmap.
  function spans(r) {
    let at = r.start || localDay();
    return r.steps.map((s) => { const from = at; at = addDays(at, Math.max(1, s.days)); return { from, to: addDays(at, -1) }; });
  }

  document.querySelector("main").insertAdjacentHTML("beforeend", `
    <section id="page-roadmap">
      <header class="page-head"><div class="grow"><h2>Lộ trình</h2><p>Đặt mục tiêu, AI chia thành từng chặng vừa sức và theo dõi bạn đi tới đâu.</p></div><div class="row" id="rm-head"></div></header>
      <div id="rm-body"></div>
    </section>`);
  document.querySelector("#page-home .dash").insertAdjacentHTML("beforebegin", '<div class="card hidden" id="roadmap-home"></div>');

  async function fetchView() {
    const mine = seq;
    const answer = await app.call("roadmaps", { today: localDay() }).catch(() => null);
    if (mine === seq && answer) view = answer;
  }
  async function load() {
    await fetchView();
    render();
    renderHome();
  }

  function render() {
    renderHead();
    const body = $("rm-body");
    if (planning) {
      body.innerHTML = `<div class="card rm-thinking"><div class="thinking">${st("mascot-think")}<span>AI đang chia mục tiêu thành từng chặng. Model trên máy có thể mất nửa phút, chờ một chút nhé.</span></div>${'<div class="skeleton"></div>'.repeat(3)}</div>`;
    } else if (editing) {
      renderEditor(body);
    } else if (draft) {
      renderPlan(body, draft.roadmap, draft.by);
    } else if (goal || !view?.active) {
      renderGoalForm(body);
    } else {
      renderPlan(body, view.active);
      // "Học tiếp" on Home asked for words for the current stage.
      if (pendingSuggest !== null) {
        const i = pendingSuggest;
        pendingSuggest = null;
        suggestFor(i);
        $("rm-stage-" + i)?.scrollIntoView({ block: "center" });
      }
    }
  }

  function renderHead() {
    const list = view?.list || [];
    $("rm-head").innerHTML = planning || editing || draft || goal ? "" : [
      list.length > 1 ? `<select id="rm-switch" aria-label="Lộ trình đang học">${list.map((r) => `<option value="${esc(r.uid)}" ${r.active ? "selected" : ""}>${esc(r.purpose)}</option>`).join("")}</select>` : "",
      view?.active ? '<button class="btn" data-rm="new-goal">Mục tiêu mới</button>' : "",
    ].join("");
  }

  function renderGoalForm(body) {
    const zh = courseOf() === "zh", g = goal || {};
    body.innerHTML = `
      <div class="card rm-start">
        <div class="rm-start-head">${st(zh ? "mascot-zh" : "mascot")}<div><b>Bạn học tiếng ${zh ? "Trung" : "Anh"} để làm gì?</b>
          <div class="muted">Cho biết mục tiêu và hạn chót. AI chia thành các chặng, mỗi chặng một nhóm chủ đề và số từ cần thuộc; bạn sửa lại được tất cả.</div></div></div>
        <label for="rm-purpose">Mục tiêu</label>
        <input id="rm-purpose" maxlength="80" placeholder="${zh ? "Ví dụ: thi HSK 4, đi làm ở Thượng Hải" : "Ví dụ: thi IELTS 6.5, giao tiếp với khách hàng"}" value="${esc(g.purpose || "")}">
        <div class="chips" id="rm-goals">${GOALS[courseOf()].map((x) => `<button type="button" data-goal="${esc(x)}">${esc(x)}</button>`).join("")}</div>
        <div class="grid3">
          <div><label for="rm-level">Trình độ hiện tại</label><select id="rm-level">${levelOptions(g.level)}</select></div>
          <div><label for="rm-deadline">Hạn chót hoặc ngày thi</label><input id="rm-deadline" type="date" min="${addDays(localDay(), 1)}" value="${g.deadline || addDays(localDay(), 90)}"></div>
          <div><label for="rm-minutes">Mỗi ngày học (phút)</label><input id="rm-minutes" type="number" min="5" max="600" step="5" value="${g.minutes || 20}"></div>
        </div>
        <div class="row actions">
          <button class="btn primary" data-rm="plan-ai">AI gợi ý lộ trình</button>
          <button class="btn" data-rm="plan-manual">Tự lập lộ trình</button>
          ${view?.active ? '<button class="btn ghost" data-rm="back">Quay lại lộ trình đang học</button>' : ""}
        </div>
        <p class="error" id="rm-error">${esc(formError)}</p>
      </div>`;
  }
  const readGoal = () => ({ purpose: $("rm-purpose").value.trim(), level: $("rm-level").value, deadline: $("rm-deadline").value, minutes: +$("rm-minutes").value || 20 });

  /// A plan for a goal: the AI's, shown to be saved, or the template, opened to be edited. A new plan for the
  /// active roadmap (`g.uid`) replaces its stages once saved.
  async function plan(ai, g) {
    const mine = seq;
    goal = g;
    formError = "";
    planning = ai;
    render();
    try {
      const r = await app.call("roadmap_plan", { goal: g, ai, today: localDay(), day: localDay() });
      if (mine !== seq) return;
      const roadmap = { uid: g.uid || "", purpose: g.purpose, level: g.level, deadline: g.deadline, minutes: g.minutes, start: localDay(), steps: r.steps };
      if (ai) draft = { roadmap, by: r }; else editing = roadmap;
      goal = null;
    } catch (e) {
      if (mine !== seq) return;
      if (g.uid) {
        goal = null;
        toast(String(e));
      } else {
        formError = String(e);
      }
    }
    planning = false;
    render();
  }

  function paceText(p) {
    if (!p) return "";
    const text = {
      done: "Bạn đã xong mọi chặng của lộ trình. Tuyệt vời!",
      test: "Bạn đã thuộc đủ từ của lộ trình, còn bài kiểm tra chặng chưa qua.",
      late: `Đã qua hạn chót mà còn ${p.remaining} từ chưa thuộc. Hãy dời hạn chót trong Sửa lộ trình.`,
      ahead: `Bạn đang đi nhanh hơn kế hoạch: gần đây bạn thuộc ${decimal(p.recent)} từ mỗi ngày, chỉ cần ${p.needed} từ mỗi ngày là kịp hạn.`,
      on_track: `Bạn đang đúng tiến độ: cần ${p.needed} từ mới mỗi ngày, gần đây bạn thuộc ${decimal(p.recent)} từ mỗi ngày.`,
      behind: `Bạn đang chậm hơn kế hoạch: cần ${p.needed} từ mới mỗi ngày để kịp hạn, gần đây bạn thuộc ${decimal(p.recent)} từ mỗi ngày.`,
    }[p.verdict];
    return text + (["ahead", "on_track", "behind"].includes(p.verdict) ? ` Còn ${p.remaining} từ trong ${p.days_left} ngày.` : "");
  }
  function paceHtml(p) {
    if (!p) return "";
    // Offered only when the number of new words a day is what holds the learner back.
    const want = Math.min(200, p.needed);
    const offer = ["on_track", "behind"].includes(p.verdict) && want > (+state.settings.new_per_day || 0)
      ? `<button class="btn small" data-rm="pace" data-n="${want}" title="Hiện mỗi ngày có ${state.settings.new_per_day} từ mới">Đặt ${want} từ mới mỗi ngày</button>` : "";
    return `<div class="rm-pace ${p.verdict}"><div class="grow">${esc(paceText(p))}</div>${offer}</div>`;
  }

  function renderPlan(body, r, by) {
    const isDraft = !!by, dates = spans(r), left = daysBetween(localDay(), r.deadline);
    const facts = [r.level ? `Trình độ lúc bắt đầu ${r.level.replace(/^HSK/, "HSK ")}` : "",
      `Hạn chót ${dateText(r.deadline)} (${left > 0 ? `còn ${left} ngày` : left === 0 ? "hôm nay" : "đã qua"})`, `${r.minutes} phút mỗi ngày`].filter(Boolean).join(", ");
    const total = r.steps.reduce((n, s) => n + s.target, 0);
    const learned = r.steps.reduce((n, s) => n + (s.state === "done" ? s.target : Math.min(s.learned || 0, s.target)), 0);
    body.innerHTML = `
      ${isDraft ? `<div class="card rm-draft">
        <b>AI đã gợi ý lộ trình ${r.steps.length} chặng.</b> Xem lại bên dưới, sửa nếu cần rồi lưu để bắt đầu.${r.uid ? " Lưu sẽ thay các chặng của lộ trình đang học." : ""}
        <div class="muted small">${byHtml(by)}</div>
        <div class="row actions"><button class="btn primary" data-rm="save-draft">Lưu lộ trình</button><button class="btn" data-rm="edit-draft">Sửa trước khi lưu</button>
          <button class="btn" data-rm="retry">AI gợi ý lại</button><button class="btn ghost" data-rm="drop-draft">Hủy</button></div>
      </div>` : ""}
      <div class="card rm-goal">
        <div class="eyebrow">Mục tiêu</div>
        <div class="rm-goal-title">${esc(r.purpose)}</div>
        <div class="muted">${esc(facts)}</div>
        ${isDraft ? "" : `
        <div class="rm-overall"><div class="progress"><div style="width:${total ? Math.round((100 * learned) / total) : 0}%"></div></div>
          <span class="muted small num">Đã thuộc ${learned}/${total} từ của lộ trình</span></div>
        ${paceHtml(view.pace)}
        <div class="row actions"><button class="btn" data-rm="edit">Sửa lộ trình</button><button class="btn" data-rm="replan">AI lập lại</button>${window.examGoal?.(r) || ""}<button class="btn danger" data-rm="delete">Xóa lộ trình</button></div>`}
      </div>
      <div class="rm-stages">${r.steps.map((s, i) => stageHtml(s, i, dates[i], isDraft)).join("")}</div>`;
  }

  function stageHtml(s, i, span, isDraft) {
    const late = !isDraft && s.state !== "done" && span.to < localDay();
    const tag = isDraft ? "" : s.state === "done" ? '<span class="tag ok">Đã xong</span>' : s.state === "current" ? '<span class="tag">Đang học</span>' : '<span class="tag plain">Sắp tới</span>';
    const pct = s.target ? Math.min(100, Math.round((100 * (s.learned || 0)) / s.target)) : 0;
    const exercises = s.exercises.map((k) => isDraft ? `<span class="tag plain">${EXERCISE[k]}</span>`
      : `<button class="btn small" data-rm="practise" data-kind="${k}" title="Luyện ${EXERCISE[k]} với từ của chặng này">${EXERCISE[k]}</button>`).join("");
    // A stage done by its count stays done; one marked done by hand can be opened again.
    const actions = (s.state === "done" ? "" : `<button class="btn small ${s.state === "current" ? "primary" : ""}" data-rm="suggest">Gợi ý từ cho chặng này</button>`)
      + (s.done ? '<button class="btn small ghost" data-rm="done">Học lại chặng này</button>' : s.state === "done" ? "" : '<button class="btn small ghost" data-rm="done">Đánh dấu xong</button>');
    return `
      <div class="card rm-stage ${isDraft ? "" : s.state}" id="rm-stage-${i}" data-i="${i}">
        <div class="rm-stage-head"><span class="rm-n">${i + 1}</span>
          <div class="grow"><b class="rm-stage-title">${esc(s.title)}</b><div class="muted small">${dayText(span.from)} đến ${dayText(span.to)}, ${s.days} ngày</div></div>
          ${tag}${late ? '<span class="tag due">Trễ hạn</span>' : ""}</div>
        <div class="rm-tags">${s.topics.map(topicTag).join("")}</div>
        <div class="rm-detail small"><span class="muted">Mục tiêu:</span> ${s.target} từ${s.skills ? `. <span class="muted">Kỹ năng:</span> ${esc(s.skills)}` : ""}</div>
        <div class="rm-ex">${exercises}</div>
        ${isDraft ? "" : `
        <div class="rm-progress"><div class="progress"><div style="width:${pct}%"></div></div>
          <span class="small muted num">Đã thuộc ${s.learned}/${s.target} từ, có ${s.words} từ trong chủ đề</span></div>
        ${window.examStage?.(s, i, view.active) || ""}
        ${actions ? `<div class="row rm-stage-actions">${actions}</div>` : ""}
        <div class="rm-sugg" id="rm-sugg-${i}"></div>`}
      </div>`;
  }

  // Editing by hand: the goal, and every stage with its order.
  function renderEditor(body) {
    const r = editing;
    const actions = '<button class="btn" data-re="cancel">Hủy</button><button class="btn primary" data-re="save">Lưu lộ trình</button>';
    body.innerHTML = `
      <div class="card">
        <div class="card-title"><span class="grow">Sửa lộ trình</span>${actions}</div>
        <div class="grid2">
          <div><label for="re-purpose">Bạn học để làm gì</label><input id="re-purpose" maxlength="80" value="${esc(r.purpose)}"></div>
          <div><label for="re-level">Trình độ lúc bắt đầu</label><select id="re-level">${levelOptions(r.level)}</select></div>
          <div><label for="re-deadline">Hạn chót hoặc ngày thi</label><input id="re-deadline" type="date" value="${esc(r.deadline)}"></div>
          <div><label for="re-minutes">Mỗi ngày học (phút)</label><input id="re-minutes" type="number" min="5" max="600" step="5" value="${r.minutes}"></div>
        </div>
        <p class="muted small" id="re-days"></p>
      </div>
      <div id="re-steps">${r.steps.map((s, i) => `
        <div class="card rm-step-form" data-i="${i}">
          <div class="row"><span class="rm-n">${i + 1}</span><input class="grow" data-s="title" maxlength="80" value="${esc(s.title)}" aria-label="Tên chặng ${i + 1}">
            <button class="btn small ghost" data-re="up" ${i === 0 ? "disabled" : ""}>Lên</button>
            <button class="btn small ghost" data-re="down" ${i === r.steps.length - 1 ? "disabled" : ""}>Xuống</button>
            <button class="btn small danger" data-re="remove" ${r.steps.length <= 1 ? "disabled" : ""}>Xóa</button></div>
          <div class="grid3">
            <div class="span2"><label for="re-${i}-topics">Chủ đề, cách nhau bởi dấu phẩy</label><input id="re-${i}-topics" data-s="topics" maxlength="200" value="${esc(s.topics.join(", "))}"></div>
            <div><label for="re-${i}-target">Số từ cần thuộc</label><input id="re-${i}-target" data-s="target" type="number" min="1" max="5000" value="${s.target}"></div>
            <div class="span2"><label for="re-${i}-skills">Kỹ năng</label><input id="re-${i}-skills" data-s="skills" maxlength="80" value="${esc(s.skills)}"></div>
            <div><label for="re-${i}-days">Số ngày</label><input id="re-${i}-days" data-s="days" type="number" min="1" max="3650" value="${s.days}"></div>
          </div>
          <div class="rm-ex-pick" role="group" aria-label="Bài tập của chặng ${i + 1}">${Object.entries(EXERCISE).map(([k, name]) =>
            `<label class="ex-chip"><input type="checkbox" data-ex="${k}" ${s.exercises.includes(k) ? "checked" : ""}>${name}</label>`).join("")}</div>
        </div>`).join("")}</div>
      <p class="error" id="re-error"></p>
      <div class="row rm-edit-actions"><button class="btn" data-re="add">Thêm chặng</button><span class="grow"></span>${actions}</div>`;
    showDays();
  }
  /// The form back into `editing`, before the stages move or the roadmap is saved.
  function collect() {
    const r = editing;
    Object.assign(r, { purpose: $("re-purpose").value.trim(), level: $("re-level").value, deadline: $("re-deadline").value, minutes: +$("re-minutes").value || 20 });
    r.steps = [...document.querySelectorAll(".rm-step-form")].map((box, i) => {
      const v = (k) => box.querySelector(`[data-s="${k}"]`).value;
      return { ...r.steps[i], title: v("title").trim(), topics: v("topics").split(/[,，、;]/).map((t) => t.trim()).filter(Boolean), target: +v("target") || 0,
               days: +v("days") || 0, skills: v("skills").trim(), exercises: [...box.querySelectorAll("[data-ex]:checked")].map((x) => x.dataset.ex) };
    });
  }
  function showDays() {
    const sum = [...document.querySelectorAll('.rm-step-form [data-s="days"]')].reduce((n, x) => n + (+x.value || 0), 0);
    const deadline = $("re-deadline").value, left = deadline ? daysBetween(editing.start || localDay(), deadline) : 0;
    $("re-days").textContent = `Các chặng dài tổng cộng ${sum} ngày; từ ngày bắt đầu ${dateText(editing.start || localDay())} tới hạn chót có ${left} ngày.`
      + (sum > left ? " Lộ trình đang kéo dài quá hạn chót." : "");
  }
  async function save(roadmap, done) {
    try {
      view = await app.call("roadmap_save", { roadmap, today: localDay() });
    } catch (e) {
      if ($("re-error")) $("re-error").textContent = e; else toast(String(e));
      return;
    }
    editing = draft = null;
    suggested = {};
    loadTopics();
    render();
    renderHome();
    if (done) toast(done);
  }

  $("rm-body").addEventListener("input", (e) => { if (editing && e.target.matches('[data-s="days"], #re-deadline')) showDays(); });
  $("rm-body").addEventListener("click", async (e) => {
    const chip = e.target.closest("[data-goal]");
    if (chip) { $("rm-purpose").value = chip.dataset.goal; return; }
    const re = e.target.closest("[data-re]");
    if (re && editing) {
      collect();
      const i = +re.closest("[data-i]")?.dataset.i, steps = editing.steps;
      if (re.dataset.re === "up" || re.dataset.re === "down") {
        const j = re.dataset.re === "up" ? i - 1 : i + 1;
        [steps[i], steps[j]] = [steps[j], steps[i]];
      }
      if (re.dataset.re === "remove") steps.splice(i, 1);
      if (re.dataset.re === "add") steps.push({ title: `Chặng ${steps.length + 1}`, topics: [], target: 50, skills: "", exercises: ["flash", "choice"], days: 14, done: false });
      if (re.dataset.re === "cancel") { editing = null; return render(); }
      if (re.dataset.re === "save") return save(editing, "Đã lưu lộ trình");
      renderEditor($("rm-body"));
      if (re.dataset.re === "add") document.querySelector(".rm-step-form:last-child [data-s='title']").focus();
      return;
    }
    const b = e.target.closest("[data-rm]");
    if (!b) return;
    const r = view?.active, i = +b.closest("[data-i]")?.dataset.i;
    switch (b.dataset.rm) {
      case "plan-ai": return plan(true, readGoal());
      case "plan-manual": return plan(false, readGoal());
      case "back": goal = null; formError = ""; return render();
      case "save-draft": return save(draft.roadmap, `Đã lưu lộ trình ${draft.roadmap.steps.length} chặng. Chúc bạn học tốt!`);
      case "edit-draft": editing = structuredClone(draft.roadmap); draft = null; return render();
      case "retry": { const g = draft.roadmap; draft = null; return plan(true, g); }
      case "drop-draft": draft = null; return render();
      case "edit": editing = structuredClone(r); return render();
      case "replan": return plan(true, { uid: r.uid, purpose: r.purpose, level: r.level, deadline: r.deadline, minutes: r.minutes });
      case "delete":
        if (!armed(b)) return;
        try {
          view = await app.call("roadmap_delete", { uid: r.uid, today: localDay() });
          toast("Đã xóa lộ trình");
        } catch (err) {
          toast(String(err));
        }
        render();
        return renderHome();
      case "pace":
        state.settings = await app.call("set_setting", { key: "new_per_day", value: b.dataset.n });
        renderSettings();
        toast(`Từ nay mỗi ngày có ${b.dataset.n} từ mới`);
        refreshBadges();
        return render();
      case "done": {
        const copy = structuredClone(r);
        copy.steps[i].done = !copy.steps[i].done;
        return save(copy, copy.steps[i].done ? `Xong chặng "${copy.steps[i].title}"!` : "");
      }
      case "suggest": return suggestFor(i);
      case "practise": return practise(b.dataset.kind, r.steps[i]);
      case "add": return addSuggested(i, +b.dataset.j);
      case "add-all": {
        for (let j = 0; j < (suggested[i] || []).length; j++) await addSuggested(i, j);
        toast(`Đã thêm từ mới vào chặng "${r.steps[i].title}"`);
        return load();
      }
      case "close-sugg": delete suggested[i]; return load();
    }
  });
  $("rm-head").addEventListener("click", (e) => {
    if (!e.target.closest('[data-rm="new-goal"]')) return;
    goal = {};
    formError = "";
    render();
    $("rm-purpose").focus();
  });
  $("rm-head").addEventListener("change", async (e) => {
    if (e.target.id !== "rm-switch") return;
    view = await app.call("roadmap_activate", { uid: e.target.value, today: localDay() }).catch(() => view);
    render();
    renderHome();
  });

  // Words for a stage from the AI, on its topics at the goal's level; they join the stage's first topic.
  async function suggestFor(i) {
    const r = view.active, s = r.steps[i], box = $("rm-sugg-" + i);
    if (!box) return;
    box.innerHTML = `<div class="thinking">${st("mascot-think")}<span>AI đang tìm từ cho chặng "${esc(s.title)}"...</span></div><div class="suggestions">${'<div class="skeleton"></div>'.repeat(4)}</div>`;
    const topic = s.topics.join(", ").slice(0, 64) + (r.level ? ` (${r.level})` : "");
    try {
      const found = await app.call("suggest", { topic, day: localDay() });
      suggested[i] = found.items;
      box.innerHTML = `
        <div class="muted small">${found.items.length} từ mới, sẽ thêm vào chủ đề "${esc(s.topics[0])}". ${byHtml(found)}</div>
        <div class="suggestions">${found.items.map((w, j) => `
          <div class="sugg" id="rm-sg-${i}-${j}"><div class="grow"><b>${esc(w.word)}</b> <span class="muted">${esc(w.ipa)}</span><div>${esc(w.meaning)}</div></div>
            <button class="btn small" data-say="${esc(w.word)}">Nghe</button><button class="btn small primary" data-rm="add" data-j="${j}">Thêm</button></div>`).join("")}</div>
        <div class="row"><button class="btn small primary" data-rm="add-all">Thêm tất cả</button><button class="btn small ghost" data-rm="close-sugg">Xong</button></div>`;
    } catch (e) {
      box.innerHTML = `<span class="error">${esc(e)}</span> <button class="btn small" data-rm="suggest">Thử lại</button>`;
    }
  }
  async function addSuggested(i, j) {
    const w = suggested[i]?.[j], box = $(`rm-sg-${i}-${j}`), topic = view.active.steps[i].topics[0] || "";
    if (!w || w.added || !box) return;
    w.added = true;
    const button = box.querySelector("[data-rm]");
    button.disabled = true;
    try {
      const saved = await app.call("save", { word: { id: 0, word: w.word, meaning: w.meaning, ipa: w.ipa, topic } });
      box.classList.add("added", "pop");
      button.textContent = "Đã thêm";
      gainXp(3, button);
      // The example and the explanation follow in the background, as for the other suggestions.
      app.call("explain", { word: w.word, day: localDay() })
        .then((e) => app.call("save", { word: { ...saved, ...Object.fromEntries(EXPLAINED.filter((f) => f !== "meaning").map((f) => [f, e[f] || ""])), topic } }))
        .catch(() => {});
    } catch {
      button.textContent = "Đã có";
    }
  }

  /// An exercise of the app on the words of all the topics of a stage, counted from the list itself: the topics'
  /// counts are loaded once and miss the words added since.
  async function practise(kind, s) {
    const usable = (await listCourse()).filter((w) => w.meaning && !w.skipped && s.topics.includes(w.topic));
    if (!usable.length && kind !== "story") return toast("Chặng này chưa có từ nào học được. Bấm Gợi ý từ cho chặng này để thêm từ trước.");
    go("practice");
    await state.sources;
    const select = $("practice-source"), value = "topics:" + s.topics.join("|");
    select.insertAdjacentHTML("beforeend", `<option value="${esc(value)}">Chặng: ${esc(s.title)}</option>`);
    select.value = value;
    startPractice(kind);
  }

  // Home: the current stage, its progress and the pace, with a button to go on.
  function renderHome() {
    const box = $("roadmap-home"), r = view?.active;
    box.classList.toggle("hidden", !r);
    if (!r) return;
    const s = r.current == null ? null : r.steps[r.current];
    const pct = s ? Math.min(100, Math.round((100 * s.learned) / s.target)) : 100;
    box.innerHTML = `
      <div class="card-title"><span class="grow">Lộ trình: ${esc(r.purpose)}</span><button class="btn small ghost" data-rh="open">Xem lộ trình</button></div>
      <div class="rm-home-stage"><b>${s ? `Chặng ${r.current + 1}/${r.steps.length}: ${esc(s.title)}` : "Bạn đã đi hết lộ trình"}</b>
        ${s ? `<span class="muted small num">Đã thuộc ${s.learned}/${s.target} từ</span>` : ""}</div>
      <div class="progress"><div style="width:${pct}%"></div></div>
      <div class="rm-home-foot"><span class="muted">${esc(s?.test?.pending ? `Bạn đã thuộc đủ từ của chặng, làm bài kiểm tra (cần đạt ${s.test.pass_text}) để sang chặng tiếp.` : paceText(view.pace))}</span>${s ? '<button class="btn primary" data-rh="go">Học tiếp</button>' : ""}</div>`;
  }
  $("roadmap-home").addEventListener("click", (e) => {
    const b = e.target.closest("[data-rh]");
    if (!b) return;
    if (b.dataset.rh === "open") return go("roadmap");
    const r = view.active, s = r.steps[r.current];
    // A hard roadmap's stage whose words are learned waits for its test.
    if (s.test?.pending && window.openExam) return window.openExam(r.uid, r.current);
    // Today's session first; with nothing left in it, words for the stage, else its exercises.
    if (state.stats && state.stats.due + state.stats.new_today > 0) return startSession({ topic: "" });
    if (s.words < s.target) { pendingSuggest = r.current; return go("roadmap"); }
    practise(s.exercises[0] || "flash", s);
  });

  hooks.page.push((page) => { if (page === "roadmap") load(); });
  (hooks.home ??= []).push(async () => { await fetchView(); renderHome(); });
  hooks.course.push(() => {
    seq++;
    view = draft = editing = goal = null;
    planning = false;
    formError = "";
    suggested = {};
    renderHome();
  });
})();
