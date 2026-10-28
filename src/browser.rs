//! The browser extension in extension/: its menu sends a word from any page, with the sentence it was in, through
//! native messaging. The browser starts the app itself as the host, with the extension's origin as its argument;
//! that copy opens no window, only puts the word in a small inbox in the data folder and answers. The app that runs,
//! now or at its next start, takes the words out and looks them up as the shortcut does.

#[cfg(windows)]
use crate::net;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

/// The name the extension asks the browser for.
pub const HOST: &str = "com.eliasbui.cardenglish";
/// What the browser puts before the extension's ID in the argument it starts the host with.
pub const ORIGIN: &str = "chrome-extension://";
/// Fixed by the public key in extension/manifest.json, so that the unpacked extension always has it.
const EXTENSION_ID: &str = "bnalkdmmggdignneaebhplodflimgiip";
/// A word with its sentence is far smaller; anything bigger is not from the extension.
const MAX_MESSAGE: u32 = 64 * 1024;
const INBOX: &str = "inbox.jsonl";
/// As much of a selection as the shortcut takes from the clipboard: a passage gets its words picked.
const MAX_TEXT: usize = 4000;
const MAX_SENTENCE: usize = 500;
const MAX_URL: usize = 2000;

/// The browsers by the folder they keep their data in, where a NativeMessagingHosts folder tells them of hosts.
#[cfg(target_os = "macos")]
const BROWSERS: [&str; 5] = ["Google/Chrome", "Microsoft Edge", "BraveSoftware/Brave-Browser", "Chromium", "CocCoc"];
#[cfg(all(unix, not(target_os = "macos")))]
const BROWSERS: [&str; 4] = ["google-chrome", "microsoft-edge", "BraveSoftware/Brave-Browser", "chromium"];
/// The browsers by their key under HKEY_CURRENT_USER\Software, whose NativeMessagingHosts key tells them of hosts.
#[cfg(windows)]
const BROWSERS: [&str; 5] = [r"Google\Chrome", r"Microsoft\Edge", r"BraveSoftware\Brave-Browser", "Chromium", r"CocCoc\Browser"];

/// A word sent from a page: the selection, the sentence around it and the address of the page.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub text: String,
    pub sentence: String,
    pub url: String,
}

/// Text without control characters other than line breaks, trimmed and cut to `max` characters.
fn clean(text: &str, max: usize) -> String {
    let text: String = text.chars().filter(|&c| c == '\n' || !c.is_control()).take(max).collect();
    text.trim().to_owned()
}

impl Entry {
    /// What a message of the extension asks to add, or None when it has no text.
    fn from_message(message: &Value) -> Option<Entry> {
        let field = |key: &str| message[key].as_str().unwrap_or_default();
        let text = clean(field("text"), MAX_TEXT);
        let sentence = field("sentence").split_whitespace().collect::<Vec<_>>().join(" ");
        let url = clean(field("url"), MAX_URL);
        let web = url.starts_with("https://") || url.starts_with("http://");
        (!text.is_empty()).then(|| Entry { text, sentence: clean(&sentence, MAX_SENTENCE), url: if web { url } else { String::new() } })
    }
}

/// Reads one message of native messaging: its length in four bytes in the machine's byte order, then the JSON.
pub fn read_message(input: &mut impl Read) -> io::Result<Value> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len);
    if len > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("a message of {len} bytes")));
    }
    let mut body = vec![0; len as usize];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes one message of native messaging, framed as `read_message` reads it.
pub fn write_message(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    output.write_all(&(body.len() as u32).to_ne_bytes())?;
    output.write_all(body.as_bytes())?;
    output.flush()
}

/// The app as the browser's host: one message in, its word into the inbox, one answer out.
pub fn host(dir: &Path) {
    let answer = match read_message(&mut io::stdin().lock()) {
        Ok(message) => match Entry::from_message(&message) {
            Some(entry) => match push(dir, &entry) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "error": format!("Card English không lưu được từ này, hãy thử lại ({e})") }),
            },
            None => json!({ "ok": false, "error": "Hãy bôi đen một từ trên trang rồi chọn lại" }),
        },
        Err(e) => json!({ "ok": false, "error": format!("Card English không đọc được yêu cầu, hãy cập nhật tiện ích ({e})") }),
    };
    if let Err(e) = write_message(&mut io::stdout().lock(), &answer) {
        eprintln!("native messaging: {e}");
    }
}

/// Adds a word at the end of the inbox, under a lock so that the app never takes half of it.
fn push(dir: &Path, entry: &Entry) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(dir.join(INBOX))?;
    file.lock()?;
    writeln!(file, "{}", json!(entry))
}

/// The words waiting in the inbox, which is empty afterwards.
fn take_entries(dir: &Path) -> io::Result<Vec<Entry>> {
    let path = dir.join(INBOX);
    if !fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
        return Ok(Vec::new());
    }
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    file.lock()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    file.set_len(0)?;
    Ok(String::from_utf8_lossy(&bytes).lines().filter_map(|line| serde_json::from_str(line).ok()).collect())
}

