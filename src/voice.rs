//! Voices run on the machine through sherpa-onnx, each pack downloaded as an archive into the data folder when the
//! learner asks for it: Kokoro, the most natural, for English and for Chinese; Supertonic, several times faster and
//! the one that reads Vietnamese; Piper, a tiny Chinese voice for weak machines. Speech plays in the app through
//! cpal, and the clips read last are kept, so that hearing a word again starts at once.

use crate::{
    hardware::{self, Tier},
    models, net, speech,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::{Value, json};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig,
    OfflineTtsVitsModelConfig,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Engine {
    Kokoro,
    Supertonic,
    Piper,
}

/// A pack of voices and the archive it comes in.
#[derive(Debug)]
pub struct Pack {
    /// What the page and the downloads call it.
    pub id: &'static str,
    /// What a voice setting starts with, as "kokoro" in "kokoro:3", shared by the Kokoro of both courses.
    family: &'static str,
    /// The archive's name without `.tar.bz2`, which is also the folder it unpacks to.
    folder: &'static str,
    size: u64,
    sha256: &'static str,
    engine: Engine,
    langs: &'static [&'static str],
    /// The model file, then the other files it needs.
    model: &'static str,
    files: &'static [&'static str],
    /// The folder of the int8 pack of earlier versions, read until this one is downloaded. It is two and a half times
    /// slower, since ONNX Runtime runs Kokoro's int8 convolutions slowly.
    old: Option<&'static str>,
}

const KOKORO_FILES: &[&str] = &["voices.bin", "tokens.txt", "lexicon-us-en.txt", "lexicon-zh.txt"];
const OLD_MODEL: &str = "model.int8.onnx";

pub const PACKS: [Pack; 4] = [
    // v1.0 has the English voices (Heart, Michael, Emma...); v1.1 is the one trained on Chinese.
    Pack {
        id: "kokoro-en",
        family: "kokoro",
        folder: "kokoro-multi-lang-v1_0",
        size: 349_886_476,
        sha256: "60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4",
        engine: Engine::Kokoro,
        langs: &["en"],
        model: "model.onnx",
        files: KOKORO_FILES,
        old: Some("kokoro-int8-multi-lang-v1_0"),
    },
    Pack {
        id: "kokoro-zh",
        family: "kokoro",
        folder: "kokoro-multi-lang-v1_1",
        size: 364_855_331,
        sha256: "b78e1ac42b852e839cceaafbe88f66806bf5860992d1d7b9a3f2ad1198427aed",
        engine: Engine::Kokoro,
        langs: &["zh"],
        model: "model.onnx",
        files: KOKORO_FILES,
        old: Some("kokoro-int8-multi-lang-v1_1"),
    },
    Pack {
        id: "supertonic",
        family: "supertonic",
        folder: "sherpa-onnx-supertonic-3-tts-int8-2026-05-11",
        size: 128_774_318,
        sha256: "82fa96f91c4ef8abaae3a14a3f4153facf88bed821d1f7331cec2700f432c427",
        engine: Engine::Supertonic,
        langs: &["en", "vi"],
        model: "vector_estimator.int8.onnx",
        files: &["duration_predictor.int8.onnx", "text_encoder.int8.onnx", "vocoder.int8.onnx", "tts.json", "unicode_indexer.bin", "voice.bin"],
        old: None,
    },
    Pack {
        id: "piper-zh",
        family: "piper",
        folder: "vits-piper-zh_CN-xiao_ya-medium-int8",
        size: 14_016_124,
        sha256: "eab027e194e70289233cf12308373611fa4e2e96ef2d97354ef433ab663d831f",
        engine: Engine::Piper,
        langs: &["zh"],
        model: "zh_CN-xiao_ya-medium.onnx",
        files: &["tokens.txt", "lexicon.txt"],
        old: None,
    },
];
const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models";

