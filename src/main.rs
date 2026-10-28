// No console window behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod autostart;
mod backup;
mod browser;
mod chatgpt;
mod clock;
mod cloud;
mod codex;
mod db;
mod fsrs;
mod models;
mod net;
mod packs;
mod pron;
mod shadow;
mod speech;
mod update;
mod voice;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{CMD_OR_CTRL, Code, HotKey, Modifiers},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Icon, Theme, Window, WindowBuilder},
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};
use wry::{WebView, WebViewBuilder, http::Request};

const APP_NAME: &str = "Card English";
const INDEX_HTML: &str = include_str!("../ui/index.html");
/// The scripts and styles of ui/parts, put together by build.rs.
const PARTS: &str = include_str!(concat!(env!("OUT_DIR"), "/parts.html"));
/// The cartoon stickers on one sheet, made by examples/make_stickers.rs; the page's CSS takes it inline.
const STICKERS: &[u8] = include_bytes!("../ui/stickers.webp");
/// Made from the artwork in assets by examples/make_icons.rs.
const WINDOW_ICON: &[u8] = include_bytes!("../assets/icon-256.png");
/// The menu bar of macOS takes a black template it recolours; the Windows taskbar is often dark, so it gets the
/// coloured icon.
const TRAY_ICON: &[u8] = if cfg!(target_os = "macos") {
    include_bytes!("../assets/tray.png")
} else {
    include_bytes!("../assets/icon-64.png")
};
/// Pages the page may open besides the app's own releases.
const OPEN_URLS: [&str; 6] = [
    chatgpt::USAGE_URL,
    codex::USAGE_URL,
    cloud::GEMINI_KEY_URL,
    cloud::GEMINI_USAGE_URL,
    cloud::DEEPSEEK_KEY_URL,
    cloud::DEEPSEEK_USAGE_URL,
];
/// The most experience one answer can bring, so that the page cannot inflate the record.
const MAX_XP: i64 = 50;
const UPDATE_FIRST_CHECK: Duration = Duration::from_secs(20);
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const REMIND_TICK: Duration = Duration::from_secs(60);
/// The shortcuts the settings offer for adding the copied word from any app; `CmdOrCtrl` is Command on a Mac.
const HOTKEYS: [&str; 4] = ["CmdOrCtrl+Shift+E", "CmdOrCtrl+Shift+Y", "Alt+Shift+E", "CmdOrCtrl+Alt+E"];
/// The most of the copied text the page is given, as much as a passage the AI reads for its words.
const MAX_COPIED: usize = 4000;
/// How often the app looks for words the browser extension left in the inbox.
const INBOX_EVERY: Duration = Duration::from_secs(2);
/// The longest text the reader keeps for a course, in bytes, and cuts into words at a time.
const MAX_READER: usize = 256 * 1024;

enum UserEvent {
    Ipc(String),
    /// A request from the page in the window of the quick quiz.
    MiniIpc(String),
    /// A message for the page: a reply to a request or something it did not ask for.
    Post(Value),
    Update(Value),
    Menu(MenuId),
    TrayClick,
    /// Time to count again what is left to study today, for the menu bar.
    Count,
    Hotkey,
    /// For the page to look up: the copied text, read after the shortcut, or what the browser extension sent.
    QuickAdd(Value),
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The learner's midnight from the local clock, or a day ago when the clock cannot be read.
fn day_start() -> i64 {
    let t = now();
    clock::now().map_or(t - 86_400, |l| t - l.seconds())
}

fn data_dir() -> PathBuf {
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let home = env("HOME").or_else(|| env("USERPROFILE")).unwrap_or_else(|| ".".into());
    if cfg!(windows) {
        env("APPDATA").unwrap_or(home).join("CardEnglish")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/CardEnglish")
    } else {
        env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")).join("card-english")
    }
}

fn main() {
    let dir = data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot make {}: {e}", dir.display());
        std::process::exit(1);
    }
    // Started by a browser as the host of the extension, which passes its origin: no window, only the word handed over.
    if std::env::args().skip(1).any(|a| a.starts_with(browser::ORIGIN)) {
        browser::host(&dir);
        return;
    }
    // One copy at a time, or every copy would send the same reminders.
    let lock = File::create(dir.join("app.lock")).and_then(|f| f.try_lock().map(|_| f).map_err(Into::into));
    let Ok(_lock) = lock else {
        eprintln!("{APP_NAME} is already running");
        return;
    };
    let db = match db::open(&dir.join("words.db")) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("cannot open the word list: {e}");
            std::process::exit(1);
        }
    };
    // Started with the computer: the tray only, until the learner opens the window.
    let background = std::env::args().any(|a| a == autostart::BACKGROUND);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    {
        let proxy = std::sync::Mutex::new(proxy.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.lock().map(|p| p.send_event(UserEvent::Menu(e.id)));
        }));
    }
    {
        let proxy = std::sync::Mutex::new(proxy.clone());
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = proxy.lock().map(|p| p.send_event(UserEvent::TrayClick));
            }
        }));
    }
    {
        let proxy = std::sync::Mutex::new(proxy.clone());
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = proxy.lock().map(|p| p.send_event(UserEvent::Hotkey));
            }
        }));
    }
    // On the main thread, where the event loop that delivers its keys runs.
    let hotkeys = GlobalHotKeyManager::new().map_err(|e| eprintln!("hotkeys: {e}")).ok();
    start_update_checks(proxy.clone());
    start_reminders(dir.clone(), proxy.clone());
    {
        let dir = dir.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(60));
            if let Ok(db) = db::open(&dir.join("words.db")) {
                chatgpt::keep_alive(&db);
                codex::keep_alive(&db);
            }
        });
    }
    {
        // The models load while the window opens, so that the first question is not the slow one.
        let dir = dir.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            ai::warm_up(&dir);
            speech::warm_up(&dir);
        });
    }
    {
        let dir = dir.clone();
        thread::spawn(move || browser::register(&dir));
    }
    {
        // Words from the browser extension, sent while the app runs or since it last ran.
        let (dir, proxy) = (dir.clone(), proxy.clone());
        thread::spawn(move || loop {
            if let Some(words) = browser::take(&dir) {
                let _ = proxy.send_event(UserEvent::QuickAdd(words));
            }
            thread::sleep(INBOX_EVERY);
        });
    }

    let mut app = App {
        dir,
        db,
        proxy,
        window: None,
        mini: None,
        tray: None,
        open_id: None,
        quiz_id: None,
        quit_id: None,
        update: None,
        hotkeys,
        hotkey: None,
        hotkey_error: String::new(),
        page_ready: false,
        pending_add: None,
    };
    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(tao::event::StartCause::Init) => {
                app.make_tray();
                app.apply_hotkey();
                app.update_count();
                if !background {
                    app.show(target);
                }
            }
            #[cfg(target_os = "macos")]
            Event::Reopen { .. } => app.show(target),
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => {
                if app.mini.as_ref().is_some_and(|(w, _)| w.id() == window_id) {
                    app.mini = None;
                } else {
                    // The app stays in the tray so that the reminders keep coming.
                    app.window = None;
                    app.page_ready = false;
                }
            }
            Event::UserEvent(UserEvent::Ipc(body)) => app.ipc(&body),
            Event::UserEvent(UserEvent::MiniIpc(body)) => app.mini_ipc(&body),
            Event::UserEvent(UserEvent::Post(message)) => app.post(&message),
            Event::UserEvent(UserEvent::Update(check)) => {
                app.post(&json!({ "event": "update", "data": check }));
                app.update = Some(check);
            }
            Event::UserEvent(UserEvent::TrayClick) => app.show(target),
            Event::UserEvent(UserEvent::Menu(id)) => {
                if app.quit_id.as_ref() == Some(&id) {
                    *flow = ControlFlow::Exit;
                } else if app.open_id.as_ref() == Some(&id) {
                    app.show(target);
                } else if app.quiz_id.as_ref() == Some(&id) {
                    app.quiz(target);
                }
            }
            Event::UserEvent(UserEvent::Count) => app.update_count(),
            Event::UserEvent(UserEvent::Hotkey) => {
                // The clipboard tool can take a moment, so it runs off the event loop.
                let proxy = app.proxy.clone();
                thread::spawn(move || {
                    let text = clipboard().map(|t| copied(&t)).unwrap_or_else(|e| {
                        eprintln!("clipboard: {e}");
                        String::new()
                    });
                    let _ = proxy.send_event(UserEvent::QuickAdd(json!(text)));
                });
            }
            Event::UserEvent(UserEvent::QuickAdd(text)) => app.quick_add(target, text),
            _ => {}
        }
    });
}

