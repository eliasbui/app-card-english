//! Natural voices from Kokoro, run on the machine through sherpa-onnx: one model for English, another for
//! Chinese, each downloaded as an archive into the data folder when the learner asks for it.

use crate::net;
use sha2::{Digest, Sha256};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Course, folder (the archive's name without `.tar.bz2`), size and SHA-256 of the archive.
const PACKS: [(&str, &str, u64, &str); 2] = [
    // v1.0 has the English voices (Heart, Michael, Emma...); v1.1 is the one trained on Chinese.
    ("en", "kokoro-int8-multi-lang-v1_0", 132_353_410, "72c347230107ef8a31098ec4d468a605244beef5f1ed00db97efa13518800653"),
    ("zh", "kokoro-int8-multi-lang-v1_1", 147_052_523, "259c9c8bc906d6078e4e0570421885dabb68b539c2f970fcbb890a58eeaf3e9c"),
];
const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models";
/// The files a pack needs once unpacked.
const FILES: [&str; 5] = ["model.int8.onnx", "voices.bin", "tokens.txt", "lexicon-us-en.txt", "lexicon-zh.txt"];

/// Voice, speaker id in its pack and the name the settings show, per course.
pub const VOICES: [(&str, i32, &str); 12] = [
    ("en", 3, "Heart, nữ, Anh-Mỹ"),
    ("en", 2, "Bella, nữ, Anh-Mỹ"),
    ("en", 6, "Nicole, nữ, Anh-Mỹ"),
    ("en", 16, "Michael, nam, Anh-Mỹ"),
    ("en", 21, "Emma, nữ, Anh-Anh"),
    ("en", 26, "George, nam, Anh-Anh"),
    ("zh", 3, "Nữ 1"),
    ("zh", 5, "Nữ 2"),
    ("zh", 9, "Nữ 3"),
    ("zh", 58, "Nam 1"),
    ("zh", 60, "Nam 2"),
    ("zh", 64, "Nam 3"),
];

/// The model loaded last, kept because loading takes a second and a session reads many words.
static LOADED: Mutex<Option<(&str, OfflineTts)>> = Mutex::new(None);
/// One download of a pack at a time.
static DOWNLOADING: Mutex<()> = Mutex::new(());

fn pack(course: &str) -> Result<(&'static str, u64, &'static str), String> {
    PACKS.iter().find(|p| p.0 == course).map(|p| (p.1, p.2, p.3)).ok_or_else(|| "Không có giọng cho khóa này".into())
}

fn folder(dir: &Path, name: &str) -> PathBuf {
    dir.join("models").join(name)
}

pub fn ready(dir: &Path, course: &str) -> bool {
    pack(course).is_ok_and(|(name, ..)| FILES.iter().all(|f| folder(dir, name).join(f).is_file()))
}

/// For the settings page: per course, whether the voice is there and how big its download is.
pub fn status(dir: &Path) -> serde_json::Value {
    let one = |course: &str| serde_json::json!({ "ready": ready(dir, course), "size": pack(course).map_or(0, |p| p.1) });
    serde_json::json!({ "en": one("en"), "zh": one("zh") })
}

/// Downloads a course's pack, checks it against its pinned SHA-256 and unpacks it into the models folder.
pub fn download(dir: &Path, course: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let (name, size, sha256) = pack(course)?;
    let _one_at_a_time = DOWNLOADING.lock().map_err(|_| "Đang tải giọng bị lỗi")?;
    if ready(dir, course) {
        return Ok(());
    }
    let models = dir.join("models");
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let archive = models.join(format!("{name}.tar.bz2"));
    net::download(&format!("{RELEASE}/{name}.tar.bz2"), &archive, |bytes| progress(bytes.min(size), size))?;
    let unpacked = check(&archive, sha256).and_then(|()| unpack(&archive, &models));
    let _ = std::fs::remove_file(&archive);
    unpacked?;
    if !ready(dir, course) {
        return Err("Gói giọng đọc tải về thiếu file, hãy thử lại".into());
    }
    progress(size, size);
    Ok(())
}

fn check(file: &Path, expected: &str) -> Result<(), String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file).map_err(|e| e.to_string())?, &mut hasher).map_err(|e| e.to_string())?;
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hex == expected { Ok(()) } else { Err("Gói giọng đọc tải về bị hỏng, hãy thử lại".into()) }
}

