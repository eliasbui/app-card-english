// No console window behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod chatgpt;
mod cloud;
mod codex;
mod db;
mod models;
mod net;
mod pron;
mod speech;
mod update;

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
    dpi::LogicalSize,
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

enum UserEvent {
    Ipc(String),
    /// A message for the page: a reply to a request or something it did not ask for.
    Post(Value),
    Update(Value),
    Menu(MenuId),
    TrayClick,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
    start_update_checks(proxy.clone());
    start_reminders(dir.clone());
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

    let mut app = App { dir, db, proxy, window: None, tray: None, open_id: None, quit_id: None, update: None };
    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(tao::event::StartCause::Init) => {
                app.make_tray();
                app.show(target);
            }
            #[cfg(target_os = "macos")]
            Event::Reopen { .. } => app.show(target),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // The app stays in the tray so that the reminders keep coming.
                app.window = None;
            }
            Event::UserEvent(UserEvent::Ipc(body)) => app.ipc(&body),
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
                }
            }
            _ => {}
        }
    });
}

struct App {
    dir: PathBuf,
    db: Connection,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<(Window, WebView)>,
    tray: Option<TrayIcon>,
    open_id: Option<MenuId>,
    quit_id: Option<MenuId>,
    /// The last update check that found something, for a window opened later.
    update: Option<Value>,
}