/// Pack, speaker id in it and the name the settings show. The Kokoro ones are those its makers grade best.
pub const VOICES: [(&str, i32, &str); 31] = [
    ("kokoro-en", 3, "Heart, nữ, Anh-Mỹ"),
    ("kokoro-en", 2, "Bella, nữ, Anh-Mỹ"),
    ("kokoro-en", 6, "Nicole, nữ, Anh-Mỹ"),
    ("kokoro-en", 1, "Aoede, nữ, Anh-Mỹ"),
    ("kokoro-en", 5, "Kore, nữ, Anh-Mỹ"),
    ("kokoro-en", 9, "Sarah, nữ, Anh-Mỹ"),
    ("kokoro-en", 7, "Nova, nữ, Anh-Mỹ"),
    ("kokoro-en", 16, "Michael, nam, Anh-Mỹ"),
    ("kokoro-en", 14, "Fenrir, nam, Anh-Mỹ"),
    ("kokoro-en", 18, "Puck, nam, Anh-Mỹ"),
    ("kokoro-en", 21, "Emma, nữ, Anh-Anh"),
    ("kokoro-en", 22, "Isabella, nữ, Anh-Anh"),
    ("kokoro-en", 26, "George, nam, Anh-Anh"),
    ("kokoro-en", 25, "Fable, nam, Anh-Anh"),
    ("kokoro-zh", 3, "Nữ 1"),
    ("kokoro-zh", 5, "Nữ 2"),
    ("kokoro-zh", 9, "Nữ 3"),
    ("kokoro-zh", 58, "Nam 1"),
    ("kokoro-zh", 60, "Nam 2"),
    ("kokoro-zh", 64, "Nam 3"),
    ("supertonic", 0, "Nữ 1"),
    ("supertonic", 1, "Nữ 2"),
    ("supertonic", 2, "Nữ 3"),
    ("supertonic", 3, "Nữ 4"),
    ("supertonic", 4, "Nữ 5"),
    ("supertonic", 5, "Nam 1"),
    ("supertonic", 6, "Nam 2"),
    ("supertonic", 7, "Nam 3"),
    ("supertonic", 8, "Nam 4"),
    ("supertonic", 9, "Nam 5"),
    ("piper-zh", 0, "Xiao Ya, nữ"),
];

/// The answer when Kokoro would leave out a word it has no pronunciation for and no other voice of the app is
/// there to read it: the caller reads it with the system's voice instead.
pub const UNKNOWN_WORD: &str = "Giọng này không có cách đọc một từ trong câu";

/// The packs of a language in the order a machine of this tier wants them: the light one first, unless it is strong
/// enough for the most natural one to answer within a second.
fn preferred(lang: &str, tier: Tier) -> &'static [&'static str] {
    match (lang, tier) {
        ("en", Tier::Strong) => &["kokoro-en", "supertonic"],
        ("en", _) => &["supertonic", "kokoro-en"],
        ("zh", Tier::Strong) => &["kokoro-zh", "piper-zh"],
        ("zh", _) => &["piper-zh", "kokoro-zh"],
        _ => &["supertonic"],
    }
}

/// A voice of a pack, reading one language.
#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pack: &'static Pack,
    pub sid: i32,
    lang: &'static str,
}

impl Voice {
    /// Kokoro's British voices, which read with the British lexicon.
    fn british(&self) -> bool {
        self.pack.id == "kokoro-en" && (20..=27).contains(&self.sid)
    }

    fn male(&self) -> bool {
        VOICES.iter().any(|v| v.0 == self.pack.id && v.1 == self.sid && v.2.to_lowercase().contains("nam"))
    }
}

fn lang_of(lang: &str) -> &'static str {
    match lang {
        "zh" => "zh",
        "vi" => "vi",
        _ => "en",
    }
}

fn pack_by_id(id: &str) -> Result<&'static Pack, String> {
    PACKS.iter().find(|p| p.id == id).ok_or_else(|| "Không có gói giọng này".into())
}

/// Where a pack's files are and which model file to open: its own folder once complete, else the older int8 one.
fn location(dir: &Path, pack: &Pack) -> Option<(PathBuf, &'static str)> {
    let complete = |folder: &Path, model: &str| folder.join(model).is_file() && pack.files.iter().all(|f| folder.join(f).is_file());
    let own = models::path(dir, pack.folder);
    if complete(&own, pack.model) {
        return Some((own, pack.model));
    }
    let old = models::path(dir, pack.old?);
    complete(&old, OLD_MODEL).then_some((old, OLD_MODEL))
}

fn ready(dir: &Path, pack: &Pack) -> bool {
    location(dir, pack).is_some()
}

/// The voice a setting such as "kokoro:3" names for `lang`, when its pack is there.
pub fn chosen(dir: &Path, setting: &str, lang: &str) -> Option<Voice> {
    let (family, sid) = setting.split_once(':')?;
    let (sid, lang) = (sid.parse().ok()?, lang_of(lang));
    let pack = PACKS.iter().find(|p| p.family == family && p.langs.contains(&lang))?;
    ready(dir, pack).then_some(Voice { pack, sid, lang })
}

/// The voice chosen, else the first voice of the pack this machine is best served by among those downloaded.
pub fn for_lang(dir: &Path, setting: &str, lang: &str) -> Option<Voice> {
    chosen(dir, setting, lang).or_else(|| {
        let lang = lang_of(lang);
        let pack = preferred(lang, hardware::machine().tier).iter().filter_map(|id| pack_by_id(id).ok()).find(|p| ready(dir, p))?;
        VOICES.iter().find(|v| v.0 == pack.id).map(|v| Voice { pack, sid: v.1, lang })
    })
}