struct App {
    dir: PathBuf,
    db: Connection,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<(Window, WebView)>,
    /// The small window of the quick quiz, opened from the tray, above the other apps.
    mini: Option<(Window, WebView)>,
    tray: Option<TrayIcon>,
    open_id: Option<MenuId>,
    quiz_id: Option<MenuId>,
    quit_id: Option<MenuId>,
    /// The last update check that found something, for a window opened later.
    update: Option<Value>,
    hotkeys: Option<GlobalHotKeyManager>,
    /// The shortcut registered now.
    hotkey: Option<HotKey>,
    hotkey_error: String,
    /// Whether the page in the window has started and can take events.
    page_ready: bool,
    /// What came to be looked up before the page was ready, handed over when it starts.
    pending_add: Option<Value>,
}

impl App {
    fn make_tray(&mut self) {
        let open = MenuItem::new(format!("Mở {APP_NAME}"), true, None);
        let quiz = MenuItem::new("Đố nhanh", true, None);
        let quit = MenuItem::new("Thoát", true, None);
        let menu = Menu::new();
        if menu.append_items(&[&open, &quiz, &quit]).is_err() {
            return;
        }
        let icon = decode_png(TRAY_ICON).and_then(|(rgba, w, h)| tray_icon::Icon::from_rgba(rgba, w, h).ok());
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(APP_NAME);
        if let Some(icon) = icon {
            // The template form exists on macOS only.
            #[cfg(target_os = "macos")]
            {
                builder = builder.with_icon_templated(icon);
            }
            #[cfg(not(target_os = "macos"))]
            {
                builder = builder.with_icon(icon);
            }
        }
        match builder.build() {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => eprintln!("tray: {e}"),
        }
        self.open_id = Some(open.id().clone());
        self.quiz_id = Some(quiz.id().clone());
        self.quit_id = Some(quit.id().clone());
    }

    /// What is left to study today in the current course, beside the icon in the menu bar of macOS and in the tooltip.
    fn update_count(&self) {
        let Some(tray) = &self.tray else { return };
        let n = db::stats(&self.db, now(), day_start(), &self.course()).map_or(0, |s| s.due + s.new_today);
        let shown = self.setting("tray_count").as_deref() != Some("0");
        tray.set_title(if shown && n > 0 { Some(n.to_string()) } else { None });
        let tip = if n > 0 { format!("{APP_NAME}: {n} từ cần học hôm nay") } else { APP_NAME.to_owned() };
        let _ = tray.set_tooltip(Some(tip));
    }

    /// Registers the shortcut chosen in the settings in place of the one before.
    fn apply_hotkey(&mut self) {
        self.hotkey_error.clear();
        let wanted = self.setting("hotkey").unwrap_or_default();
        let Some(manager) = &self.hotkeys else {
            if !wanted.is_empty() {
                self.hotkey_error = "Máy này không cho dùng phím tắt toàn cục".into();
            }
            return;
        };
        if let Some(old) = self.hotkey.take() {
            let _ = manager.unregister(old);
        }
        if let Some(hotkey) = hotkey_of(&wanted) {
            match manager.register(hotkey) {
                Ok(()) => self.hotkey = Some(hotkey),
                Err(e) => self.hotkey_error = format!("Không dùng được phím tắt này, có thể app khác đã giữ nó ({e})"),
            }
        }
    }

    /// Opens the window on what is to be looked up; a page still loading gets it when it starts.
    fn quick_add(&mut self, target: &EventLoopWindowTarget<UserEvent>, data: Value) {
        self.show(target);
        if self.page_ready {
            self.post(&json!({ "event": "quick_add", "data": data }));
        } else {
            self.pending_add = Some(data);
        }
    }

    fn show(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if let Some((window, _)) = &self.window {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
            return;
        }
        match self.open_window(target, false) {
            Ok(pair) => self.window = Some(pair),
            Err(e) => eprintln!("window: {e}"),
        }
    }

    /// The quick quiz: one window, brought to the front again if it is already open.
    fn quiz(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if let Some((window, _)) = &self.mini {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
            return;
        }
        match self.open_window(target, true) {
            Ok(pair) => self.mini = Some(pair),
            Err(e) => eprintln!("quiz window: {e}"),
        }
    }

