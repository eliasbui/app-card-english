// What the tutor's `do`, `plan` and `guide` cards (chat-cards.js) and the voice mode (chat-voice.js) can have the app do.
// Each tool takes the arguments the backend validated (src/assistant.rs) and answers with a short line to show, or
// throws the reason in Vietnamese. The tools use the page's own functions and commands, so the page stays as if the
// learner had clicked there.
(() => {
  "use strict";
  if (window.MINI) return;

  const PAGES = ["home", "words", "practice", "roadmap", "stats", "settings", "talk"];
  const PAGE_NAMES = { home: "Học hôm nay", words: "Từ vựng", practice: "Bài tập", roadmap: "Lộ trình", stats: "Thống kê", settings: "Cài đặt", talk: "Gia sư nói" };
  const EXERCISES = ["flash", "fill", "choice", "dict", "speak", "write", "match", "order", "shadow", "smart", "tone", "hanzi"];
  const TIERS = ["local:llm", "local:llm15", "chatgpt", "codex", "gemini", "deepseek"];
  const fail = (text) => { throw new Error(text); };
  const text = (v) => String(v ?? "").trim();
  const num = (v) => (text(v) === "" ? NaN : Number(v));

  /// "20:00", "8:5", "20h30" or "" as the "HH:MM" the settings keep; "" turns the time off.
  function clock(v, what) {
    const s = text(v).toLowerCase().replace("h", ":");
    if (!s) return "";
    const m = /^(\d{1,2}):?(\d{1,2})?$/.exec(s);
    if (!m || +m[1] > 23 || +(m[2] || 0) > 59) fail(`${what} không hợp lệ, hãy dùng dạng giờ:phút như 20:00`);
    return `${m[1].padStart(2, "0")}:${String(m[2] || 0).padStart(2, "0")}`;
  }
  /// The page in view, such as "words".
  const pageNow = () => document.querySelector("main section.active")?.id.replace("page-", "") || "home";
  /// A word of the course by its text, as the list shows it; the list is asked afresh, since it may not be on screen.
  async function wordNamed(word) {
    const w = text(word);
    if (!w) fail("Chưa nói rõ từ nào");
    const found = (await listCourse(w)).find((x) => x.word.toLowerCase() === w.toLowerCase());
    return found || fail(`Không thấy từ "${w}" trong danh sách của bạn`);
  }
  /// The numbers on the sidebar and the word list, after words were added, skipped or deleted.
  function refreshWords() {
    refreshBadges();
    if (pageNow() === "words") loadWords();
  }
  /// What the page shows after a setting was saved, as the controls of the settings page do it.
  async function saved(key, value) {
    state.settings = await app.call("set_setting", { key, value: String(value) });
    renderSettings();
    if (key === "theme") applyTheme(state.settings.theme);
    if (key === "palette") applyPalette(state.settings.palette);
    if (key === "tutor_ai") {
      const box = $("chat-model");
      if (box && window.chatChoices) box.innerHTML = window.chatChoices();
    }
  }
  const rate = (v) => (+v).toFixed(2).replace(/0$/, "").replace(/\.0$/, "");

  /// The values a setting takes: [check of the value, text to save, words for the result line].
  const SETTINGS = {
    new_per_day: (v) => { const n = Math.round(num(v)); return n >= 1 && n <= 200 ? [n, `${n} từ mới mỗi ngày`] : fail("Số từ mới mỗi ngày phải từ 1 đến 200"); },
    daily_goal: (v) => { const n = Math.round(num(v)); return n >= 10 && n <= 1000 ? [n, `mục tiêu ${n} XP mỗi ngày`] : fail("Mục tiêu XP phải từ 10 đến 1000"); },
    theme: (v) => ({ system: [v, "giao diện theo hệ thống"], light: [v, "giao diện sáng"], dark: [v, "giao diện tối"] })[text(v)] || fail("Giao diện phải là system, light hoặc dark"),
    palette: (v) => ({ blue: "xanh dương", pink: "hồng", purple: "tím", red: "đỏ" })[text(v)] ? [text(v), `màu chủ đạo ${({ blue: "xanh dương", pink: "hồng", purple: "tím", red: "đỏ" })[text(v)]}`] : fail("Màu phải là blue, pink, purple hoặc red"),
    accent: (v) => ({ "en-US": "giọng Anh - Mỹ", "en-GB": "giọng Anh - Anh" })[text(v)] ? [text(v), `${{ "en-US": "giọng Anh - Mỹ", "en-GB": "giọng Anh - Anh" }[text(v)]}`] : fail("Giọng phải là en-US hoặc en-GB"),
    speech_rate: (v) => { const n = num(v); return n >= 0.5 && n <= 1.5 ? [rate(n), `tốc độ đọc ${rate(n)}`] : fail("Tốc độ đọc phải từ 0.5 đến 1.5"); },
    desired_retention: (v) => { const n = num(v) > 1 ? num(v) / 100 : num(v); return n >= 0.7 && n <= 0.97 ? [rate(n), `mức ghi nhớ ${Math.round(n * 100)}%`] : fail("Mức ghi nhớ phải từ 70% đến 97%"); },
    streak_freeze: (v) => { const on = ["1", "true", "bật"].includes(text(v).toLowerCase()); return [on ? "1" : "0", on ? "cho nghỉ một ngày mỗi tuần mà giữ chuỗi" : "không nghỉ ngày nào giữ chuỗi"]; },
    easy_days: (v) => {
      const days = (Array.isArray(v) ? v : text(v).split(",")).map(text).filter((d) => d !== "");
      if (days.some((d) => !/^[0-6]$/.test(d))) fail("Ngày nhẹ phải là các số từ 0 (Chủ nhật) đến 6 (thứ Bảy)");
      const list = [...new Set(days)].sort().join(",");
      return [list, list ? `ngày nhẹ: ${list}` : "không có ngày nhẹ"];
    },
    tutor_ai: (v) => (text(v) === "" || TIERS.includes(text(v)) ? [text(v), text(v) ? `trợ lý trả lời bằng ${text(v)}` : "trợ lý theo nguồn AI chung"] : fail("Nguồn AI của trợ lý không hợp lệ")),
  };

  const tools = {
    async open_page({ page } = {}) {
      if (!PAGES.includes(page)) fail("Không có trang này");
      if (page === "talk" && !$("page-talk")) fail("Trang Gia sư nói chưa dùng được");
      window.chatClose?.();
      go(page);
      return `Đã mở trang ${PAGE_NAMES[page]}`;
    },

    async start_study({ topic } = {}) {
      window.chatClose?.();
      go("home");
      const t = text(topic);
      const started = await startSession(t ? { topic: t } : {});
      return started ? (t ? `Bắt đầu học chủ đề ${t}` : "Bắt đầu buổi học hôm nay") : fail(t ? `Chưa có từ nào cần học ở chủ đề ${t}` : "Hôm nay không còn từ nào cần học");
    },

    async start_practice({ exercise, source } = {}) {
      if (!EXERCISES.includes(exercise)) fail("Không có dạng bài tập này");
      const from = text(source) || "all";
      if (!/^(all|hard|recent|weak|topic:.+)$/.test(from)) fail("Nguồn từ của bài tập không hợp lệ");
      go("practice");
      await state.sources;
      const select = $("practice-source"), value = from === "recent" ? "all" : from;
      if (![...select.options].some((o) => o.value === value)) {
        select.insertAdjacentHTML("beforeend", `<option value="${esc(value)}">${esc(value.replace(/^topic:/, "Chủ đề: "))}</option>`);
      }
      select.value = value;
      await startPractice(exercise);
      // startPractice reports why it could not begin on the page, not by throwing.
      const why = $("practice-error").textContent.trim();
      if (why) fail(why);
      window.chatClose?.();
      return `Bắt đầu bài tập ${exercise}`;
    },

    async open_exam({ stage } = {}) {
      const view = await app.call("roadmaps", { today: localDay() });
      if (!view.active) fail("Bạn chưa có lộ trình nào để thi thử");
      if (!window.openExam) fail("Bài thi chưa dùng được");
      const at = stage === undefined || stage === null || stage === "" ? -1 : Math.round(num(stage));
      if (!Number.isFinite(at)) fail("Số chặng không hợp lệ");
      window.chatClose?.();
      window.openExam(view.active.uid, at);
      return at < 0 ? "Đã mở bài thi thử" : `Đã mở bài kiểm tra chặng ${at + 1}`;
    },

    async open_vocab_test() {
      if (!window.openVocabTest) fail("Bài kiểm tra vốn từ chưa dùng được");
      window.chatClose?.();
      window.openVocabTest();
      return "Đã mở bài kiểm tra vốn từ";
    },

    async open_packs() {
      if (!window.openPacks) fail("Kho bộ từ chưa dùng được");
      window.chatClose?.();
      window.openPacks();
      return "Đã mở kho bộ từ";
    },

    async open_word({ word } = {}) {
      const w = text(word);
      if (!w) fail("Chưa nói rõ từ nào");
      // The search box is set before the page loads, so that its list is the one asked for.
      $("search").value = w;
      window.chatClose?.();
      go("words");
      await loadWords();
      const found = state.words.find((x) => x.word.toLowerCase() === w.toLowerCase());
      if (!found) return state.words.length ? `Chưa có đúng từ "${w}", đây là các từ gần giống` : fail(`Không thấy từ "${w}" trong danh sách của bạn`);
      openWord(found.id);
      return `Đã mở từ "${found.word}"`;
    },

    async add_words({ words, topic } = {}) {
      const list = [...new Set((Array.isArray(words) ? words : []).map(text).filter(Boolean))];
      if (!list.length) fail("Chưa có từ nào để thêm");
      const zhCourse = courseOf() === "zh", t = text(topic);
      let added = 0;
      const already = [];
      for (const word of list) {
        try {
          const saved = await app.call("save", { word: { id: 0, word, meaning: "", ipa: "", topic: t } });
          added++;
          // The explanation follows in the background, as for the cards of suggested words.
          app.call("explain", { word, day: localDay() })
            .then((x) => app.call("save", { word: { ...saved, ...Object.fromEntries(EXPLAINED.map((f) => [f, x[f] || ""])), topic: saved.topic } }))
            .catch(() => {});
        } catch {
          already.push(word);
        }
      }
      refreshWords();
      if (!added) fail(`Các từ này đã có trong danh sách: ${already.join(", ")}`);
      return `Đã thêm ${added} từ${already.length ? `, ${already.length} từ đã có sẵn` : ""}${zhCourse ? "" : ", AI đang giải thích từng từ"}`;
    },

    async skip_word({ word } = {}) {
      const w = await wordNamed(word);
      await app.call("skip", { id: w.id, on: true });
      refreshWords();
      return `Đã bỏ qua từ "${w.word}"`;
    },

    async delete_word({ word } = {}) {
      const w = await wordNamed(word);
      await app.call("delete", { id: w.id });
      refreshWords();
      return `Đã xóa từ "${w.word}"`;
    },

    async set_reminder({ at, every_hours, quiet_from, quiet_to } = {}) {
      const changes = [];
      const set = (key, value, line) => changes.push([key, value, line]);
      if (at !== undefined) {
        const v = clock(at, "Giờ nhắc");
        set("remind_at", v, v ? `nhắc hằng ngày lúc ${v}` : "tắt nhắc hằng ngày");
      }
      if (every_hours !== undefined) {
        const n = Math.round(num(every_hours));
        if (!(n >= 0 && n <= 24)) fail("Số giờ giữa các lần nhắc phải từ 0 đến 24");
        set("remind_hours", n, n ? `nhắc sau mỗi ${n} giờ` : "tắt nhắc theo giờ");
      }
      if (quiet_from !== undefined) { const v = clock(quiet_from, "Giờ bắt đầu im lặng"); set("quiet_from", v, v ? `im lặng từ ${v}` : "bỏ giờ im lặng"); }
      if (quiet_to !== undefined) { const v = clock(quiet_to, "Giờ hết im lặng"); set("quiet_to", v, v ? `im lặng tới ${v}` : "bỏ giờ im lặng"); }
      if (!changes.length) fail("Chưa nói rõ nhắc học lúc mấy giờ");
      for (const [key, value] of changes) await saved(key, value);
      return `Đã đặt nhắc học: ${changes.map((c) => c[2]).join(", ")}`;
    },

    async set_setting({ key, value } = {}) {
      if (key === "course") return tools.switch_course({ course: value });
      const check = SETTINGS[key];
      if (!check) fail("Không đổi được cài đặt này");
      const [save, line] = check(value);
      await saved(key, save);
      if (key === "accent" && !String(state.settings.voice).includes(":")) await saved("voice", "");
      return `Đã đổi: ${line}`;
    },

    async switch_course({ course } = {}) {
      if (course !== "en" && course !== "zh") fail("Khóa học phải là en hoặc zh");
      const name = course === "zh" ? "tiếng Trung" : "tiếng Anh";
      if (course === courseOf()) return `Bạn đang học ${name} rồi`;
      await switchCourse(course);
      return `Đã chuyển sang khóa ${name}`;
    },

    async roadmap_create({ purpose, deadline, minutes, level, ai } = {}) {
      const p = text(purpose);
      if (!p) fail("Chưa nói rõ mục tiêu của lộ trình");
      if (!/^\d{4}-\d{2}-\d{2}$/.test(text(deadline)) || text(deadline) <= localDay()) fail("Hạn chót phải là một ngày sau hôm nay, dạng năm-tháng-ngày");
      const m = Math.round(num(minutes));
      if (!(m >= 5 && m <= 600)) fail("Số phút học mỗi ngày phải từ 5 đến 600");
      const goal = { purpose: p, level: text(level), deadline: text(deadline), minutes: m };
      const useAi = ai !== false;
      const r = await app.call("roadmap_plan", { goal, ai: useAi, today: localDay(), day: localDay() });
      const roadmap = { uid: "", ...goal, start: localDay(), steps: r.steps };
      // Saving a roadmap makes it the active one; the activation is asked again so that this holds whatever the backend does.
      const view = await app.call("roadmap_save", { roadmap, today: localDay() });
      if (view.active?.uid) await app.call("roadmap_activate", { uid: view.active.uid, today: localDay() });
      window.chatClose?.();
      go("roadmap");
      return `Đã tạo lộ trình "${p}" gồm ${r.steps.length} chặng`;
    },

    async roadmap_activate({ uid } = {}) {
      if (!text(uid)) fail("Chưa nói rõ lộ trình nào");
      const view = await app.call("roadmap_activate", { uid: text(uid), today: localDay() });
      if (pageNow() === "roadmap") reloadPage("roadmap");
      return `Đã chuyển sang lộ trình "${view.active?.purpose || ""}"`;
    },

    async roadmap_delete({ uid } = {}) {
      if (!text(uid)) fail("Chưa nói rõ lộ trình nào");
      await app.call("roadmap_delete", { uid: text(uid), today: localDay() });
      if (pageNow() === "roadmap") reloadPage("roadmap");
      return "Đã xóa lộ trình";
    },

    async sync_now() {
      const status = await app.call("sync_status").catch(() => null);
      if (!status?.signed_in) fail("Bạn chưa đăng nhập Google Drive trong Cài đặt");
      const r = await app.call("sync_now");
      if (r.changed) {
        state.settings = await app.call("settings");
        applyPalette(state.settings.palette);
        renderSettings();
        reloadPage(pageNow());
      }
      return r.words || r.reviews ? `Đồng bộ xong, có thêm ${r.words || 0} từ và ${r.reviews || 0} lượt ôn từ máy khác` : "Đồng bộ Google Drive xong";
    },
  };

  window.chatTools = tools;
})();