/// For a recording with several speakers: `voice` when it is of the sex asked for ("M" or "W"), else the first voice
/// of its pack that is.
pub fn for_speaker(voice: Voice, who: &str) -> Voice {
    let want = match who {
        "M" => true,
        "W" => false,
        _ => return voice,
    };
    if voice.male() == want {
        return voice;
    }
    VOICES.iter().filter(|v| v.0 == voice.pack.id).map(|v| Voice { sid: v.1, ..voice }).find(|v| v.male() == want).unwrap_or(voice)
}

/// The models in memory, the one used last at the end: two, so that words read in one voice and their meaning in
/// another do not wait for a load each time.
static LOADED: Mutex<Vec<(String, OfflineTts)>> = Mutex::new(Vec::new());
/// The clips read last, by voice, speed and text, the newest at the back.
static CLIPS: Mutex<VecDeque<(String, Arc<Vec<f32>>, u32)>> = Mutex::new(VecDeque::new());
const CLIPS_KEPT: usize = 64;
/// How long the last word took to make in each pack, in milliseconds, for the settings.
static TIMES: Mutex<Vec<(&str, u128)>> = Mutex::new(Vec::new());
/// The words of each Kokoro lexicon, read once, for finding those it would leave out.
static LEXICONS: Mutex<Vec<(PathBuf, Arc<HashSet<String>>)>> = Mutex::new(Vec::new());
/// One download of a pack at a time.
static DOWNLOADING: Mutex<()> = Mutex::new(());
/// One reading at a time: two voices at once cannot be followed.
static SPEAKING: Mutex<()> = Mutex::new(());

/// For the settings page: the machine, each pack with its size, whether it is there (or only its slower int8
/// version) and for which languages it is the suggestion here, how long a word took, and the voices.
pub fn status(dir: &Path) -> Value {
    let machine = hardware::machine();
    let times = TIMES.lock().map(|t| t.clone()).unwrap_or_default();
    let packs: Vec<Value> = PACKS
        .iter()
        .map(|p| {
            let at = location(dir, p);
            json!({
                "id": p.id,
                "family": p.family,
                "langs": p.langs,
                "size": p.size,
                "ready": at.is_some(),
                "old": at.as_ref().is_some_and(|(_, model)| *model == OLD_MODEL),
                "suggested": p.langs.iter().filter(|l| preferred(l, machine.tier)[0] == p.id).collect::<Vec<_>>(),
                "ms": times.iter().find(|t| t.0 == p.id).map(|t| t.1),
            })
        })
        .collect();
    let voices: Vec<Value> = VOICES.iter().map(|(pack, sid, name)| json!({ "pack": pack, "sid": sid, "name": name })).collect();
    json!({ "machine": machine, "packs": packs, "voices": voices })
}

/// Downloads a pack, checks it against its pinned SHA-256 and unpacks it into the models folder; the slower int8
/// pack it replaces goes once it is there.
pub fn download(dir: &Path, id: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let pack = pack_by_id(id)?;
    let _one_at_a_time = DOWNLOADING.lock().map_err(|_| "Đang tải giọng bị lỗi")?;
    if location(dir, pack).is_some_and(|(_, model)| model == pack.model) {
        return Ok(());
    }
    models::fetch_archive(dir, &format!("{RELEASE}/{}.tar.bz2", pack.folder), pack.size, pack.sha256, &progress)?;
    if location(dir, pack).is_none_or(|(_, model)| model != pack.model) {
        return Err("Gói giọng đọc tải về thiếu file, hãy thử lại".into());
    }
    if let Some(old) = pack.old {
        let _ = remove(dir, old);
    }
    progress(pack.size, pack.size);
    Ok(())
}

/// Removes a pack from the disk, with the older one it replaced.
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let pack = pack_by_id(id)?;
    let _not_downloading = DOWNLOADING.try_lock().map_err(|_| "Đang tải giọng đọc, chờ tải xong rồi xóa")?;
    remove(dir, pack.folder)?;
    pack.old.map_or(Ok(()), |old| remove(dir, old))
}

/// Lets go of a folder's models in memory and its clips, since Windows does not delete open files, then deletes it.
fn remove(dir: &Path, folder: &str) -> Result<(), String> {
    let path = models::path(dir, folder);
    let key = path.display().to_string();
    LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).retain(|(k, _)| !k.starts_with(&format!("{key}|")));
    CLIPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Không xóa được giọng đọc: {e}")),
        _ => Ok(()),
    }
}

/// Reads `text` aloud with `voice` at `speed` (1 is normal) and returns once it is heard, with what was heard: its
/// samples and their rate, for holding a learner's voice against it.
pub fn speak(dir: &Path, voice: &Voice, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
    let _one_at_a_time = SPEAKING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (samples, rate) = synth(dir, voice, text, speed)?;
    play(dir, &samples, rate)?;
    Ok((samples.to_vec(), rate))
}

/// The speech of `text` in `voice`, without playing it: the voice tutor streams it through play_stream.
pub fn render(dir: &Path, voice: &Voice, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
    synth(dir, voice, text, speed).map(|(samples, rate)| (samples.to_vec(), rate))
}