    /// The main window, or with `mini` the small one of the quick quiz near the top right of the screen; both load the
    /// same page, which shows only the quiz when `window.MINI` is set.
    fn open_window(&self, target: &EventLoopWindowTarget<UserEvent>, mini: bool) -> Result<(Window, WebView), String> {
        let icon = decode_png(WINDOW_ICON).and_then(|(rgba, w, h)| Icon::from_rgba(rgba, w, h).ok());
        let builder = if mini {
            let size = LogicalSize::new(360.0, 460.0);
            let mut builder =
                WindowBuilder::new().with_title("Đố nhanh").with_inner_size(size).with_resizable(false).with_always_on_top(true);
            if let Some(screen) = target.primary_monitor() {
                let (origin, area) = (screen.position().to_logical::<f64>(screen.scale_factor()), screen.size().to_logical::<f64>(screen.scale_factor()));
                // Clear of the menu bar of macOS and of the window's own frame.
                builder = builder.with_position(LogicalPosition::new(origin.x + area.width - size.width - 24.0, origin.y + 48.0));
            }
            builder
        } else {
            WindowBuilder::new()
                .with_title(APP_NAME)
                .with_inner_size(LogicalSize::new(1000.0, 700.0))
                .with_min_inner_size(LogicalSize::new(440.0, 560.0))
        };
        let window = builder.with_window_icon(icon).with_theme(self.theme()).build(target).map_err(|e| e.to_string())?;
        let proxy = self.proxy.clone();
        use base64::Engine;
        let sheet = format!("data:image/webp;base64,{}", base64::engine::general_purpose::STANDARD.encode(STICKERS));
        let mut html = INDEX_HTML.replacen("__STICKERS__", &sheet, 1).replacen("<!--PARTS-->", PARTS, 1);
        if mini {
            // Before any of the page's scripts and styles, so that the main layout never shows.
            html = html.replacen("<head>", "<head><script>window.MINI = true; document.documentElement.classList.add(\"mini-window\");</script>", 1);
        }
        let builder = WebViewBuilder::new()
            .with_html(html)
            .with_ipc_handler(move |request: Request<String>| {
                let body = request.into_body();
                let _ = proxy.send_event(if mini { UserEvent::MiniIpc(body) } else { UserEvent::Ipc(body) });
            })
            .with_devtools(cfg!(debug_assertions));
        #[cfg(not(target_os = "linux"))]
        let webview = builder.build(&window);
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(window.default_vbox().ok_or("no GTK box in the window")?)
        };
        Ok((window, webview.map_err(|e| e.to_string())?))
    }

    /// Sends a message to the main page; a reply to the quick quiz, whose request id `mini_ipc` wrapped, goes back to
    /// that page with the id it asked with.
    fn post(&self, message: &Value) {
        let send = |view: &Option<(Window, WebView)>, message: &Value| {
            if let Some((_, webview)) = view {
                let _ = webview.evaluate_script(&format!("window.app && app.receive({message})"));
            }
        };
        match message["id"].get("mini") {
            Some(id) => {
                let mut reply = message.clone();
                reply["id"] = id.clone();
                send(&self.mini, &reply);
            }
            None => send(&self.window, message),
        }
    }

    /// A request of the quick quiz, answered like one of the main page. Its id is wrapped so that the reply, even one
    /// from a background thread, finds its way back here rather than to the main page.
    fn mini_ipc(&mut self, body: &str) {
        let Ok(mut request) = serde_json::from_str::<Value>(body) else { return };
        let cmd = request["cmd"].as_str().unwrap_or_default().to_owned();
        if cmd == "close_mini" {
            self.mini = None;
            return;
        }
        request["id"] = json!({ "mini": request["id"] });
        if !request["args"].is_object() {
            request["args"] = json!({});
        }
        request["args"]["mini"] = json!(true);
        self.ipc(&request.to_string());
        // The main page counts again what is left to study, and its experience, after an answer given here.
        if matches!(cmd.as_str(), "grade" | "xp" | "undo" | "skip") {
            self.post(&json!({ "event": "mini_answered" }));
        }
    }

    fn ipc(&mut self, body: &str) {
        let Ok(request) = serde_json::from_str::<Value>(body) else { return };
        let id = request["id"].clone();
        let cmd = request["cmd"].as_str().unwrap_or_default().to_owned();
        let args = request["args"].clone();
        let reply = |result: Result<Value, String>| match result {
            Ok(data) => json!({ "id": id, "ok": true, "data": data }),
            Err(error) => json!({ "id": id, "ok": false, "error": error }),
        };
        // The slow work runs on a thread of its own and posts its reply when done.
        let background = |work: Box<dyn FnOnce() -> Result<Value, String> + Send>| {
            let proxy = self.proxy.clone();
            let id = id.clone();
            thread::spawn(move || {
                let message = match work() {
                    Ok(data) => json!({ "id": id, "ok": true, "data": data }),
                    Err(error) => json!({ "id": id, "ok": false, "error": error }),
                };
                let _ = proxy.send_event(UserEvent::Post(message));
            });
        };
        let dir = self.dir.clone();
        let text = |key: &str| args[key].as_str().unwrap_or_default().to_owned();
        let day = text("day");
        match cmd.as_str() {
            "explain" => {
                let word = text("word");
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.explain(&word);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|e| json!(e))
                }));
            }
            "suggest" => {
                let topic = text("topic");
                let settings = self.ai_settings();
                let course = self.course();
                let words = db::list(&self.db, "").unwrap_or_default();
                let known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
                // The most recently learned words of the course, which the list already has first.
                let learned: Vec<String> =
                    words.iter().filter(|w| w.learned_at > 0 && w.lang == course).map(|w| w.word.clone()).collect();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.suggest(&topic, &learned, &known, course == "zh");
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|s| json!(s))
                }));
            }
            "extract" => {
                let passage = text("text");
                let settings = self.ai_settings();
                let known: Vec<String> = db::list(&self.db, "").unwrap_or_default().into_iter().map(|w| w.word).collect();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.extract(&passage, &known);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|s| json!(s))
                }));
            }
            "check_sentence" => {
                let (word, sentence) = (text("word"), text("sentence"));
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.check_sentence(&word, &sentence);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|c| json!(c))
                }));
            }
            "explain_mistake" => {
                let [word, meaning, expected, given, context] = ["word", "meaning", "expected", "given", "context"].map(text);
                let zh = self.course() == "zh";
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.explain_mistake(&word, &meaning, &expected, &given, &context, zh);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|m| json!(m))
                }));
            }
            "story" => {
                let asked: Vec<String> = args["words"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_owned)).collect();
                // Without a list, the words answered today, or else the last ones learned.
                let since = args["day_start"].as_i64().unwrap_or_else(day_start);
                let words =
                    if asked.is_empty() { db::recent_words(&self.db, since, 8, &self.course()).unwrap_or_default() } else { asked };
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.story(&words);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|s| json!(s))
                }));
            }
            "pack_add" => {
                let pack = text("pack");
                let n = args["n"].as_u64().unwrap_or(10) as usize;
                let settings = self.ai_settings();
                let proxy = self.proxy.clone();
                background(Box::new(move || {
                    packs::add(&dir, &settings, &pack, n, &day, |data| {
                        let _ = proxy.send_event(UserEvent::Post(json!({ "event": "pack_progress", "data": data })));
                    })
                }));
            }
            "ai_models" => {
                let provider = text("provider");
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let (models, selected) = ai::Ai::new(&settings, &dir).choices(&provider).map_err(|e| e.message)?;
                    Ok(json!({ "models": models, "selected": selected }))
                }));
            }
            "chatgpt_sign_in" => {
                let fresh = args["fresh"].as_bool().unwrap_or(false);
                background(Box::new(move || {
                    let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                    chatgpt::sign_in(&db, fresh)?;
                    Ok(chatgpt::status(&db))
                }));
            }
            "codex_sign_in" => background(Box::new(move || {
                let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                codex::sign_in(&db)?;
                Ok(codex::status(&db))
            })),
            "codex_usage" => background(Box::new(move || {
                let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                let usage = codex::usage(&db).map_err(|e| e.message)?;
                let record = json!({ "at": now(), "server": usage });
                db::set_setting(&db, "quota_codex", &record.to_string()).map_err(|e| e.to_string())?;
                Ok(record)
            })),
            "chatgpt_sign_out" => background(Box::new(move || {
                let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                let revoked = chatgpt::sign_out(&db)?;
                Ok(json!({ "revoked": revoked, "status": chatgpt::status(&db) }))
            })),
            "deepseek_balance" => {
                let key = self.setting("deepseek_key").unwrap_or_default();
                background(Box::new(move || {
                    let balance = cloud::deepseek_balance(&key).map_err(|e| e.message)?;
                    let record = json!({ "at": now(), "balance": balance["balance"], "available": balance["available"] });
                    let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                    db::set_setting(&db, "quota_deepseek", &record.to_string()).map_err(|e| e.to_string())?;
                    Ok(record)
                }));
            }
            "download" => {
                let group = text("group");
                let proxy = self.proxy.clone();
                background(Box::new(move || {
                    let event = group.clone();
                    models::download(&dir, &group, |done, total| {
                        let message = json!({ "event": "progress", "data": { "group": event, "done": done, "total": total } });
                        let _ = proxy.send_event(UserEvent::Post(message));
                    })?;
                    Ok(models::status(&dir))
                }));
            }
            "delete_model" => {
                let group = text("group");
                // Deleting a model changes no setting, so they are read here, on the thread that holds them.
                let settings = self.settings();
                background(Box::new(move || {
                    models::delete(&dir, &group)?;
                    Ok(json!({ "models": models::status(&dir), "settings": settings }))
                }));
            }
            "rec_stop" => {
                let target = text("target");
                let word_id = args["id"].as_i64().unwrap_or(0);
                background(Box::new(move || {
                    let pcm = speech::stop()?;
                    let score = speech::assess(&target, &pcm, &dir)?;
                    if word_id > 0 {
                        let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                        db::set_score(&db, word_id, i64::from(score.score)).map_err(|e| e.to_string())?;
                    }
                    Ok(json!(score))
                }));
            }
            "export_csv" | "backup" => {
                let full = cmd == "backup";
                background(Box::new(move || {
                    let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                    let date = clock::now().map(|l| l.date()).unwrap_or_else(|| now().to_string());
                    let (name, content) = if full {
                        (format!("card-english-{date}.json"), backup::backup(&db, now()).map_err(|e| e.to_string())?)
                    } else {
                        (format!("card-english-{date}.csv"), backup::csv(&db::list(&db, "").map_err(|e| e.to_string())?))
                    };
                    let path = backup::download_path(&name);
                    backup::write(&path, &content)?;
                    reveal(&path);
                    Ok(json!({ "path": path }))
                }));
            }
            "import" => {
                let (name, content) = (text("name"), text("text"));
                background(Box::new(move || {
                    let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
                    backup::import(&db, &name, &content, now()).map(|r| json!(r))
                }));
            }
            "check_update" => background(Box::new(|| update::check().map(|c| json!(c)))),
            "install_update" => {
                let proxy = self.proxy.clone();
                background(Box::new(move || {
                    let app = update::installed().ok_or("Bản này không tự cập nhật được, hãy tải từ trang phát hành")?;
                    let check = update::check()?;
                    if !check.newer {
                        return Err("Bạn đang dùng bản mới nhất".into());
                    }
                    update::install(&check, &app, |percent| {
                        let _ = proxy.send_event(UserEvent::Post(json!({ "event": "update_progress", "data": percent })));
                    })?;
                    Ok(json!(check.latest))
                }));
            }
            "speak" => {
                let words = text("text");
                let slow = args["slow"].as_bool().unwrap_or(false);
                let accent = text("accent");
                let rate = args["rate"].as_f64().unwrap_or(0.95);
                // The Kokoro voice chosen for the language of the text, once its pack is downloaded.
                let lang = if pron::is_chinese(&words) { "zh" } else { "en" };
                let kokoro = db::setting(&self.db, &db::scoped("voice", lang))
                    .ok()
                    .flatten()
                    .and_then(|v| v.strip_prefix("kokoro:").and_then(|sid| sid.parse::<i32>().ok()))
                    .filter(|_| voice::ready(&dir, lang));
                background(Box::new(move || {
                    match kokoro {
                        Some(sid) => voice::speak(&dir, lang, sid, &words, (rate * if slow { 0.7 } else { 1.0 }) as f32),
                        None => speak(&words, slow, &accent, rate),
                    }
                    .map(|_| Value::Null)
                }));
            }
            // Shadowing reads with the course's Kokoro voice whenever its pack is there, even when the system's is
            // chosen, since the tune the learner copies is the one their own is held against.
            "shadow_say" => {
                let words = text("text");
                let rate = args["rate"].as_f64().unwrap_or(0.95) as f32;
                let lang = if pron::is_chinese(&words) { "zh" } else { "en" };
                let sid = db::setting(&self.db, &db::scoped("voice", lang))
                    .ok()
                    .flatten()
                    .and_then(|v| v.strip_prefix("kokoro:").and_then(|sid| sid.parse::<i32>().ok()))
                    .or_else(|| voice::VOICES.iter().find(|v| v.0 == lang).map(|v| v.1))
                    .unwrap_or_default();
                background(Box::new(move || {
                    let (samples, rate) = voice::speak_and_keep(&dir, lang, sid, &words, rate)?;
                    Ok(json!({ "reference": shadow::contour(&samples, rate) }))
                }));
            }
            "shadow_stop" => {
                let target = text("target");
                let reference = args["reference"].as_array().map(|a| a.iter().map(|v| v.as_f64().map(|x| x as f32)).collect());
                background(Box::new(move || {
                    let pcm = speech::stop()?;
                    let score = speech::assess(&target, &pcm, &dir)?;
                    Ok(json!(shadow::compare(&target, score, &pcm, reference)))
                }));
            }
            "download_voice" => {
                let course = if text("course") == "zh" { "zh" } else { "en" };
                let proxy = self.proxy.clone();
                background(Box::new(move || {
                    voice::download(&dir, course, |done, total| {
                        let message = json!({ "event": "progress", "data": { "group": format!("voice-{course}"), "done": done, "total": total } });
                        let _ = proxy.send_event(UserEvent::Post(message));
                    })?;
                    Ok(voice::status(&dir))
                }));
            }
            "delete_voice" => {
                let course = if text("course") == "zh" { "zh" } else { "en" };
                let result = voice::delete(&dir, course).and_then(|()| {
                    // The voice chosen in its place is gone with it: the course reads with the system's voice again.
                    let key = db::scoped("voice", course);
                    if db::setting(&self.db, &key).ok().flatten().is_some_and(|v| v.starts_with("kokoro:")) {
                        db::set_setting(&self.db, &key, "").map_err(|e| e.to_string())?;
                    }
                    Ok(json!({ "voices": voice::status(&dir), "settings": self.settings() }))
                });
                self.post(&reply(result));
            }
            _ => {
                let result = self.handle(&cmd, &args);
                self.post(&reply(result));
            }
        }
    }

    /// The quick requests, answered on the spot.
    fn handle(&mut self, cmd: &str, args: &Value) -> Result<Value, String> {
        let db = &self.db;
        let e = |e: rusqlite::Error| e.to_string();
        let day_start = args["day_start"].as_i64().unwrap_or_else(day_start);
        match cmd {
            "restart" => update::relaunch(),
            "init" => {
                // The quick quiz loads the same page, but neither copied text nor the update dialog is for it.
                let main = !args["mini"].as_bool().unwrap_or(false);
                self.page_ready |= main;
                Ok(json!({
                    "version": update::CURRENT,
                    "settings": self.settings(),
                    "models": models::status(&self.dir),
                    "voices": voice::status(&self.dir),
                    "kokoro_voices": voice::VOICES.iter().map(|(lang, sid, name)| json!({ "lang": lang, "sid": sid, "name": name })).collect::<Vec<_>>(),
                    "update": self.update.as_ref().filter(|_| main),
                    "data_dir": self.dir,
                    "platform": std::env::consts::OS,
                    "hotkeys": HOTKEYS,
                    "brew": installed_by_brew(),
                    "script": installed_by_script(),
                    "quick_add": if main { self.pending_add.take() } else { None },
                }))
            }
            "stats" => {
                let course = self.course();
                let stats = db::stats(db, now(), day_start, &course).map_err(e)?;
                let (days, total_xp) = db::activity(db, 400, &course).map_err(e)?;
                let forecast = db::forecast(db, day_start, 7, &course).map_err(e)?;
                // The page asks after every change, which keeps the count in the menu bar right too.
                self.update_count();
                Ok(json!({ "words": stats, "all": stats, "days": days, "total_xp": total_xp, "forecast": forecast }))
            }
            "xp" => {
                let day = args["day"].as_str().unwrap_or_default();
                let xp = args["xp"].as_i64().unwrap_or(0);
                if !db::is_day(day) || !(0..=MAX_XP).contains(&xp) {
                    return Err("Điểm kinh nghiệm không hợp lệ".into());
                }
                db::add_xp(db, day, &self.course(), xp, args["correct"].as_bool()).map(|_| Value::Null).map_err(e)
            }
            "list" => db::list(db, args["q"].as_str().unwrap_or_default()).map(|w| json!(w)).map_err(e),
            "session" => {
                // Reviewing ahead, for today only: the words due in the next 1 to 3 days come too, the soonest first,
                // and their schedule changes only when they are answered.
                let ahead = args["ahead"].as_i64().unwrap_or(0).clamp(0, 3);
                let until = if ahead > 0 { day_start + (ahead + 1) * db::DAY - 1 } else { now() };
                db::session(db, until, day_start, &self.course()).map(|w| json!(w)).map_err(e)
            }
            "postpone" => {
                let days = args["days"].as_i64().unwrap_or(0);
                if !(1..=7).contains(&days) {
                    return Err("Hãy chọn hoãn từ 1 đến 7 ngày".into());
                }
                let moved = db::postpone(db, now(), day_start, &self.course(), days).map_err(e)?;
                self.update_count();
                Ok(json!(moved))
            }
            "ipa" => Ok(json!(pron::ipa(args["word"].as_str().unwrap_or_default()))),
            "hanviet" => Ok(json!(pron::hanviet(args["word"].as_str().unwrap_or_default()))),
            "segment" => {
                let text = args["text"].as_str().unwrap_or_default();
                if text.len() > MAX_READER {
                    return Err("Văn bản quá dài, hãy chia thành nhiều phần".into());
                }
                Ok(json!(pron::segment(text)))
            }
            "reader_text" => {
                // The last text of the reader, a file for each course, kept so that it is there after a restart.
                let path = self.dir.join(format!("reader-{}.txt", self.course()));
                match args["text"].as_str() {
                    Some(text) if text.len() > MAX_READER => Err("Văn bản quá dài, hãy chia thành nhiều phần".into()),
                    Some(text) => std::fs::write(&path, text).map(|()| Value::Null).map_err(|e| format!("Không lưu được văn bản: {e}")),
                    None => Ok(json!(std::fs::read_to_string(&path).unwrap_or_default())),
                }
            }
            "save" => {
                let mut word: db::Word = serde_json::from_value(args["word"].clone()).map_err(|e| e.to_string())?;
                word.word = word.word.trim().to_owned();
                if word.word.is_empty() || word.word.chars().count() > 80 {
                    return Err("Từ phải có từ 1 đến 80 ký tự".into());
                }
                // Only a CEFR or HSK level, so that the list can be sorted by it.
                word.level = valid_level(&word.level);
                match db::save(db, &word, now()) {
                    Ok(id) => db::get(db, id).map(|w| json!(w)).map_err(e),
                    Err(rusqlite::Error::SqliteFailure(f, _)) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                        Err(format!("Từ \"{}\" đã có trong danh sách", word.word))
                    }
                    Err(err) => Err(e(err)),
                }
            }
            "delete" => db::delete(db, args["id"].as_i64().unwrap_or(0)).map(|_| Value::Null).map_err(e),
            "grade" => {
                let rating = args["q"].as_u64().unwrap_or(0).clamp(1, 4) as u8;
                // The next review goes to the lightest day near the one FSRS gives, on the learner's own calendar.
                let t = now();
                let today = clock::now().map(|l| db::Today { start: t - l.seconds(), weekday: l.weekday() });
                db::grade_on(db, args["id"].as_i64().unwrap_or(0), rating, t, self.retention(), today).map(|w| json!(w)).map_err(e)
            }
            "undo" => {
                let day = args["day"].as_str().unwrap_or_default();
                let xp = args["xp"].as_i64().unwrap_or(0);
                if !db::is_day(day) || !(0..=MAX_XP).contains(&xp) {
                    return Err("Không hoàn tác được".into());
                }
                let word = db::undo(db, args["id"].as_i64().unwrap_or(0)).map_err(e)?.ok_or("Không còn câu trả lời nào để hoàn tác")?;
                db::take_back_xp(db, day, &self.course(), xp, args["correct"].as_bool().unwrap_or(false)).map_err(e)?;
                Ok(json!(word))
            }
            "set_setting" => {
                let key = args["key"].as_str().unwrap_or_default();
                let value = args["value"].as_str().unwrap_or_default().trim();
                if !valid_setting(key, value) {
                    return Err("Cài đặt không hợp lệ".into());
                }
                db::set_setting(db, &db::scoped(key, &self.course()), value).map_err(e)?;
                match key {
                    "theme" => {
                        for (window, _) in self.window.iter().chain(&self.mini) {
                            window.set_theme(self.theme());
                        }
                    }
                    "hotkey" => self.apply_hotkey(),
                    "tray_count" | "new_per_day" | "course" => self.update_count(),
                    // The model in memory waits for a question in progress, so it is let go off the event loop.
                    "local_model" => drop(thread::spawn(ai::unload)),
                    _ => {}
                }
                Ok(self.settings())
            }
            "autostart" => {
                autostart::set(args["on"].as_bool().unwrap_or(false))?;
                Ok(self.settings())
            }
            "open_backups" => {
                let folder = self.dir.join("backups");
                std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                open_folder(&folder).map(|_| Value::Null)
            }
            "usage" => {
                let days = [&args["today"], &args["since7"], &args["since30"]].map(|d| d.as_str().unwrap_or_default());
                if !days.iter().all(|d| db::is_day(d)) {
                    return Err("Ngày không hợp lệ".into());
                }
                let mut summary = db::usage_summary(db, days[0], days[1], days[2]).map_err(e)?;
                for provider in ai::PROVIDERS {
                    let mut entry = summary.get(provider).cloned().unwrap_or_else(|| json!({}));
                    for kind in ["limit", "quota"] {
                        let saved = self.setting(&format!("{kind}_{provider}")).and_then(|s| serde_json::from_str::<Value>(&s).ok());
                        if let Some(saved) = saved {
                            entry[kind] = saved;
                        }
                    }
                    summary[provider] = entry;
                }
                Ok(summary)
            }
            "cancel_sign_in" => {
                chatgpt::cancel();
                Ok(Value::Null)
            }
            "codex_sign_out" => codex::sign_out(db).map(|_| codex::status(db)),
            "models" => Ok(models::status(&self.dir)),
            "rec_start" => speech::start().map(|_| Value::Null),
            "spell" => {
                let word: String = args["word"].as_str().unwrap_or_default().chars().take(80).collect();
                let (ok, suggestions) = pron::spell(&word);
                Ok(json!({ "ok": ok, "suggestions": suggestions }))
            }
            "skip" => db::set_skipped(db, args["id"].as_i64().unwrap_or(0), args["on"].as_bool().unwrap_or(true))
                .map_err(e)?
                .map(|w| json!(w))
                .ok_or_else(|| "Không tìm thấy từ này".into()),
            "rec_level" => Ok(json!(speech::level())),
            "packs" => packs::status(db, &self.course()).map_err(e),
            "pack_cancel" => {
                packs::cancel();
                Ok(Value::Null)
            }
            // Where packs start: after the bands a test found known, or back at the start.
            "pack_move" => {
                for (id, at) in args["moves"].as_object().into_iter().flatten() {
                    packs::set_position(db, id, at.as_u64().unwrap_or(0) as usize)?;
                }
                packs::status(db, &self.course()).map_err(e)
            }
            "vocab_test" => {
                let course = self.course();
                Ok(json!({ "lang": course, "items": packs::test(&course) }))
            }
            "vocab_result" => {
                let lang = if args["lang"] == "zh" { "zh" } else { "en" };
                let answers: Vec<(Option<usize>, bool)> = args["answers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| (a["band"].as_u64().map(|b| b as usize), a["yes"].as_bool().unwrap_or(false)))
                    .collect();
                let mut estimate = packs::estimate(lang, &answers);
                // Only the packs that would move forward.
                estimate.moves.retain(|m| packs::find(m.pack).is_some_and(|p| m.to > packs::position(db, p)));
                Ok(json!(estimate))
            }
            "open_url" => {
                let url = args["url"].as_str().unwrap_or_default();
                if !(update::is_release_url(url) || OPEN_URLS.contains(&url)) {
                    return Err("Không mở được địa chỉ này".into());
                }
                open_url(url)
            }
            _ => Err(format!("Không có lệnh {cmd}")),
        }
    }

    /// The chosen AI; before providers existed the choice was `ai_mode`, whose DeepSeek still counts.
    fn provider(&self) -> String {
        self.setting("ai_provider").filter(|p| ai::PROVIDERS.contains(&p.as_str())).unwrap_or_else(|| {
            if self.setting("ai_mode").as_deref() == Some("deepseek") { "deepseek".into() } else { "local".into() }
        })
    }

    fn ai_settings(&self) -> ai::Settings {
        let text = |key: &str| self.setting(key).unwrap_or_default();
        ai::Settings {
            provider: self.provider(),
            local_model: text("local_model"),
            chatgpt_model: text("chatgpt_model"),
            codex_model: text("codex_model"),
            gemini_key: text("gemini_key"),
            gemini_model: text("gemini_model"),
            deepseek_key: text("deepseek_key"),
            deepseek_model: text("deepseek_model"),
        }
    }

    /// The course being studied: "en" or "zh".
    fn course(&self) -> String {
        db::setting(&self.db, "course").ok().flatten().filter(|c| c == "zh").unwrap_or_else(|| "en".into())
    }

    /// The chance of recall the learner wants at each review, 90% unless the settings say otherwise.
    fn retention(&self) -> f64 {
        self.setting("desired_retention")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|r| (0.7..=0.97).contains(r))
            .unwrap_or(fsrs::DEFAULT_RETENTION)
    }

    fn theme(&self) -> Option<Theme> {
        match self.setting("theme").as_deref() {
            Some("dark") => Some(Theme::Dark),
            Some("light") => Some(Theme::Light),
            _ => None,
        }
    }

    /// A setting of the current course, or a shared one.
    fn setting(&self, key: &str) -> Option<String> {
        db::setting(&self.db, &db::scoped(key, &self.course())).ok().flatten().filter(|v| !v.is_empty())
    }

    /// What the page sees of the settings: never a key or token, only whether there is one.
    fn settings(&self) -> Value {
        let text = |key: &str| self.setting(key).unwrap_or_default();
        let number = |key: &str, default: i64| self.setting(key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(default);
        let on = |key: &str| self.setting(key).as_deref() != Some("0");
        json!({
            "ai_provider": self.provider(),
            "local_model": if text("local_model") == "llm15" { "llm15" } else { "llm" },
            "has_gemini_key": self.setting("gemini_key").is_some(),
            "has_deepseek_key": self.setting("deepseek_key").is_some(),
            "chatgpt_model": text("chatgpt_model"),
            "codex_model": text("codex_model"),
            "gemini_model": text("gemini_model"),
            "deepseek_model": text("deepseek_model"),
            "chatgpt": chatgpt::status(&self.db),
            "codex": codex::status(&self.db),
            "chatgpt_welcomed": self.setting("chatgpt_welcomed").is_some(),
            "course": self.course(),
            "theme": self.setting("theme").unwrap_or_else(|| "system".into()),
            // Each course has a colour of its own until the learner picks one.
            "palette": self.setting("palette").unwrap_or_else(|| if self.course() == "zh" { "red" } else { "blue" }.into()),
            "new_per_day": number("new_per_day", 10),
            "remind_hours": number("remind_hours", 4),
            "daily_goal": number("daily_goal", 50),
            "desired_retention": self.retention(),
            "streak_freeze": on("streak_freeze"),
            "easy_days": text("easy_days"),
            "remind_at": text("remind_at"),
            "quiet_from": text("quiet_from"),
            "quiet_to": text("quiet_to"),
            "tray_count": on("tray_count"),
            "autostart": autostart::enabled(),
            "accent": if text("accent") == "en-GB" { "en-GB" } else { "en-US" },
            "voice": text("voice"),
            "speech_rate": self.setting("speech_rate").and_then(|v| v.parse::<f64>().ok()).filter(|r| (0.5..=1.5).contains(r)).unwrap_or(0.95),
            "hotkey": text("hotkey"),
            "hotkey_error": self.hotkey_error,
            // Kept apart from "none yet", so that the first ones reached are celebrated.
            "achievements": db::setting(&self.db, &db::scoped("achievements", &self.course())).ok().flatten(),
            "match_best": number("match_best", 0),
        })
    }
}

