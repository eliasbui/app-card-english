//! Google sign-in for installed apps (the system browser, a loopback redirect, PKCE) and the few Drive v3 calls the
//! sync needs, all in the hidden folder of the app in the learner's own Drive (`appDataFolder`).

use crate::{
    chatgpt::{challenge, exclusive, now, post_form, random_token, wait_for_redirect},
    db, net,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, net::TcpListener, path::Path};

/// The OAuth client of the app, baked in by whoever builds it; for an installed app Google does not treat the
/// secret as confidential. Without them the build has no Google sign-in.
const CLIENT_ID: Option<&str> = option_env!("CARD_ENGLISH_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("CARD_ENGLISH_GOOGLE_CLIENT_SECRET");
const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const REVOKE: &str = "https://oauth2.googleapis.com/revoke";
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
/// The app's own hidden folder, which Google lists as a non-sensitive scope; `openid email` names the account.
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
const SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.appdata";
const SESSION_KEY: &str = "google_session";
/// An access token (one hour) is renewed once it has less than this left.
const REFRESH_MARGIN: i64 = 120;
const NOT_CONFIGURED: &str = "Bản Card English này chưa được cấu hình đăng nhập Google nên chưa đồng bộ được";
const EXPIRED: &str = "Phiên đăng nhập Google đã hết hạn hoặc đã bị thu hồi. Hãy bấm Đăng nhập Google lại.";
const NO_SCOPE: &str = "Bạn chưa cho Card English lưu dữ liệu vào Google Drive. Hãy đăng nhập lại và đánh dấu ô cho phép app xem, tạo và xóa dữ liệu riêng của nó trong Google Drive.";
pub const OFFLINE: &str = "Không kết nối được Google Drive. Kiểm tra mạng rồi bấm Đồng bộ ngay.";

/// The signed-in Google account. Never shown to the page, never in a backup or the synced document.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The refresh token stopped working: the account needs to sign in again.
    pub reauth: bool,
}

fn client() -> Option<&'static str> {
    CLIENT_ID.filter(|id| !id.is_empty())
}

pub fn configured() -> bool {
    client().is_some()
}

/// A request to the token endpoint, with the client's ID and, when the build has one, its secret.
fn token_request(pairs: &[(&str, &str)]) -> Result<net::Reply, String> {
    let mut pairs = pairs.to_vec();
    pairs.push(("client_id", client().ok_or(NOT_CONFIGURED)?));
    if let Some(secret) = CLIENT_SECRET.filter(|s| !s.is_empty()) {
        pairs.push(("client_secret", secret));
    }
    post_form(TOKEN, &pairs).map_err(|_| OFFLINE.to_owned())
}

