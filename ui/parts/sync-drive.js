// Google Drive sync: the card in the settings, and when it runs. It runs after the page loads, a few seconds
// after an edit or once a session or an exercise is over, every 15 minutes while the window is open, and on the
// button; each run that finds nothing new costs one listing of the app's Drive folder.
(() => {
  "use strict";
  // The small quiz window loads this page too; only the main window syncs.
  if (window.MINI) return;
  const AFTER_EDIT = 8000;
  const EVERY = 15 * 60 * 1000;
  /// Commands that change what the sync carries.
  const EDITS = new Set(["save", "delete", "grade", "undo", "skip", "xp", "set_image", "set_setting", "import", "rec_stop"]);
  let status = null, running = false, signingIn = false, problem = "", added = "", timer = 0;

  const studying = () => (typeof session !== "undefined" && !!session) || (typeof practice !== "undefined" && !!practice);
  const at = (sec) => {
    const d = new Date(sec * 1000), time = d.toLocaleTimeString("vi-VN", { hour: "2-digit", minute: "2-digit" });
    return d.toDateString() === new Date().toDateString() ? `hôm nay lúc ${time}` : `${shortDate(sec)} lúc ${time}`;
  };

  function mount() {
    if ($("sync-card")) return;
    // Before the data group, whose heading comes just before its card.
    const heading = $("btn-export").closest(".card").previousElementSibling;
    heading.insertAdjacentHTML("beforebegin", `<h3 class="group">Đồng bộ Google Drive</h3><div class="card" id="sync-card"></div>`);
    $("sync-card").addEventListener("click", (e) => {
      const act = e.target.closest("[data-sync]")?.dataset.sync;
      if (act === "sign-in") signIn();
      if (act === "cancel") app.call("cancel_sign_in").catch(() => {});
      if (act === "now") syncNow(true);
      if (act === "sign-out") signOut(e.target.closest("button"));
    });
  }

  function render() {
    const card = $("sync-card");
    if (!card || !status) return;
    const about = `<p class="muted">Từ vựng, lịch ôn, lịch sử trả lời, XP, hình ảnh và các cài đặt học tập được đồng bộ giữa các máy bạn dùng.
      Dữ liệu nằm trong thư mục ẩn dành riêng cho Card English trên Google Drive của chính bạn; app không xem được các tệp khác của bạn.</p>`;
    let body;
    if (!status.configured) {
      body = `<div class="setting-row"><div><b>Chưa có đăng nhập Google</b>
        <div class="muted">Bản Card English này được build mà không có cấu hình đăng nhập Google, nên chưa đồng bộ được. Hãy dùng bản phát hành chính thức.</div></div>
        <button class="btn" disabled>Đăng nhập Google</button></div>`;
    } else if (signingIn) {
      body = `<div class="setting-row"><div><b>Đang chờ bạn đăng nhập trong trình duyệt...</b>
        <div class="muted">Chọn tài khoản Google và cho phép Card English lưu dữ liệu của nó trong Google Drive.</div></div>
        <button class="btn" data-sync="cancel">Hủy</button></div>`;
    } else if (!status.signed_in) {
      body = `<div class="setting-row"><div><b>${status.expired ? `Phiên đăng nhập của ${esc(status.email || "bạn")} đã hết hạn` : "Chưa kết nối"}</b>
        <div class="muted">${status.expired ? "Đăng nhập lại để tiếp tục đồng bộ; dữ liệu trên máy vẫn còn nguyên." : "Đăng nhập Google một lần trên mỗi máy, app tự đồng bộ từ đó."}</div></div>
        <button class="btn primary" data-sync="sign-in">Đăng nhập Google</button></div>`;
    } else {
      const last = running ? "Đang đồng bộ..." : status.last_sync ? `Đồng bộ lần cuối ${at(status.last_sync)}.` : "Chưa đồng bộ lần nào.";
      body = `<div class="setting-row"><div><b class="sync-email">${esc(status.email || "Tài khoản Google")}</b>
        <div class="muted" id="sync-last">${last}</div></div>
        <div class="row"><button class="btn primary${running ? " loading" : ""}" data-sync="now" ${running ? "disabled" : ""}><span>Đồng bộ ngay</span></button>
        <button class="btn" data-sync="sign-out">Đăng xuất</button></div></div>
        ${added ? `<p class="ok">${esc(added)}</p>` : ""}
        ${status.error && !running ? `<p class="error">${esc(status.error)}</p>` : ""}`;
    }
    card.innerHTML = body + about + (problem ? `<p class="error">${esc(problem)}</p>` : "");
  }

  async function signIn() {
    signingIn = true;
    problem = "";
    render();
    try {
      status = await app.call("sync_sign_in");
      signingIn = false;
      toast(`Đã kết nối Google Drive: ${status.email || "tài khoản của bạn"}`);
      syncNow(true);
    } catch (e) {
      signingIn = false;
      if (String(e) !== "Đã hủy đăng nhập") problem = String(e);
    }
    render();
  }

  async function signOut(button) {
    button.disabled = true;
    try {
      const r = await app.call("sync_sign_out");
      status = r.status;
      added = problem = "";
      toast(r.revoked ? "Đã đăng xuất Google. Dữ liệu trên máy và trên Drive vẫn còn nguyên." : "Đã đăng xuất trên máy này. Google chưa xác nhận thu hồi quyền, bạn có thể gỡ quyền trong tài khoản Google.");
    } catch (e) {
      toast(String(e));
    }
    render();
  }

  /// What a sync brought from the other machines, in words.
  function news(r) {
    const parts = [];
    if (r.words) parts.push(`${r.words} từ`);
    if (r.reviews) parts.push(`${r.reviews} lượt ôn`);
    return parts.length ? `Đã thêm ${parts.join(" và ")} từ máy khác.` : "";
  }

  async function syncNow(manual) {
    if (running || !status || !status.signed_in) return;
    running = true;
    render();
    try {
      const r = await app.call("sync_now");
      status = r.status;
      added = news(r) || (manual ? "" : added);
      if (added && (r.words || r.reviews)) toast(`Đồng bộ Google Drive xong. ${added}`);
      else if (manual) toast("Đồng bộ Google Drive xong");
      if (r.changed) refresh().catch(() => {});
    } catch (e) {
      status = await app.call("sync_status").catch(() => status);
      if (manual) toast(String(e));
    }
    running = false;
    render();
  }

  /// What the page shows again after a sync changed the data: the settings that travel and the page in view.
  async function refresh() {
    state.settings = await app.call("settings");
    applyPalette(state.settings.palette);
    renderSettings();
    const page = document.querySelector("main section.active")?.id.replace("page-", "");
    if (page) reloadPage(page);
  }

  /// A sync once the learner has stopped changing things, never in the middle of a session or an exercise.
  function soon() {
    if (!status || !status.signed_in) return;
    clearTimeout(timer);
    timer = setTimeout(function wait() {
      if (studying()) timer = setTimeout(wait, AFTER_EDIT);
      else syncNow(false);
    }, AFTER_EDIT);
  }
  const call = app.call.bind(app);
  app.call = (cmd, args) => {
    const reply = call(cmd, args);
    if (EDITS.has(cmd)) reply.then(soon, () => {});
    return reply;
  };
  setInterval(() => { if (!studying()) syncNow(false); }, EVERY);

  hooks.init.push(async () => {
    mount();
    status = await app.call("sync_status").catch(() => null);
    render();
    if (status && status.signed_in) setTimeout(() => syncNow(false), 3000);
  });
})();