/// A level such as `B2` or `HSK3` from what the page or a model wrote, or nothing.
fn valid_level(text: &str) -> String {
    let up = text.trim().to_ascii_uppercase().replace(' ', "");
    let b = up.as_bytes();
    let cefr = b.len() == 2 && matches!(b[0], b'A' | b'B' | b'C') && matches!(b[1], b'1' | b'2');
    let hsk = b.len() == 4 && up.starts_with("HSK") && (b'1'..=b'9').contains(&b[3]);
    if cefr || hsk { up } else { String::new() }
}

fn hotkey_of(name: &str) -> Option<HotKey> {
    let (mods, key) = match name {
        "CmdOrCtrl+Shift+E" => (CMD_OR_CTRL | Modifiers::SHIFT, Code::KeyE),
        "CmdOrCtrl+Shift+Y" => (CMD_OR_CTRL | Modifiers::SHIFT, Code::KeyY),
        "Alt+Shift+E" => (Modifiers::ALT | Modifiers::SHIFT, Code::KeyE),
        "CmdOrCtrl+Alt+E" => (CMD_OR_CTRL | Modifiers::ALT, Code::KeyE),
        _ => return None,
    };
    Some(HotKey::new(Some(mods), key))
}

/// The settings the page may change, each with the values it may take; anything else is refused.
fn valid_setting(key: &str, value: &str) -> bool {
    let model = |v: &str| v.len() <= 100 && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'/' | b':'));
    let between = |v: &str, low: f64, high: f64| v.len() <= 5 && v.parse::<f64>().is_ok_and(|n| (low..=high).contains(&n));
    match key {
        "ai_provider" => ai::PROVIDERS.contains(&value),
        "theme" => matches!(value, "system" | "light" | "dark"),
        "course" => matches!(value, "en" | "zh"),
        "palette" => matches!(value, "blue" | "pink" | "purple" | "red"),
        "chatgpt_model" | "codex_model" | "gemini_model" | "deepseek_model" => model(value),
        "gemini_key" | "deepseek_key" => value.len() <= 500 && !value.chars().any(char::is_control),
        "new_per_day" | "remind_hours" | "daily_goal" => value.parse::<u32>().is_ok_and(|n| n <= 1000),
        "chatgpt_welcomed" => value == "1",
        "local_model" => matches!(value, "llm" | "llm15"),
        "desired_retention" => between(value, 0.7, 0.97),
        "speech_rate" => between(value, 0.5, 1.5),
        "streak_freeze" | "tray_count" => matches!(value, "0" | "1"),
        "remind_at" | "quiet_from" | "quiet_to" => value.is_empty() || clock::parse_hm(value).is_some(),
        "accent" => matches!(value, "en-US" | "en-GB"),
        "voice" => value.len() <= 100 && !value.chars().any(char::is_control),
        "hotkey" => value.is_empty() || HOTKEYS.contains(&value),
        // Weekdays, 0 for Sunday, such as `0,6`.
        "easy_days" => value.len() <= 13 && (value.is_empty() || value.split(',').all(|d| matches!(d.as_bytes(), [b'0'..=b'6']))),
        "achievements" => value.len() <= 2000 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b','),
        // The best time of the matching game, in milliseconds.
        "match_best" => value.parse::<u32>().is_ok_and(|ms| (1..=3_600_000).contains(&ms)),
        _ => false,
    }
}

