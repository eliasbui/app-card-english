//! Scoring how a word was said: the microphone through cpal, a recognizer to hear it (Parakeet for English and
//! SenseVoice for Chinese through sherpa-onnx, or the Whisper tiny of the first versions), and the phonemes of what
//! was heard against those of the word.

use crate::{
    db,
    hardware::{self, Tier},
    models, pron,
};
use candle_core::{D, Device, IndexOp, Tensor};
use candle_transformers::models::whisper::{self as w, audio, quantized_model::Whisper};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OfflineStream, OfflineTransducerModelConfig};
use sherpa_onnx_sys as sys;
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokenizers::Tokenizer;

/// A recording stops by itself after this, in case the stop never comes; long enough for an example sentence.
const MAX_SECONDS: u64 = 16;
/// Quieter than this (root mean square of the loudest tenth of a second) is taken as no voice at all.
const SILENCE_RMS: f32 = 0.008;
const MAX_TOKENS: usize = 64;
/// How sure a recognizer that gives no probabilities, such as SenseVoice, is taken to be of what it heard.
const UNKNOWN_CONFIDENCE: f32 = 0.9;
/// The silence put on both sides of a recording for sherpa-onnx: without it Parakeet misses sounds at the edges of a
/// word cut close, and hears "sixth" as "six".
const PADDING: usize = w::SAMPLE_RATE * 3 / 10;
const MISSING: &str = "Chưa tải model chấm phát âm, vào Cài đặt, mục Model trên máy để tải";
const MISSING_ZH: &str = "Chưa tải model chấm phát âm tiếng Trung, vào Cài đặt, mục Model trên máy để tải";

type Take = Result<(Vec<f32>, u32), String>;

struct Recording {
    stop: mpsc::Sender<()>,
    done: JoinHandle<Take>,
}

static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);
/// How loud the last piece of sound was (root mean square, as f32 bits), for the wave the page draws.
static LEVEL: AtomicU32 = AtomicU32::new(0);

/// The loudness of what the microphone heard last, 0 when not recording.
pub fn level() -> f32 {
    f32::from_bits(LEVEL.load(Ordering::Relaxed))
}

/// Starts listening on the default microphone and answers its name. The stream lives on a thread of its own because
/// cpal's streams cannot be moved between threads on every platform.
pub fn start() -> Result<String, String> {
    let mut slot = RECORDING.lock().map_err(|_| "ghi âm đang lỗi")?;
    if slot.is_some() {
        return Err("Đang ghi âm".into());
    }
    LEVEL.store(0, Ordering::Relaxed);
    let (stop, stopped) = mpsc::channel();
    let (ready, started) = mpsc::channel();
    let done = thread::spawn(move || {
        let opened = open_microphone();
        let (stream, samples, rate) = match opened {
            Ok((stream, samples, rate, name)) => {
                let _ = ready.send(Ok(name));
                (stream, samples, rate)
            }
            Err(e) => {
                let _ = ready.send(Err(e.clone()));
                return Err(e);
            }
        };
        let _ = stopped.recv_timeout(Duration::from_secs(MAX_SECONDS));
        drop(stream);
        let samples = samples.lock().map(|s| s.clone()).unwrap_or_default();
        Ok((samples, rate))
    });
    let name = started.recv().map_err(|_| "Không mở được micro".to_owned())??;
    *slot = Some(Recording { stop, done });
    Ok(name)
}

fn open_microphone() -> Result<(cpal::Stream, Arc<Mutex<Vec<f32>>>, u32, String), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("Không tìm thấy micro")?;
    let name = device.name().unwrap_or_default();
    let config = device.default_input_config().map_err(|e| format!("Micro: {e}"))?;
    let rate = config.sample_rate().0;
    let channels = usize::from(config.channels()).max(1);
    let samples = Arc::new(Mutex::new(Vec::new()));
    let sink = samples.clone();
    // Mixed down to mono as it arrives.
    let push = move |data: &mut dyn Iterator<Item = f32>| {
        if let Ok(mut s) = sink.lock() {
            let frame: Vec<f32> = data.collect();
            let mono: Vec<f32> = frame.chunks(channels).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
            let rms = (mono.iter().map(|x| x * x).sum::<f32>() / mono.len().max(1) as f32).sqrt();
            LEVEL.store(rms.to_bits(), Ordering::Relaxed);
            s.extend(mono);
        }
    };
    let error = |e| eprintln!("micro: {e}");
    let stream_config = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |d: &[f32], _: &_| push(&mut d.iter().copied()),
            error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |d: &[i16], _: &_| push(&mut d.iter().map(|&s| f32::from(s) / 32768.0)),
            error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |d: &[u16], _: &_| push(&mut d.iter().map(|&s| (f32::from(s) - 32768.0) / 32768.0)),
            error,
            None,
        ),
        other => return Err(format!("Micro dùng định dạng {other} chưa hỗ trợ")),
    }
    .map_err(|e| format!("Micro: {e}"))?;
    stream.play().map_err(|e| format!("Micro: {e}"))?;
    Ok((stream, samples, rate, name))
}