/// The speech of `text` in speaker `sid` of the course's Kokoro, as samples and their rate, for the tests that
/// need a voice.
#[cfg(test)]
pub(crate) fn generate(dir: &Path, course: &str, sid: i32, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
    let lang = lang_of(course);
    let pack = PACKS.iter().find(|p| p.family == "kokoro" && p.langs.contains(&lang)).ok_or("Không có giọng cho khóa này")?;
    synth(dir, &Voice { pack, sid, lang }, text, speed).map(|(samples, rate)| (samples.to_vec(), rate))
}

/// Opens the voice the learner chose and makes a word, so that the first one asked for is not the slow one.
pub fn warm_up(dir: &Path, setting: &str, lang: &str) {
    if let Some(voice) = chosen(dir, setting, lang)
        && let Err(e) = synth(dir, &voice, if lang == "zh" { "你好。" } else { "Hello." }, 1.0)
    {
        eprintln!("warm up voice: {e}");
    }
}

/// Makes the clips of words about to be read, one at a time on a thread of their own, so that each is there when
/// asked for. Each waits for the reading under way, asked for just before, and a reading asked for meanwhile waits
/// for one word at most.
pub fn prefetch(dir: PathBuf, voice: Voice, texts: Vec<String>, speed: f32) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        for text in texts.iter().take(8) {
            drop(SPEAKING.lock());
            let _ = synth(&dir, &voice, text, speed);
        }
    });
}

fn clip(key: &str) -> Option<(Arc<Vec<f32>>, u32)> {
    let mut clips = CLIPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = clips.iter().position(|c| c.0 == key)?;
    let hit = clips.remove(at)?;
    let found = (hit.1.clone(), hit.2);
    clips.push_back(hit);
    Some(found)
}

/// The speech of `text`: the clip kept when it was read before, else made now. A word Kokoro has no pronunciation
/// for is read by Supertonic when it is there, in a voice of the same sex.
fn synth(dir: &Path, voice: &Voice, text: &str, speed: f32) -> Result<(Arc<Vec<f32>>, u32), String> {
    let speed = speed.clamp(0.5, 1.5);
    let key = format!("{}:{}:{}:{}:{text}", voice.pack.id, voice.sid, voice.lang, (speed * 100.0).round());
    if let Some(hit) = clip(&key) {
        return Ok(hit);
    }
    let (folder, _) = location(dir, voice.pack).ok_or("Chưa tải giọng đọc này, vào Cài đặt để tải")?;
    let (samples, rate) = match prepare(&folder, voice, text) {
        Some(input) => make(dir, voice, &input, speed)?,
        None => {
            let pack = pack_by_id("supertonic")?;
            if !ready(dir, pack) {
                return Err(UNKNOWN_WORD.into());
            }
            let other = Voice { pack, sid: if voice.male() { 5 } else { 0 }, lang: "en" };
            make(dir, &other, &english(text), speed)?
        }
    };
    if samples.is_empty() {
        return Err("Giọng đọc không đọc được câu này".into());
    }
    let samples = Arc::new(samples);
    let mut clips = CLIPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if clips.len() >= CLIPS_KEPT {
        clips.pop_front();
    }
    clips.push_back((key, samples.clone(), rate));
    Ok((samples, rate))
}

/// Runs the model of `voice` on `input`, opening it first unless it is one of the two in memory.
fn make(dir: &Path, voice: &Voice, input: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
    let (folder, model) = location(dir, voice.pack).ok_or("Chưa tải giọng đọc này, vào Cài đặt để tải")?;
    let british = english_lexicon(&folder, voice) == "lexicon-gb-en.txt";
    let key = format!("{}|{british}", folder.display());
    let mut loaded = LOADED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match loaded.iter().position(|(k, _)| *k == key) {
        Some(at) => {
            let model = loaded.remove(at);
            loaded.push(model);
        }
        None => {
            let tts = OfflineTts::create(&config(voice.pack, &folder, model, british)).ok_or("Không mở được giọng đọc")?;
            if loaded.len() >= 2 {
                loaded.remove(0);
            }
            loaded.push((key, tts));
        }
    }
    let (_, tts) = loaded.last().ok_or("Không mở được giọng đọc")?;
    let extra = (voice.pack.engine == Engine::Supertonic).then(|| HashMap::from([("lang".to_owned(), json!(voice.lang))]));
    let config = GenerationConfig { sid: voice.sid, speed, extra, ..Default::default() };
    let start = Instant::now();
    let audio = tts.generate_with_config(input, &config, None::<fn(&[f32], f32) -> bool>).ok_or("Giọng đọc không đọc được câu này")?;
    if input.chars().count() <= 16 {
        let mut times = TIMES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        times.retain(|t| t.0 != voice.pack.id);
        times.push((voice.pack.id, start.elapsed().as_millis()));
    }
    Ok((audio.samples().to_vec(), audio.sample_rate().max(1) as u32))
}