/// Keeps what the AI calls of one request took, and the limits they ran into, for measuring usage.
fn record_usage(dir: &Path, day: &str, attempts: &[ai::Attempt]) {
    let Ok(db) = db::open(&dir.join("words.db")) else { return };
    for a in attempts {
        if db::is_day(day) {
            let _ = db::add_usage(&db, day, a.provider, &a.model, a.ok, a.input_tokens, a.output_tokens);
        }
        let limit = format!("limit_{}", a.provider);
        if a.limit {
            let _ = db::set_setting(&db, &limit, &json!({ "at": now(), "message": a.error, "details": a.details }).to_string());
        } else if a.ok {
            // An answer means the limit, if one was reached, has passed.
            let _ = db::set_setting(&db, &limit, "");
        }
        if !a.limits.is_null() {
            let _ = db::set_setting(&db, &format!("quota_{}", a.provider), &json!({ "at": now(), "server": a.limits }).to_string());
        }
    }
}

fn start_update_checks(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        if let Some(app) = update::app_path() {
            update::tidy(&app);
        }
        thread::sleep(UPDATE_FIRST_CHECK);
        loop {
            match update::check() {
                Ok(check) if check.newer => {
                    let _ = proxy.send_event(UserEvent::Update(json!(check)));
                }
                Ok(_) => {}
                Err(e) => eprintln!("update check: {e}"),
            }
            thread::sleep(UPDATE_EVERY);
        }
    });
}

