//! Whether a newer version is out, from the latest GitHub release, and on macOS putting it in place of the
//! installed app, the way figghost updates: download, check, swap beside the app, then a restart.
//!
//! The installed app is not touched until the new one has passed every check; a failure at any step removes
//! what was downloaded and leaves the app as it was.

use crate::net;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

pub const REPO: &str = "eliasbui/app-card-english";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const BUNDLE_ID: &str = "com.eliasbui.cardenglish";
/// The app inside the release zip, whatever the installed copy has been renamed to.
const APP: &str = "Card English.app";
/// Where the new copy is unpacked and where the old one waits to be deleted: beside the installed app, on the
/// same disk, so that the swap is a rename.
const STAGE: &str = ".CardEnglish-update";
const OLD: &str = ".CardEnglish-old";

#[derive(Debug, Serialize, PartialEq)]
pub struct Check {
    pub current: String,
    pub latest: String,
    /// The release page, where the new version is downloaded by hand.
    pub url: String,
    pub newer: bool,
    pub notes: String,
    /// The app can download and put the new version in place by itself.
    pub installable: bool,
    #[serde(skip)]
    pub asset: Option<Asset>,
}

/// The macOS zip a release carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    /// The SHA-256 GitHub computed on upload, in lower case hex, when it says.
    pub sha256: Option<String>,
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
    let name = format!("CardEnglish-{latest}-macos.zip");
    let asset = json["assets"].as_array().and_then(|a| a.iter().find(|a| a["name"] == name.as_str())).and_then(|a| {
        Some(Asset {
            url: a["browser_download_url"].as_str().filter(|u| u.starts_with(&releases()))?.to_owned(),
            size: a["size"].as_u64()?,
            sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|h| h.len() == 64).map(str::to_lowercase),
        })
    });
    let installable = asset.is_some() && running_bundle().is_some();
    Ok(Check { newer: newer(&latest, CURRENT), current: CURRENT.into(), latest, url, notes, installable, asset })
}

/// The app this program runs from, with every link followed; None off macOS and for a build folder.
pub fn running_bundle() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    bundle_of(&std::env::current_exe().ok()?.canonicalize().ok()?)
}

fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent().filter(|d| d.file_name() == Some(OsStr::new("MacOS")))?;
    let contents = macos.parent().filter(|d| d.file_name() == Some(OsStr::new("Contents")))?;
    let bundle = contents.parent()?;
    bundle.extension().is_some_and(|e| e == "app").then(|| bundle.to_path_buf())
}

/// Removes what an update cut short left beside the app; the old copy only while the app itself is there,
/// since after a failed swap it could be the only one.
pub fn tidy(bundle: &Path) {
    let Some(folder) = bundle.parent() else { return };
    let _ = fs::remove_dir_all(folder.join(STAGE));
    if bundle.exists() {
        let _ = fs::remove_dir_all(folder.join(OLD));
    }
}

/// Downloads the release's app and puts it where `bundle` is; `progress` hears the percent downloaded.
pub fn install(check: &Check, bundle: &Path, progress: impl Fn(u8)) -> Result<(), String> {
    // Put there already by other means (Homebrew) while this copy was running: a restart is all that is left.
    if plist_value(bundle, "CFBundleShortVersionString").is_ok_and(|v| v == check.latest) {
        return Ok(());
    }
    let asset = check.asset.as_ref().ok_or("Bản phát hành này không có gói cho macOS")?;
    let folder = bundle.parent().ok_or("Không thấy thư mục của ứng dụng")?;
    tidy(bundle);
    let stage = folder.join(STAGE);
    fs::create_dir_all(&stage).map_err(|e| format!("Không ghi được vào {}: {e}", folder.display()))?;
    let zip = stage.join("update.zip");
    let done = net::download(&asset.url, &zip, |bytes| {
        // Never 100 before the checks.
        progress((bytes.saturating_mul(100) / asset.size.max(1)).min(99) as u8)
    })
    .and_then(|()| put_in_place(&zip, asset, &check.latest, bundle));
    let _ = fs::remove_dir_all(&stage);
    done
}

