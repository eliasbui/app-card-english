//! Scoring how a word was said: the microphone through cpal, Whisper tiny.en to hear it, and the phonemes of
//! what was heard against those of the word.

use crate::{models, pron};
use candle_core::{D, Device, IndexOp, Tensor};
use candle_transformers::models::whisper::{self as w, audio, quantized_model::Whisper};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokenizers::Tokenizer;

/// A recording stops by itself after this, in case the stop never comes; long enough for an example sentence.
const MAX_SECONDS: u64 = 16;
/// Quieter than this (root mean square of the samples) is taken as no voice at all.
const SILENCE_RMS: f32 = 0.008;
const MAX_TOKENS: usize = 64;

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

/// Starts listening on the default microphone. The stream lives on a thread of its own because cpal's
/// streams cannot be moved between threads on every platform.
pub fn start() -> Result<(), String> {
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
            Ok(o) => {
                let _ = ready.send(Ok(()));
                o
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
    started.recv().map_err(|_| "Không mở được micro".to_owned())??;
    *slot = Some(Recording { stop, done });
    Ok(())
}

fn open_microphone() -> Result<(cpal::Stream, Arc<Mutex<Vec<f32>>>, u32), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("Không tìm thấy micro")?;
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
    Ok((stream, samples, rate))
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
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
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

/// Hears the sound and scores it against `target`.
pub fn assess(target: &str, pcm: &[f32], dir: &Path) -> Result<Score, String> {
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
    if pcm.len() < w::SAMPLE_RATE / 4 || rms < SILENCE_RMS {
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
        let (heard, confidence) = transcribe(pcm, dir, &CHINESE)?;
        return Ok(score_chinese(target, &heard, confidence));
    }
    let (heard, confidence) = transcribe(pcm, dir, &ENGLISH)?;
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

/// The score of what was heard: the word itself scores by how sure Whisper was, anything else by how close
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
/// sure Whisper was, anything else by how close the syllables are, half with their tones and half without, so that
/// a wrong tone costs less than a wrong sound.
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

/// A Whisper model: its files, and the language it is told to hear, for one that hears more than English.
struct Model {
    group: &'static str,
    weights: &'static str,
    config: &'static str,
    tokenizer: &'static str,
    language: Option<&'static str>,
    missing: &'static str,
    loaded: &'static Mutex<Option<Stt>>,
}

static STT: Mutex<Option<Stt>> = Mutex::new(None);
static STT_ZH: Mutex<Option<Stt>> = Mutex::new(None);

static ENGLISH: Model = Model {
    group: "stt",
    weights: models::WHISPER,
    config: models::WHISPER_CONFIG,
    tokenizer: models::WHISPER_TOKENIZER,
    language: None,
    missing: "Chưa tải model chấm phát âm, vào Cài đặt để tải (42 MB)",
    loaded: &STT,
};
static CHINESE: Model = Model {
    group: "sttzh",
    weights: models::WHISPER_ZH,
    config: models::WHISPER_ZH_CONFIG,
    tokenizer: models::WHISPER_ZH_TOKENIZER,
    language: Some("<|zh|>"),
    missing: "Chưa tải model chấm phát âm tiếng Trung, vào Cài đặt để tải (45 MB)",
    loaded: &STT_ZH,
};

/// Lets go of the Whisper of a group of models in memory, before its files are deleted.
pub fn unload(group: &str) {
    for kind in [&ENGLISH, &CHINESE].into_iter().filter(|k| k.group == group) {
        *kind.loaded.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

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

fn load(dir: &Path, kind: &Model) -> Result<Stt, String> {
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

/// Loads the English Whisper ahead of the first recording and runs it once, so that the GPU kernels are compiled by
/// then; the Chinese one loads on first use.
pub fn warm_up(dir: &Path) {
    if models::ready(dir, ENGLISH.group) {
        let hum: Vec<f32> = (0..w::SAMPLE_RATE).map(|i| (i as f32 * 0.05).sin() * 0.01).collect();
        if let Err(e) = transcribe(&hum, dir, &ENGLISH) {
            eprintln!("warm up: {e}");
        }
    }
}

/// The text Whisper hears and how sure it was, from 0 to 1 (the mean probability of its tokens).
fn transcribe(pcm: &[f32], dir: &Path, kind: &Model) -> Result<(String, f32), String> {
    let err = |e: candle_core::Error| e.to_string();
    let mut guard = kind.loaded.lock().map_err(|_| "model giọng nói đang lỗi")?;
    if guard.is_none() {
        if !models::ready(dir, kind.group) {
            return Err(kind.missing.into());
        }
        *guard = Some(load(dir, kind)?);
    }
    let stt = guard.as_mut().ok_or("model chưa nạp")?;
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

    /// The 16-bit samples of a WAV file, as floats.
    fn read_wav(bytes: &[u8]) -> Vec<f32> {
        let at = bytes.windows(4).position(|w| w == b"data").expect("a data chunk") + 8;
        bytes[at..].chunks_exact(2).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect()
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the downloaded models: cargo test -- --ignored"]
    fn whisper_hears_a_spoken_word() {
        let wav = std::env::temp_dir().join("card-english-test.wav");
        let said = std::process::Command::new("/usr/bin/say")
            .args(["--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
            .arg(&wav)
            .arg("beautiful")
            .status()
            .unwrap();
        assert!(said.success());
        let pcm = read_wav(&std::fs::read(&wav).unwrap());
        let start = std::time::Instant::now();
        let right = assess("beautiful", &pcm, &crate::data_dir()).unwrap();
        println!("{right:?} in {:?}", start.elapsed());
        assert!(right.score >= 80, "{right:?}");
        let wrong = assess("computer", &pcm, &crate::data_dir()).unwrap();
        println!("{wrong:?}");
        assert!(wrong.score < 50, "{wrong:?}");
        assert_eq!(assess("beautiful", &vec![0.0; 16_000], &crate::data_dir()).unwrap().score, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the downloaded models: cargo test -- --ignored"]
    fn whisper_hears_a_chinese_word() {
        let say = |text: &str| {
            let wav = std::env::temp_dir().join(format!("card-english-test-{}.wav", text.len()));
            let said = std::process::Command::new("/usr/bin/say")
                .args(["-v", "Tingting", "--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
                .arg(&wav)
                .arg(text)
                .status()
                .unwrap();
            assert!(said.success());
            read_wav(&std::fs::read(&wav).unwrap())
        };
        let word = say("学习");
        let right = assess("学习", &word, &crate::data_dir()).unwrap();
        println!("{right:?}");
        assert!(right.score >= 80, "{right:?}");
        let wrong = assess("电脑", &word, &crate::data_dir()).unwrap();
        println!("{wrong:?}");
        assert!(wrong.score < 50, "{wrong:?}");
        let sentence = assess("我们明天去北京。", &say("我们明天去北京。"), &crate::data_dir()).unwrap();
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
    }

    #[test]
    fn resample_keeps_the_duration() {
        let one_second = vec![0.5; 48_000];
        let out = resample(&one_second, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }
}