/// Stops listening and answers the sound at 16 kHz.
pub fn stop() -> Result<Vec<f32>, String> {
    let recording = RECORDING
        .lock()
        .map_err(|_| "ghi âm đang lỗi")?
        .take()
        .ok_or("Chưa bắt đầu ghi âm")?;
    let _ = recording.stop.send(());
    LEVEL.store(0, Ordering::Relaxed);
    let (samples, rate) = recording.done.join().map_err(|_| "ghi âm bị dừng đột ngột")??;
    Ok(resample(&samples, rate, w::SAMPLE_RATE as u32))
}

// shortcut: linear interpolation, plenty for speech recognition; use a windowed-sinc resampler if quality matters.
pub(crate) fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let len = (samples.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let at = i as f64 * ratio;
            let (j, frac) = (at as usize, (at.fract()) as f32);
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Score {
    /// 0 to 100.
    pub score: u8,
    pub heard: String,
    pub heard_ipa: String,
    pub target_ipa: String,
    pub message: String,
    /// For a sentence, its words that were not heard.
    pub missed: Vec<String>,
}

/// How loud the voice of a 16 kHz recording is: the root mean square of its loudest tenth of a second, so that a word
/// is not averaged away by the silence around it, as it was on a quiet microphone (a Mac mini has none of its own).
fn loudness(pcm: &[f32]) -> f32 {
    let (tenths, _) = pcm.as_chunks::<{ w::SAMPLE_RATE / 10 }>();
    tenths.iter().map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).fold(0.0, f32::max)
}

/// Whether a recording is too short or too quiet to hold a voice.
pub(crate) fn silent(pcm: &[f32]) -> bool {
    pcm.len() < w::SAMPLE_RATE / 4 || loudness(pcm) < SILENCE_RMS
}

/// The recording brought up to a speaking level, at most twenty times louder: a faint voice is heard worse, and a
/// microphone whose second channel is silent sends half of it.
pub(crate) fn louder(pcm: &[f32]) -> Vec<f32> {
    let gain = (0.1 / loudness(pcm).max(1e-4)).clamp(1.0, 20.0);
    pcm.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect()
}

/// What was said and how sure the recognizer was of it, from 0 to 1, or nothing for a stretch of silence: a long
/// answer is heard a piece at a time, and a pause between two pieces is no error. A microphone sending nothing at all is.
pub fn hear_scored(pcm: &[f32], lang: &str, dir: &Path) -> Result<(String, f32), String> {
    if silent(pcm) {
        return if !pcm.is_empty() && pcm.iter().all(|&s| s == 0.0) { Err(silence_message(pcm).into()) } else { Ok((String::new(), 0.0)) };
    }
    let (text, confidence) = transcribe(pcm, dir, lang == "zh")?;
    Ok((text.trim().to_owned(), confidence))
}

/// What was said, in English or, for "zh", in Chinese, by the recognizer of that course.
pub fn hear(pcm: &[f32], lang: &str, dir: &Path) -> Result<String, String> {
    if silent(pcm) {
        return Err(silence_message(pcm).into());
    }
    let (text, _) = transcribe(pcm, dir, lang == "zh")?;
    let text = text.trim();
    if text.is_empty() { Err("Chưa nghe rõ, hãy nói lại hoặc gõ câu trả lời".into()) } else { Ok(text.to_owned()) }
}

/// Hears the sound and scores it against `target`.
pub fn assess(target: &str, pcm: &[f32], dir: &Path) -> Result<Score, String> {
    if silent(pcm) {
        return Ok(Score {
            score: 0,
            heard: String::new(),
            heard_ipa: String::new(),
            target_ipa: pron::ipa(target),
            message: silence_message(pcm).into(),
            missed: Vec::new(),
        });
    }
    if pron::is_chinese(target) {
        let (heard, confidence) = transcribe(pcm, dir, true)?;
        return Ok(score_chinese(target, &heard, confidence));
    }
    let (heard, confidence) = transcribe(pcm, dir, false)?;
    Ok(score(target, &heard, confidence))
}

/// Why nothing was heard. A microphone the system keeps from the app sends exact zeros, which even a quiet room
/// never does, so that case points to the setting that allows it.
fn silence_message(pcm: &[f32]) -> &'static str {
    if pcm.is_empty() || pcm.iter().any(|&s| s != 0.0) {
        "Không nghe thấy giọng, hãy nói to và gần micro hơn"
    } else if cfg!(windows) {
        "Micro không gửi âm thanh nào. Mở Settings > Privacy & security > Microphone, bật Microphone access và Let desktop apps access your microphone, rồi thử lại"
    } else if cfg!(target_os = "macos") {
        "Micro không gửi âm thanh nào. Mở System Settings > Privacy & Security > Microphone, bật cho Card English, rồi thử lại"
    } else {
        "Micro không gửi âm thanh nào, hãy kiểm tra micro trong cài đặt âm thanh của máy"
    }
}