/// The words waiting, as the page's quick add takes them: one word with its sentence and page, or several a line
/// each, which it reads as a passage to pick the words from.
pub fn take(dir: &Path) -> Option<Value> {
    let entries = take_entries(dir).map_err(|e| eprintln!("inbox: {e}")).ok()?;
    match entries.as_slice() {
        [] => None,
        [one] => Some(json!(one)),
        many => Some(json!(many.iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n"))),
    }
}

/// The file that tells a browser how to start the host and which extension may talk to it.
fn manifest(exe: &Path) -> String {
    let manifest = json!({
        "name": HOST,
        "description": "Card English: thêm từ từ trình duyệt",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("{ORIGIN}{EXTENSION_ID}/")],
    });
    serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n"
}

/// Writes a file unless it already holds `text`; answers whether it wrote.
fn write_if_changed(path: &Path, text: &str) -> io::Result<bool> {
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(path, text).map(|()| true)
}

/// Tells each browser installed where the host is, at every start since the app may have moved. A browser that is
/// not installed is passed over, and nothing that fails here stops the app.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn register(dir: &Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let text = manifest(&exe);
    let name = format!("{HOST}.json");
    #[cfg(unix)]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let base = if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".config") };
        for browser in BROWSERS.map(|b| base.join(b)).iter().filter(|b| b.is_dir()) {
            if let Err(e) = write_if_changed(&browser.join("NativeMessagingHosts").join(&name), &text) {
                eprintln!("native messaging host for {}: {e}", browser.display());
            }
        }
    }
    #[cfg(windows)]
    {
        // The key only points at the file, which stays in the data folder.
        let file = dir.join(&name);
        if let Err(e) = write_if_changed(&file, &text) {
            eprintln!("native messaging host: {e}");
            return;
        }
        let file = file.display().to_string();
        for browser in BROWSERS {
            let key = format!(r"HKCU\Software\{browser}\NativeMessagingHosts\{HOST}");
            let query = net::command("reg").args(["query", &key, "/ve"]).output();
            if query.is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains(&file)) {
                continue;
            }
            let added = net::command("reg")
                .args(["add", &key, "/ve", "/t", "REG_SZ", "/d", &file, "/f"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !added.is_ok_and(|s| s.success()) {
                eprintln!("native messaging host: cannot write {key}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_framed_by_their_length() {
        let mut wire = Vec::new();
        write_message(&mut wire, &json!({ "text": "negotiate", "sentence": "We negotiated a deal." })).unwrap();
        let len = u32::from_ne_bytes(wire[..4].try_into().unwrap()) as usize;
        assert_eq!(len, wire.len() - 4);
        assert_eq!(read_message(&mut wire.as_slice()).unwrap()["text"], "negotiate");
        // Cut short, too long, or not JSON.
        assert!(read_message(&mut &wire[..wire.len() - 1]).is_err());
        assert!(read_message(&mut &(MAX_MESSAGE + 1).to_ne_bytes()[..]).is_err());
        let mut junk = 3u32.to_ne_bytes().to_vec();
        junk.extend(b"abc");
        assert!(read_message(&mut junk.as_slice()).is_err());
    }

    #[test]
    fn a_message_gives_a_clean_entry() {
        let entry = Entry::from_message(&json!({ "text": " take\u{7} off ", "sentence": "The plane\n will  take off.", "url": "https://example.com/a" }));
        assert_eq!(entry, Some(Entry { text: "take off".into(), sentence: "The plane will take off.".into(), url: "https://example.com/a".into() }));
        assert_eq!(Entry::from_message(&json!({ "text": "word", "url": "javascript:alert(1)" })).unwrap().url, "");
        assert_eq!(Entry::from_message(&json!({ "text": "  " })), None);
        assert_eq!(Entry::from_message(&json!({ "sentence": "no word" })), None);
    }

    #[test]
    fn the_inbox_hands_over_its_words_once() {
        let dir = std::env::temp_dir().join(format!("card-english-inbox-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let _ = fs::remove_file(dir.join(INBOX));
        assert_eq!(take(&dir), None, "no inbox yet");
        let word = |text: &str| Entry { text: text.into(), sentence: format!("A {text} here."), url: String::new() };
        push(&dir, &word("deadline")).unwrap();
        assert_eq!(take(&dir), Some(json!({ "text": "deadline", "sentence": "A deadline here.", "url": "" })));
        assert_eq!(take(&dir), None, "taken once");
        push(&dir, &word("学习")).unwrap();
        fs::OpenOptions::new().append(true).open(dir.join(INBOX)).unwrap().write_all(b"not json\n").unwrap();
        push(&dir, &word("itinerary")).unwrap();
        assert_eq!(take(&dir), Some(json!("学习\nitinerary")), "several words come as a passage");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_extension_id_comes_from_its_key() {
        use base64::Engine;
        use sha2::Digest;
        let manifest: Value = serde_json::from_str(include_str!("../extension/manifest.json")).unwrap();
        let key = base64::engine::general_purpose::STANDARD.decode(manifest["key"].as_str().unwrap()).unwrap();
        // The first half of the key's SHA-256, each hexadecimal digit written as a letter from a to p.
        let id: String = sha2::Sha256::digest(&key)[..16].iter().flat_map(|b| [b >> 4, b & 15]).map(|n| char::from(b'a' + n)).collect();
        assert_eq!(id, EXTENSION_ID);
    }

    #[test]
    fn the_host_file_is_written_only_when_it_changes() {
        let text = manifest(Path::new("/Applications/Card English.app/Contents/MacOS/card-english"));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["name"], HOST);
        assert_eq!(parsed["path"], "/Applications/Card English.app/Contents/MacOS/card-english");
        assert_eq!(parsed["allowed_origins"], json!([format!("chrome-extension://{EXTENSION_ID}/")]));
        let dir = std::env::temp_dir().join(format!("card-english-host-{}", std::process::id()));
        let file = dir.join("NativeMessagingHosts").join(format!("{HOST}.json"));
        assert!(write_if_changed(&file, &text).unwrap());
        assert!(!write_if_changed(&file, &text).unwrap());
        assert!(write_if_changed(&file, "{}").unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }
}