impl App {
    fn make_tray(&mut self) {
        let open = MenuItem::new(format!("Mở {APP_NAME}"), true, None);
        let quit = MenuItem::new("Thoát", true, None);
        let menu = Menu::new();
        if menu.append_items(&[&open, &quit]).is_err() {
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
        self.quit_id = Some(quit.id().clone());
    }

    fn show(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if let Some((window, _)) = &self.window {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
            return;
        }
        match self.open_window(target) {
            Ok(pair) => self.window = Some(pair),
            Err(e) => eprintln!("window: {e}"),
        }
    }

    fn open_window(&self, target: &EventLoopWindowTarget<UserEvent>) -> Result<(Window, WebView), String> {
        let icon = decode_png(WINDOW_ICON).and_then(|(rgba, w, h)| Icon::from_rgba(rgba, w, h).ok());
        let window = WindowBuilder::new()
            .with_title(APP_NAME)
            .with_inner_size(LogicalSize::new(1000.0, 700.0))
            .with_min_inner_size(LogicalSize::new(760.0, 540.0))
            .with_window_icon(icon)
            .with_theme(self.theme())
            .build(target)
            .map_err(|e| e.to_string())?;
        let proxy = self.proxy.clone();
        let builder = WebViewBuilder::new()
            .with_html(INDEX_HTML)
            .with_ipc_handler(move |request: Request<String>| {
                let _ = proxy.send_event(UserEvent::Ipc(request.into_body()));
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

    fn post(&self, message: &Value) {
        if let Some((_, webview)) = &self.window {
            let _ = webview.evaluate_script(&format!("window.app && app.receive({message})"));
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
        match cmd.as_str() {
            "explain" => {
                let word = args["word"].as_str().unwrap_or_default().to_owned();
                let day = args["day"].as_str().unwrap_or_default().to_owned();
                let settings = self.ai_settings();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.explain(&word);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|e| json!(e))
                }));
            }
            "suggest" => {
                let topic = args["topic"].as_str().unwrap_or_default().to_owned();
                let day = args["day"].as_str().unwrap_or_default().to_owned();
                let settings = self.ai_settings();
                let words = db::list(&self.db, "").unwrap_or_default();
                let known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
                // The most recently learned words, which the list already has first.
                let learned: Vec<String> = words.iter().filter(|w| w.learned_at > 0).map(|w| w.word.clone()).collect();
                background(Box::new(move || {
                    let ai = ai::Ai::new(&settings, &dir);
                    let result = ai.suggest(&topic, &learned, &known);
                    record_usage(&dir, &day, &ai.attempts.borrow());
                    result.map(|s| json!(s))
                }));
            }
            "ai_models" => {
                let provider = args["provider"].as_str().unwrap_or_default().to_owned();
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
                let group = args["group"].as_str().unwrap_or_default().to_owned();
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
            "rec_stop" => {
                let target = args["target"].as_str().unwrap_or_default().to_owned();
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
            "check_update" => background(Box::new(|| update::check().map(|c| json!(c)))),
            "speak" => {
                let text = args["text"].as_str().unwrap_or_default().to_owned();
                let slow = args["slow"].as_bool().unwrap_or(false);
                background(Box::new(move || speak(&text, slow).map(|_| Value::Null)));
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
        let day_start = args["day_start"].as_i64().unwrap_or_else(|| now() - 86_400);
        match cmd {
            "init" => Ok(json!({
                "version": update::CURRENT,
                "settings": self.settings(),
                "models": models::status(&self.dir),
                "update": self.update,
                "data_dir": self.dir,
            })),
            "stats" => {
                let stats = db::stats(db, now(), day_start).map_err(e)?;
                let (days, total_xp) = db::activity(db, 60).map_err(e)?;
                Ok(json!({ "words": stats, "days": days, "total_xp": total_xp }))
            }
            "xp" => {
                let day = args["day"].as_str().unwrap_or_default();
                let xp = args["xp"].as_i64().unwrap_or(0);
                if !db::is_day(day) || !(0..=MAX_XP).contains(&xp) {
                    return Err("Điểm kinh nghiệm không hợp lệ".into());
                }
                db::add_xp(db, day, xp, args["correct"].as_bool()).map(|_| Value::Null).map_err(e)
            }
            "list" => db::list(db, args["q"].as_str().unwrap_or_default()).map(|w| json!(w)).map_err(e),
            "session" => db::session(db, now(), day_start).map(|w| json!(w)).map_err(e),
            "ipa" => Ok(json!(pron::ipa(args["word"].as_str().unwrap_or_default()))),
            "save" => {
                let mut word: db::Word = serde_json::from_value(args["word"].clone()).map_err(|e| e.to_string())?;
                word.word = word.word.trim().to_owned();
                if word.word.is_empty() || word.word.chars().count() > 80 {
                    return Err("Từ phải có từ 1 đến 80 ký tự".into());
                }
                if word.ipa.trim().is_empty() {
                    word.ipa = pron::ipa(&word.word);
                }
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
                let quality = args["q"].as_u64().unwrap_or(0).min(5) as u8;
                db::grade(db, args["id"].as_i64().unwrap_or(0), quality, now()).map(|w| json!(w)).map_err(e)
            }
            "set_setting" => {
                let key = args["key"].as_str().unwrap_or_default();
                let value = args["value"].as_str().unwrap_or_default().trim();
                if !valid_setting(key, value) {
                    return Err("Cài đặt không hợp lệ".into());
                }
                db::set_setting(db, key, value).map_err(e)?;
                if key == "theme" {
                    if let Some((window, _)) = &self.window {
                        window.set_theme(self.theme());
                    }
                }
                Ok(self.settings())
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
            chatgpt_model: text("chatgpt_model"),
            codex_model: text("codex_model"),
            gemini_key: text("gemini_key"),
            gemini_model: text("gemini_model"),
            deepseek_key: text("deepseek_key"),
            deepseek_model: text("deepseek_model"),
        }
    }

    fn theme(&self) -> Option<Theme> {
        match self.setting("theme").as_deref() {
            Some("dark") => Some(Theme::Dark),
            Some("light") => Some(Theme::Light),
            _ => None,
        }
    }

    fn setting(&self, key: &str) -> Option<String> {
        db::setting(&self.db, key).ok().flatten().filter(|v| !v.is_empty())
    }

    /// What the page sees of the settings: never a key or token, only whether there is one.
    fn settings(&self) -> Value {
        let text = |key: &str| self.setting(key).unwrap_or_default();
        json!({
            "ai_provider": self.provider(),
            "has_gemini_key": self.setting("gemini_key").is_some(),
            "has_deepseek_key": self.setting("deepseek_key").is_some(),
            "chatgpt_model": text("chatgpt_model"),
            "codex_model": text("codex_model"),
            "gemini_model": text("gemini_model"),
            "deepseek_model": text("deepseek_model"),
            "chatgpt": chatgpt::status(&self.db),
            "codex": codex::status(&self.db),
            "chatgpt_welcomed": self.setting("chatgpt_welcomed").is_some(),
            "theme": self.setting("theme").unwrap_or_else(|| "system".into()),
            "new_per_day": self.setting("new_per_day").and_then(|v| v.parse::<i64>().ok()).unwrap_or(10),
            "remind_hours": self.setting("remind_hours").and_then(|v| v.parse::<i64>().ok()).unwrap_or(4),
            "daily_goal": self.setting("daily_goal").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50),
        })
    }
}

/// The settings the page may change, each with the values it may take; anything else is refused.
fn valid_setting(key: &str, value: &str) -> bool {
    let model = |v: &str| v.len() <= 100 && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'/' | b':'));
    match key {
        "ai_provider" => ai::PROVIDERS.contains(&value),
        "theme" => matches!(value, "system" | "light" | "dark"),
        "chatgpt_model" | "codex_model" | "gemini_model" | "deepseek_model" => model(value),
        "gemini_key" | "deepseek_key" => value.len() <= 500 && !value.chars().any(char::is_control),
        "new_per_day" | "remind_hours" | "daily_goal" => value.parse::<u32>().is_ok_and(|n| n <= 1000),
        "chatgpt_welcomed" => value == "1",
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

/// Every `remind_hours`, a notification when there are words to review or new ones waiting.
fn start_reminders(dir: PathBuf) {
    thread::spawn(move || {
        let Ok(db) = db::open(&dir.join("words.db")) else { return };
        #[cfg(target_os = "macos")]
        let _ = notify_rust::set_application("com.eliasbui.cardenglish");
        loop {
            thread::sleep(REMIND_TICK);
            let hours: i64 = db::setting(&db, "remind_hours").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(4);
            if hours <= 0 {
                continue;
            }
            let last: i64 = db::setting(&db, "last_remind").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
            if last == 0 {
                // No reminder straight after the first launch.
                let _ = db::set_setting(&db, "last_remind", &now().to_string());
                continue;
            }
            if now() - last < hours * 3600 {
                continue;
            }
            let _ = db::set_setting(&db, "last_remind", &now().to_string());
            let Ok(stats) = db::stats(&db, now(), now() - 86_400) else { continue };
            if stats.due == 0 && stats.new_waiting == 0 {
                continue;
            }
            let body = format!("{} từ cần ôn, {} từ mới đang chờ. Học vài phút nhé!", stats.due, stats.new_waiting);
            if let Err(e) = notify_rust::Notification::new().summary("Đến giờ học tiếng Anh").body(&body).show() {
                eprintln!("notification: {e}");
            }
        }
    });
}

/// Reads text aloud with the voice of the system, for a page whose web view has no speech of its own.
fn speak(text: &str, slow: bool) -> Result<(), String> {
    let text: String = text.chars().filter(|c| !c.is_control()).take(300).collect();
    let windows_script = format!(
        "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.Rate = {}; $s.Speak([Console]::In.ReadToEnd())",
        if slow { -5 } else { -1 }
    );
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("/usr/bin/say", vec!["-r", if slow { "90" } else { "160" }])
    } else if cfg!(windows) {
        ("powershell", vec!["-NoProfile", "-Command", &windows_script])
    } else {
        ("espeak", vec!["--stdin", "-s", if slow { "100" } else { "160" }])
    };
    // The text goes in on standard input, never on the command line.
    let mut child = net::command(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Không đọc được: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().map(|_| ()).map_err(|e| e.to_string())
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

/// The RGBA pixels and side of one of the PNG icons in assets.
fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight;
    rgba.then(|| (buf[..info.buffer_size()].to_vec(), info.width, info.height))
}
