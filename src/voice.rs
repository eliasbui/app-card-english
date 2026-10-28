//! Natural voices from Kokoro, run on the machine through sherpa-onnx: one model for English, another for
//! Chinese, each downloaded as an archive into the data folder when the learner asks for it.

use crate::net;
use sha2::{Digest, Sha256};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
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
/// One reading at a time: they share the WAV file, and two voices at once cannot be followed anyway.
static SPEAKING: Mutex<()> = Mutex::new(());

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

/// Removes a course's pack from the disk, letting go of it first if it is the one loaded.
pub fn delete(dir: &Path, course: &str) -> Result<(), String> {
    let (name, ..) = pack(course)?;
    let _not_downloading = DOWNLOADING.try_lock().map_err(|_| "Đang tải giọng đọc, chờ tải xong rồi xóa")?;
    let mut loaded = LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if loaded.as_ref().is_some_and(|(n, _)| *n == name) {
        *loaded = None;
    }
    match std::fs::remove_dir_all(folder(dir, name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Không xóa được giọng đọc: {e}")),
        _ => Ok(()),
    }
}

fn check(file: &Path, expected: &str) -> Result<(), String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file).map_err(|e| e.to_string())?, &mut hasher).map_err(|e| e.to_string())?;
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hex == expected { Ok(()) } else { Err("Gói giọng đọc tải về bị hỏng, hãy thử lại".into()) }
}

/// Unpacks a `.tar.bz2` here rather than with the system's tar, which on Windows 10 cannot read bzip2. Only files
/// and folders: links need rights Windows does not give, and `unpack_in` refuses paths that leave `into`.
fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Không giải nén được gói giọng đọc: {e}");
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

/// Reads `text` aloud with voice `sid` of the course's pack, at `speed` (1 is normal), and returns once it is heard.
pub fn speak(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<(), String> {
    speak_and_keep(dir, course, sid, text, speed).map(drop)
}

/// Reads `text` aloud like `speak` and gives back what was heard, its samples and their rate, for holding a
/// learner's voice against it.
pub fn speak_and_keep(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
    let _one_at_a_time = SPEAKING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (wav, samples, rate) = synthesize(dir, course, sid, text, speed)?;
    play(&wav)?;
    Ok((samples, rate))
}

/// A path sherpa-onnx can open. Its C API takes narrow strings, which on Windows lose letters such as those of a
/// Vietnamese user name, so there a path that is not ASCII goes in its short 8.3 form when the disk keeps one.
#[cfg(windows)]
fn narrow(path: &Path) -> String {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let text = path.to_string_lossy().into_owned();
    if text.is_ascii() {
        return text;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut short = vec![0u16; 1024];
    // SAFETY: `wide` ends in a zero and `short` holds as many units as it is said to.
    let n = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32)
    } as usize;
    if n == 0 || n >= short.len() {
        return text;
    }
    std::ffi::OsString::from_wide(&short[..n]).to_string_lossy().into_owned()
}