fn message_of(score: u8) -> &'static str {
    match score {
        90.. => "Xuất sắc! Phát âm rất chuẩn",
        75..90 => "Tốt, gần chuẩn rồi",
        50..75 => "Khá, nghe lại mẫu và đọc rõ từng âm",
        1..50 => "Chưa đúng, hãy nghe mẫu đọc chậm rồi thử lại",
        0 => "Không nhận ra từ, hãy thử lại",
    }
}

/// The score of what was heard: the word itself scores by how sure the recognizer was, anything else by how close
/// its phonemes are to the word's.
pub fn score(target: &str, heard: &str, confidence: f32) -> Score {
    let (want, got) = (pron::words(target), pron::words(heard));
    let hit = !want.is_empty() && got.windows(want.len()).any(|w| w == want.as_slice());
    let value = if got.is_empty() {
        0.0
    } else if hit {
        70.0 + 30.0 * confidence
    } else if let (Some(a), Some(b)) = (pron::phones(target), pron::phones(heard)) {
        80.0 * pron::similarity(&a, &b)
    } else {
        let (a, b): (Vec<char>, Vec<char>) = (want.concat().chars().collect(), got.concat().chars().collect());
        60.0 * pron::similarity(&a, &b)
    };
    let score = value.round().clamp(0.0, 100.0) as u8;
    Score {
        score,
        heard: heard.trim().to_owned(),
        heard_ipa: pron::ipa(heard),
        target_ipa: pron::ipa(target),
        message: message_of(score).into(),
        missed: if want.len() > 1 { missed(&want, &got) } else { Vec::new() },
    }
}

/// Chinese scores by its syllables, which simplified and traditional characters share: the target itself by how
/// sure the recognizer was, anything else by how close the syllables are, half with their tones and half without, so
/// that a wrong tone costs less than a wrong sound.
pub fn score_chinese(target: &str, heard: &str, confidence: f32) -> Score {
    let (want, got) = (pron::syllables(target), pron::syllables(heard));
    let hit = !want.is_empty() && got.windows(want.len()).any(|w| w == want.as_slice());
    let toneless = |s: &[String]| s.iter().map(|x| x.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned()).collect::<Vec<_>>();
    let (tones, sounds) = (pron::similarity(&want, &got), pron::similarity(&toneless(&want), &toneless(&got)));
    let value = if got.is_empty() {
        0.0
    } else if hit {
        70.0 + 30.0 * confidence
    } else {
        40.0 * (tones + sounds)
    };
    let score = value.round().clamp(0.0, 100.0) as u8;
    let message = if !hit && score > 0 && sounds - tones > 0.2 {
        "Đúng âm nhưng sai thanh điệu, nghe mẫu và chú ý dấu thanh"
    } else {
        message_of(score)
    };
    Score {
        score,
        heard: heard.trim().to_owned(),
        heard_ipa: pron::ipa(heard),
        target_ipa: pron::ipa(target),
        message: message.into(),
        // For a sentence, the syllables that were not heard, with their tone marks.
        missed: if want.len() > 3 { missed(&want, &got).iter().map(|s| pron::mark(s)).collect() } else { Vec::new() },
    }
}

/// The words of `want` that `got` lacks, in order, by the longest run they share.
fn missed(want: &[String], got: &[String]) -> Vec<String> {
    let (n, m) = (want.len(), got.len());
    let mut common = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if want[i] == got[j] { common[i + 1][j + 1] + 1 } else { common[i + 1][j].max(common[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n {
        if j < m && want[i] == got[j] {
            (i, j) = (i + 1, j + 1);
        } else if j < m && common[i][j + 1] >= common[i + 1][j] {
            j += 1;
        } else {
            out.push(want[i].clone());
            i += 1;
        }
    }
    out
}

/// A course's recognizers, the one suggested for a machine of this tier first and the Whisper of the first versions
/// last. Parakeet 0.6B hears English best but takes four times as long as the 110M, so it leads on strong machines only.
fn recognizers(zh: bool, tier: Tier) -> &'static [&'static str] {
    match (zh, tier) {
        (true, _) => &["sttsv", "sttzh"],
        (false, Tier::Strong) => &["stt06b", "stt110m", "stt"],
        (false, _) => &["stt110m", "stt06b", "stt"],
    }
}

/// Whether `group` is the recognizer the settings suggest for this machine.
pub(crate) fn suggested(group: &str) -> bool {
    let tier = hardware::machine().tier;
    [false, true].into_iter().any(|zh| recognizers(zh, tier)[0] == group)
}

/// The group of the recognizer a course hears with: the learner's choice `wanted` once it is downloaded, else the
/// first one downloaded of those the machine prefers, or none at all.
pub fn chosen(dir: &Path, lang: &str, wanted: &str) -> Option<&'static str> {
    pick(recognizers(lang == "zh", hardware::machine().tier), wanted, |group| models::ready(dir, group))
}

fn pick(list: &'static [&'static str], wanted: &str, ready: impl Fn(&str) -> bool) -> Option<&'static str> {
    list.iter().find(|&&g| g == wanted && ready(g)).or_else(|| list.iter().find(|&&g| ready(g))).copied()
}