pub fn session(db: &Connection) -> Option<Session> {
    db::setting(db, SESSION_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

fn save(db: &Connection, session: &Session) -> Result<(), String> {
    let text = serde_json::to_string(session).map_err(|e| e.to_string())?;
    db::set_setting(db, SESSION_KEY, &text).map_err(|e| e.to_string())
}

/// Signs in through the system browser and keeps the session. The browser comes back to a free port of
/// 127.0.0.1, which Google allows any port of for an installed app.
pub fn sign_in(db: &Connection) -> Result<Session, String> {
    let client_id = client().ok_or(NOT_CONFIGURED)?;
    exclusive(|| {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("Không mở được cổng nhận kết quả đăng nhập: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect = format!("http://127.0.0.1:{port}");
        let (state, verifier) = (random_token()?, random_token()?);
        crate::open_url(&authorize_url(client_id, &redirect, &state, &verifier))?;
        let callback = wait_for_redirect(std::slice::from_ref(&listener), "/", &state)?;
        match callback.get("error").map(String::as_str) {
            Some("access_denied") => return Err("Bạn đã từ chối cho phép, Card English chưa đồng bộ được với Google Drive".into()),
            Some(error) => return Err(format!("Google báo lỗi đăng nhập: {error}")),
            None => {}
        }
        let code = callback.get("code").filter(|c| !c.is_empty()).ok_or("Google không trả về mã đăng nhập, hãy thử lại")?;
        let reply = token_request(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
        ])?;
        if reply.status != 200 {
            return Err(format!("Google không chấp nhận mã đăng nhập ({}), hãy thử lại", oauth_error(&reply.body).1));
        }
        let token: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let session = from_token(&token, client_id, Session::default(), now())?;
        if session.refresh_token.is_empty() {
            return Err("Google không trả về refresh token, hãy đăng nhập lại".into());
        }
        save(db, &session)?;
        Ok(session)
    })
}

/// Google's consent page for this attempt, with the PKCE challenge of `verifier`.
fn authorize_url(client_id: &str, redirect: &str, state: &str, verifier: &str) -> String {
    let challenge = challenge(verifier);
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("state", state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        // A refresh token every time, also for an account that agreed before.
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    format!("{AUTHORIZE}?{}", net::form(&params))
}

/// A session from a token response; `base` is the session being renewed, whose refresh token stays when Google
/// does not send a new one.
fn from_token(token: &Value, client_id: &str, base: Session, now: i64) -> Result<Session, String> {
    let text = |key: &str| token[key].as_str().unwrap_or_default().to_owned();
    let mut s = base;
    s.access_token = text("access_token");
    if s.access_token.is_empty() {
        return Err("Google không trả về access token, hãy đăng nhập lại".into());
    }
    // The consent screen lets each permission be unticked.
    if token["scope"].as_str().is_some_and(|scope| !scope.split_whitespace().any(|x| x == DRIVE_SCOPE)) {
        return Err(NO_SCOPE.into());
    }
    let refresh = text("refresh_token");
    if !refresh.is_empty() {
        s.refresh_token = refresh;
    }
    s.expires_at = now + token["expires_in"].as_i64().unwrap_or(3600);
    s.reauth = false;
    let id_token = text("id_token");
    if !id_token.is_empty() {
        s.email = email_of(&id_token, client_id, now)?;
    }
    Ok(s)
}

/// The address of the account in an ID token, after checking who issued it, for whom and until when. Its signature
/// is not checked: it comes straight from Google's token endpoint over TLS (OpenID Connect Core 3.1.3.7), and it
/// only names the account on the settings page.
fn email_of(token: &str, client_id: &str, now: i64) -> Result<String, String> {
    let invalid = |why: &str| format!("ID token của Google không hợp lệ ({why})");
    let payload = token.split('.').nth(1).ok_or_else(|| invalid("định dạng"))?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| invalid("mã hóa"))?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("nội dung"))?;
    if !matches!(claims["iss"].as_str(), Some("https://accounts.google.com" | "accounts.google.com")) {
        return Err(invalid("issuer"));
    }
    if claims["aud"] != client_id {
        return Err(invalid("audience"));
    }
    if claims["exp"].as_i64().unwrap_or(0) + 60 < now {
        return Err(invalid("hết hạn"));
    }
    Ok(claims["email"].as_str().unwrap_or_default().to_owned())
}

/// The code and description of an OAuth error body.
fn oauth_error(body: &str) -> (String, String) {
    let json: Value = serde_json::from_str(body).unwrap_or_default();
    let code = json["error"].as_str().unwrap_or("không rõ lý do").to_owned();
    let detail = json["error_description"].as_str().map_or_else(|| code.clone(), |d| format!("{code}, {d}"));
    (code, detail)
}

/// A valid access token, renewed first when it is about to run out.
pub fn access_token(db: &Connection) -> Result<String, String> {
    let client_id = client().ok_or(NOT_CONFIGURED)?;
    let s = match session(db) {
        Some(s) if !s.refresh_token.is_empty() => s,
        Some(s) if s.reauth => return Err(EXPIRED.into()),
        _ => return Err("Chưa đăng nhập Google, hãy bấm Đăng nhập Google trong Cài đặt".into()),
    };
    let now = now();
    if !s.access_token.is_empty() && s.expires_at - REFRESH_MARGIN > now {
        return Ok(s.access_token);
    }
    let reply = token_request(&[("grant_type", "refresh_token"), ("refresh_token", &s.refresh_token)])?;
    if reply.status == 200 {
        let token: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
        let renewed = from_token(&token, client_id, s, now)?;
        save(db, &renewed)?;
        return Ok(renewed.access_token);
    }
    // Revoked, or seven days old while the app's consent screen is still in testing.
    if oauth_error(&reply.body).0 == "invalid_grant" {
        save(db, &Session { email: s.email, reauth: true, ..Session::default() })?;
        return Err(EXPIRED.into());
    }
    Err(format!("Google chưa gia hạn được phiên đăng nhập (lỗi {}), hãy thử lại sau ít phút", reply.status))
}

/// Signs out: revokes the refresh token at Google, then forgets the account. Answers whether Google confirmed it.
pub fn sign_out(db: &Connection) -> Result<bool, String> {
    let revoked = match session(db).filter(|s| !s.refresh_token.is_empty()) {
        Some(s) => post_form(REVOKE, &[("token", &s.refresh_token)]).is_ok_and(|r| r.status == 200),
        None => true,
    };
    db::set_setting(db, SESSION_KEY, "").map_err(|e| e.to_string())?;
    Ok(revoked)
}

/// What the settings page shows of the account: never a token.
pub fn status(db: &Connection) -> Value {
    match session(db) {
        Some(s) if !s.refresh_token.is_empty() => json!({ "configured": configured(), "signed_in": true, "email": s.email }),
        Some(s) if s.reauth => json!({ "configured": configured(), "signed_in": false, "expired": true, "email": s.email }),
        _ => json!({ "configured": configured(), "signed_in": false }),
    }
}

/// A file of the app's folder; `version` grows with every change of its content.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct File {
    pub id: String,
    pub name: String,
    pub version: String,
}

