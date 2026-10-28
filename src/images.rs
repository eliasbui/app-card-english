//! Pictures on the cards: files in `images` of the data folder, named by the SHA-256 of what is in them, which a
//! word refers to by that name. The page sends them already scaled down and gets them back as data: URLs.

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The page scales a picture to 512 px on its long side, which is far smaller than this.
pub const MAX_BYTES: usize = 1 << 20;

pub fn folder(dir: &Path) -> PathBuf {
    dir.join("images")
}

/// The type of a picture from its first bytes: WebP, JPEG or PNG, as its file's extension.
fn kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else {
        None
    }
}

/// Whether `name` is one that `name_of` makes, and so a file in the images folder and nowhere else.
pub fn valid(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else { return false };
    stem.len() == 64 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) && matches!(ext, "webp" | "jpg" | "png")
}

/// The name a picture is kept under: the hash of its bytes and the extension of its type.
pub fn name_of(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Ảnh quá lớn, hãy chọn ảnh khác nhỏ hơn 1 MB".into());
    }
    let ext = kind(bytes).ok_or("Chỉ dùng được ảnh WebP, JPEG hoặc PNG, hãy chọn ảnh khác")?;
    let hex: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{hex}.{ext}"))
}

/// The bytes of what the page sent: a data: URL or plain base64.
pub fn decode(data: &str) -> Result<Vec<u8>, String> {
    let base64 = if data.starts_with("data:") { data.split_once(',').map_or("", |(_, b)| b) } else { data };
    if base64.len() > MAX_BYTES / 3 * 4 + 4 {
        return Err("Ảnh quá lớn, hãy chọn ảnh khác nhỏ hơn 1 MB".into());
    }
    STANDARD.decode(base64.trim()).map_err(|_| "Không đọc được ảnh này, hãy chọn ảnh khác".into())
}

/// Keeps a picture and gives its name; the same picture twice is one file.
pub fn store(dir: &Path, bytes: &[u8]) -> Result<String, String> {
    let name = name_of(bytes)?;
    let path = folder(dir).join(&name);
    if !path.is_file() {
        let fail = |e: std::io::Error| format!("Không lưu được ảnh: {e}");
        fs::create_dir_all(folder(dir)).map_err(fail)?;
        let part = path.with_extension("part");
        fs::write(&part, bytes).map_err(fail)?;
        fs::rename(&part, &path).map_err(fail)?;
    }
    Ok(name)
}

pub fn exists(dir: &Path, name: &str) -> bool {
    valid(name) && folder(dir).join(name).is_file()
}

/// A picture as the page shows it, whose CSP takes images from data: URLs only.
pub fn data_url(dir: &Path, name: &str) -> Result<String, String> {
    if !valid(name) {
        return Err("Tên ảnh không hợp lệ".into());
    }
    let bytes = fs::read(folder(dir).join(name)).map_err(|_| "Không tìm thấy ảnh này trên máy".to_owned())?;
    let mime = match name.rsplit('.').next() {
        Some("jpg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "image/webp",
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

/// Removes the files of `names` that no word uses any more. Called once the database has changed, so a picture a
/// word still shows always stays; only valid names, which are files of the images folder, are ever removed.
pub fn release(db: &Connection, dir: &Path, names: &[String]) {
    for name in names.iter().filter(|n| valid(n)) {
        let used = db.query_row("SELECT 1 FROM words WHERE image = ?1 LIMIT 1", [name], |_| Ok(())).optional();
        if matches!(used, Ok(None)) {
            let _ = fs::remove_file(folder(dir).join(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    const WEBP: &[u8] = b"RIFF\x1a\x00\x00\x00WEBPVP8L\x0d\x00\x00\x00\x2f\x00\x00\x00\x10\x07\x10\x11\x11\x88\x88\xfe\x07\x00";

    #[test]
    fn pictures_are_named_by_content_and_checked_by_their_bytes() {
        let name = name_of(WEBP).unwrap();
        assert!(valid(&name) && name.ends_with(".webp"), "{name}");
        assert_eq!(name_of(b"\xff\xd8\xff\xe0 jpeg").unwrap().rsplit('.').next(), Some("jpg"));
        assert!(name_of(b"<svg onload=alert(1)>").is_err());
        assert!(name_of(&vec![0xff; MAX_BYTES + 1]).is_err());
        for bad in ["../words.db", "a.webp", &format!("{}.exe", "a".repeat(64)), &format!("{}.webp", "A".repeat(64))] {
            assert!(!valid(bad), "{bad}");
        }
        let url = format!("data:image/webp;base64,{}", STANDARD.encode(WEBP));
        assert_eq!(decode(&url).unwrap(), WEBP);
        assert_eq!(decode(&STANDARD.encode(WEBP)).unwrap(), WEBP);
        assert!(decode("data:image/webp;base64,%%%").is_err());
    }

    #[test]
    fn a_file_goes_only_when_no_word_shows_it() {
        let dir = std::env::temp_dir().join(format!("card-english-images-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let db = db::open(Path::new(":memory:")).unwrap();
        let name = store(&dir, WEBP).unwrap();
        assert_eq!(store(&dir, WEBP).unwrap(), name, "the same picture is one file");
        assert!(data_url(&dir, &name).unwrap().starts_with("data:image/webp;base64,UklGR"));
        let a = db::save(&db, &db::Word { word: "apple".into(), ..db::Word::default() }, 1).unwrap();
        let b = db::save(&db, &db::Word { word: "pear".into(), ..db::Word::default() }, 1).unwrap();
        db::set_image(&db, a, &name).unwrap();
        db::set_image(&db, b, &name).unwrap();
        assert_eq!(db::set_image(&db, a, "").unwrap(), Some(name.clone()));
        release(&db, &dir, std::slice::from_ref(&name));
        assert!(exists(&dir, &name), "the pear still shows it");
        db::delete(&db, b).unwrap();
        release(&db, &dir, &[name.clone(), "../words.db".into()]);
        assert!(!exists(&dir, &name));
        assert!(data_url(&dir, "../words.db").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