/// The recognizer the learner chose for a course, read from the database, which every thread can open.
fn wanted(dir: &Path, lang: &str) -> String {
    let db = db::open(&dir.join("words.db")).ok();
    db.and_then(|db| db::setting(&db, &db::scoped("stt_model", lang)).ok().flatten()).unwrap_or_default()
}

/// What hears a course: a recognizer of sherpa-onnx, or a Whisper of the first versions.
enum Recognizer {
    Sherpa(OfflineRecognizer),
    Whisper(Box<Stt>),
}

/// The recognizer of each course in memory, English then Chinese, with its group: loading one takes a second or more,
/// and a lesson hears many words.
static LOADED: [Mutex<Option<(&str, Recognizer)>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// Lets go of the recognizer of a group of models in memory, before its files are deleted.
pub fn unload(group: &str) {
    for slot in &LOADED {
        let mut loaded = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if loaded.as_ref().is_some_and(|(g, _)| *g == group) {
            *loaded = None;
        }
    }
}

/// Loads the English recognizer ahead of the first recording and runs it once, so that the first word is not the
/// slow one; the Chinese one loads on first use.
pub fn warm_up(dir: &Path) {
    if let Some(group) = chosen(dir, "en", &wanted(dir, "en")) {
        let hum: Vec<f32> = (0..w::SAMPLE_RATE).map(|i| (i as f32 * 0.05).sin() * 0.01).collect();
        if let Err(e) = hear_with(group, &hum, dir, false) {
            eprintln!("warm up: {e}");
        }
    }
}

/// The text the course's recognizer hears and how sure it was, from 0 to 1.
fn transcribe(pcm: &[f32], dir: &Path, zh: bool) -> Result<(String, f32), String> {
    let lang = if zh { "zh" } else { "en" };
    let group = chosen(dir, lang, &wanted(dir, lang)).ok_or(if zh { MISSING_ZH } else { MISSING })?;
    hear_with(group, pcm, dir, zh)
}

fn hear_with(group: &'static str, pcm: &[f32], dir: &Path, zh: bool) -> Result<(String, f32), String> {
    let pcm = &louder(pcm);
    let mut loaded = LOADED[usize::from(zh)].lock().map_err(|_| "model giọng nói đang lỗi")?;
    if loaded.as_ref().is_none_or(|(g, _)| *g != group) {
        // The old one goes first, so that both are never in memory together.
        *loaded = None;
        let recognizer = match group {
            "stt" => Recognizer::Whisper(Box::new(load_whisper(dir, &WHISPER_EN)?)),
            "sttzh" => Recognizer::Whisper(Box::new(load_whisper(dir, &WHISPER_ZH)?)),
            _ => Recognizer::Sherpa(load_sherpa(dir, group)?),
        };
        *loaded = Some((group, recognizer));
    }
    match loaded.as_mut().map(|(_, r)| r) {
        Some(Recognizer::Sherpa(recognizer)) => hear_sherpa(recognizer, pcm),
        Some(Recognizer::Whisper(stt)) => whisper(stt, pcm),
        None => Err("model chưa nạp".into()),
    }
}

/// A recognizer of sherpa-onnx on the threads the machine gives its models: Parakeet's transducer, or SenseVoice told
/// that it hears Mandarin, with the inverse text normalization it was measured with.
fn load_sherpa(dir: &Path, group: &str) -> Result<OfflineRecognizer, String> {
    let folder = models::folder(dir, group).ok_or("Không có model này")?;
    let file = |name: &str| Some(crate::voice::narrow(&folder.join(name)));
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.num_threads = hardware::model_threads();
    config.model_config.tokens = file("tokens.txt");
    if group == "sttsv" {
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig { model: file("model.int8.onnx"), language: Some("zh".into()), use_itn: true };
    } else {
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: file("encoder.int8.onnx"),
            decoder: file("decoder.int8.onnx"),
            joiner: file("joiner.int8.onnx"),
        };
        config.model_config.model_type = Some("nemo_transducer".into());
    }
    OfflineRecognizer::create(&config).ok_or_else(|| "Không mở được model chấm phát âm, hãy xóa rồi tải lại trong Cài đặt".into())
}