/// The lexicon Kokoro reads English with: the British one for its British voices, when the pack has it.
fn english_lexicon(folder: &Path, voice: &Voice) -> &'static str {
    if voice.british() && folder.join("lexicon-gb-en.txt").is_file() { "lexicon-gb-en.txt" } else { "lexicon-us-en.txt" }
}

fn config(pack: &Pack, folder: &Path, model: &str, british: bool) -> OfflineTtsConfig {
    let path = |f: &str| Some(narrow(&folder.join(f)));
    let fsts = |names: [&str; 3]| Some(names.iter().filter_map(|f| path(f)).collect::<Vec<_>>().join(","));
    let base = OfflineTtsModelConfig { num_threads: hardware::model_threads(), ..Default::default() };
    match pack.engine {
        Engine::Kokoro => {
            let english = if british { "lexicon-gb-en.txt" } else { "lexicon-us-en.txt" };
            let lexicon = [path(english), path("lexicon-zh.txt")].into_iter().flatten().collect::<Vec<_>>().join(",");
            let kokoro = OfflineTtsKokoroModelConfig {
                model: path(model),
                voices: path("voices.bin"),
                tokens: path("tokens.txt"),
                dict_dir: path("dict"),
                lexicon: Some(lexicon),
                length_scale: 1.0,
                ..Default::default()
            };
            // How numbers, dates and phone numbers in Chinese text are read; English text has its numbers in words
            // by then, and these rules would read its digits in Chinese.
            let rule_fsts = if pack.langs.contains(&"zh") { fsts(["date-zh.fst", "phone-zh.fst", "number-zh.fst"]) } else { None };
            OfflineTtsConfig { model: OfflineTtsModelConfig { kokoro, ..base }, rule_fsts, ..Default::default() }
        }
        Engine::Supertonic => {
            let supertonic = OfflineTtsSupertonicModelConfig {
                duration_predictor: path("duration_predictor.int8.onnx"),
                text_encoder: path("text_encoder.int8.onnx"),
                vector_estimator: path(model),
                vocoder: path("vocoder.int8.onnx"),
                tts_json: path("tts.json"),
                unicode_indexer: path("unicode_indexer.bin"),
                voice_style: path("voice.bin"),
            };
            OfflineTtsConfig { model: OfflineTtsModelConfig { supertonic, ..base }, ..Default::default() }
        }
        Engine::Piper => {
            let vits = OfflineTtsVitsModelConfig { model: path(model), tokens: path("tokens.txt"), lexicon: path("lexicon.txt"), ..Default::default() };
            OfflineTtsConfig { model: OfflineTtsModelConfig { vits, ..base }, rule_fsts: fsts(["date.fst", "phone.fst", "number.fst"]), ..Default::default() }
        }
    }
}

/// What `voice` is given to read for `text`: English with its numbers in words, and for Kokoro a compound missing
/// from its lexicon as its two words and a lone word closed with a full stop, which it reads more clearly. None when
/// Kokoro would still leave a word out.
fn prepare(folder: &Path, voice: &Voice, text: &str) -> Option<String> {
    if voice.lang != "en" {
        return Some(text.to_owned());
    }
    let text = english(text);
    if voice.pack.engine != Engine::Kokoro {
        return Some(text);
    }
    let known = lexicon(&folder.join(english_lexicon(folder, voice)));
    let mut words = 0;
    let mut out = String::with_capacity(text.len() + 8);
    let mut word = String::new();
    let mut flush = |word: &mut String, out: &mut String| -> Option<()> {
        if !word.is_empty() {
            words += 1;
            out.push_str(&kokoro_word(word, &known)?);
            word.clear();
        }
        Some(())
    };
    for c in text.chars() {
        if c.is_whitespace() || ",.!?;:".contains(c) {
            flush(&mut word, &mut out)?;
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out)?;
    let out = out.trim().to_owned();
    Some(if words == 1 && !out.ends_with(['.', '!', '?']) { format!("{out}.") } else { out })
}

/// A word as Kokoro can read it, or None when it has no pronunciation for it.
fn kokoro_word(word: &str, known: &HashSet<String>) -> Option<String> {
    let lower = word.to_lowercase();
    if known.contains(&lower) {
        return Some(word.to_owned());
    }
    // A possessive it does not know sounds like the plural it may know: "teacher's" as "teachers".
    if let Some(stem) = lower.strip_suffix("'s").or_else(|| lower.strip_suffix("s'"))
        && known.contains(&format!("{stem}s"))
    {
        return Some(format!("{stem}s"));
    }
    // Kokoro looks up the parts of a hyphenated word it does not know on their own.
    if lower.contains('-') {
        return lower.split('-').filter(|p| !p.is_empty()).all(|p| known.contains(p)).then(|| word.to_owned());
    }
    if lower.len() >= 6 && lower.bytes().all(|b| b.is_ascii_lowercase()) {
        // The first split into two words it knows, each of three letters or more: "cell phone", "web page".
        return (3..=lower.len() - 3).find(|&i| known.contains(&lower[..i]) && known.contains(&lower[i..])).map(|i| format!("{} {}", &lower[..i], &lower[i..]));
    }
    None
}

fn lexicon(path: &Path) -> Arc<HashSet<String>> {
    let mut all = LEXICONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, words)) = all.iter().find(|(p, _)| p == path) {
        return words.clone();
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let words: Arc<HashSet<String>> = Arc::new(text.lines().filter_map(|l| l.split_whitespace().next()).map(str::to_lowercase).collect());
    all.push((path.to_owned(), words.clone()));
    words
}

