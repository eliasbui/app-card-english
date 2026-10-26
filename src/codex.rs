//! ChatGPT through Codex: signs in with the Codex CLI's public OAuth client and calls the backend Codex uses, so
//! any plan that includes Codex works, Free included while OpenAI offers it. This is not an official path for
//! other apps (OpenCode and ChatMock take it too) and OpenAI may change it at any time. The protocol follows
//! ChatMock as of October 2026.

use crate::{
    ai::{Answer, CallError, ModelInfo, Prompt},
    chatgpt::{self, challenge, exclusive, host_id, now, oauth_error, parse_stream, post_form, random_token, unix_time, wait_for_callback},
    db, net, update,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::TcpListener, sync::Mutex};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN: &str = "https://auth.openai.com/oauth/token";
/// The Codex client accepts this address only.
const PORT: u16 = 1455;
const REDIRECT: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const BASE: &str = "https://chatgpt.com/backend-api/codex";
const USAGE: &str = "https://chatgpt.com/backend-api/wham/usage";
/// The model catalogue answers according to the client version; this is the one ChatMock verified.
const MODELS_CLIENT_VERSION: &str = "0.160.0";
/// Who is calling, said plainly instead of passing for the Codex CLI.
const ORIGINATOR: &str = "card_english";
pub const USAGE_URL: &str = "https://chatgpt.com/codex/settings/usage";
const SESSION_KEY: &str = "codex_session";
/// An access token is renewed this long before it runs out.
const REFRESH_MARGIN: i64 = 120;
const EXPIRED: &str = "Phiên Codex đã hết hạn, vào Cài đặt bấm Đăng nhập lại";

/// Refresh tokens rotate, so two refreshes must never race.
static REFRESHING: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub email: String,
    pub account_id: String,
    /// `free`, `go`, `plus`, `pro`, `team`...
    pub plan: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The session ran out and could not be renewed: sign in again.
    pub reauth: bool,
}

impl Session {
    pub fn signed_in(&self) -> bool {
        !self.access_token.is_empty() || !self.refresh_token.is_empty()
    }
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
        Some(s) if s.signed_in() => json!({ "signed_in": true, "email": s.email, "plan": s.plan }),
        Some(s) if s.reauth => json!({ "signed_in": false, "expired": true, "email": s.email }),
        _ => json!({ "signed_in": false }),
    }
}

/// The installation as Codex reports it: the UUID of this machine's host ID.
fn installation_id(db: &Connection) -> Result<String, String> {
    Ok(host_id(db)?.trim_start_matches("urn:uuid:").to_owned())
}

fn authorize_url(state: &str, challenge: &str) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", ORIGINATOR),
    ];
    format!("{AUTHORIZE}?{}", net::form(&params))
}

/// Signs in through the system browser and keeps the session.
pub fn sign_in(db: &Connection) -> Result<Session, String> {
    exclusive(|| {
        // `localhost` may come in over IPv4 or IPv6; the IPv4 one is enough when the other is not available.
        let v4 = TcpListener::bind(("127.0.0.1", PORT)).map_err(|_| {
            "Cổng 1455 đang bận (có thể Codex CLI hoặc một app khác đang chờ đăng nhập). Hãy đóng nó rồi thử lại.".to_owned()
        })?;
        let mut listeners = vec![v4];
        if let Ok(v6) = TcpListener::bind(("::1", PORT)) {
            listeners.push(v6);
        }
        let (state, verifier) = (random_token()?, random_token()?);
        crate::open_url(&authorize_url(&state, &challenge(&verifier)))?;
        let callback = wait_for_callback(&listeners, &state)?;
        let code = callback.get("code").filter(|c| !c.is_empty()).ok_or("OpenAI không trả về mã đăng nhập")?;
        let reply = post_form(
            TOKEN,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT),
                ("client_id", CLIENT_ID),
                ("code_verifier", &verifier),
            ],
        )?;
        if reply.status != 200 {
            let (_, detail) = oauth_error(&reply);
            return Err(format!("Đổi mã đăng nhập thất bại: {detail}"));
        }
        let token: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let session = from_token(&token, Session::default(), now())?;
        save(db, &session)?;
        Ok(session)
    })
}

/// The payload of a JWT, unchecked: only the expiry and the plan are read from it, and the server checks the
/// token itself on every request.
fn claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()
}