fn hear_sherpa(recognizer: &OfflineRecognizer, pcm: &[f32]) -> Result<(String, f32), String> {
    let silence = vec![0.0; PADDING];
    let stream = recognizer.create_stream();
    stream.accept_waveform(w::SAMPLE_RATE as i32, &[silence.as_slice(), pcm, silence.as_slice()].concat());
    recognizer.decode(&stream);
    result_json(&stream).as_deref().and_then(heard).ok_or_else(|| "Không đọc được kết quả nhận giọng".into())
}

// ponytail: sherpa-onnx's OfflineStream keeps its C pointer to itself, and the safe wrapper drops the probabilities
// of the result, so its JSON is read here through that pointer, the stream's only field in 2.0.1, to which both
// crates are pinned. The build stops if its size changes, not if a later version puts something else of a pointer's
// size there: build the recognizer through sherpa-onnx-sys before moving off 2.0.1.
const _: () = assert!(std::mem::size_of::<OfflineStream>() == std::mem::size_of::<*const sys::OfflineStream>());

/// The result of a decoded stream as sherpa-onnx writes it, with the log probability of each token.
fn result_json(stream: &OfflineStream) -> Option<String> {
    // SAFETY: the stream is its pointer to the C stream and nothing else, as checked above; the C string is copied
    // before it is freed.
    unsafe {
        let ptr = *(stream as *const OfflineStream).cast::<*const sys::OfflineStream>();
        let json = sys::SherpaOnnxGetOfflineStreamResultAsJson(ptr);
        if json.is_null() {
            return None;
        }
        let text = std::ffi::CStr::from_ptr(json).to_string_lossy().into_owned();
        sys::SherpaOnnxDestroyOfflineStreamResultJson(json);
        Some(text)
    }
}

/// The text of a result and how sure the recognizer was of it, from 0 to 1: the mean probability of its tokens, or
/// UNKNOWN_CONFIDENCE for a recognizer that gives none.
fn heard(json: &str) -> Option<(String, f32)> {
    #[derive(Deserialize)]
    struct Heard {
        text: String,
        #[serde(default)]
        ys_log_probs: Vec<f32>,
    }
    let Heard { text, ys_log_probs: p } = serde_json::from_str(json).ok()?;
    let confidence = if text.trim().is_empty() {
        0.0
    } else if p.is_empty() {
        UNKNOWN_CONFIDENCE
    } else {
        p.iter().map(|l| l.exp()).sum::<f32>() / p.len() as f32
    };
    Some((text, confidence))
}

/// A Whisper of the first versions: its files, and the language it is told to hear, for one that hears more than
/// English.
struct Model {
    weights: &'static str,
    config: &'static str,
    tokenizer: &'static str,
    language: Option<&'static str>,
}

static WHISPER_EN: Model = Model {
    weights: models::WHISPER,
    config: models::WHISPER_CONFIG,
    tokenizer: models::WHISPER_TOKENIZER,
    language: None,
};
static WHISPER_ZH: Model = Model {
    weights: models::WHISPER_ZH,
    config: models::WHISPER_ZH_CONFIG,
    tokenizer: models::WHISPER_ZH_TOKENIZER,
    language: Some("<|zh|>"),
};

// Always the CPU: tiny Whisper takes about a second there, and the Metal kernels return garbage on some Macs (macOS 27).
struct Stt {
    model: Whisper,
    tokenizer: Tokenizer,
    config: w::Config,
    filters: Vec<f32>,
    prefix: Vec<u32>,
    eot: u32,
    /// Added to the logits: minus infinity on the tokens that are never text.
    suppress: Tensor,
}

fn load_whisper(dir: &Path, kind: &Model) -> Result<Stt, String> {
    let err = |e: candle_core::Error| e.to_string();
    let config: w::Config = std::fs::read_to_string(models::path(dir, kind.config))
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))?;
    let vb = candle_transformers::quantized_var_builder::VarBuilder::from_gguf(
        models::path(dir, kind.weights),
        &Device::Cpu,
    )
    .map_err(err)?;
    let model = Whisper::load(&vb, config.clone()).map_err(err)?;
    let tokenizer = Tokenizer::from_file(models::path(dir, kind.tokenizer)).map_err(|e| e.to_string())?;
    let id = |t: &str| tokenizer.token_to_id(t).ok_or(format!("thiếu token {t}"));
    let (sot, no_timestamps, eot) = (id(w::SOT_TOKEN)?, id(w::NO_TIMESTAMPS_TOKEN)?, id(w::EOT_TOKEN)?);
    // A multilingual model is told the language and to write down what it hears, not to translate it.
    let mut prefix = vec![sot];
    if let Some(language) = kind.language {
        prefix.extend([id(language)?, id(w::TRANSCRIBE_TOKEN)?]);
    }
    prefix.push(no_timestamps);
    // Every special token and timestamp comes after <|endoftext|> in the vocabulary.
    let suppress: Vec<f32> = (0..config.vocab_size as u32)
        .map(|t| {
            let never = t > eot || config.suppress_tokens.contains(&t);
            if never { f32::NEG_INFINITY } else { 0.0 }
        })
        .collect();
    let suppress = Tensor::new(suppress.as_slice(), &Device::Cpu).map_err(err)?;
    let filters = mel_filters(config.num_mel_bins);
    Ok(Stt { model, tokenizer, config, filters, prefix, eot, suppress })
}

