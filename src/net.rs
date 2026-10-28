//! HTTP through the system's curl, as figghost does: no TLS stack in the binary, and the request (with its
//! API key or token) is handed over on standard input so that it never shows in the list of processes.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_BODY: u64 = 4 << 20;

fn curl() -> PathBuf {
    if cfg!(windows) {
        // Shipped with Windows 10 1803 and later.
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\curl.exe")
    } else {
        PathBuf::from("/usr/bin/curl")
    }
}

/// A command that does not flash a console window on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A value for a curl configuration file, quoted and escaped.
fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An answer: the status, the headers in lower case, and the body.
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// GET, or POST when there is a body. The body is read even for an error, so that the reason the server gives
/// can be shown.
pub fn fetch(url: &str, headers: &[(&str, &str)], body: Option<&str>, seconds: u64) -> Result<Reply, String> {
    let mut config = format!("url = {}\n", quote(url));
    for (name, value) in headers {
        config.push_str(&format!("header = {}\n", quote(&format!("{name}: {value}"))));
    }
    if let Some(body) = body {
        config.push_str(&format!("data-binary = {}\n", quote(body)));
    }
    run(&config, &[], seconds)
}

/// A request for binary content, which `fetch` cannot carry: the body is read from the file `body` and the answer's
/// body written to `out`, when given (the reply's own body is then empty). `method` is `GET`, `POST`, `PATCH`
/// and the like. The paths go on the command line, as `download` has them; the address and headers do not.
pub fn transfer(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&Path>, out: Option<&Path>, seconds: u64) -> Result<Reply, String> {
    let mut config = format!("url = {}\nrequest = {}\n", quote(url), quote(method));
    for (name, value) in headers {
        config.push_str(&format!("header = {}\n", quote(&format!("{name}: {value}"))));
    }
    let mut args = Vec::new();
    if let Some(body) = body {
        args.push(std::ffi::OsString::from("--data-binary"));
        let mut at = std::ffi::OsString::from("@");
        at.push(body);
        args.push(at);
    }
    if let Some(out) = out {
        args.push("--output".into());
        args.push(out.into());
    }
    run(&config, &args, seconds)
}

/// Runs curl on a configuration given on standard input, with `args` before it.
fn run(config: &str, args: &[std::ffi::OsString], seconds: u64) -> Result<Reply, String> {
    let mut child = command(curl())
        .args(["--silent", "--location", "--proto", "=https", "--max-time"])
        .arg(seconds.to_string())
        .args(args)
        .args(["--dump-header", "-", "--write-out", "\n%{http_code}", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Không tìm thấy curl trên máy".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes()).map_err(|e| e.to_string())?;
    }
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(MAX_BODY).read_to_string(&mut out).map_err(|e| e.to_string())?;
    }
    let _ = child.wait();
    split_reply(&out).ok_or_else(|| "Không kết nối được mạng".into())
}

/// The status and body only.
pub fn request(url: &str, headers: &[(&str, &str)], body: Option<&str>, seconds: u64) -> Result<(u16, String), String> {
    fetch(url, headers, body, seconds).map(|r| (r.status, r.body))
}

/// What curl printed: header blocks (one per redirect or `100 Continue`), the body, then the status code that
/// `--write-out` adds on a line of its own. `None` when nothing came back.
fn split_reply(out: &str) -> Option<Reply> {
    let (rest, code) = out.rsplit_once('\n')?;
    let status = code.trim().parse::<u16>().ok().filter(|&s| s != 0)?;
    let mut rest = rest;
    let mut headers = Vec::new();
    while rest.starts_with("HTTP/") {
        let (head, body) = rest.split_once("\r\n\r\n").unwrap_or((rest, ""));
        headers = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        rest = body;
    }
    Some(Reply { status, headers, body: rest.to_owned() })
}

/// Percent-encodes everything but the unreserved characters of RFC 3986.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The reverse of `encode`, with `+` as a space as forms have it.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(high), Some(low)) => {
                    out.push((high * 16 + low) as u8);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `name=value&...`, for a query string or a form body.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Downloads `url` to `to`, resuming a part left by an earlier try, and calls `progress` with the bytes so far
/// about every half second. The file only takes its name once complete.
pub fn download(url: &str, to: &Path, mut progress: impl FnMut(u64)) -> Result<(), String> {
    let part = to.with_extension("part");
    let mut child = command(curl())
        .args(["--silent", "--fail", "--location", "--proto", "=https", "--retry", "3"])
        .args(["--continue-at", "-", "--output"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Không tìm thấy curl trên máy".to_owned())?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        progress(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0));
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    if !status.success() {
        return Err(format!("Tải {url} thất bại"));
    }
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_keeps_the_config_on_one_line() {
        assert_eq!(quote("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
    }

    #[test]
    fn form_values_round_trip() {
        assert_eq!(encode("a b&c/é~"), "a%20b%26c%2F%C3%A9~");
        assert_eq!(form(&[("scope", "openid email"), ("x", "1")]), "scope=openid%20email&x=1");
        assert_eq!(decode("a+b%20c%2Fd%C3%A9"), "a b c/dé");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%aé"), "%aé");
    }

    #[test]
    fn replies_split_into_headers_and_body() {
        let out = "HTTP/1.1 100 Continue\r\n\r\nHTTP/2 429\r\nX-RateLimit-Remaining-Requests: 0\r\ncontent-type: application/json\r\n\r\n{\"a\":1}\n429";
        let reply = split_reply(out).unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.body, "{\"a\":1}");
        assert_eq!(reply.headers[0], ("x-ratelimit-remaining-requests".into(), "0".into()));
        assert_eq!(split_reply("\n000"), None);
        assert_eq!(split_reply("").map(|r| r.status), None);
    }
}