/// Every minute: the count in the menu bar, the daily copy of the list, and the reminders.
fn start_reminders(dir: PathBuf, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let Ok(db) = db::open(&dir.join("words.db")) else { return };
        #[cfg(target_os = "macos")]
        let _ = notify_rust::set_application("com.eliasbui.cardenglish");
        loop {
            thread::sleep(REMIND_TICK);
            let _ = proxy.send_event(UserEvent::Count);
            let local = clock::now();
            if let Some(l) = &local
                && let Err(e) = backup::daily(&dir, &db, &l.date(), now())
            {
                eprintln!("daily copy: {e}");
            }
            let get = |key: &str| db::setting(&db, key).ok().flatten().unwrap_or_default();
            let due = reminders_due(now(), local.as_ref().map(|l| (l.minutes(), l.date())), &get);
            if due.first_launch {
                // No reminder straight after the first launch.
                let _ = db::set_setting(&db, "last_remind", &now().to_string());
            }
            if due.every {
                let _ = db::set_setting(&db, "last_remind", &now().to_string());
            }
            if let (true, Some(l)) = (due.daily, &local) {
                let _ = db::set_setting(&db, "last_daily_remind", &l.date());
            }
            if !(due.every || due.daily) {
                continue;
            }
            // The course the learner is on now, as the window and the menu bar show it.
            let zh = get("course") == "zh";
            let Ok(stats) = db::stats(&db, now(), day_start(), if zh { "zh" } else { "en" }) else { continue };
            if stats.due == 0 && stats.new_waiting == 0 {
                continue;
            }
            let body = format!("{} từ cần ôn, {} từ mới đang chờ. Học vài phút nhé!", stats.due, stats.new_waiting);
            let title = if zh { "Đến giờ học tiếng Trung" } else { "Đến giờ học tiếng Anh" };
            if let Err(e) = notify_rust::Notification::new().summary(title).body(&body).show() {
                eprintln!("notification: {e}");
            }
        }
    });
}

