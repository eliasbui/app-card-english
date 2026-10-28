//! The model files, downloaded once into the data folder instead of shipped with the app.

use crate::net;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Two downloads into the same `.part` file would append to each other and corrupt the model.
static DOWNLOADING: Mutex<()> = Mutex::new(());

pub const QWEN: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
/// The larger Qwen, slower but better at Vietnamese; it reads with the same tokenizer as the small one.
pub const QWEN15: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
pub const QWEN_TOKENIZER: &str = "qwen2.5-tokenizer.json";
pub const WHISPER: &str = "whisper-tiny-en-q80.gguf";
pub const WHISPER_CONFIG: &str = "whisper-tiny-en-config.json";
pub const WHISPER_TOKENIZER: &str = "whisper-tiny-en-tokenizer.json";
/// The multilingual Whisper, which hears Chinese.
pub const WHISPER_ZH: &str = "whisper-tiny-q80.gguf";
pub const WHISPER_ZH_CONFIG: &str = "whisper-tiny-config.json";
pub const WHISPER_ZH_TOKENIZER: &str = "whisper-tiny-tokenizer.json";

/// Group, file name, source and size in bytes (for the progress bar only).
// shortcut: files are checked by size only, not by hash; add SHA-256 pins if a mirror is ever used.
const FILES: [(&str, &str, &str, u64); 10] = [
    ("llm", QWEN, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf", 491_400_032),
    ("llm", QWEN_TOKENIZER, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json", 7_031_645),
    ("llm15", QWEN15, "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf", 1_117_320_736),
    // The very same file as the 0.5B one's (same ETag on Hugging Face), shared by both groups.
    ("llm15", QWEN_TOKENIZER, "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct/resolve/main/tokenizer.json", 7_031_645),
    ("stt", WHISPER, "https://huggingface.co/lmz/candle-whisper/resolve/main/model-tiny-en-q80.gguf", 41_841_216),
    ("stt", WHISPER_CONFIG, "https://huggingface.co/lmz/candle-whisper/resolve/main/config-tiny-en.json", 1_937),
    ("stt", WHISPER_TOKENIZER, "https://huggingface.co/lmz/candle-whisper/resolve/main/tokenizer-tiny-en.json", 2_405_679),
    ("sttzh", WHISPER_ZH, "https://huggingface.co/lmz/candle-whisper/resolve/main/model-tiny-q80.gguf", 41_841_632),
    ("sttzh", WHISPER_ZH_CONFIG, "https://huggingface.co/lmz/candle-whisper/resolve/main/config-tiny.json", 1_983),
    ("sttzh", WHISPER_ZH_TOKENIZER, "https://huggingface.co/lmz/candle-whisper/resolve/main/tokenizer-tiny.json", 2_480_452),
];

pub fn path(dir: &Path, file: &str) -> PathBuf {
    dir.join("models").join(file)
}

/// The file is there and has the size of the source: a longer or shorter one is a broken download.
fn intact(dir: &Path, name: &str, size: u64) -> bool {
    std::fs::metadata(path(dir, name)).is_ok_and(|m| m.is_file() && m.len() == size)
}

pub fn ready(dir: &Path, group: &str) -> bool {
    FILES
        .iter()
        .filter(|f| f.0 == group)
        .all(|f| intact(dir, f.1, f.3))
}

pub fn status(dir: &Path) -> Value {
    let size = |group: &str| FILES.iter().filter(|f| f.0 == group).map(|f| f.3).sum::<u64>();
    json!({
        "llm": { "ready": ready(dir, "llm"), "size": size("llm") },
        "llm15": { "ready": ready(dir, "llm15"), "size": size("llm15") },
        "stt": { "ready": ready(dir, "stt"), "size": size("stt") },
        "sttzh": { "ready": ready(dir, "sttzh"), "size": size("sttzh") },
    })
}

/// Removes the files of a group, but not one that another downloaded group still reads. The models in memory are let
/// go first: Windows does not delete a file that is open.
pub fn delete(dir: &Path, group: &str) -> Result<(), String> {
    let files: Vec<_> = FILES.iter().filter(|f| f.0 == group).collect();
    if files.is_empty() {
        return Err("Không có model này".into());
    }
    let _no_download = DOWNLOADING.try_lock().map_err(|_| "Đang tải model, hãy chờ tải xong rồi xóa")?;
    crate::ai::unload();
    crate::speech::unload(group);
    for (_, name, ..) in files {
        let shared = FILES.iter().any(|f| f.0 != group && f.1 == *name && ready(dir, f.0));
        if shared {
            continue;
        }
        let to = path(dir, name);
        let _ = std::fs::remove_file(to.with_extension("part"));
        if let Err(e) = std::fs::remove_file(&to)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!("Không xóa được {name}: {e}"));
        }
    }
    Ok(())
}