/// Checks the downloaded zip, unpacks it beside it and swaps the app in.
fn put_in_place(zip: &Path, asset: &Asset, version: &str, bundle: &Path) -> Result<(), String> {
    let size = fs::metadata(zip).map_err(|e| e.to_string())?.len();
    if size != asset.size {
        return Err(format!("Gói tải về nặng {size} byte, không phải {}", asset.size));
    }
    if asset.sha256.as_ref().is_some_and(|expected| sha256(zip).ok().as_ref() != Some(expected)) {
        return Err("Gói tải về không khớp mã kiểm tra trên GitHub".into());
    }
    let stage = zip.parent().ok_or("Không thấy thư mục tải về")?;
    run("/usr/bin/ditto", [OsStr::new("-x"), OsStr::new("-k"), zip.as_os_str(), stage.as_os_str()])?;
    let fresh = stage.join(APP);
    check_app(&fresh, version)?;
    let old = bundle.with_file_name(OLD);
    swap(bundle, &fresh, &old)?;
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

fn sha256(file: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut fs::File::open(file).map_err(|e| e.to_string())?, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// This app, of the version the release says, with a signature that holds together.
fn check_app(app: &Path, version: &str) -> Result<(), String> {
    if plist_value(app, "CFBundleIdentifier").ok().as_deref() != Some(BUNDLE_ID) {
        return Err("Gói tải về không phải Card English".into());
    }
    let found = plist_value(app, "CFBundleShortVersionString")?;
    if found != version {
        return Err(format!("Gói tải về là bản {found}, không phải {version}"));
    }
    if !app.join("Contents/MacOS/card-english").is_file() {
        return Err("Gói tải về không có chương trình".into());
    }
    run("/usr/bin/codesign", [OsStr::new("--verify"), OsStr::new("--strict"), app.as_os_str()]).map(drop)
}

fn plist_value(app: &Path, key: &str) -> Result<String, String> {
    let plist = app.join("Contents/Info.plist");
    let args = [OsStr::new("-extract"), OsStr::new(key), OsStr::new("raw"), OsStr::new("-o"), OsStr::new("-"), plist.as_os_str()];
    run("/usr/bin/plutil", args)
}

/// Moves the installed app aside and the new one in; when the second move fails the first is undone, so there
/// is always an app where `installed` is.
fn swap(installed: &Path, fresh: &Path, old: &Path) -> Result<(), String> {
    fs::rename(installed, old).map_err(|e| format!("Không dời được bản đang cài: {e}"))?;
    if let Err(e) = fs::rename(fresh, installed) {
        let _ = fs::rename(old, installed);
        return Err(format!("Không đặt được bản mới vào chỗ: {e}"));
    }
    Ok(())
}

/// Starts the installed app again once this copy has quit (it holds the lock of the data folder until then).
pub fn relaunch() -> Result<Value, String> {
    let bundle = running_bundle().ok_or("Bản này không khởi động lại được")?;
    net::command("/bin/sh")
        .args(["-c", "sleep 1; /usr/bin/open -n \"$0\""])
        .arg(&bundle)
        .spawn()
        .map_err(|e| e.to_string())?;
    std::process::exit(0);
}

/// Runs a system tool and gives what it printed, or the first line of its complaint.
fn run<'a>(program: &str, args: impl IntoIterator<Item = &'a OsStr>) -> Result<String, String> {
    let output = net::command(program).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let complaint = String::from_utf8_lossy(&output.stderr);
    Err(format!("{program}: {}", complaint.lines().next().unwrap_or("lỗi")))
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

    #[test]
    fn parse_finds_the_macos_zip_of_this_repository() {
        let body = r#"{"tag_name":"v9.0.0","html_url":"https://github.com/eliasbui/app-card-english/releases/tag/v9.0.0","assets":[
            {"name":"CardEnglish-9.0.0-windows-x64.zip","size":1,"browser_download_url":"https://github.com/eliasbui/app-card-english/releases/download/v9.0.0/CardEnglish-9.0.0-windows-x64.zip"},
            {"name":"CardEnglish-9.0.0-macos.zip","size":42,"digest":"sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
             "browser_download_url":"https://github.com/eliasbui/app-card-english/releases/download/v9.0.0/CardEnglish-9.0.0-macos.zip"}]}"#;
        let asset = parse(body).unwrap().asset.unwrap();
        assert_eq!(asset.size, 42);
        assert_eq!(asset.sha256.as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        let evil = body.replace("https://github.com/eliasbui/app-card-english/releases/download", "https://evil.example");
        assert!(parse(&evil).unwrap().asset.is_none());
    }

    #[test]
    fn the_app_around_the_program_is_found() {
        let found = bundle_of(Path::new("/Applications/Card English.app/Contents/MacOS/card-english"));
        assert_eq!(found, Some(PathBuf::from("/Applications/Card English.app")));
        assert_eq!(bundle_of(Path::new("/x/target/release/card-english")), None);
        assert_eq!(bundle_of(Path::new("/x/Foo/Contents/MacOS/card-english")), None);
    }

    /// The whole swap from a zip made the way the release workflow makes it, without the network.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_checked_zip_replaces_the_app_and_a_bad_one_leaves_it_alone() {
        let root = std::env::temp_dir().join(format!("card-english-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let fake = |folder: &Path, version: &str| {
            let app = folder.join(APP);
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::copy("/usr/bin/true", app.join("Contents/MacOS/card-english")).unwrap();
            fs::write(app.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
<key>CFBundleExecutable</key><string>card-english</string><key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#)).unwrap();
            run("/usr/bin/codesign", [OsStr::new("--force"), OsStr::new("--sign"), OsStr::new("-"), app.as_os_str()]).unwrap();
            app
        };
        let installed = fake(&root.join("Applications"), "1.0.0");
        let new = fake(&root.join("built"), "2.0.0");
        let zip_of = |name: &str| {
            let zip = root.join(name);
            run("/usr/bin/ditto", [OsStr::new("-c"), OsStr::new("-k"), OsStr::new("--keepParent"), new.as_os_str(), zip.as_os_str()]).unwrap();
            let stage = root.join("Applications").join(STAGE);
            fs::create_dir_all(&stage).unwrap();
            let staged = stage.join("update.zip");
            fs::copy(&zip, &staged).unwrap();
            let asset = Asset { url: String::new(), size: fs::metadata(&zip).unwrap().len(), sha256: Some(sha256(&zip).unwrap()) };
            (staged, asset)
        };
        let version = || plist_value(&installed, "CFBundleShortVersionString").unwrap();

        let (zip, mut asset) = zip_of("a.zip");
        asset.sha256 = Some("0".repeat(64));
        assert!(put_in_place(&zip, &asset, "2.0.0", &installed).is_err(), "the checksum");
        let (zip, asset) = zip_of("b.zip");
        assert!(put_in_place(&zip, &asset, "3.0.0", &installed).is_err(), "the version");
        assert_eq!(version(), "1.0.0");

        let (zip, asset) = zip_of("c.zip");
        put_in_place(&zip, &asset, "2.0.0", &installed).unwrap();
        assert_eq!(version(), "2.0.0");
        assert!(!installed.with_file_name(OLD).exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
