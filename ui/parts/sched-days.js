// The days of review: easy days in the settings, and putting off or reviewing ahead from the home page.
(() => {
  "use strict";
  if (window.MINI) return;

  // Easy days: the weekdays the learner wants few reviews on, Monday first as a Vietnamese week goes; 0 is Sunday.
  const WEEK = [[1, "T2"], [2, "T3"], [3, "T4"], [4, "T5"], [5, "T6"], [6, "T7"], [0, "CN"]];
  const NAMES = { 0: "Chủ nhật", 1: "Thứ hai", 2: "Thứ ba", 3: "Thứ tư", 4: "Thứ năm", 5: "Thứ sáu", 6: "Thứ bảy" };
  const easy = document.createElement("div");
  easy.className = "sched-easy";
  easy.innerHTML = `
    <div class="sched-easy-title" id="sched-easy-title">Ngày ôn nhẹ</div>
    <div class="sched-chips" role="group" aria-labelledby="sched-easy-title">
      ${WEEK.map(([d, short]) => `<button type="button" class="sched-chip" data-day="${d}" aria-pressed="false" title="${NAMES[d]}">${short}</button>`).join("")}
    </div>
    <p class="muted small">Lịch ôn luôn chia đều số từ giữa những ngày gần nhau. Ngày bạn chọn ở đây sẽ có ít từ ôn nhất, phần còn lại dời sang ngày bên cạnh. Mỗi khóa học chọn riêng.</p>`;
  const card = $("s-desired_retention").closest(".card");
  card.insertBefore(easy, card.querySelector(".switch"));
  const chosen = () => new Set(String(state.settings.easy_days || "").split(",").filter((d) => /^[0-6]$/.test(d)));
  const renderEasy = () => {
    const days = chosen();
    easy.querySelectorAll("[data-day]").forEach((b) => b.setAttribute("aria-pressed", String(days.has(b.dataset.day))));
  };
  easy.addEventListener("click", async (e) => {
    const chip = e.target.closest("[data-day]");
    if (!chip) return;
    const days = chosen();
    days.has(chip.dataset.day) ? days.delete(chip.dataset.day) : days.add(chip.dataset.day);
    try {
      await saveSetting("easy_days", [...days].sort().join(","));
    } catch (err) {
      toast(String(err));
    }
    renderEasy();
  });
  hooks.init.push(renderEasy);
  hooks.course.push(renderEasy);

  // Putting off and reviewing ahead, under the session card.
  const tools = document.createElement("div");
  tools.className = "sched-tools";
  tools.innerHTML = `<button class="btn small ghost hidden" id="sched-ahead">Ôn trước từ sắp đến hạn</button>
    <button class="btn small ghost hidden" id="sched-postpone">Hoãn từ cần ôn</button>`;
  $("session-start").after(tools);
  const modal = document.createElement("div");
  modal.className = "modal hidden";
  modal.innerHTML = `<div class="dialog sched-dialog" role="dialog" aria-modal="true" aria-labelledby="sched-title">
    <h3 id="sched-title"></h3><p class="muted" id="sched-text"></p>
    <label id="sched-days-label"></label><div class="seg" id="sched-days" role="radiogroup" aria-labelledby="sched-days-label"></div>
    <p class="sched-note" id="sched-note"></p>
    <div class="row actions"><button class="btn" id="sched-cancel">Thôi</button><button class="btn primary" id="sched-ok"></button></div></div>`;
  document.body.append(modal);

  let counts = { due: 0, ahead: [0, 0, 0, 0] };
  hooks.home.push((r) => {
    // `ahead[n]`: the reviews of the next n days not due yet; the first day of the forecast counts those overdue too.
    const f = r.forecast || [];
    const later = Math.max(0, (f[0] || 0) - r.words.due);
    counts = { due: r.words.due, ahead: [0, 1, 2, 3].map((n) => later + f.slice(1, n + 1).reduce((a, b) => a + b, 0)) };
    $("sched-postpone").classList.toggle("hidden", counts.due === 0);
    $("sched-ahead").classList.toggle("hidden", counts.ahead[3] === 0);
  });

  let dialog = null;
  const close = () => { modal.classList.add("hidden"); dialog = null; };
  /// The dialog of either action: a number of days to pick, what it would do, and the button that does it.
  function open(kind) {
    const postpone = kind === "postpone";
    const max = postpone ? 7 : 3;
    dialog = { postpone, days: postpone ? 3 : 1 };
    $("sched-title").textContent = postpone ? "Hoãn từ cần ôn" : "Ôn trước";
    $("sched-text").textContent = postpone
      ? `Hôm nay bận? ${counts.due} từ đang đến hạn sẽ được dời sang những ngày tới. Từ bạn còn nhớ chắc dời xa nhất, từ sắp quên vẫn ôn sớm nhất.`
      : "Còn thời gian? Học luôn hôm nay những từ sắp đến hạn, từ gần hạn nhất trước. Lịch ôn của từ chỉ đổi khi bạn trả lời.";
    $("sched-days-label").textContent = postpone ? "Dời trong bao nhiêu ngày" : "Lấy từ đến hạn trong";
    $("sched-days").innerHTML = Array.from({ length: max }, (_, i) => i + 1).map((n) => `
      <input type="radio" name="sched-days" id="sched-d${n}" value="${n}" ${n === dialog.days ? "checked" : ""}>
      <label for="sched-d${n}">${postpone ? n : n === 1 ? "Ngày mai" : `${n} ngày tới`}${postpone ? "" : ` <span class="n">${counts.ahead[n]}</span>`}</label>`).join("");
    $("sched-ok").textContent = postpone ? "Hoãn" : "Bắt đầu học";
    renderNote();
    modal.classList.remove("hidden");
    $("sched-ok").focus();
  }
  function renderNote() {
    const n = dialog.days;
    $("sched-note").textContent = dialog.postpone
      ? `${n === 1 ? "Tất cả sang ngày mai" : `Khoảng ${Math.ceil(counts.due / n)} từ mỗi ngày, trong ${n} ngày tới`}.`
      : counts.ahead[n] ? `Buổi học hôm nay có thêm ${counts.ahead[n]} từ sắp đến hạn.` : "Chưa có từ nào đến hạn trong những ngày này.";
    $("sched-ok").disabled = !dialog.postpone && !counts.ahead[n];
  }
  $("sched-days").addEventListener("change", (e) => { dialog.days = +e.target.value; renderNote(); });
  $("sched-postpone").onclick = () => open("postpone");
  $("sched-ahead").onclick = () => open("ahead");
  $("sched-cancel").onclick = close;
  modal.addEventListener("click", (e) => { if (e.target === modal) close(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && dialog) close(); });
  $("sched-ok").onclick = async () => {
    const { postpone, days } = dialog;
    close();
    if (!postpone) return startSession({ ahead: days });
    try {
      const moved = await app.call("postpone", { days, day_start: dayStart() });
      toast(`Đã dời ${moved} từ sang ${days === 1 ? "ngày mai" : `${days} ngày tới`}`);
    } catch (err) {
      toast(String(err));
    }
    refreshHome();
  };
})();