/// The text Whisper hears and how sure it was, from 0 to 1 (the mean probability of its tokens).
fn whisper(stt: &mut Stt, pcm: &[f32]) -> Result<(String, f32), String> {
    let err = |e: candle_core::Error| e.to_string();
    let pcm = &pcm[..pcm.len().min(w::N_SAMPLES)];
    let mel = audio::pcm_to_mel(&stt.config, pcm, &stt.filters);
    let bins = stt.config.num_mel_bins;
    let frames = mel.len() / bins;
    let mel = Tensor::from_vec(mel, (1, bins, frames), &Device::Cpu)
        .and_then(|m| m.narrow(2, 0, frames.min(w::N_FRAMES)))
        .and_then(|m| m.contiguous())
        .map_err(err)?;
    let features = stt.model.encoder.forward(&mel, true).map_err(err)?;
    let mut tokens = stt.prefix.clone();
    let mut probability = 0.0;
    let mut count = 0;
    for i in 0..MAX_TOKENS {
        let input = Tensor::new(tokens.as_slice(), &Device::Cpu).and_then(|t| t.unsqueeze(0)).map_err(err)?;
        let ys = stt.model.decoder.forward(&input, &features, i == 0).map_err(err)?;
        let last = ys.dim(1).map_err(err)? - 1;
        let logits = ys
            .i((..1, last..))
            .and_then(|y| stt.model.decoder.final_linear(&y))
            .and_then(|l| l.i(0)?.i(0))
            .and_then(|l| l.broadcast_add(&stt.suppress))
            .map_err(err)?;
        let log_probs = candle_nn::ops::log_softmax(&logits, D::Minus1)
            .and_then(|l| l.to_vec1::<f32>())
            .map_err(err)?;
        let (next, best) = log_probs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(t, &p)| (t as u32, p))
            .ok_or("không có kết quả")?;
        if next == stt.eot {
            break;
        }
        tokens.push(next);
        probability += best.exp();
        count += 1;
    }
    let text = stt
        .tokenizer
        .decode(&tokens[stt.prefix.len()..], true)
        .map_err(|e| e.to_string())?;
    Ok((text, if count == 0 { 0.0 } else { probability / count as f32 }))
}