/// A Drive error in words the learner can act on. The first reason Google gives decides it.
fn drive_error(status: u16, body: &str) -> String {
    let json: Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    let reasons: Vec<&str> = error["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(error["details"].as_array().into_iter().flatten())
        .filter_map(|e| e["reason"].as_str())
        .collect();
    let has = |names: &[&str]| reasons.iter().any(|r| names.contains(r));
    if has(&["storageQuotaExceeded", "quotaExceeded"]) {
        "Google Drive của bạn đã hết dung lượng. Hãy xóa bớt tệp trong Drive rồi bấm Đồng bộ ngay.".into()
    } else if has(&["insufficientPermissions", "insufficientScopes", "ACCESS_TOKEN_SCOPE_INSUFFICIENT"]) {
        NO_SCOPE.into()
    } else if has(&["accessNotConfigured", "SERVICE_DISABLED"]) {
        "Dự án Google của Card English chưa bật Google Drive API. Hãy báo cho người phát hành app.".into()
    } else if status == 429 || has(&["rateLimitExceeded", "userRateLimitExceeded", "RATE_LIMIT_EXCEEDED"]) {
        "Google Drive đang giới hạn số lần gọi. Hãy thử lại sau ít phút.".into()
    } else if status == 401 {
        EXPIRED.into()
    } else if status >= 500 {
        "Google Drive đang gặp sự cố. Hãy thử lại sau ít phút.".into()
    } else {
        let message = error["message"].as_str().unwrap_or("không rõ lý do");
        format!("Google Drive báo lỗi {status}: {message}")
    }
}

fn check(reply: net::Reply) -> Result<net::Reply, String> {
    if (200..300).contains(&reply.status) { Ok(reply) } else { Err(drive_error(reply.status, &reply.body)) }
}

/// Every file of the app's folder, the oldest first.
pub fn list(token: &str) -> Result<Vec<File>, String> {
    let auth = format!("Bearer {token}");
    let mut files = Vec::new();
    let mut page = String::new();
    loop {
        let mut query = vec![
            ("spaces", "appDataFolder"),
            ("pageSize", "1000"),
            ("orderBy", "createdTime"),
            ("fields", "nextPageToken,files(id,name,version)"),
        ];
        if !page.is_empty() {
            query.push(("pageToken", &page));
        }
        let reply = net::fetch(&format!("{FILES}?{}", net::form(&query)), &[("Authorization", &auth)], None, 30).map_err(|_| OFFLINE.to_owned())?;
        let (more, next) = parse_list(&check(reply)?.body)?;
        files.extend(more);
        match next {
            Some(next) => page = next,
            None => return Ok(files),
        }
    }
}

/// The files of one page of a listing, and the token of the next page.
fn parse_list(body: &str) -> Result<(Vec<File>, Option<String>), String> {
    let json: Value = serde_json::from_str(body).map_err(|_| "Google Drive trả về danh sách tệp không đọc được".to_owned())?;
    let files = serde_json::from_value(json["files"].clone()).unwrap_or_default();
    Ok((files, json["nextPageToken"].as_str().filter(|t| !t.is_empty()).map(str::to_owned)))
}

/// The version of one file now, to see whether another machine changed it meanwhile.
pub fn version(token: &str, id: &str) -> Result<String, String> {
    let auth = format!("Bearer {token}");
    let reply = net::fetch(&format!("{FILES}/{}?fields=version", net::encode(id)), &[("Authorization", &auth)], None, 30).map_err(|_| OFFLINE.to_owned())?;
    let json: Value = serde_json::from_str(&check(reply)?.body).unwrap_or_default();
    Ok(json["version"].as_str().unwrap_or_default().to_owned())
}

/// Downloads a file's content to `to`.
pub fn download(token: &str, id: &str, to: &Path) -> Result<(), String> {
    let auth = format!("Bearer {token}");
    let url = format!("{FILES}/{}?alt=media", net::encode(id));
    let reply = net::transfer("GET", &url, &[("Authorization", &auth)], None, Some(to), 300).map_err(|_| OFFLINE.to_owned())?;
    if (200..300).contains(&reply.status) {
        return Ok(());
    }
    // The error's body went to the file.
    let body = fs::read_to_string(to).unwrap_or_default();
    let _ = fs::remove_file(to);
    Err(drive_error(reply.status, &body))
}

/// Uploads `content` as a new file of the app's folder, through `part`, a scratch file for the request's body.
pub fn create(token: &str, name: &str, content: &[u8], mime: &str, part: &Path) -> Result<File, String> {
    let boundary = format!("card-english-{}", random_token()?);
    let meta = json!({ "name": name, "parents": ["appDataFolder"] });
    let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    fs::write(part, &body).map_err(|e| e.to_string())?;
    let auth = format!("Bearer {token}");
    let kind = format!("multipart/related; boundary={boundary}");
    let url = format!("{UPLOAD}?uploadType=multipart&fields=id,name,version");
    let reply = net::transfer("POST", &url, &[("Authorization", &auth), ("Content-Type", &kind)], Some(part), None, 300);
    let _ = fs::remove_file(part);
    parse_file(check(reply.map_err(|_| OFFLINE.to_owned())?)?)
}

/// Replaces the content of a file, through the scratch file `part`.
pub fn update(token: &str, id: &str, content: &[u8], mime: &str, part: &Path) -> Result<File, String> {
    fs::write(part, content).map_err(|e| e.to_string())?;
    let auth = format!("Bearer {token}");
    let url = format!("{UPLOAD}/{}?uploadType=media&fields=id,name,version", net::encode(id));
    let reply = net::transfer("PATCH", &url, &[("Authorization", &auth), ("Content-Type", mime)], Some(part), None, 300);
    let _ = fs::remove_file(part);
    parse_file(check(reply.map_err(|_| OFFLINE.to_owned())?)?)
}

fn parse_file(reply: net::Reply) -> Result<File, String> {
    serde_json::from_str(&reply.body).map_err(|_| "Google Drive trả về thông tin tệp không đọc được".to_owned())
}

pub fn delete(token: &str, id: &str) -> Result<(), String> {
    let auth = format!("Bearer {token}");
    let reply = net::transfer("DELETE", &format!("{FILES}/{}", net::encode(id)), &[("Authorization", &auth)], None, None, 30).map_err(|_| OFFLINE.to_owned())?;
    check(reply).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn the_consent_page_asks_for_the_app_folder_with_pkce() {
        let url = authorize_url("client-1", "http://127.0.0.1:50123", "st", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?client_id=client-1&redirect_uri=http%3A%2F%2F127.0.0.1%3A50123&"));
        // RFC 7636, appendix B.
        assert!(url.contains("&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256&"));
        assert!(url.contains("scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.appdata&state=st&"));
        assert!(url.contains("access_type=offline") && !url.contains("dBjftJeZ"), "the verifier never leaves the app");
    }

    #[test]
    fn a_token_response_gives_the_session_and_its_account() {
        let id = jwt(json!({ "iss": "https://accounts.google.com", "aud": "client-1", "exp": 5000, "sub": "1", "email": "hoc@gmail.com" }));
        let token = json!({ "access_token": "ya29.a", "expires_in": 3599, "refresh_token": "1//r", "id_token": id, "token_type": "Bearer",
                            "scope": "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/drive.appdata" });
        let s = from_token(&token, "client-1", Session::default(), 1000).unwrap();
        assert_eq!((s.email.as_str(), s.refresh_token.as_str(), s.expires_at), ("hoc@gmail.com", "1//r", 4599));
        // A refresh repeats neither the refresh token nor, always, the ID token.
        let renewed = from_token(&json!({ "access_token": "ya29.b", "expires_in": 3599, "scope": DRIVE_SCOPE }), "client-1", s.clone(), 2000).unwrap();
        assert_eq!((renewed.access_token.as_str(), renewed.refresh_token.as_str(), renewed.email.as_str()), ("ya29.b", "1//r", "hoc@gmail.com"));
        // The Drive permission left unticked on the consent screen.
        let without = json!({ "access_token": "ya29.c", "scope": "openid https://www.googleapis.com/auth/userinfo.email" });
        assert_eq!(from_token(&without, "client-1", Session::default(), 1000).unwrap_err(), NO_SCOPE);
        assert!(from_token(&json!({ "error": "x" }), "client-1", Session::default(), 1000).is_err());
        for bad in [json!({ "iss": "https://evil", "aud": "client-1", "exp": 5000 }), json!({ "iss": "accounts.google.com", "aud": "other", "exp": 5000 }),
                    json!({ "iss": "accounts.google.com", "aud": "client-1", "exp": 10 })] {
            assert!(email_of(&jwt(bad), "client-1", 1000).is_err());
        }
    }

    #[test]
    fn oauth_and_drive_errors_say_what_to_do() {
        assert_eq!(oauth_error(r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#).0, "invalid_grant");
        let quota = r#"{"error":{"code":403,"message":"The user's Drive storage quota has been exceeded.","errors":[{"domain":"usageLimits","reason":"storageQuotaExceeded"}]}}"#;
        assert!(drive_error(403, quota).contains("hết dung lượng"));
        let scope = r#"{"error":{"code":403,"message":"Request had insufficient authentication scopes.","status":"PERMISSION_DENIED",
            "details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#;
        assert_eq!(drive_error(403, scope), NO_SCOPE);
        let disabled = r#"{"error":{"code":403,"errors":[{"reason":"accessNotConfigured"}],"message":"Drive API has not been used"}}"#;
        assert!(drive_error(403, disabled).contains("Drive API"));
        assert!(drive_error(429, "").contains("thử lại"));
        assert_eq!(drive_error(401, r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#), EXPIRED);
        assert!(drive_error(404, r#"{"error":{"code":404,"message":"File not found: x."}}"#).contains("File not found"));
    }

    #[test]
    fn listings_read_files_and_the_next_page() {
        let body = r#"{"nextPageToken":"p2","files":[{"id":"1a","name":"card-english-sync.json.gz","version":"7"},{"id":"1b","name":"x.webp","version":"1"}]}"#;
        let (files, next) = parse_list(body).unwrap();
        assert_eq!(files[0], File { id: "1a".into(), name: "card-english-sync.json.gz".into(), version: "7".into() });
        assert_eq!((files.len(), next.as_deref()), (2, Some("p2")));
        assert_eq!(parse_list(r#"{"files":[]}"#).unwrap(), (vec![], None));
        assert!(parse_list("<html>").is_err());
    }

    #[test]
    fn signed_out_and_unconfigured_builds_say_so() {
        let db = db::open(Path::new(":memory:")).unwrap();
        assert_eq!(status(&db)["signed_in"], false);
        save(&db, &Session { email: "a@b.c".into(), reauth: true, ..Session::default() }).unwrap();
        assert_eq!(status(&db)["expired"], true);
        if !configured() {
            assert_eq!(access_token(&db).unwrap_err(), NOT_CONFIGURED);
        }
    }
}
