//! Whether a newer version is out, from the latest GitHub release, the way figghost checks.

use crate::net;
use serde::Serialize;
use serde_json::Value;

pub const REPO: &str = "eliasbui/app-card-english";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, PartialEq)]
pub struct Check {
    pub current: String,
    pub latest: String,
    /// The release page, where the new version is downloaded by hand.
    pub url: String,
    pub newer: bool,
    pub notes: String,
}

fn releases() -> String {
    format!("https://github.com/{REPO}/releases/")
}

// shortcut: the API only (sixty calls an hour without sign-in, ample for a daily check); figghost also falls
// back to releases.atom, add it if the limit is ever hit.
pub fn check() -> Result<Check, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let headers = [("Accept", "application/vnd.github+json"), ("User-Agent", "card-english")];
    let (status, body) = net::request(&url, &headers, None, 10)?;
    match status {
        200 => parse(&body),
        404 => Err("Chưa có bản phát hành nào".into()),
        _ => Err(format!("GitHub trả lỗi {status}")),
    }
}

fn parse(body: &str) -> Result<Check, String> {
    let json: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let latest = json["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_owned();
    let url = json["html_url"].as_str().unwrap_or_default().to_owned();
    // The page is opened in the browser, so it has to be one of this repository's releases.
    if !is_version(&latest) || !url.starts_with(&releases()) {
        return Err("Bản phát hành không có số phiên bản".into());
    }
    let notes: String = json["body"].as_str().unwrap_or_default().chars().take(2000).collect();
    Ok(Check { newer: newer(&latest, CURRENT), current: CURRENT.into(), latest, url, notes })
}

pub fn is_release_url(url: &str) -> bool {
    url.starts_with(&releases())
}

/// Dotted numbers, with an optional `-beta.1` after them.
fn is_version(text: &str) -> bool {
    let numbers = text.split('-').next().unwrap_or_default();
    numbers.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `latest` comes after `current`; anything after the numbers (`-beta.1`) is ignored.
pub fn newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split('-').next().unwrap_or_default().split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (mut a, mut b) = (parts(latest), parts(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("1.0.0", "1.0"));
        assert!(!newer("1.0.0-beta.1", "1.0.0"));
        assert!(is_version("1.2.3") && is_version("1.2.3-beta.1"));
        assert!(!is_version("") && !is_version("v1") && !is_version("1..2"));
    }

    #[test]
    fn parse_takes_only_this_repositorys_release() {
        let ok = r#"{"tag_name":"v9.0.0","html_url":"https://github.com/eliasbui/app-card-english/releases/tag/v9.0.0","body":"- new"}"#;
        let c = parse(ok).unwrap();
        assert!(c.newer && c.latest == "9.0.0" && c.notes == "- new");
        let elsewhere = r#"{"tag_name":"v9.0.0","html_url":"https://evil.example/releases/tag/v9.0.0"}"#;
        assert!(parse(elsewhere).is_err());
    }
}