/// A session from a token response. `base` is the session being renewed, whose fields stay when a refresh does
/// not repeat them.
fn from_token(token: &Value, base: Session, now: i64) -> Result<Session, String> {
    let text = |key: &str| token[key].as_str().unwrap_or_default().to_owned();
    let mut s = base;
    s.access_token = text("access_token");
    if s.access_token.is_empty() {
        return Err("OpenAI không trả về access token".into());
    }
    let refresh = text("refresh_token");
    if !refresh.is_empty() {
        s.refresh_token = refresh;
    }
    let access = claims(&s.access_token).unwrap_or_default();
    s.expires_at = access["exp"]
        .as_i64()
        .or(token["expires_in"].as_i64().map(|e| now + e))
        .unwrap_or(now + 3600);
    let id_token = text("id_token");
    if !id_token.is_empty() {
        let id = chatgpt::id_token_claims(&id_token, CLIENT_ID, None, now)?;
        let auth = &id["https://api.openai.com/auth"];
        let account = auth["chatgpt_account_id"].as_str().unwrap_or_default();
        if !s.account_id.is_empty() && !account.is_empty() && account != s.account_id {
            return Err("Phiên Codex đổi sang tài khoản khác, hãy đăng nhập lại".into());
        }
        if !account.is_empty() {
            s.account_id = account.to_owned();
        }
        if let Some(email) = id["email"].as_str() {
            s.email = email.to_owned();
        }
        let plan = auth["chatgpt_plan_type"].as_str().or(access["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str());
        if let Some(plan) = plan {
            s.plan = plan.to_owned();
        }
    }
    if s.account_id.is_empty() {
        return Err("Không đọc được tài khoản ChatGPT từ phiên đăng nhập".into());
    }
    s.reauth = false;
    Ok(s)
}

/// A valid access token and the account it belongs to, renewed first when it is about to run out or when
/// `force` says the server refused it.
fn access_token(db: &Connection, force: bool) -> Result<(String, String), CallError> {
    let _one_at_a_time = REFRESHING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let s = match session(db) {
        Some(s) if s.signed_in() => s,
        Some(s) if s.reauth => return Err(EXPIRED.into()),
        _ => return Err("Chưa đăng nhập ChatGPT qua Codex".into()),
    };
    if !force && !s.access_token.is_empty() && s.expires_at - REFRESH_MARGIN > now() {
        return Ok((s.access_token, s.account_id));
    }
    let expired = |s: Session| Session { email: s.email, plan: s.plan, reauth: true, ..Session::default() };
    if s.refresh_token.is_empty() {
        save(db, &expired(s))?;
        return Err(EXPIRED.into());
    }
    let reply = post_form(TOKEN, &[("grant_type", "refresh_token"), ("refresh_token", &s.refresh_token), ("client_id", CLIENT_ID)])?;
    if reply.status == 200 {
        let token: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let renewed = from_token(&token, s, now())?;
        save(db, &renewed)?;
        return Ok((renewed.access_token, renewed.account_id));
    }
    let (_, detail) = oauth_error(&reply);
    if matches!(reply.status, 400 | 401) {
        save(db, &expired(s))?;
        return Err(format!("{EXPIRED} ({detail})").into());
    }
    // A network or server failure keeps the tokens for the next try.
    Err(format!("Không làm mới được phiên Codex ({}), thử lại sau", reply.status).into())
}

/// Renews the session in the background shortly before it runs out, while the app is open.
pub fn keep_alive(db: &Connection) {
    let due = session(db).is_some_and(|s| s.signed_in() && s.expires_at - REFRESH_MARGIN <= now());
    if due {
        if let Err(e) = access_token(db, false) {
            eprintln!("codex keep alive: {}", e.message);
        }
    }
}

/// Forgets the session. The Codex CLI signs out the same way, with no revocation.
pub fn sign_out(db: &Connection) -> Result<(), String> {
    save(db, &Session::default())
}

fn user_agent() -> String {
    format!("card-english/{} ({}; {})", update::CURRENT, std::env::consts::OS, std::env::consts::ARCH)
}

/// Sends a request with the session's token, once more with a renewed token if the server refuses it.
fn send(db: &Connection, request: impl Fn(&str, &str) -> Result<net::Reply, String>) -> Result<net::Reply, CallError> {
    let (token, account) = access_token(db, false)?;
    let reply = request(&format!("Bearer {token}"), &account)?;
    if reply.status != 401 {
        return Ok(reply);
    }
    let (token, account) = access_token(db, true)?;
    Ok(request(&format!("Bearer {token}"), &account)?)
}

/// The models of the account, in the order the server recommends them.
pub fn models(db: &Connection) -> Result<Vec<ModelInfo>, CallError> {
    let install = installation_id(db)?;
    let agent = user_agent();
    let url = format!("{BASE}/models?client_version={MODELS_CLIENT_VERSION}");
    let reply = send(db, |auth, account| {
        let headers = [
            ("Authorization", auth),
            ("ChatGPT-Account-ID", account),
            ("Accept", "application/json"),
            ("User-Agent", agent.as_str()),
            ("originator", ORIGINATOR),
            ("x-codex-installation-id", install.as_str()),
        ];
        net::fetch(&url, &headers, None, 20)
    })?;
    if reply.status != 200 {
        return Err(http_error(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    Ok(parse_models(&json))
}

fn parse_models(json: &Value) -> Vec<ModelInfo> {
    let mut listed: Vec<&Value> = json["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list" && m["slug"].as_str().is_some_and(|s| !s.is_empty()))
        .collect();
    listed.sort_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX));
    listed
        .into_iter()
        .filter_map(|m| {
            let id = m["slug"].as_str()?;
            Some(ModelInfo { id: id.to_owned(), name: m["display_name"].as_str().unwrap_or(id).to_owned() })
        })
        .collect()
}

/// One answer, streamed from the backend Codex uses.
pub fn chat(db: &Connection, model: &str, prompt: &Prompt) -> Result<Answer, CallError> {
    let install = installation_id(db)?;
    let agent = user_agent();
    let message = |role: &str, kind: &str, text: &str| json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] });
    let mut body = json!({
        "model": model,
        "instructions": prompt.system,
        "input": [
            message("user", "input_text", prompt.example.0),
            message("assistant", "output_text", prompt.example.1),
            message("user", "input_text", prompt.question),
        ],
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": false,
        "stream": true,
        "prompt_cache_key": install,
        // A dictionary entry needs little thought: less is faster and uses less of the plan.
        "reasoning": { "effort": "low" },
        "client_metadata": { "x-codex-installation-id": install },
    });
    let post = |body: &Value| {
        send(db, |auth, account| {
            let headers = [
                ("Authorization", auth),
                ("ChatGPT-Account-ID", account),
                ("Content-Type", "application/json"),
                ("Accept", "text/event-stream"),
                ("User-Agent", agent.as_str()),
                ("originator", ORIGINATOR),
                ("OpenAI-Beta", "responses=experimental"),
                ("session-id", install.as_str()),
                ("x-codex-installation-id", install.as_str()),
            ];
            net::fetch(&format!("{BASE}/responses"), &headers, Some(&body.to_string()), 120)
        })
    };
    let mut reply = post(&body)?;
    if reply.status == 400 && reply.body.contains("reasoning") {
        // A model without that setting takes the request without it.
        if let Some(fields) = body.as_object_mut() {
            fields.remove("reasoning");
        }
        reply = post(&body)?;
    }
    let limits = header_limits(&reply.headers, now());
    if reply.status != 200 {
        let mut error = http_error(&reply);
        if !limits.is_null() {
            error.details["server"] = limits;
        }
        return Err(error);
    }
    let mut answer = parse_stream(&reply.body)?;
    answer.limits = limits;
    Ok(answer)
}