#[cfg(not(windows))]
fn narrow(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Writes the speech to a WAV file in the data folder and gives its path, with the samples and their rate.
pub(crate) fn synthesize(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<(PathBuf, Vec<f32>, u32), String> {
    let (name, ..) = pack(course)?;
    if !ready(dir, course) {
        return Err("Chưa tải giọng đọc Kokoro".into());
    }
    let mut loaded = LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if loaded.as_ref().is_none_or(|(n, _)| *n != name) {
        *loaded = None;
        let path = |f: &str| Some(narrow(&folder(dir, name).join(f)));
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
    let (wav, rate) = (dir.join("speech.wav"), audio.sample_rate().max(1) as u32);
    write_wav(&wav, audio.samples(), rate).map_err(|e| format!("Không ghi được file âm thanh: {e}"))?;
    Ok((wav, audio.samples().to_vec(), rate))
}

/// A 16-bit mono WAV of `samples` (from -1 to 1), written here because sherpa-onnx's own writer takes a narrow path.
fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    // The format: PCM, one channel, the rate, bytes a second, bytes a frame, bits a sample.
    out.extend_from_slice(b"WAVEfmt ");
    for (value, width) in [(16, 4), (1, 2), (1, 2), (rate, 4), (rate * 2, 4), (2, 2), (16, 2)] {
        out.extend_from_slice(&u32::to_le_bytes(value)[..width]);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out)
}

// shortcut: plays through the system's player, one file at a time; stream through cpal if the start is ever too slow.
fn play(wav: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = net::command("/usr/bin/afplay");
        c.arg(wav);
        c
    } else if cfg!(windows) {
        // The path goes in through the environment: arguments after -Command join the script's text, not $args.
        let mut c = net::command("powershell");
        c.args(["-NoProfile", "-Command", "(New-Object Media.SoundPlayer $env:CARD_ENGLISH_WAV).PlaySync()"]).env("CARD_ENGLISH_WAV", wav);
        c
    } else {
        let mut c = net::command("aplay");
        c.arg("-q").arg(wav);
        c
    };
    let status = cmd.stdin(Stdio::null()).status().map_err(|e| format!("Không phát được âm thanh: {e}"))?;
    if status.success() { Ok(()) } else { Err("Không phát được âm thanh".into()) }
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

    #[test]
    fn a_pack_unpacks_its_files_and_nothing_else_and_can_be_deleted() {
        let root = std::env::temp_dir().join(format!("card-english-voice-{}", std::process::id()));
        let models = root.join("models");
        std::fs::create_dir_all(&models).unwrap();
        let archive = models.join("pack.tar.bz2");
        {
            let mut tar = tar::Builder::new(bzip2::write::BzEncoder::new(std::fs::File::create(&archive).unwrap(), Default::default()));
            let name = PACKS[0].1;
            for f in FILES {
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o644);
                tar.append_data(&mut header, format!("{name}/{f}"), &b"abc"[..]).unwrap();
            }
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            tar.append_link(&mut link, format!("{name}/link"), "/etc/passwd").unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        unpack(&archive, &models).unwrap();
        assert!(ready(&root, "en"));
        assert!(!folder(&root, PACKS[0].1).join("link").exists(), "links are skipped");
        delete(&root, "en").unwrap();
        assert!(!ready(&root, "en") && !folder(&root, PACKS[0].1).exists());
        delete(&root, "en").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_wav_header_says_what_follows() {
        let path = std::env::temp_dir().join(format!("card-english-wav-{}.wav", std::process::id()));
        write_wav(&path, &[0.0, 1.0, -1.0, 2.0], 24_000).unwrap();
        let b = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(b.len(), 44 + 8);
        assert_eq!((&b[..4], &b[8..16], &b[36..40]), (&b"RIFF"[..], &b"WAVEfmt "[..], &b"data"[..]));
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 24_000);
        assert_eq!(i16::from_le_bytes([b[46], b[47]]), 32767);
        assert_eq!(i16::from_le_bytes([b[50], b[51]]), 32767, "clamped");
    }

    /// Reads a sentence of each course with the packs in the data folder. Run it by hand:
    /// `cargo test kokoro_reads -- --ignored`.
    #[test]
    #[ignore = "needs the downloaded packs"]
    fn kokoro_reads_english_and_chinese() {
        let dir = std::env::var_os("KOKORO_DIR").map(PathBuf::from).expect("KOKORO_DIR is the data folder");
        for (course, text) in [("en", "Hello, I am learning new words today."), ("zh", "你好，我在学习中文。")] {
            let (wav, ..) = synthesize(&dir, course, VOICES.iter().find(|v| v.0 == course).unwrap().1, text, 1.0).unwrap();
            assert!(std::fs::metadata(&wav).unwrap().len() > 10_000, "{course}");
            if std::env::var_os("KOKORO_PLAY").is_some() {
                play(&wav).unwrap();
            }
        }
    }
}
