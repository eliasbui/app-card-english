//! The model files, downloaded once into the data folder instead of shipped with the app.

use crate::net;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const QWEN: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
pub const QWEN_TOKENIZER: &str = "qwen2.5-tokenizer.json";
pub const WHISPER: &str = "whisper-tiny-en-q80.gguf";
pub const WHISPER_CONFIG: &str = "whisper-tiny-en-config.json";
pub const WHISPER_TOKENIZER: &str = "whisper-tiny-en-tokenizer.json";

/// Group, file name, source and size in bytes (for the progress bar only).
// shortcut: files are not checked against a hash, HTTPS is trusted; add SHA-256 pins if a mirror is ever used.
const FILES: [(&str, &str, &str, u64); 5] = [
    ("llm", QWEN, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf", 491_400_032),
    ("llm", QWEN_TOKENIZER, "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json", 7_031_645),
    ("stt", WHISPER, "https://huggingface.co/lmz/candle-whisper/resolve/main/model-tiny-en-q80.gguf", 41_841_216),
    ("stt", WHISPER_CONFIG, "https://huggingface.co/lmz/candle-whisper/resolve/main/config-tiny-en.json", 1_937),
    ("stt", WHISPER_TOKENIZER, "https://huggingface.co/lmz/candle-whisper/resolve/main/tokenizer-tiny-en.json", 2_405_679),
];

pub fn path(dir: &Path, file: &str) -> PathBuf {
    dir.join("models").join(file)
}

pub fn ready(dir: &Path, group: &str) -> bool {
    FILES
        .iter()
        .filter(|f| f.0 == group)
        .all(|f| path(dir, f.1).is_file())
}

pub fn status(dir: &Path) -> Value {
    let size = |group: &str| FILES.iter().filter(|f| f.0 == group).map(|f| f.3).sum::<u64>();
    json!({
        "llm": { "ready": ready(dir, "llm"), "size": size("llm") },
        "stt": { "ready": ready(dir, "stt"), "size": size("stt") },
    })
}

/// Downloads the missing files of a group, reporting the bytes done out of the total.
pub fn download(dir: &Path, group: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("models")).map_err(|e| e.to_string())?;
    let files: Vec<_> = FILES.iter().filter(|f| f.0 == group).collect();
    if files.is_empty() {
        return Err("Không có model này".into());
    }
    let total: u64 = files.iter().map(|f| f.3).sum();
    let mut done = 0;
    for (_, name, url, size) in files {
        let to = path(dir, name);
        if !to.is_file() {
            net::download(url, &to, |bytes| progress(done + bytes.min(*size), total))?;
        }
        done += size;
        progress(done, total);
    }
    Ok(())
}