/// The system's tar, which reads bzip2 on macOS, Linux and Windows 10 and later.
fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let tar = if cfg!(windows) {
        PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\tar.exe")
    } else {
        PathBuf::from("tar")
    };
    let status = net::command(tar).arg("-xjf").arg(archive).arg("-C").arg(into).status().map_err(|e| format!("Không chạy được tar: {e}"))?;
    if status.success() { Ok(()) } else { Err("Không giải nén được gói giọng đọc".into()) }
}

/// Reads `text` aloud with voice `sid` of the course's pack, at `speed` (1 is normal).
pub fn speak(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<(), String> {
    let wav = synthesize(dir, course, sid, text, speed)?;
    play(&wav)
}

/// Writes the speech to a WAV file in the data folder and gives its path.
fn synthesize(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<PathBuf, String> {
    let (name, ..) = pack(course)?;
    if !ready(dir, course) {
        return Err("Chưa tải giọng đọc Kokoro".into());
    }
    let mut loaded = LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if loaded.as_ref().is_none_or(|(n, _)| *n != name) {
        *loaded = None;
        let path = |f: &str| Some(folder(dir, name).join(f).to_string_lossy().into_owned());
        let lexicon = [path("lexicon-us-en.txt"), path("lexicon-zh.txt")].into_iter().flatten().collect::<Vec<_>>().join(",");
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: path("model.int8.onnx"),
                    voices: path("voices.bin"),
                    tokens: path("tokens.txt"),
                    dict_dir: path("dict"),
                    lexicon: Some(lexicon),
                    length_scale: 1.0,
                    ..Default::default()
                },
                num_threads: 2,
                ..Default::default()
            },
            // How numbers, dates and phone numbers in Chinese text are read.
            rule_fsts: Some(["date-zh.fst", "phone-zh.fst", "number-zh.fst"].iter().filter_map(|f| path(f)).collect::<Vec<_>>().join(",")),
            ..Default::default()
        };
        let tts = OfflineTts::create(&config).ok_or("Không mở được giọng đọc Kokoro")?;
        *loaded = Some((name, tts));
    }
    let (_, tts) = loaded.as_ref().ok_or("Không mở được giọng đọc Kokoro")?;
    let config = GenerationConfig { sid, speed: speed.clamp(0.5, 1.5), ..Default::default() };
    let audio = tts.generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>).ok_or("Kokoro không đọc được câu này")?;
    let wav = dir.join("speech.wav");
    if !audio.save(&wav.to_string_lossy()) {
        return Err("Không ghi được file âm thanh".into());
    }
    Ok(wav)
}

// shortcut: plays through the system's player, one file at a time; stream through cpal if the start is ever too slow.
fn play(wav: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = net::command("/usr/bin/afplay");
        c.arg(wav);
        c
    } else if cfg!(windows) {
        let mut c = net::command("powershell");
        c.args(["-NoProfile", "-Command", "(New-Object Media.SoundPlayer $args[0]).PlaySync()"]).arg(wav);
        c
    } else {
        let mut c = net::command("aplay");
        c.arg("-q").arg(wav);
        c
    };
    cmd.status().map(drop).map_err(|e| format!("Không phát được âm thanh: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_voice_belongs_to_a_pack() {
        for (course, sid, _) in VOICES {
            assert!(pack(course).is_ok());
            assert!((0..103).contains(&sid));
        }
        assert!(pack("fr").is_err());
    }

    /// Reads a sentence of each course with the packs in the data folder. Run it by hand:
    /// `cargo test kokoro_reads -- --ignored`.
    #[test]
    #[ignore = "needs the downloaded packs"]
    fn kokoro_reads_english_and_chinese() {
        let dir = std::env::var_os("KOKORO_DIR").map(PathBuf::from).expect("KOKORO_DIR is the data folder");
        for (course, text) in [("en", "Hello, I am learning new words today."), ("zh", "你好，我在学习中文。")] {
            let wav = synthesize(&dir, course, VOICES.iter().find(|v| v.0 == course).unwrap().1, text, 1.0).unwrap();
            assert!(std::fs::metadata(&wav).unwrap().len() > 10_000, "{course}");
            if std::env::var_os("KOKORO_PLAY").is_some() {
                play(&wav).unwrap();
            }
        }
    }
}