/// English text as a voice reads it best: typographic apostrophes plain, brackets and quotes gone (Kokoro would
/// take them as part of a word it then leaves out), "&" and numbers in words.
fn english(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let (words, used) = number(&chars[i..]);
            out.push_str(&words);
            i += used;
            continue;
        }
        match c {
            '\u{2019}' | '\u{2018}' => out.push('\''),
            '&' => out.push_str(" and "),
            '"' | '\u{201c}' | '\u{201d}' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '*' | '_' | '#' | '<' | '>' | '|' | '~' | '`' => out.push(' '),
            '\u{2013}' | '\u{2014}' => out.push_str(", "),
            c => out.push(c),
        }
        i += 1;
    }
    out.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The number at the start of `chars` in words, and how many characters it took: "1,200", "3.5", "50%", "21st",
/// and a lone four-digit year such as "1999" read the way years are.
fn number(chars: &[char]) -> (String, usize) {
    let mut digits = String::new();
    let mut i = 0;
    // Digits, with commas between groups of three.
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            digits.push(chars[i]);
            i += 1;
        } else if chars[i] == ',' && chars.get(i + 1..i + 4).is_some_and(|g| g.iter().all(char::is_ascii_digit)) && chars.get(i + 4).is_none_or(|c| !c.is_ascii_digit()) {
            i += 1;
        } else {
            break;
        }
    }
    let grouped = digits.len() != i;
    let mut words = match digits.parse::<u64>() {
        Ok(n) if digits.len() == 4 && !grouped && (1100..=2099).contains(&n) => year(n),
        Ok(n) if digits.len() <= 12 && !(digits.len() > 1 && digits.starts_with('0')) => cardinal(n),
        // Codes and long numbers digit by digit.
        _ => digits.chars().map(|d| cardinal(u64::from(d.to_digit(10).unwrap_or(0)))).collect::<Vec<_>>().join(" "),
    };
    if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
        i += 1;
        words.push_str(" point");
        while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
            words.push(' ');
            words.push_str(&cardinal(u64::from(d)));
            i += 1;
        }
    }
    let suffix: String = chars[i..].iter().take(2).collect::<String>().to_lowercase();
    if chars.get(i) == Some(&'%') {
        words.push_str(" percent");
        i += 1;
    } else if matches!(suffix.as_str(), "st" | "nd" | "rd" | "th") && chars.get(i + 2).is_none_or(|c| !c.is_alphanumeric()) {
        words = ordinal(&words);
        i += 2;
    }
    (words, i)
}

fn cardinal(n: u64) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen",
        "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    match n {
        0..=19 => ONES[n as usize].to_owned(),
        20..=99 if n % 10 == 0 => TENS[n as usize / 10].to_owned(),
        20..=99 => format!("{}-{}", TENS[n as usize / 10], ONES[n as usize % 10]),
        100..=999 if n % 100 == 0 => format!("{} hundred", ONES[n as usize / 100]),
        100..=999 => format!("{} hundred {}", ONES[n as usize / 100], cardinal(n % 100)),
        _ => {
            let (scale, name) = [(1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")].into_iter().find(|(s, _)| n >= *s).unwrap_or((1_000, "thousand"));
            let rest = n % scale;
            format!("{} {name}{}", cardinal(n / scale), if rest == 0 { String::new() } else { format!(" {}", cardinal(rest)) })
        }
    }
}

/// 1999 as "nineteen ninety-nine", 1905 as "nineteen oh five", 2005 as "two thousand five", 2024 as "twenty
/// twenty-four".
fn year(n: u64) -> String {
    let (high, low) = (n / 100, n % 100);
    match low {
        _ if (2000..=2009).contains(&n) => cardinal(n),
        0 => format!("{} hundred", cardinal(high)),
        1..=9 => format!("{} oh {}", cardinal(high), cardinal(low)),
        _ => format!("{} {}", cardinal(high), cardinal(low)),
    }
}