/// The mel filter bank Whisper was trained with: librosa's Slaney scale and normalisation, 16 kHz, 400-point
/// FFT, laid out as `bins` rows of 201.
fn mel_filters(bins: usize) -> Vec<f32> {
    let (rate, fft) = (w::SAMPLE_RATE as f64, w::N_FFT);
    let columns = fft / 2 + 1;
    let (f_sp, min_log_hz, min_log_mel, step) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
    let to_mel = |hz: f64| if hz < min_log_hz { hz / f_sp } else { min_log_mel + (hz / min_log_hz).ln() / step };
    let to_hz = |mel: f64| if mel < min_log_mel { mel * f_sp } else { min_log_hz * ((mel - min_log_mel) * step).exp() };
    let top = to_mel(rate / 2.0);
    let points: Vec<f64> = (0..bins + 2).map(|i| to_hz(top * i as f64 / (bins + 1) as f64)).collect();
    let mut out = vec![0f32; bins * columns];
    for m in 0..bins {
        let norm = 2.0 / (points[m + 2] - points[m]);
        for k in 0..columns {
            let f = k as f64 * rate / fft as f64;
            let lower = (f - points[m]) / (points[m + 1] - points[m]);
            let upper = (points[m + 2] - f) / (points[m + 2] - points[m + 1]);
            out[m * columns + k] = (lower.min(upper).max(0.0) * norm) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_word_on_a_quiet_microphone_is_heard() {
        // Half a second of a faint voice in five seconds of a quiet room: too little on average, not in its loudest part.
        let mut pcm: Vec<f32> = (0..w::SAMPLE_RATE * 5).map(|i| 0.001 * (i as f32 * 0.7).sin()).collect();
        for (i, s) in pcm[w::SAMPLE_RATE * 2..w::SAMPLE_RATE * 5 / 2].iter_mut().enumerate() {
            *s = 0.03 * (i as f32 * 0.05).sin();
        }
        assert!(!silent(&pcm));
        assert!((loudness(&louder(&pcm)) - 0.1).abs() < 0.01);
        assert!(silent(&vec![0.002; w::SAMPLE_RATE * 5]));
    }

    #[test]
    fn the_word_scores_by_confidence_and_a_near_miss_by_its_sounds() {
        assert_eq!(score("hello", "Hello.", 1.0).score, 100);
        assert_eq!(score("hello", " hello", 0.5).score, 85);
        let miss = score("ship", "sheep", 0.9);
        assert_eq!(miss.score, 53);
        assert_eq!(miss.heard_ipa, "/ʃip/");
        assert_eq!(score("ship", "", 0.9).score, 0);
        assert_eq!(score("thank you", "Thank you!", 1.0).score, 100);
    }

    #[test]
    fn chinese_scores_by_syllables_and_tones() {
        // Traditional characters read as the simplified ones, and punctuation does not count.
        assert_eq!(score_chinese("学习", "學習。", 1.0).score, 100);
        let tone = score_chinese("买", "卖", 0.9);
        assert_eq!(tone.score, 40);
        assert!(tone.message.contains("thanh điệu"), "{tone:?}");
        assert_eq!((tone.heard_ipa.as_str(), tone.target_ipa.as_str()), ("mài", "mǎi"));
        assert!(score_chinese("电脑", "学习", 0.9).score < 30);
        assert_eq!(score_chinese("学习", "", 0.9).score, 0);
        let sentence = score_chinese("我们明天去北京。", "我们去北京。", 0.8);
        assert_eq!(sentence.missed, ["míng", "tiān"]);
        assert!((50..80).contains(&sentence.score), "{sentence:?}");
    }

    #[test]
    fn a_sentence_names_the_words_that_were_not_heard() {
        let s = score("We need to negotiate a better price.", "We need to go she ate a bit of price", 0.8);
        assert_eq!(s.missed, ["negotiate", "better"]);
        assert!(s.score > 0 && s.score < 80, "{s:?}");
        assert!(score("Thank you very much.", "Thank you very much", 0.9).missed.is_empty());
        assert!(score("hello", "yellow", 0.9).missed.is_empty());
    }

    #[test]
    fn mel_filters_match_librosa() {
        let f = mel_filters(80);
        assert_eq!(f.len(), 80 * 201);
        // Values read from whisper/assets/mel_filters.npz (mel_80) of openai/whisper.
        assert!((f[201 + 1] - 0.001_990_822).abs() < 1e-6, "{}", f[202]);
        let last: f32 = f[79 * 201..].iter().sum();
        assert!((last - 0.024_925_34).abs() < 1e-5, "{last}");
        let row = &f[40 * 201..41 * 201];
        let peak = row.iter().copied().fold(0.0, f32::max);
        assert!((peak - 0.014_735_566).abs() < 1e-6, "{peak}");
        assert_eq!(row.iter().position(|&v| v == peak), Some(43));
    }

    #[test]
    fn the_choice_once_downloaded_else_the_one_the_machine_prefers() {
        let all = |_: &str| true;
        assert_eq!(pick(recognizers(false, Tier::Strong), "", all), Some("stt06b"));
        assert_eq!(pick(recognizers(false, Tier::Medium), "", all), Some("stt110m"));
        assert_eq!(pick(recognizers(false, Tier::Weak), "stt", all), Some("stt"));
        assert_eq!(pick(recognizers(true, Tier::Weak), "", all), Some("sttsv"));
        // A choice that is not downloaded, or that hears the other course, is no choice.
        assert_eq!(pick(recognizers(false, Tier::Weak), "stt06b", |g| g != "stt06b"), Some("stt110m"));
        assert_eq!(pick(recognizers(false, Tier::Strong), "sttsv", all), Some("stt06b"));
        // The old Whisper still hears for a learner who has only it.
        assert_eq!(pick(recognizers(false, Tier::Strong), "", |g| g == "stt"), Some("stt"));
        assert_eq!(pick(recognizers(true, Tier::Strong), "sttsv", |g| g == "sttzh"), Some("sttzh"));
        assert_eq!(pick(recognizers(false, Tier::Weak), "", |_| false), None);
        // Each course has one suggestion on any machine, and never the old Whisper.
        assert!(suggested("sttsv") && suggested("stt110m") != suggested("stt06b"));
        assert!(!suggested("stt") && !suggested("sttzh") && !suggested("llm"));
    }

    #[test]
    fn confidence_is_the_mean_probability_of_the_tokens() {
        // Cut from what Parakeet wrote for "beautiful"; exp(-0.105361) is 0.9.
        let json = r#"{"lang": "", "emotion": "", "event": "", "text": " beautiful", "timestamps": [0.32, 0.48],
            "durations": [], "tokens":[" beaut", "iful"], "ys_log_probs": [-0.105361, 0.000000], "words": []}"#;
        let (text, confidence) = heard(json).unwrap();
        assert_eq!(text, " beautiful");
        assert!((confidence - 0.95).abs() < 1e-5, "{confidence}");
        // SenseVoice gives no probabilities; nothing heard is no confidence.
        assert_eq!(heard(r#"{"text": "学习。", "ys_log_probs": []}"#), Some(("学习。".into(), UNKNOWN_CONFIDENCE)));
        assert_eq!(heard(r#"{"text": "学习。"}"#).unwrap().1, UNKNOWN_CONFIDENCE);
        assert_eq!(heard(r#"{"text": " ", "ys_log_probs": []}"#).unwrap().1, 0.0);
        assert!(heard("{\"text\": ").is_none());
    }

    /// The 16-bit samples of a WAV file, as floats.
    fn read_wav(bytes: &[u8]) -> Vec<f32> {
        let at = bytes.windows(4).position(|w| w == b"data").expect("a data chunk") + 8;
        bytes[at..].chunks_exact(2).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect()
    }

    /// What a voice of macOS says, at 16 kHz.
    #[cfg(target_os = "macos")]
    fn say(voice: &str, text: &str) -> Vec<f32> {
        let wav = std::env::temp_dir().join(format!("card-english-test-{}-{voice}-{}.wav", std::process::id(), text.len()));
        let said = std::process::Command::new("/usr/bin/say")
            .args(["-v", voice, "--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
            .arg(&wav)
            .arg(text)
            .status()
            .unwrap();
        assert!(said.success());
        read_wav(&std::fs::read(&wav).unwrap())
    }

    /// Both Parakeets, each chosen in turn. With the archives unpacked in a folder of their own rather than in the data
    /// folder: `CARD_ENGLISH_MODELS=<folder> cargo test hears_a -- --ignored --nocapture`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the downloaded models: cargo test -- --ignored"]
    fn parakeet_hears_a_spoken_word() {
        let dir = crate::models_only();
        let db = db::open(&dir.join("words.db")).unwrap();
        let pcm = say("Samantha", "beautiful");
        for group in ["stt110m", "stt06b"] {
            db::set_setting(&db, "stt_model", group).unwrap();
            assert_eq!(chosen(&dir, "en", group), Some(group));
            let start = std::time::Instant::now();
            let right = assess("beautiful", &pcm, &dir).unwrap();
            let loading = start.elapsed();
            let start = std::time::Instant::now();
            let wrong = assess("computer", &pcm, &dir).unwrap();
            println!("{group}: {right:?} in {loading:?} with loading; {wrong:?} in {:?}", start.elapsed());
            assert!(right.score >= 80, "{right:?}");
            assert!(wrong.score < 50, "{wrong:?}");
            assert!(hear(&pcm, "en", &dir).unwrap().to_lowercase().contains("beautiful"));
        }
        assert_eq!(assess("beautiful", &vec![0.0; 16_000], &dir).unwrap().score, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the downloaded models: cargo test -- --ignored"]
    fn sensevoice_hears_a_chinese_word() {
        let dir = crate::models_only();
        assert_eq!(chosen(&dir, "zh", ""), Some("sttsv"));
        let word = say("Tingting", "学习");
        let start = std::time::Instant::now();
        let right = assess("学习", &word, &dir).unwrap();
        println!("{right:?} in {:?} with loading", start.elapsed());
        assert!(right.score >= 80, "{right:?}");
        let start = std::time::Instant::now();
        let wrong = assess("电脑", &word, &dir).unwrap();
        println!("{wrong:?} in {:?}", start.elapsed());
        assert!(wrong.score < 50, "{wrong:?}");
        let sentence = assess("我们明天去北京。", &say("Tingting", "我们明天去北京。"), &dir).unwrap();
        println!("{sentence:?}");
        assert!(sentence.score >= 75, "{sentence:?}");
    }

    #[test]
    fn a_blocked_microphone_is_told_apart_from_a_quiet_voice() {
        let dir = Path::new("/nonexistent");
        let blocked = assess("hello", &vec![0.0; 16_000], dir).unwrap();
        assert!(blocked.score == 0 && blocked.message.starts_with("Micro không gửi"), "{blocked:?}");
        let quiet: Vec<f32> = (0..16_000).map(|i| if i % 2 == 0 { 0.001 } else { -0.001 }).collect();
        assert!(assess("hello", &quiet, dir).unwrap().message.starts_with("Không nghe thấy giọng"));
        // A reply to transcribe says the same, before any model is needed.
        assert!(hear(&vec![0.0; 16_000], "en", dir).unwrap_err().starts_with("Micro không gửi"));
        assert!(hear(&quiet, "zh", dir).unwrap_err().starts_with("Không nghe thấy giọng"));
        let voice: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        assert!(hear(&voice, "zh", dir).unwrap_err().starts_with("Chưa tải model chấm phát âm tiếng Trung"));
        assert_eq!(hear(&voice, "en", dir).unwrap_err(), MISSING);
    }

    #[test]
    fn resample_keeps_the_duration() {
        let one_second = vec![0.5; 48_000];
        let out = resample(&one_second, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }
}
