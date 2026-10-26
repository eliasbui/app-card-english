//! Sign in with ChatGPT for open-source, locally run apps: the user's ChatGPT plan pays for the app's AI
//! requests. Follows https://developers.openai.com/siwc/token-sharing-open-source (preview, October 2026).

use crate::{
    ai::{Answer, CallError, ModelInfo, Prompt},
    db, net,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const MODELS: &str = "https://api.openai.com/v1/models";
const RESPONSES: &str = "https://api.openai.com/v1/responses";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The permission to use the plan; a valid sign-in without it may not run inference.
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// The entrypoint of the first registration; OpenAI answers with the client ID to keep, never this one.
const NEW_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME: &str = "Card English";
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
/// The port of the documented example; another free one is fine, only the port may vary.
const PORT: u16 = 1455;
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// An access token (one hour) is renewed once it has less than this left, as OpenAI's devkit does.
const REFRESH_MARGIN: i64 = 60;
/// Refresh errors after which the tokens are of no further use.
const UNUSABLE: [&str; 6] = [
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

const SESSION_KEY: &str = "chatgpt_session";
const EXPIRED: &str = "Phiên đăng nhập ChatGPT đã hết hạn, vào Cài đặt bấm Đăng nhập lại";
/// The issued client ID, kept after signing out so that the next sign-in reuses the registration.
const CLIENT_KEY: &str = "chatgpt_client_id";
/// The stable, opaque `ext_agent_host_id` of this installation.
const HOST_KEY: &str = "host_id";

static CANCEL: AtomicBool = AtomicBool::new(false);
static SIGNING_IN: AtomicBool = AtomicBool::new(false);
/// Refresh tokens rotate, so two refreshes must never race.
static REFRESHING: Mutex<()> = Mutex::new(());

/// The signed-in account. Signing out clears the tokens but keeps who it was.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub email: String,
    pub subject: String,
    pub client_id: String,
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// OpenAI refuses a refresh before this time (Unix seconds), and a refused refresh ends the session.
    pub earliest_refresh_at: i64,
    pub scopes: Vec<String>,
    /// The session ran out and could not be renewed: the account needs to sign in again.
    pub reauth: bool,
}

impl Session {
    pub fn signed_in(&self) -> bool {
        !self.access_token.is_empty() || !self.refresh_token.is_empty()
    }

    pub fn plan(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }

    fn signed_out(self) -> Self {
        Self { access_token: String::new(), refresh_token: String::new(), id_token: String::new(), reauth: false, ..self }
    }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn session(db: &Connection) -> Option<Session> {
    db::setting(db, SESSION_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

fn save(db: &Connection, session: &Session) -> Result<(), String> {
    let text = serde_json::to_string(session).map_err(|e| e.to_string())?;
    db::set_setting(db, SESSION_KEY, &text).map_err(|e| e.to_string())
}

/// What the settings page shows: never a token.
pub fn status(db: &Connection) -> Value {
    match session(db) {
        Some(s) if s.signed_in() => json!({ "signed_in": true, "email": s.email, "plan": s.plan() }),
        Some(s) if s.reauth => json!({ "signed_in": false, "expired": true, "email": s.email }),
        _ => json!({ "signed_in": false }),
    }
}

pub(crate) fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("Không tạo được số ngẫu nhiên: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The PKCE challenge of a verifier: base64url of its SHA-256, without padding.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn host_id(db: &Connection) -> Result<String, String> {
    if let Some(id) = db::setting(db, HOST_KEY).map_err(|e| e.to_string())?.filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    // UUID version 4, variant 1.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let id = format!("urn:uuid:{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
    db::set_setting(db, HOST_KEY, &id).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Stops a sign-in that is waiting for the browser.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Signs in through the system browser and keeps the session. `fresh` registers a new client, for another
/// ChatGPT account; otherwise the client registered earlier on this machine is reused.
pub fn sign_in(db: &Connection, fresh: bool) -> Result<Session, String> {
    exclusive(|| {
        // A refused one-time code (used or expired) takes one fresh authorization with the client just issued,
        // never a new registration, as OpenAI's devkit does.
        let result = match authorize(db, fresh) {
            Err(Refused::Grant(_)) if !CANCEL.load(Ordering::SeqCst) => authorize(db, false),
            other => other,
        };
        result.map_err(|e| match e {
            // Plan usage is for Plus and Pro: a Free account gets through registration and is refused here.
            Refused::Grant(message) => format!("{message}. Nếu bạn dùng ChatGPT Free: gói Free không được dùng cho app khác, cần gói Plus hoặc Pro; hãy dùng ChatGPT qua Codex."),
            Refused::Other(message) => message,
        })
    })
}

/// Runs one browser sign-in at a time, for this flow and for Codex's, which share the cancel button.
pub(crate) fn exclusive<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if SIGNING_IN.swap(true, Ordering::SeqCst) {
        return Err("Đang chờ bạn đăng nhập trong trình duyệt".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let result = work();
    SIGNING_IN.store(false, Ordering::SeqCst);
    result
}

/// Why a sign-in stopped: the token endpoint refused the code (worth one more try), or anything else.
enum Refused {
    Grant(String),
    Other(String),
}

impl From<String> for Refused {
    fn from(message: String) -> Self {
        Refused::Other(message)
    }
}

impl From<&str> for Refused {
    fn from(message: &str) -> Self {
        Refused::Other(message.to_owned())
    }
}

fn authorize(db: &Connection, fresh: bool) -> Result<Session, Refused> {
    let e = |e: rusqlite::Error| Refused::Other(e.to_string());
    let host = host_id(db)?;
    let saved_client = if fresh { None } else { db::setting(db, CLIENT_KEY).map_err(e)?.filter(|c| !c.is_empty()) };
    // The account registered with that client, for the hints and to make sure the same account comes back.
    let previous = session(db).filter(|s| saved_client.as_deref() == Some(s.client_id.as_str()));
    let listener = TcpListener::bind(("127.0.0.1", PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("Không mở được cổng nhận kết quả đăng nhập: {e}"))?;
    let port = listener.local_addr().map_err(|e| Refused::Other(e.to_string()))?.port();
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let (state, nonce, verifier) = (random_token()?, random_token()?, random_token()?);
    let challenge = challenge(&verifier);
    let client = saved_client.clone().unwrap_or_else(|| NEW_CLIENT.to_owned());
    let mut params = vec![
        ("client_id", client.as_str()),
        ("ext_agent_host_id", host.as_str()),
        ("response_type", "code"),
        ("redirect_uri", redirect.as_str()),
        ("scope", SCOPE),
        ("resource", RESOURCE),
        ("state", state.as_str()),
        ("nonce", nonce.as_str()),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
    ];
    if saved_client.is_none() {
        params.push(("agent_name_hint", AGENT_NAME));
    }
    if let Some(p) = &previous {
        // No `id_token_hint`: the address goes to the browser on a command line, where tokens do not belong.
        if !p.email.is_empty() {
            params.push(("login_hint", p.email.as_str()));
        }
        // Signed in before without the plan permission: ask for it again.
        if p.signed_in() && !p.plan() {
            params.push(("prompt", "consent"));
        }
    }
    crate::open_url(&format!("{AUTHORIZE}?{}", net::form(&params)))?;
    let callback = wait_for_callback(std::slice::from_ref(&listener), &state)?;
    let issued = issued_client(saved_client.as_deref(), callback.get("client_id").map(String::as_str))?;
    // Kept at once: if the exchange fails, the next try reuses this registration instead of making another.
    db::set_setting(db, CLIENT_KEY, &issued).map_err(e)?;
    let code = callback.get("code").filter(|c| !c.is_empty()).ok_or("OpenAI không trả về mã đăng nhập")?;
    let reply = post_form(
        TOKEN,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &issued),
            ("code", code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("resource", RESOURCE),
        ],
    )?;
    if reply.status != 200 {
        let (code, detail) = oauth_error(&reply);
        let message = format!("Đổi mã đăng nhập thất bại: {detail}");
        return Err(if code == "invalid_grant" { Refused::Grant(message) } else { Refused::Other(message) });
    }
    let token: Value = serde_json::from_str(&reply.body).map_err(|e| Refused::Other(e.to_string()))?;
    let session = from_token(&token, &issued, Some(&nonce), Session::default(), now())?;
    if let Some(p) = previous.filter(|p| !p.subject.is_empty()) {
        if p.subject != session.subject {
            return Err("Tài khoản vừa đăng nhập khác tài khoản đã đăng ký trên máy này. Hãy chọn Dùng tài khoản khác.".into());
        }
    }
    if session.refresh_token.is_empty() && session.scopes.iter().any(|s| s == "offline_access") {
        return Err("OpenAI không trả về refresh token, hãy đăng nhập lại".into());
    }
    save(db, &session)?;
    Ok(session)
}

/// The client ID to use after the callback: the one just issued for a new registration, or the saved one, which
/// a callback may repeat but never change.
fn issued_client(saved: Option<&str>, from_callback: Option<&str>) -> Result<String, String> {
    let from_callback = from_callback.filter(|c| !c.is_empty() && *c != NEW_CLIENT);
    match (saved, from_callback) {
        (None, Some(issued)) => Ok(issued.to_owned()),
        (None, None) => Err("Đăng ký chưa hoàn tất: OpenAI không trả về client ID".into()),
        (Some(saved), Some(other)) if other != saved => Err("OpenAI trả về client ID khác với đăng ký đã lưu".into()),
        (Some(saved), _) => Ok(saved.to_owned()),
    }
}

/// Waits for the browser to come back to a loopback address, until cancelled or out of time.
pub(crate) fn wait_for_callback(listeners: &[TcpListener], state: &str) -> Result<HashMap<String, String>, String> {
    for listener in listeners {
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    }
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            return Err("Đã hủy đăng nhập".into());
        }
        if Instant::now() > deadline {
            return Err("Hết thời gian chờ đăng nhập, hãy thử lại".into());
        }
        let mut idle = true;
        for listener in listeners {
            match listener.accept() {
                Ok((stream, _)) => {
                    idle = false;
                    if let Some(result) = answer_browser(stream, state) {
                        return result;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if idle {
            thread::sleep(Duration::from_millis(200));
        }
    }
}

const PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Card English</title>\
<body style=\"font:16px system-ui;text-align:center;padding:60px\"><h2>{title}</h2><p>{text}</p></body>";

fn page(stream: &mut TcpStream, status: &str, title: &str, text: &str) {
    let body = PAGE.replace("{title}", title).replace("{text}", text);
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

/// Reads one request; `None` when it is not the callback of this attempt (a favicon, a stale tab).
fn answer_browser(mut stream: TcpStream, state: &str) -> Option<Result<HashMap<String, String>, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = vec![0u8; 8192];
    let mut len = 0;
    while len < buf.len() {
        match stream.read(&mut buf[len..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                len += n;
                if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let head = String::from_utf8_lossy(&buf[..len]);
    let target = head.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" {
        page(&mut stream, "404 Not Found", "Không tìm thấy", "");
        return None;
    }
    let params = parse_query(query);
    if params.get("state").map(String::as_str) != Some(state) {
        page(&mut stream, "400 Bad Request", "Phiên đăng nhập không khớp", "Hãy bắt đầu đăng nhập lại từ Card English.");
        return None;
    }
    if let Some(error) = params.get("error") {
        page(&mut stream, "200 OK", "Chưa đăng nhập", "Bạn có thể đóng tab này và quay lại Card English.");
        return Some(Err(if error == "access_denied" {
            "Bạn đã từ chối cho phép, app chưa dùng được gói ChatGPT".into()
        } else {
            format!("OpenAI báo lỗi đăng nhập: {error}")
        }));
    }
    page(&mut stream, "200 OK", "Đăng nhập thành công", "Bạn có thể đóng tab này và quay lại Card English.");
    Some(Ok(params))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (net::decode(name), net::decode(value)))
        .collect()
}

pub(crate) fn post_form(url: &str, pairs: &[(&str, &str)]) -> Result<net::Reply, String> {
    let headers = [("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")];
    net::fetch(url, &headers, Some(&net::form(pairs)), 30)
}

/// The code of an OAuth error and a line for the user with the server's reason. The request ID goes to the log,
/// for a report to OpenAI.
pub(crate) fn oauth_error(reply: &net::Reply) -> (String, String) {
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    let code = json["error"].as_str().or(json["error"]["code"].as_str()).unwrap_or("không rõ lý do").to_owned();
    let mut detail = code.clone();
    for key in ["error_reason", "error_description"] {
        if let Some(text) = json[key].as_str().filter(|t| !t.is_empty()) {
            detail.push_str(&format!(", {text}"));
        }
    }
    let request = reply.headers.iter().find(|(name, _)| name == "x-request-id").map_or("", |(_, v)| v.as_str());
    eprintln!("chatgpt oauth: {} {detail} request {request}", reply.status);
    (code, detail)
}

/// `earliest_refresh_at` as Unix seconds: a number, or an RFC 3339 time; 0 when absent or unreadable.
pub(crate) fn unix_time(value: &Value) -> i64 {
    value.as_i64().or(value.as_f64().map(|f| f as i64)).or(value.as_str().and_then(parse_rfc3339)).unwrap_or(0)
}

/// `2026-10-26T08:00:00Z`, `2026-10-26T15:00:00.5+07:00` and the like, as Unix seconds.
fn parse_rfc3339(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (clock, zone) = match time.strip_suffix(['Z', 'z']) {
        Some(clock) => (clock, 0),
        None => {
            let at = time.rfind(['+', '-'])?;
            let (clock, zone) = time.split_at(at);
            let (h, mi) = zone[1..].split_once(':')?;
            let offset = h.parse::<i64>().ok()? * 3600 + mi.parse::<i64>().ok()? * 60;
            (clock, if zone.starts_with('-') { -offset } else { offset })
        }
    };
    let mut t = clock.split('.').next()?.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (h, mi, sec) = (t.next()??, t.next()??, t.next()??);
    // Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - zone)
}

/// A session from a token response. `base` is the session being renewed, whose fields stay when a refresh
/// does not repeat them; `nonce` is the value of this sign-in, `None` for a refresh.
fn from_token(token: &Value, client_id: &str, nonce: Option<&str>, base: Session, now: i64) -> Result<Session, String> {
    let text = |key: &str| token[key].as_str().unwrap_or_default().to_owned();
    let mut s = base;
    s.client_id = client_id.to_owned();
    s.access_token = text("access_token");
    if s.access_token.is_empty() {
        return Err("OpenAI không trả về access token".into());
    }
    let refresh = text("refresh_token");
    if !refresh.is_empty() {
        s.refresh_token = refresh;
    }
    s.expires_at = now + token["expires_in"].as_i64().unwrap_or(3600);
    s.earliest_refresh_at = unix_time(&token["earliest_refresh_at"]);
    s.reauth = false;
    if let Some(scope) = token["scope"].as_str() {
        s.scopes = scope.split_whitespace().map(str::to_owned).collect();
    }
    let id_token = text("id_token");
    if id_token.is_empty() {
        if nonce.is_some() {
            return Err("OpenAI không trả về ID token".into());
        }
        return Ok(s);
    }
    let claims = id_token_claims(&id_token, client_id, nonce, now)?;
    let subject = claims["sub"].as_str().unwrap_or_default().to_owned();
    if nonce.is_none() && !s.subject.is_empty() && s.subject != subject {
        return Err("Phiên ChatGPT đổi sang tài khoản khác, hãy đăng nhập lại".into());
    }
    s.subject = subject;
    if let Some(email) = claims["email"].as_str() {
        s.email = email.to_owned();
    }
    s.id_token = id_token;
    Ok(s)
}

/// The claims of an ID token after checking issuer, audience, expiry, nonce and subject. Its signature is not
/// checked: OpenID Connect Core 3.1.3.7 lets a client rely on TLS for an ID token received directly from the
/// token endpoint, which is the only place this one comes from, and the app uses it for the account's name only.
pub(crate) fn id_token_claims(token: &str, client_id: &str, nonce: Option<&str>, now: i64) -> Result<Value, String> {
    let invalid = |why: &str| format!("ID token không hợp lệ ({why})");
    let payload = token.split('.').nth(1).ok_or_else(|| invalid("định dạng"))?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| invalid("mã hóa"))?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("nội dung"))?;
    if claims["iss"] != ISSUER {
        return Err(invalid("issuer"));
    }
    let audience_ok = match &claims["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(list) => list.iter().any(|a| a == client_id),
        _ => false,
    };
    if !audience_ok {
        return Err(invalid("audience"));
    }
    // A minute of leeway for a clock that is slightly off.
    if claims["exp"].as_i64().unwrap_or(0) + 60 < now {
        return Err(invalid("hết hạn"));
    }
    if let Some(nonce) = nonce {
        if claims["nonce"] != nonce {
            return Err(invalid("nonce"));
        }
    }
    if claims["sub"].as_str().unwrap_or_default().is_empty() {
        return Err(invalid("subject"));
    }
    Ok(claims)
}

/// A valid access token, renewed first when it is about to run out.
pub fn access_token(db: &Connection) -> Result<String, CallError> {
    let _one_at_a_time = REFRESHING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let s = match session(db) {
        Some(s) if s.signed_in() => s,
        Some(s) if s.reauth => return Err(EXPIRED.into()),
        _ => return Err("Chưa đăng nhập ChatGPT".into()),
    };
    if !s.plan() {
        return Err("Bạn chưa cho phép app dùng gói ChatGPT, hãy bấm Cho phép trong Cài đặt".into());
    }
    let now = now();
    if !s.access_token.is_empty() && s.expires_at - REFRESH_MARGIN > now {
        return Ok(s.access_token);
    }
    if s.earliest_refresh_at > now {
        // Too early to renew; the token still works until it runs out.
        if !s.access_token.is_empty() && s.expires_at > now {
            return Ok(s.access_token);
        }
        return Err("Phiên ChatGPT chưa làm mới được, thử lại sau ít phút".into());
    }
    if s.refresh_token.is_empty() {
        save(db, &Session { reauth: true, ..s.signed_out() })?;
        return Err(EXPIRED.into());
    }
    let reply = post_form(
        TOKEN,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &s.client_id),
            ("refresh_token", &s.refresh_token),
            ("resource", RESOURCE),
        ],
    )?;
    if reply.status == 200 {
        let token: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let renewed = from_token(&token, &s.client_id, None, s.clone(), now)?;
        save(db, &renewed)?;
        return Ok(renewed.access_token);
    }
    let (code, _) = oauth_error(&reply);
    if UNUSABLE.contains(&code.as_str()) {
        save(db, &Session { reauth: true, ..s.signed_out() })?;
        return Err(EXPIRED.into());
    }
    if code == "invalid_client" {
        // The registration is gone (removed in ChatGPT settings): the next sign-in registers again.
        save(db, &s.signed_out())?;
        let _ = db::set_setting(db, CLIENT_KEY, "");
        return Err("Đăng ký của app với ChatGPT không còn, hãy đăng nhập lại".into());
    }
    // A network or server failure keeps the tokens for the next try.
    Err(format!("Không làm mới được phiên ChatGPT ({}), thử lại sau", reply.status).into())
}

/// Renews the session in the background shortly before it runs out, while the app is open: a session left idle for
/// a few hours can no longer be renewed (openai/sign-in-with-chatgpt-devkit#5), so it is kept in use.
pub fn keep_alive(db: &Connection) {
    let now = now();
    let due = session(db).is_some_and(|s| {
        s.signed_in() && s.plan() && s.expires_at - REFRESH_MARGIN <= now && s.earliest_refresh_at <= now
    });
    if due {
        if let Err(e) = access_token(db) {
            eprintln!("chatgpt keep alive: {}", e.message);
        }
    }
}

/// Signs out: revokes the renewable session, then forgets the tokens. Answers whether OpenAI confirmed it.
pub fn sign_out(db: &Connection) -> Result<bool, String> {
    let Some(s) = session(db) else { return Ok(true) };
    let confirmed = s.refresh_token.is_empty() || (0..3).any(|attempt| {
        if attempt > 0 {
            thread::sleep(Duration::from_secs(attempt));
        }
        revoke(&s).is_ok()
    });
    save(db, &s.signed_out())?;
    Ok(confirmed)
}

fn revoke(s: &Session) -> Result<(), String> {
    let (status, body) = net::request(DISCOVERY, &[], None, 15)?;
    let discovery: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let endpoint = discovery["revocation_endpoint"]
        .as_str()
        .filter(|u| status == 200 && u.starts_with("https://auth.openai.com/"))
        .ok_or("không có revocation endpoint")?;
    let reply = post_form(
        endpoint,
        &[("token", &s.refresh_token), ("token_type_hint", "refresh_token"), ("client_id", &s.client_id)],
    )?;
    if reply.status == 200 { Ok(()) } else { Err(format!("revoke {}", reply.status)) }
}

/// The models of the signed-in account, in the server's order.
pub fn models(db: &Connection) -> Result<Vec<ModelInfo>, CallError> {
    let token = access_token(db)?;
    let auth = format!("Bearer {token}");
    let reply = net::fetch(MODELS, &[("Authorization", &auth)], None, 20)?;
    if reply.status != 200 {
        return Err(api_error(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    Ok(parse_models(&json))
}

fn parse_models(json: &Value) -> Vec<ModelInfo> {
    let list = json["models"].as_array().or(json["data"].as_array()).cloned().unwrap_or_default();
    list.iter()
        .filter(|m| m["visibility"].as_str().is_none_or(|v| v == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().or(m["id"].as_str())?;
            let name = m["display_name"].as_str().unwrap_or(id);
            Some(ModelInfo { id: id.to_owned(), name: name.to_owned() })
        })
        .collect()
}

/// One answer from the Responses API, streamed as the flow requires.
pub fn chat(db: &Connection, model: &str, prompt: &Prompt) -> Result<Answer, CallError> {
    let token = access_token(db)?;
    let body = json!({
        "model": model,
        "instructions": prompt.system,
        "input": [
            { "role": "user", "content": prompt.example.0 },
            { "role": "assistant", "content": prompt.example.1 },
            { "role": "user", "content": prompt.question },
        ],
        "store": false,
        "stream": true,
    });
    let auth = format!("Bearer {token}");
    let headers = [("Authorization", auth.as_str()), ("Content-Type", "application/json"), ("Accept", "text/event-stream")];
    let reply = net::fetch(RESPONSES, &headers, Some(&body.to_string()), 120)?;
    let limits = rate_limits(&reply.headers);
    if reply.status != 200 {
        let mut error = api_error(&reply);
        if !limits.is_null() {
            error.details = json!({ "headers": limits });
        }
        return Err(error);
    }
    let mut answer = parse_stream(&reply.body)?;
    answer.limits = limits;
    Ok(answer)
}

/// The rate-limit headers, if the server sends any, as one object.
fn rate_limits(headers: &[(String, String)]) -> Value {
    let found: serde_json::Map<String, Value> = headers
        .iter()
        .filter(|(name, _)| name.starts_with("x-ratelimit-") || name.contains("usage"))
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    if found.is_empty() { Value::Null } else { Value::Object(found) }
}

/// The text of a stream of Responses events, which only counts once `response.completed` arrives.
pub(crate) fn parse_stream(body: &str) -> Result<Answer, CallError> {
    let mut text = String::new();
    let mut completed = None;
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else { continue };
        match event["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => text.push_str(event["delta"].as_str().unwrap_or_default()),
            "response.completed" => completed = Some(event["response"].clone()),
            "response.failed" => {
                let error = &event["response"]["error"];
                return Err(plan_error(error["code"].as_str(), error["message"].as_str(), error["param"].as_str()));
            }
            "response.incomplete" => {
                let reason = event["response"]["incomplete_details"]["reason"].as_str().unwrap_or("không rõ lý do");
                return Err(format!("ChatGPT trả lời dở dang ({reason})").into());
            }
            "error" => {
                let code = event["code"].as_str().or(event["error"]["code"].as_str());
                let message = event["message"].as_str().or(event["error"]["message"].as_str());
                return Err(plan_error(code, message, None));
            }
            _ => {}
        }
    }
    let response = completed.ok_or("Mất kết nối trước khi ChatGPT trả lời xong")?;
    if text.is_empty() {
        // Some streams carry the whole text in the final event only.
        for item in response["output"].as_array().into_iter().flatten() {
            for part in item["content"].as_array().into_iter().flatten() {
                if part["type"] == "output_text" {
                    text.push_str(part["text"].as_str().unwrap_or_default());
                }
            }
        }
    }
    Ok(Answer {
        text,
        input_tokens: response["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: response["usage"]["output_tokens"].as_u64().unwrap_or(0),
        limits: Value::Null,
    })
}

/// An error before the stream opened: the standard error object, or a bare `detail`.
fn api_error(reply: &net::Reply) -> CallError {
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    let error = &json["error"];
    if error["code"].is_string() {
        return plan_error(error["code"].as_str(), error["message"].as_str(), error["param"].as_str());
    }
    let detail = json["detail"].as_str().or(error["message"].as_str()).unwrap_or("không rõ lý do");
    match reply.status {
        401 => "Tài khoản ChatGPT không được chấp nhận, hãy đăng nhập lại".into(),
        403 => format!("ChatGPT từ chối yêu cầu: {detail}").into(),
        429 => CallError { message: "Đã chạm giới hạn sử dụng gói ChatGPT".into(), limit: true, details: Value::Null },
        503 => "ChatGPT tạm thời không dùng được, thử lại sau".into(),
        status => format!("ChatGPT lỗi {status}: {detail}").into(),
    }
}

/// The documented errors of plan usage, in words a learner understands.
fn plan_error(code: Option<&str>, message: Option<&str>, param: Option<&str>) -> CallError {
    let code = code.unwrap_or_default();
    let text = match code {
        "subscription_sharing_usage_limit_exceeded" => {
            return CallError {
                message: "Đã chạm giới hạn sử dụng gói ChatGPT (của gói hoặc giới hạn riêng cho app)".into(),
                limit: true,
                details: json!({ "code": code }),
            };
        }
        "usage_limit_reached" | "rate_limit_exceeded" => {
            return CallError {
                message: format!("Đã hết hạn mức sử dụng ({})", message.unwrap_or(code)),
                limit: true,
                details: json!({ "code": code }),
            };
        }
        "subscription_sharing_user_not_eligible" => "Tài khoản hoặc workspace này không được dùng gói ChatGPT cho app".into(),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            "ChatGPT tạm thời chưa kiểm tra được hạn mức, thử lại sau".into()
        }
        "subscription_sharing_unsupported_capability" => {
            format!("Model hoặc tham số không được hỗ trợ ({}), hãy chọn model khác", param.unwrap_or("?"))
        }
        "subscription_sharing_invalid_user" => "Không xác thực được tài khoản ChatGPT, hãy đăng nhập lại".into(),
        "model_not_found" => "Model này không có trong gói của bạn, hãy chọn model khác".into(),
        _ => format!("ChatGPT lỗi {code}: {}", message.unwrap_or("không rõ lý do")),
    };
    CallError { message: text, limit: false, details: json!({ "code": code }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let a = random_token().unwrap();
        assert_eq!(a.len(), 43);
        assert_ne!(a, random_token().unwrap());
    }

    #[test]
    fn host_id_is_a_stable_uuid_urn() {
        let db = db::open(std::path::Path::new(":memory:")).unwrap();
        let id = host_id(&db).unwrap();
        assert!(id.starts_with("urn:uuid:") && id.len() == 45, "{id}");
        assert_eq!(&id[23..24], "4");
        assert_eq!(host_id(&db).unwrap(), id);
    }

    #[test]
    fn the_callback_may_issue_a_client_but_never_swap_one() {
        assert_eq!(issued_client(None, Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert!(issued_client(None, None).is_err());
        assert!(issued_client(None, Some(NEW_CLIENT)).is_err());
        assert_eq!(issued_client(Some("oaiapp_1"), None).unwrap(), "oaiapp_1");
        assert_eq!(issued_client(Some("oaiapp_1"), Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert!(issued_client(Some("oaiapp_1"), Some("oaiapp_2")).is_err());
    }

    fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn id_tokens_are_checked_and_sessions_built() {
        let good = json!({ "iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n1", "sub": "user-1", "email": "a@b.c" });
        assert!(id_token_claims(&jwt(good.clone()), "oaiapp_1", Some("n1"), 1000).is_ok());
        for (key, value) in [("iss", json!("https://evil")), ("aud", json!("other")), ("exp", json!(10)), ("nonce", json!("n2")), ("sub", json!(""))] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(id_token_claims(&jwt(bad), "oaiapp_1", Some("n1"), 1000).is_err(), "{key}");
        }
        let mut list = good.clone();
        list["aud"] = json!(["x", "oaiapp_1"]);
        assert!(id_token_claims(&jwt(list), "oaiapp_1", Some("n1"), 1000).is_ok());

        let token = json!({ "access_token": "at", "refresh_token": "rt", "id_token": jwt(good.clone()), "expires_in": 3600,
                            "scope": "openid email chatgpt.tokens.use.direct" });
        let s = from_token(&token, "oaiapp_1", Some("n1"), Session::default(), 1000).unwrap();
        assert!(s.plan() && s.signed_in());
        assert_eq!((s.email.as_str(), s.subject.as_str(), s.expires_at), ("a@b.c", "user-1", 4600));
        // A refresh without an ID token keeps who it was and rotates the refresh token.
        let renewed = from_token(&json!({ "access_token": "at2", "refresh_token": "rt2" }), "oaiapp_1", None, s.clone(), 5000).unwrap();
        assert_eq!((renewed.refresh_token.as_str(), renewed.email.as_str(), renewed.scopes.len()), ("rt2", "a@b.c", 3));
        // A refresh may not switch the account.
        let mut other = good.clone();
        other["sub"] = json!("user-2");
        let switched = json!({ "access_token": "at3", "id_token": jwt(other) });
        assert!(from_token(&switched, "oaiapp_1", None, s.clone(), 1000).is_err());
        let signed_out = s.signed_out();
        assert!(!signed_out.signed_in() && signed_out.email == "a@b.c");
    }

    #[test]
    fn streams_count_only_when_completed() {
        let ok = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n\
                  data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n\
                  data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n";
        let answer = parse_stream(ok).unwrap();
        assert_eq!((answer.text.as_str(), answer.input_tokens, answer.output_tokens), ("Hello", 12, 3));
        let only_final = "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"content\":[{\"type\":\"output_text\",\"text\":\"Hi\"}]}]}}\n";
        assert_eq!(parse_stream(only_final).unwrap().text, "Hi");
        let cut = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n";
        assert!(parse_stream(cut).is_err());
        let limit = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n";
        let e = parse_stream(limit).unwrap_err();
        assert!(e.limit && e.message.contains("giới hạn"));
    }

    #[test]
    fn models_keep_the_listed_ones_in_order() {
        let json = json!({ "models": [
            { "slug": "gpt-a", "display_name": "GPT A", "visibility": "list" },
            { "slug": "gpt-hidden", "display_name": "Hidden", "visibility": "hide" },
            { "slug": "gpt-b", "display_name": "GPT B", "visibility": "list" } ] });
        let ids: Vec<String> = parse_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["gpt-a", "gpt-b"]);
        assert_eq!(parse_models(&json!({ "data": [{ "id": "x" }] }))[0].name, "x");
    }

    #[test]
    fn errors_before_the_stream_read_both_shapes() {
        let reply = |status, body: &str| net::Reply { status, headers: vec![], body: body.into() };
        assert!(api_error(&reply(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#)).limit);
        assert!(api_error(&reply(403, r#"{"detail":"region not supported"}"#)).message.contains("region not supported"));
        assert!(!api_error(&reply(401, "")).limit);
        let found = rate_limits(&[("x-ratelimit-remaining-requests".into(), "9".into()), ("content-type".into(), "x".into())]);
        assert_eq!(found, json!({ "x-ratelimit-remaining-requests": "9" }));
    }

    #[test]
    fn the_callback_listener_takes_only_this_attempt() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = thread::spawn(move || {
            let send = |path: &str| {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
                let mut out = String::new();
                stream.read_to_string(&mut out).unwrap();
                out
            };
            (send("/favicon.ico"), send("/auth/callback?code=x&state=old"), send("/auth/callback?code=abc&state=s1&client_id=oaiapp_9"))
        });
        let params = wait_for_callback(std::slice::from_ref(&listener), "s1").unwrap();
        let (favicon, stale, good) = browser.join().unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
        assert!(stale.starts_with("HTTP/1.1 400"), "{stale}");
        assert!(good.starts_with("HTTP/1.1 200") && good.contains("Đăng nhập thành công"));
        assert_eq!((params["code"].as_str(), params["client_id"].as_str()), ("abc", "oaiapp_9"));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.write_all(b"GET /auth/callback?error=access_denied&state=s2 HTTP/1.1\r\n\r\n").unwrap();
            let _ = stream.read_to_string(&mut String::new());
        });
        assert!(wait_for_callback(std::slice::from_ref(&listener), "s2").unwrap_err().contains("từ chối"));
    }

    #[test]
    fn earliest_refresh_times_read_both_shapes() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-26T08:00:00Z"), Some(1_792_972_800 + 8 * 3600));
        assert_eq!(parse_rfc3339("2026-10-26T15:00:00.5+07:00"), parse_rfc3339("2026-10-26T08:00:00Z"));
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00-01:30"), Some(951_782_400 + 5400));
        assert_eq!(parse_rfc3339("not a time"), None);
        assert_eq!(unix_time(&json!(1_790_000_000)), 1_790_000_000);
        assert_eq!(unix_time(&json!("2026-10-26T08:00:00Z")), parse_rfc3339("2026-10-26T08:00:00Z").unwrap());
        assert_eq!(unix_time(&Value::Null), 0);
    }

    #[test]
    fn refresh_waits_for_openai_and_an_expired_session_asks_to_sign_in() {
        let db = db::open(std::path::Path::new(":memory:")).unwrap();
        let plan = vec![PLAN_SCOPE.to_owned()];
        let base = Session { client_id: "oaiapp_1".into(), email: "a@b.c".into(), access_token: "at".into(), refresh_token: "rt".into(), scopes: plan, ..Session::default() };
        // About to run out but too early to renew: the current token is used.
        save(&db, &Session { expires_at: now() + 30, earliest_refresh_at: now() + 600, ..base.clone() }).unwrap();
        assert_eq!(access_token(&db).unwrap(), "at");
        // Run out and still too early: a wait, not a lost session.
        save(&db, &Session { expires_at: now() - 5, earliest_refresh_at: now() + 600, ..base.clone() }).unwrap();
        assert!(access_token(&db).unwrap_err().message.contains("thử lại"));
        assert!(session(&db).unwrap().signed_in());
        // No way to renew: the account is kept and asked to sign in again.
        save(&db, &Session { expires_at: now() - 5, refresh_token: String::new(), ..base.clone() }).unwrap();
        assert_eq!(access_token(&db).unwrap_err().message, EXPIRED);
        assert_eq!(status(&db), json!({ "signed_in": false, "expired": true, "email": "a@b.c" }));
        assert_eq!(access_token(&db).unwrap_err().message, EXPIRED);
    }

    #[test]
    fn oauth_errors_keep_the_servers_reason() {
        let reply = net::Reply {
            status: 400,
            headers: vec![("x-request-id".into(), "req_1".into())],
            body: r#"{"error":"invalid_grant","error_reason":"refresh_token_invalidated","error_description":"A fresh sign-in is required."}"#.into(),
        };
        let (code, detail) = oauth_error(&reply);
        assert_eq!(code, "invalid_grant");
        assert_eq!(detail, "invalid_grant, refresh_token_invalidated, A fresh sign-in is required.");
    }

    #[test]
    fn callback_queries_decode() {
        let q = parse_query("code=a%2Bb&state=s&scope=openid+email");
        assert_eq!(q["code"], "a+b");
        assert_eq!(q["scope"], "openid email");
    }
}