/// "twenty-one" as "twenty-first": only the last word changes.
fn ordinal(words: &str) -> String {
    let (head, last) = words.rsplit_once([' ', '-']).map_or(("", words), |(h, l)| (&words[..h.len() + 1], l));
    let last = match last {
        "one" => "first".to_owned(),
        "two" => "second".to_owned(),
        "three" => "third".to_owned(),
        "five" => "fifth".to_owned(),
        "eight" => "eighth".to_owned(),
        "nine" => "ninth".to_owned(),
        "twelve" => "twelfth".to_owned(),
        l if l.ends_with('y') => format!("{}ieth", &l[..l.len() - 1]),
        l => format!("{l}th"),
    };
    format!("{head}{last}")
}

/// A path sherpa-onnx can open. Its C API takes narrow strings, which on Windows lose letters such as those of a
/// Vietnamese user name, so there a path that is not ASCII goes in its short 8.3 form when the disk keeps one.
#[cfg(windows)]
pub(crate) fn narrow(path: &Path) -> String {
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
pub(crate) fn narrow(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Plays speech and returns once it is heard: in the app, else by the system's player when no output opens. A
/// moment of silence goes first, which a Bluetooth headset that wakes up on the sound would otherwise swallow.
fn play(dir: &Path, samples: &[f32], rate: u32) -> Result<(), String> {
    let (send, pieces) = mpsc::channel();
    let _ = send.send(([vec![0.0; rate as usize / 20], samples.to_vec()].concat(), rate));
    drop(send);
    play_stream(pieces, &|| false).or_else(|e| {
        eprintln!("{e}");
        let wav = dir.join("speech.wav");
        write_wav(&wav, samples, rate).map_err(|e| format!("Không ghi được file âm thanh: {e}"))?;
        play_file(&wav)
    })
}

/// Plays the pieces of speech sent on `pieces`, each with its rate, one after another on the default output as they
/// come, and returns once the sender is gone and all of it was heard, or as soon as `stop` says so. Played here rather
/// than by the system's player: no file and no process for each sentence, and a stop is heard at once.
pub fn play_stream(pieces: mpsc::Receiver<(Vec<f32>, u32)>, stop: &dyn Fn() -> bool) -> Result<(), String> {
    let device = cpal::default_host().default_output_device().ok_or("Không tìm thấy loa")?;
    let config = device.default_output_config().map_err(|e| format!("Loa: {e}"))?;
    let (rate, channels) = (config.sample_rate().0, usize::from(config.channels()).max(1));
    let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => output::<f32>(&device, &config.config(), queue.clone(), channels),
        cpal::SampleFormat::I16 => output::<i16>(&device, &config.config(), queue.clone(), channels),
        cpal::SampleFormat::U16 => output::<u16>(&device, &config.config(), queue.clone(), channels),
        other => return Err(format!("Loa dùng định dạng {other} chưa hỗ trợ")),
    }?;
    stream.play().map_err(|e| format!("Loa: {e}"))?;
    let left = || queue.lock().map_or(0, |q| q.len());
    while !stop() {
        match pieces.recv_timeout(Duration::from_millis(20)) {
            Ok((samples, from)) => queue.lock().map_err(|_| "loa đang lỗi")?.extend(speech::resample(&samples, from, rate)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) if left() == 0 => {
                // What the device still holds.
                std::thread::sleep(Duration::from_millis(120));
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    Ok(())
}

fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    channels: usize,
) -> Result<cpal::Stream, String> {
    let fill = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        let mut queue = queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for frame in out.chunks_mut(channels) {
            frame.fill(T::from_sample(queue.pop_front().unwrap_or(0.0)));
        }
    };
    device.build_output_stream(config, fill, |e| eprintln!("loa: {e}"), None).map_err(|e| format!("Loa: {e}"))
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

/// The system's player, for a machine whose output the app cannot open itself.
fn play_file(wav: &Path) -> Result<(), String> {
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
    fn every_voice_belongs_to_a_pack_and_every_language_has_one() {
        for (id, sid, _) in VOICES {
            assert!(pack_by_id(id).is_ok() && (0..103).contains(&sid), "{id}");
        }
        for tier in [Tier::Weak, Tier::Medium, Tier::Strong] {
            for lang in ["en", "zh", "vi"] {
                for id in preferred(lang, tier) {
                    assert!(pack_by_id(id).unwrap().langs.contains(&lang), "{id} {lang}");
                }
            }
        }
        assert!(pack_by_id("fr").is_err());
    }

    #[test]
    fn a_setting_names_a_voice_of_a_downloaded_pack() {
        let dir = std::env::temp_dir().join(format!("card-english-voices-{}", std::process::id()));
        let old = models::path(&dir, "kokoro-int8-multi-lang-v1_0");
        std::fs::create_dir_all(&old).unwrap();
        for f in [OLD_MODEL].iter().chain(KOKORO_FILES) {
            std::fs::write(old.join(f), "x").unwrap();
        }
        // The int8 Kokoro of an earlier version still reads, as the old one.
        let voice = chosen(&dir, "kokoro:21", "en").unwrap();
        assert!(voice.pack.id == "kokoro-en" && voice.british() && !voice.male());
        assert!(chosen(&dir, "kokoro:3", "zh").is_none() && chosen(&dir, "supertonic:3", "vi").is_none() && chosen(&dir, "Samantha", "en").is_none());
        assert_eq!(for_lang(&dir, "", "en").unwrap().sid, 3);
        assert_eq!(status(&dir)["packs"][0]["old"], true);
        assert_eq!(for_speaker(voice, "M").sid, 16, "Emma is a woman's voice, Michael the first man's");
        assert_eq!(for_speaker(voice, "W").sid, 21);
        assert_eq!(for_speaker(voice, "").sid, 21);
        delete(&dir, "kokoro-en").unwrap();
        assert!(for_lang(&dir, "kokoro:21", "en").is_none() && !old.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn english_is_read_with_its_numbers_in_words() {
        assert_eq!(english("I have 3 cats & 2 dogs."), "I have three cats and two dogs.");
        assert_eq!(english("It costs 1,250 dollars, 12.5% off"), "It costs one thousand two hundred fifty dollars, twelve point five percent off");
        assert_eq!(english("In 1999 and 2024, on the 21st"), "In nineteen ninety-nine and twenty twenty-four, on the twenty-first");
        assert_eq!(english("the 3rd, 12th and 40th, 2005 or 1905"), "the third, twelfth and fortieth, two thousand five or nineteen oh five");
        assert_eq!(english("call 0912 or 100"), "call zero nine one two or one hundred");
        assert_eq!(english("don\u{2019}t (really) say \"no\""), "don't really say no");
    }

    #[test]
    fn kokoro_reads_compounds_as_their_words_and_gives_up_on_unknown_ones() {
        let known: HashSet<String> = ["cell", "phone", "web", "page", "hello", "world", "well", "known"].iter().map(|s| s.to_string()).collect();
        assert_eq!(kokoro_word("Hello", &known).as_deref(), Some("Hello"));
        assert_eq!(kokoro_word("cellphone", &known).as_deref(), Some("cell phone"));
        assert_eq!(kokoro_word("Webpage", &known).as_deref(), Some("web page"));
        assert_eq!(kokoro_word("well-known", &known).as_deref(), Some("well-known"));
        assert_eq!(kokoro_word("donut", &known), None);
        assert_eq!(kokoro_word("well-done", &known), None);
        let known: HashSet<String> = ["teachers"].iter().map(|s| s.to_string()).collect();
        assert_eq!(kokoro_word("teacher's", &known).as_deref(), Some("teachers"));
        assert_eq!(kokoro_word("teachers'", &known).as_deref(), Some("teachers"));
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

    /// Reads with every pack in a folder whose models/ holds them, timing each. Run it by hand:
    /// `VOICES_DIR=<that folder> cargo test packs_read -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the downloaded packs"]
    fn packs_read_english_chinese_and_vietnamese() {
        let dir = std::env::var_os("VOICES_DIR").map(PathBuf::from).expect("VOICES_DIR holds models/");
        for (id, lang, text) in [
            ("kokoro-en", "en", "cellphone"),
            ("kokoro-en", "en", "We need 3 tickets for the 21st."),
            ("supertonic", "en", "Hello, I am learning new words today."),
            ("supertonic", "vi", "Hôm nay trời rất đẹp."),
            ("kokoro-zh", "zh", "你好，我在学习中文。"),
            ("piper-zh", "zh", "你好，我在学习中文。"),
        ] {
            let pack = pack_by_id(id).unwrap();
            if !ready(&dir, pack) {
                println!("{id}: not downloaded");
                continue;
            }
            let voice = Voice { pack, sid: VOICES.iter().find(|v| v.0 == id).unwrap().1, lang };
            let start = Instant::now();
            let (samples, rate) = synth(&dir, &voice, text, 1.0).unwrap();
            let made = start.elapsed();
            let again = Instant::now();
            synth(&dir, &voice, text, 1.0).unwrap();
            println!("{id} {lang} {text:?}: {:.2} s of audio in {made:?}, again in {:?}", samples.len() as f32 / rate as f32, again.elapsed());
            assert!(samples.len() as u32 > rate / 4, "{id}");
            if std::env::var_os("VOICES_PLAY").is_some() {
                play(&dir, &samples, rate).unwrap();
            }
        }
        // The speaker opens: a fifth of a second of silence through cpal, at the rate of each engine.
        for rate in [24_000, 44_100, 22_050] {
            let (send, pieces) = mpsc::channel();
            send.send((vec![0.0; rate as usize / 5], rate)).unwrap();
            drop(send);
            let start = Instant::now();
            play_stream(pieces, &|| false).unwrap();
            println!("silence at {rate} Hz played in {:?}", start.elapsed());
        }
    }
}