/// Downloads the missing files of a group, reporting the bytes done out of the total.
pub fn download(dir: &Path, group: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("models")).map_err(|e| e.to_string())?;
    let files: Vec<_> = FILES.iter().filter(|f| f.0 == group).collect();
    if files.is_empty() {
        return Err("Không có model này".into());
    }
    let _one_at_a_time = DOWNLOADING.lock().map_err(|_| "Đang tải model bị lỗi")?;
    let total: u64 = files.iter().map(|f| f.3).sum();
    let mut done = 0;
    for (_, name, url, size) in files {
        let to = path(dir, name);
        if !intact(dir, name, *size) {
            let _ = std::fs::remove_file(&to);
            let _ = std::fs::remove_file(to.with_extension("part"));
            net::download(url, &to, |bytes| progress(done + bytes.min(*size), total))?;
            if !intact(dir, name, *size) {
                let _ = std::fs::remove_file(&to);
                return Err(format!("File {name} tải về bị lỗi, hãy thử lại"));
            }
        }
        done += size;
        progress(done, total);
    }
    Ok(())
}

/// Downloads the `.tar.bz2` at `url` into the models folder, checks it against its pinned SHA-256 and unpacks it
/// there. The archive goes either way; the caller holds the lock of its kind of download.
pub(crate) fn fetch_archive(dir: &Path, url: &str, size: u64, sha256: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let models = dir.join("models");
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let archive = models.join(url.rsplit('/').next().unwrap_or("pack.tar.bz2"));
    net::download(url, &archive, |bytes| progress(bytes.min(size), size))?;
    let unpacked = check(&archive, sha256).and_then(|()| unpack(&archive, &models));
    let _ = std::fs::remove_file(&archive);
    unpacked
}

fn check(file: &Path, expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file).map_err(|e| e.to_string())?, &mut hasher).map_err(|e| e.to_string())?;
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hex == expected { Ok(()) } else { Err("Gói tải về bị hỏng, hãy thử lại".into()) }
}

/// Unpacks a `.tar.bz2` here rather than with the system's tar, which on Windows 10 cannot read bzip2. Only files
/// and folders: links need rights Windows does not give, and `unpack_in` refuses paths that leave `into`.
pub(crate) fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Không giải nén được gói tải về: {e}");
    let file = std::fs::File::open(archive).map_err(err)?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(file)));
    for entry in tar.entries().map_err(err)? {
        let mut entry = entry.map_err(err)?;
        let kind = entry.header().entry_type();
        if kind.is_file() || kind.is_dir() {
            entry.unpack_in(into).map_err(err)?;
        }
    }
    Ok(())
}

/// Files of the right size, standing in for the downloads of a group in a test.
#[cfg(test)]
pub fn fake_download(dir: &Path, group: &str) {
    std::fs::create_dir_all(dir.join("models")).unwrap();
    for (_, name, _, size) in FILES.iter().filter(|f| f.0 == group) {
        std::fs::File::create(path(dir, name)).unwrap().set_len(*size).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_model_keeps_the_tokenizer_the_other_one_reads() {
        let dir = std::env::temp_dir().join(format!("card-english-model-test-{}", std::process::id()));
        fake_download(&dir, "llm");
        fake_download(&dir, "llm15");
        assert!(ready(&dir, "llm") && ready(&dir, "llm15"));
        delete(&dir, "llm15").unwrap();
        assert!(ready(&dir, "llm") && !ready(&dir, "llm15"));
        delete(&dir, "llm").unwrap();
        assert!(!path(&dir, QWEN_TOKENIZER).exists() && !path(&dir, QWEN).exists());
        delete(&dir, "llm").unwrap();
        assert!(delete(&dir, "nope").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