#[derive(Debug, Default, PartialEq)]
struct Due {
    /// Every `remind_hours`, outside the quiet hours.
    every: bool,
    /// At `remind_at`, once a day, within three hours of it.
    daily: bool,
    /// No reminder was ever sent: the interval starts counting now.
    first_launch: bool,
}

/// Which reminders are due at `now`, on the local clock `local` (minutes after midnight and the date).
fn reminders_due(now: i64, local: Option<(u32, String)>, get: &dyn Fn(&str) -> String) -> Due {
    let hours: i64 = get("remind_hours").parse().unwrap_or(4);
    let last: i64 = get("last_remind").parse().unwrap_or(0);
    let minute = local.as_ref().map(|l| l.0);
    let quiet = minute.is_some_and(|m| clock::quiet(m, clock::parse_hm(&get("quiet_from")), clock::parse_hm(&get("quiet_to"))));
    let daily = match (clock::parse_hm(&get("remind_at")), &local) {
        (Some(at), Some((m, date))) => *m >= at && *m < at + 180 && get("last_daily_remind") != *date,
        _ => false,
    };
    Due { every: hours > 0 && last > 0 && now - last >= hours * 3600 && !quiet, daily, first_launch: hours > 0 && last == 0 }
}

/// Reads text aloud with the voice of the system, for a page whose web view has no speech of its own; `accent` is
/// `en-GB` for a British voice, `rate` 1 for the normal speed. Chinese text always gets a Mandarin voice.
fn speak(text: &str, slow: bool, accent: &str, rate: f64) -> Result<(), String> {
    let text: String = text.chars().filter(|c| !c.is_control()).take(2000).collect();
    let (culture, voice, espeak) = if pron::is_chinese(&text) {
        ("zh-CN", "Tingting", "cmn")
    } else if accent == "en-GB" {
        ("en-GB", "Daniel", "en-gb")
    } else {
        ("en-US", "Samantha", "en-us")
    };
    let rate = rate.clamp(0.5, 1.5) * if slow { 0.6 } else { 1.0 };
    let per_minute = ((175.0 * rate).round() as u32).to_string();
    // Standard input is read as UTF-8, not in the console's code page, which has no Chinese or Vietnamese letters.
    let windows_script = format!(
        "[Console]::InputEncoding = [Text.Encoding]::UTF8; Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         try {{ $s.SelectVoiceByHints([System.Speech.Synthesis.VoiceGender]::NotSet, [System.Speech.Synthesis.VoiceAge]::NotSet, 0, \
         [System.Globalization.CultureInfo]::new('{culture}')) }} catch {{}}; $s.Rate = {}; $s.Speak([Console]::In.ReadToEnd())",
        ((rate - 1.0) * 10.0).round().clamp(-10.0, 10.0) as i32,
    );
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("/usr/bin/say", vec!["-v", voice, "-r", &per_minute])
    } else if cfg!(windows) {
        ("powershell", vec!["-NoProfile", "-Command", &windows_script])
    } else {
        ("espeak", vec!["--stdin", "-v", espeak, "-s", &per_minute])
    };
    // The text goes in on standard input, never on the command line.
    let run = |args: &[&str]| -> Result<bool, String> {
        let mut child = net::command(program)
            .args(args)
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Không đọc được: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        child.wait().map(|s| s.success()).map_err(|e| e.to_string())
    };
    // A voice that is not installed fails, and macOS reads with its own default one instead.
    if run(&args)? || (cfg!(target_os = "macos") && run(&["-r", &per_minute])?) { Ok(()) } else { Err("Máy không đọc được câu này".into()) }
}