/// The plan's allowance as the usage endpoint reports it, for the measure button.
pub fn usage(db: &Connection) -> Result<Value, CallError> {
    let reply = send(db, |auth, account| {
        let headers = [("Authorization", auth), ("ChatGPT-Account-Id", account), ("Accept", "application/json"), ("User-Agent", "Mozilla/5.0")];
        net::fetch(USAGE, &headers, None, 20)
    })?;
    if reply.status != 200 {
        return Err(http_error(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    Ok(parse_usage(&json, now()))
}

/// The windows of the allowance, each as the share used and when it starts over. The shape has changed several
/// times in 2026, so each field has its known aliases, and a window is told five-hour or weekly by its length.
fn parse_usage(json: &Value, now: i64) -> Value {
    let root = json.get("rate_limit").filter(|v| v.is_object()).or(json.get("rate_limits").filter(|v| v.is_object()));
    let mut windows = Vec::new();
    if let Some(root) = root {
        for (weekly, names) in [(false, ["five_hour", "primary_window", "primary"]), (true, ["weekly", "secondary_window", "secondary"])] {
            if let Some(found) = names.iter().find_map(|name| root.get(*name).filter(|v| v.is_object())) {
                windows.extend(window(found, weekly, now));
            }
        }
    }
    json!({
        "plan": json["plan_type"],
        "limit_reached": root.map_or(Value::Null, |r| r["limit_reached"].clone()),
        "windows": windows,
    })
}

fn window(w: &Value, weekly_by_place: bool, now: i64) -> Option<Value> {
    let used = w["used_percent"]
        .as_f64()
        .or(w["usedPercent"].as_f64())
        .or(w["percent_left"].as_f64().map(|left| 100.0 - left))?;
    let seconds = w["limit_window_seconds"].as_i64();
    let reset = w["reset_time_ms"]
        .as_i64()
        .map(|ms| ms / 1000)
        .or_else(|| Some(unix_time(&w["reset_at"])).filter(|&t| t > 0))
        .or_else(|| Some(unix_time(&w["resetsAt"])).filter(|&t| t > 0))
        .or_else(|| w["reset_after_seconds"].as_i64().map(|s| now + s));
    Some(json!({
        "kind": if seconds.map_or(weekly_by_place, |s| s >= 172_800) { "week" } else { "5h" },
        "used_percent": used,
        "window_minutes": seconds.map(|s| s / 60),
        "resets_at": reset,
    }))
}

/// The same windows from the headers of an answer.
fn header_limits(headers: &[(String, String)], now: i64) -> Value {
    let get = |name: String| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.trim().to_owned());
    let mut windows = Vec::new();
    for (prefix, weekly_by_place) in [("primary", false), ("secondary", true)] {
        let Some(used) = get(format!("x-codex-{prefix}-used-percent")).and_then(|v| v.parse::<f64>().ok()) else { continue };
        let minutes = get(format!("x-codex-{prefix}-window-minutes")).and_then(|v| v.parse::<i64>().ok());
        let reset = get(format!("x-codex-{prefix}-reset-after-seconds")).and_then(|v| v.parse::<i64>().ok()).map(|s| now + s);
        windows.push(json!({
            "kind": if minutes.map_or(weekly_by_place, |m| m >= 2880) { "week" } else { "5h" },
            "used_percent": used,
            "window_minutes": minutes,
            "resets_at": reset,
        }));
    }
    if windows.is_empty() { Value::Null } else { json!({ "windows": windows }) }
}

/// Codex's errors; a used-up allowance says which plan it was and when it comes back.
fn http_error(reply: &net::Reply) -> CallError {
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    let error = &json["error"];
    let kind = error["type"].as_str().or(error["code"].as_str()).unwrap_or_default();
    let message = error["message"].as_str().or(json["detail"].as_str()).unwrap_or("không rõ lý do");
    if reply.status == 429 || kind == "usage_limit_reached" {
        let mut text = "Đã hết hạn mức Codex".to_owned();
        if let Some(plan) = error["plan_type"].as_str() {
            text.push_str(&format!(" của gói {plan}"));
        }
        if let Some(seconds) = error["resets_in_seconds"].as_i64() {
            text.push_str(&format!(", dùng lại được sau {}", duration(seconds)));
        }
        let details = json!({ "type": kind, "plan": error["plan_type"], "resets_in_seconds": error["resets_in_seconds"] });
        return CallError { message: text, limit: true, details };
    }
    match reply.status {
        401 => EXPIRED.into(),
        403 => format!("Codex từ chối yêu cầu (403): {message}").into(),
        404 => "Model này không có trong gói của bạn, hãy chọn model khác".into(),
        status => format!("Codex lỗi {status}: {message}").into(),
    }
}

fn duration(seconds: i64) -> String {
    match seconds {
        s if s >= 86_400 => format!("{} ngày", (s + 43_200) / 86_400),
        s if s >= 3600 => format!("{} giờ", (s + 1800) / 3600),
        s if s >= 60 => format!("{} phút", (s + 30) / 60),
        s => format!("{} giây", s.max(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn the_sign_in_address_has_what_the_codex_client_needs() {
        let url = authorize_url("st", "ch");
        for part in [
            "https://auth.openai.com/oauth/authorize?",
            "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            "redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
            "scope=openid%20profile%20email%20offline_access",
            "code_challenge=ch",
            "code_challenge_method=S256",
            "id_token_add_organizations=true",
            "codex_cli_simplified_flow=true",
            "state=st",
            "originator=card_english",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
    }

    #[test]
    fn a_token_response_gives_the_account_plan_and_expiry() {
        let id = jwt(json!({ "iss": "https://auth.openai.com", "aud": CLIENT_ID, "exp": 9_999_999_999i64, "sub": "u1", "email": "a@b.c",
                             "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1", "chatgpt_plan_type": "free" } }));
        let access = jwt(json!({ "exp": 5000 }));
        let token = json!({ "access_token": access, "refresh_token": "rt", "id_token": id });
        let s = from_token(&token, Session::default(), 1000).unwrap();
        assert_eq!((s.email.as_str(), s.account_id.as_str(), s.plan.as_str(), s.expires_at), ("a@b.c", "acc-1", "free", 5000));
        assert!(s.signed_in() && !s.reauth);
        // A refresh without an ID token keeps the account and rotates the refresh token.
        let renewed = from_token(&json!({ "access_token": jwt(json!({ "exp": 9000 })), "refresh_token": "rt2" }), s.clone(), 4000).unwrap();
        assert_eq!((renewed.account_id.as_str(), renewed.refresh_token.as_str(), renewed.expires_at), ("acc-1", "rt2", 9000));
        // An ID token for another client or without an account is refused.
        let other = jwt(json!({ "iss": "https://auth.openai.com", "aud": "other", "exp": 9_999_999_999i64, "sub": "u1" }));
        assert!(from_token(&json!({ "access_token": "x", "id_token": other }), Session::default(), 1000).is_err());
        assert!(from_token(&json!({ "access_token": "x" }), Session::default(), 1000).is_err());
    }

    #[test]
    fn usage_reads_every_shape_of_2026() {
        let two = json!({ "plan_type": "free", "rate_limit": { "limit_reached": false,
            "primary_window": { "used_percent": 12, "limit_window_seconds": 18000, "reset_after_seconds": 600, "reset_at": 1_790_000_000 },
            "secondary_window": { "used_percent": 30.5, "limit_window_seconds": 604800, "reset_at": 1_790_500_000 } } });
        let u = parse_usage(&two, 100);
        assert_eq!(u["plan"], "free");
        assert_eq!(u["windows"][0], json!({ "kind": "5h", "used_percent": 12.0, "window_minutes": 300, "resets_at": 1_790_000_000 }));
        assert_eq!(u["windows"][1]["kind"], "week");
        // One weekly window under the primary name, as reported since July.
        let one = json!({ "rate_limit": { "primary_window": { "used_percent": 1, "limit_window_seconds": 604800, "reset_after_seconds": 50 }, "secondary_window": null } });
        assert_eq!(parse_usage(&one, 100)["windows"], json!([{ "kind": "week", "used_percent": 1.0, "window_minutes": 10080, "resets_at": 150 }]));
        // The older names, share left and milliseconds.
        let legacy = json!({ "rate_limits": { "five_hour": { "percent_left": 73.5, "reset_time_ms": 1_716_393_600_000i64 } } });
        assert_eq!(parse_usage(&legacy, 0)["windows"][0], json!({ "kind": "5h", "used_percent": 26.5, "window_minutes": null, "resets_at": 1_716_393_600 }));
        assert_eq!(parse_usage(&json!({}), 0)["windows"], json!([]));
    }

    #[test]
    fn answer_headers_give_the_same_windows() {
        let headers = vec![
            ("x-codex-primary-used-percent".to_owned(), "40".to_owned()),
            ("x-codex-primary-window-minutes".to_owned(), "300".to_owned()),
            ("x-codex-primary-reset-after-seconds".to_owned(), "60".to_owned()),
            ("x-codex-secondary-used-percent".to_owned(), "5.5".to_owned()),
            ("x-codex-secondary-window-minutes".to_owned(), "10080".to_owned()),
        ];
        let l = header_limits(&headers, 1000);
        assert_eq!(l["windows"][0], json!({ "kind": "5h", "used_percent": 40.0, "window_minutes": 300, "resets_at": 1060 }));
        assert_eq!(l["windows"][1]["kind"], "week");
        assert_eq!(header_limits(&[], 0), Value::Null);
    }

    #[test]
    fn a_used_up_allowance_says_when_it_comes_back() {
        let reply = net::Reply {
            status: 429,
            headers: vec![],
            body: r#"{"error":{"type":"usage_limit_reached","message":"limit","plan_type":"free","resets_in_seconds":5400}}"#.into(),
        };
        let e = http_error(&reply);
        assert!(e.limit);
        assert_eq!(e.message, "Đã hết hạn mức Codex của gói free, dùng lại được sau 2 giờ");
        let missing = net::Reply { status: 404, headers: vec![], body: String::new() };
        assert!(!http_error(&missing).limit);
        assert_eq!((duration(45), duration(90), duration(86_400 * 3)), ("45 giây".into(), "2 phút".into(), "3 ngày".into()));
    }

    #[test]
    fn models_keep_the_listed_ones_by_priority() {
        let json = json!({ "models": [
            { "slug": "gpt-b", "visibility": "list", "priority": 2 },
            { "slug": "gpt-hidden", "visibility": "hide", "priority": 0 },
            { "slug": "gpt-a", "display_name": "GPT A", "visibility": "list", "priority": 1 } ] });
        let models = parse_models(&json);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gpt-a", "gpt-b"]);
        assert_eq!(models[0].name, "GPT A");
    }
}
