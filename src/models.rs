//! The model files, downloaded once into the data folder instead of shipped with the app.

use crate::net;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Two downloads into the same `.part` file would append to each other and corrupt the model.
static DOWNLOADING: Mutex<()> = Mutex::new(());

pub const QWEN: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
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
const FILES: [(&str, &str, &str, u64); 8] = [
    ("llm", QWEN, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf", 491_400_032),
    ("llm", QWEN_TOKENIZER, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json", 7_031_645),
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
        "stt": { "ready": ready(dir, "stt"), "size": size("stt") },
        "sttzh": { "ready": ready(dir, "sttzh"), "size": size("sttzh") },
    })
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