/// The text on the clipboard, through the system's own tool.
fn clipboard() -> Result<String, String> {
    let output = if cfg!(target_os = "macos") {
        // Its default locale would turn every accented letter into a question mark.
        net::command("/usr/bin/pbpaste").env("LANG", "en_US.UTF-8").output()
    } else if cfg!(windows) {
        net::command("powershell")
            .args(["-NoProfile", "-Command", "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-Clipboard -Raw"])
            .output()
    } else {
        net::command("wl-paste").arg("--no-newline").output().or_else(|_| net::command("xclip").args(["-selection", "clipboard", "-o"]).output())
    }
    .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Copied text as the page takes it: trimmed, without control characters other than line breaks, and short.
fn copied(text: &str) -> String {
    let text: String = text.chars().filter(|&c| c == '\n' || !c.is_control()).take(MAX_COPIED).collect();
    text.trim().to_owned()
}

/// Whether Homebrew put the app here, so that an update is `brew upgrade` instead of a download.
fn installed_by_brew() -> bool {
    cfg!(target_os = "macos") && ["/opt/homebrew/Caskroom/card-english", "/usr/local/Caskroom/card-english"].iter().any(|p| Path::new(p).is_dir())
}

/// Whether scripts/install.ps1 put the app here, so that an update is running the script again.
fn installed_by_script() -> bool {
    cfg!(windows)
        && std::env::current_exe()
            .is_ok_and(|p| p.to_string_lossy().to_lowercase().ends_with(r"\programs\cardenglish\cardenglish.exe"))
}

fn open_url(url: &str) -> Result<Value, String> {
    let mut cmd = if cfg!(target_os = "macos") {
        net::command("/usr/bin/open")
    } else if cfg!(windows) {
        let mut c = net::command("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        net::command("xdg-open")
    };
    cmd.arg(url).spawn().map(|_| Value::Null).map_err(|e| e.to_string())
}

fn open_folder(folder: &Path) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    net::command(program).arg(folder).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Shows a file just written in Finder or Explorer, or opens its folder elsewhere.
fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let shown = net::command("/usr/bin/open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    let shown = {
        use std::os::windows::process::CommandExt;
        // Explorer reads its own command line: the path is quoted after the comma, not the whole argument.
        net::command("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let shown = net::command("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(e) = shown {
        eprintln!("reveal {}: {e}", path.display());
    }
}

/// The RGBA pixels and side of one of the PNG icons in assets.
fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight;
    rgba.then(|| (buf[..info.buffer_size()].to_vec(), info.width, info.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn settings_take_only_what_they_allow() {
        assert!(valid_setting("desired_retention", "0.85") && !valid_setting("desired_retention", "0.99") && !valid_setting("desired_retention", "x"));
        assert!(valid_setting("remind_at", "20:00") && valid_setting("remind_at", "") && !valid_setting("quiet_from", "25:00"));
        assert!(valid_setting("hotkey", "CmdOrCtrl+Shift+E") && valid_setting("hotkey", "") && !valid_setting("hotkey", "Ctrl+Q"));
        assert!(valid_setting("accent", "en-GB") && !valid_setting("accent", "fr-FR"));
        assert!(valid_setting("achievements", "streak7,words50") && !valid_setting("achievements", "<b>"));
        assert!(valid_setting("speech_rate", "1.2") && !valid_setting("speech_rate", "3"));
        assert!(valid_setting("match_best", "15400") && !valid_setting("match_best", "0") && !valid_setting("match_best", "1.5"));
        assert!(HOTKEYS.iter().all(|h| hotkey_of(h).is_some()));
        assert_eq!([" b2 ", "B3", "C1+", "hsk 3", "HSK0", "HSK10"].map(valid_level), ["B2", "", "", "HSK3", "", ""]);
        assert!(valid_setting("course", "zh") && !valid_setting("course", "fr"));
        assert!(valid_setting("easy_days", "0,6") && valid_setting("easy_days", "") && !valid_setting("easy_days", "7") && !valid_setting("easy_days", "1,,2"));
        assert_eq!(copied("  take off\u{7}\r\n"), "take off");
    }

    #[test]
    fn reminders_follow_the_interval_the_quiet_hours_and_the_set_time() {
        let settings = |pairs: &[(&str, &str)]| -> HashMap<String, String> { pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
        let ask = |s: &HashMap<String, String>, now: i64, minute: u32| {
            reminders_due(now, Some((minute, "2026-10-27".into())), &|k: &str| s.get(k).cloned().unwrap_or_default())
        };
        let fresh = settings(&[]);
        assert_eq!(ask(&fresh, 1_000, 600), Due { first_launch: true, ..Due::default() });
        let waited = settings(&[("remind_hours", "4"), ("last_remind", "1000")]);
        assert!(!ask(&waited, 1_000 + 3 * 3600, 600).every);
        assert!(ask(&waited, 1_000 + 4 * 3600, 600).every);
        let quiet = settings(&[("remind_hours", "4"), ("last_remind", "1000"), ("quiet_from", "22:00"), ("quiet_to", "07:00")]);
        assert!(!ask(&quiet, 1_000 + 5 * 3600, 23 * 60).every);
        assert!(ask(&quiet, 1_000 + 5 * 3600, 8 * 60).every);
        let daily = settings(&[("remind_hours", "0"), ("remind_at", "20:00")]);
        assert_eq!(ask(&daily, 0, 19 * 60 + 59), Due::default());
        assert_eq!(ask(&daily, 0, 20 * 60 + 5), Due { daily: true, ..Due::default() });
        assert!(!ask(&daily, 0, 23 * 60 + 30).daily, "more than three hours late");
        let sent = settings(&[("remind_at", "20:00"), ("last_daily_remind", "2026-10-27")]);
        assert!(!ask(&sent, 0, 20 * 60 + 5).daily);
    }
}
