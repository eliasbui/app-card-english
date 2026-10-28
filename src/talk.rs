//! The speaking tutor: the learner talks, Whisper writes down what it heard (in Vietnamese or in the language of the
//! course), the AI answers as a teacher would, and its answer is read aloud a sentence at a time while it is still
//! being written, so that the first words come out as soon as they exist.

use crate::{ai, db, net, speech, voice};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sherpa_onnx::{OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

/// Whisper small, multilingual, int8 for sherpa-onnx: of the models tried on an M4 it alone wrote down both Vietnamese
/// with English words in it and English with a Vietnamese accent, in 0.5 to 1 s a sentence on the CPU (tiny and base
/// garble the English, the Vietnamese zipformers the English entirely). File, size and SHA-256; none is published
/// for the tokens.
const WHISPER: [(&str, u64, &str); 3] = [
    ("small-encoder.int8.onnx", 112_442_483, "4cbe7b22fa9026b843b60a68640c747de05bafb1a11b57edc0e66c232d9f33a9"),
    ("small-decoder.int8.onnx", 262_226_114, "acad50b5c782696e91b55914cc5ab4f756f1532f76e22aa6fc615f39fb69a8ee"),
    ("small-tokens.txt", 816_730, ""),
];
const SOURCE: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-small/resolve/main";
const FOLDER: &str = "whisper-small-int8";

static RECOGNIZER: Mutex<Option<OfflineRecognizer>> = Mutex::new(None);
/// One download at a time, and no delete during one.
static DOWNLOADING: Mutex<()> = Mutex::new(());

/// The turn under way; a newer turn or a stop makes the older one fall silent.
static TURN: AtomicU64 = AtomicU64::new(0);

/// Silences the tutor at once.
pub fn interrupt() {
    TURN.fetch_add(1, Ordering::Relaxed);
}

/// What a turn needs from the app.
pub struct Turn {
    pub course: String,
    pub settings: ai::Settings,
    pub words: Vec<db::Word>,
    /// The talk so far, oldest first, as ("user" or "ai", text).
    pub history: Vec<(String, String)>,
    /// The sentence the tutor asked the learner to say, last turn.
    pub target: String,
    /// A line typed instead of said.
    pub typed: String,
    /// The Kokoro voice of the course, when its pack is there; the system's voice otherwise.
    pub voice: Option<i32>,
    pub accent: String,
    pub rate: f32,
    pub day: String,
}

/// One turn: hears the learner (or takes the typed line), asks the AI and reads its answer as it comes. `post` tells
/// the page what was heard and the answer so far; the reply comes once all of it was heard.
pub fn turn(dir: &Path, t: Turn, post: impl Fn(Value)) -> Result<Value, String> {
    let me = TURN.fetch_add(1, Ordering::Relaxed) + 1;
    let zh = t.course == "zh";
    let typed = t.typed.trim();
    let heard = if typed.is_empty() { hear(&speech::stop()?, dir)? } else { typed.to_owned() };
    // The sentence to repeat, said: the words that were not heard are the ones to work on. This Whisper gives no
    // confidence, so a sentence heard whole counts as said right.
    let check = (typed.is_empty() && !t.target.is_empty())
        .then(|| if zh { speech::score_chinese(&t.target, &heard, 1.0) } else { speech::score(&t.target, &heard, 1.0) });
    post(json!({ "kind": "heard", "text": heard, "check": check }));

    let question = question(&t.history, &heard, &t.target, check.as_ref());
    let system = system(&ai::level(&t.words, zh), &ai::practice_words(&t.words, 12), zh);
    let prompt = ai::Prompt { system: &system, example: if zh { SHOT_ZH } else { SHOT }, question: &question, max_tokens: 220, cjk: zh };

    let (to_say, said) = mpsc::channel::<(String, bool)>();
    let speaker = speak_all(dir.to_path_buf(), t.course.clone(), t.voice, t.accent.clone(), t.rate, said, me);
    let reader = RefCell::new(Reader::default());
    let full = RefCell::new(String::new());
    let result = {
        let on_text = |piece: &str| {
            full.borrow_mut().push_str(piece);
            post(json!({ "kind": "text", "text": *full.borrow() }));
            for line in reader.borrow_mut().push(piece) {
                let _ = to_say.send(line);
            }
        };
        let ai = ai::Ai::new(&t.settings, dir).streaming(&on_text);
        let result = ai.chat(&prompt);
        crate::record_usage(dir, &t.day, &ai.attempts.borrow());
        result
    };
    for line in reader.borrow_mut().finish() {
        let _ = to_say.send(line);
    }
    drop(to_say);
    let spoken = speaker.join().unwrap_or_else(|_| Err("giọng đọc bị dừng đột ngột".into()));
    let (text, by) = result?;
    let reply = parse(&text);
    if reply.say.is_empty() {
        return Err("AI chưa trả lời được, hãy nói lại".into());
    }
    Ok(json!({ "heard": heard, "check": check, "say": reply.say, "try": reply.repeat, "fix": reply.fix, "by": by, "spoken": spoken.err() }))
}

/// Reads the lines sent on `said` one after another, the sentence to repeat a little slower, until the turn is no
/// longer the current one. Kokoro writes the next sentence while the one before is playing. A sentence that cannot be
/// read is skipped; the first such failure is the answer.
fn speak_all(dir: PathBuf, course: String, sid: Option<i32>, accent: String, rate: f32, said: mpsc::Receiver<(String, bool)>, me: u64) -> thread::JoinHandle<Result<(), String>> {
    let current = move || TURN.load(Ordering::Relaxed) == me;
    thread::spawn(move || {
        let mut failed = Ok(());
        let Some(sid) = sid else {
            for (line, slow) in said.iter().filter(|_| current()) {
                failed = failed.and(crate::speak(&line, slow, &accent, f64::from(rate)));
            }
            return failed;
        };
        let (to_play, pieces) = mpsc::channel();
        let player = thread::spawn(move || voice::play_stream(pieces, &|| !current()));
        for (line, slow) in said.iter().filter(|_| current()) {
            match voice::generate(&dir, &course, sid, &line, rate * if slow { 0.85 } else { 1.0 }) {
                Ok(speech) => drop(to_play.send(speech)),
                Err(e) => failed = failed.and(Err(e)),
            }
        }
        drop(to_play);
        player.join().unwrap_or_else(|_| Err("loa bị dừng đột ngột".into())).and(failed)
    })
}

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join("models").join(FOLDER).join(name)
}

fn intact(dir: &Path, name: &str, size: u64) -> bool {
    std::fs::metadata(file(dir, name)).is_ok_and(|m| m.is_file() && m.len() == size)
}

pub fn ready(dir: &Path) -> bool {
    WHISPER.iter().all(|&(name, size, _)| intact(dir, name, size))
}

/// Whether the model that hears the learner is there, and how big its download is.
pub fn status(dir: &Path) -> Value {
    json!({ "ready": ready(dir), "size": WHISPER.iter().map(|f| f.1).sum::<u64>() })
}

/// Downloads the files that are missing, each checked against its size and hash, reporting the bytes done.
pub fn download(dir: &Path, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let _one_at_a_time = DOWNLOADING.lock().map_err(|_| "Đang tải model bị lỗi")?;
    std::fs::create_dir_all(file(dir, "")).map_err(|e| e.to_string())?;
    let total = WHISPER.iter().map(|f| f.1).sum();
    let mut done = 0;
    for (name, size, sha256) in WHISPER {
        let to = file(dir, name);
        if !intact(dir, name, size) {
            let _ = std::fs::remove_file(&to);
            net::download(&format!("{SOURCE}/{name}"), &to, |bytes| progress(done + bytes.min(size), total))?;
            if !intact(dir, name, size) || (!sha256.is_empty() && hash(&to)? != sha256) {
                let _ = std::fs::remove_file(&to);
                return Err(format!("File {name} tải về bị lỗi, hãy thử lại"));
            }
        }
        done += size;
        progress(done, total);
    }
    Ok(())
}

fn hash(path: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path).map_err(|e| e.to_string())?, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Removes the model, letting go of it first: Windows does not delete a file that is open.
pub fn delete(dir: &Path) -> Result<(), String> {
    let _not_downloading = DOWNLOADING.try_lock().map_err(|_| "Đang tải model, hãy chờ tải xong rồi xóa")?;
    *RECOGNIZER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    match std::fs::remove_dir_all(file(dir, "")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Không xóa được model: {e}")),
        _ => Ok(()),
    }
}

/// Loads the models ahead of the first sentence, which would otherwise wait a second or two for each: the one that
/// hears, and the voice of the course (`voice`, its Kokoro speaker) when there is one.
pub fn warm_up(dir: &Path, course: &str, voice: Option<i32>) {
    if ready(dir)
        && let Err(e) = recognizer(dir).map(drop)
    {
        eprintln!("talk warm up: {e}");
    }
    if let Some(sid) = voice
        && let Err(e) = voice::generate(dir, course, sid, "Hi.", 1.0)
    {
        eprintln!("talk warm up: {e}");
    }
}

fn recognizer(dir: &Path) -> Result<std::sync::MutexGuard<'static, Option<OfflineRecognizer>>, String> {
    let mut guard = RECOGNIZER.lock().map_err(|_| "model nghe giọng nói đang lỗi")?;
    if guard.is_none() {
        if !ready(dir) {
            return Err("Chưa tải model nghe giọng nói của Gia sư nói".into());
        }
        let path = |name: &str| Some(voice::narrow(&file(dir, name)));
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                whisper: OfflineWhisperModelConfig {
                    encoder: path(WHISPER[0].0),
                    decoder: path(WHISPER[1].0),
                    // No language: Whisper finds it for each sentence. Told "vi", it turns English into made-up Vietnamese.
                    language: None,
                    // What its own help suggests for a multilingual model; 100 already made it hear Hawaiian.
                    tail_paddings: 300,
                    ..Default::default()
                },
                tokens: path(WHISPER[2].0),
                num_threads: thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4) as i32),
                ..Default::default()
            },
            ..Default::default()
        };
        *guard = Some(OfflineRecognizer::create(&config).ok_or("Không mở được model nghe giọng nói")?);
    }
    Ok(guard)
}

/// What was said, in whichever language, from 16 kHz samples.
fn hear(pcm: &[f32], dir: &Path) -> Result<String, String> {
    if !pcm.is_empty() && pcm.iter().all(|&s| s == 0.0) {
        return Err("Micro không gửi âm thanh nào, hãy cho phép Card English dùng micro trong phần quyền riêng tư của máy rồi thử lại".into());
    }
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
    if pcm.len() < 4_000 || rms < 0.004 {
        return Err("Không nghe thấy giọng, hãy nói to và gần micro hơn".into());
    }
    let guard = recognizer(dir)?;
    let whisper = guard.as_ref().ok_or("model chưa nạp")?;
    // Half a second of silence after the voice, or Whisper drops the last word.
    let mut padded = pcm.to_vec();
    padded.resize(pcm.len() + 8_000, 0.0);
    let stream = whisper.create_stream();
    stream.accept_waveform(16_000, &padded);
    whisper.decode(&stream);
    let text = stream.get_result().map(|r| r.text.trim().to_owned()).unwrap_or_default();
    if text.is_empty() { Err("Chưa nghe rõ, hãy nói lại".into()) } else { Ok(text) }
}

const SHOT: (&str, &str) = (
    "Conversation so far:\nTutor: Hi! What did you do last weekend?\nThe learner just said: \"I go to the bitch with my family.\"",
    "SAY: The beach sounds lovely! Just one thing: it was last weekend, so we say I went, not I go. Who did you go with?\n\
     TRY: I went to the beach with my family.\n\
     FIX: \"I go\" -> \"I went\": chuyện đã xảy ra thì dùng quá khứ. \"beach\" nghe thành \"bitch\": âm /iː/ của beach phải kéo dài, căng môi như đang cười.",
);
const SHOT_ZH: (&str, &str) = (
    "Conversation so far:\nTutor: 你好！你周末做了什么？\nThe learner just said: \"我去海边了，跟我的家人。\"",
    "SAY: 海边很好玩！我们一般说：我跟家人去海边了。你们玩了什么？\n\
     TRY: 我跟家人去海边了。\n\
     FIX: \"去海边了，跟我的家人\" -> \"跟家人去海边了\": trạng ngữ \"跟 ai\" đứng trước động từ. \"家人\" jiārén: thanh 1 rồi thanh 2, giữ giọng cao đều ở jiā.",
);

fn system(level: &str, words: &[String], zh: bool) -> String {
    let language = if zh { "Mandarin Chinese (simplified characters)" } else { "English" };
    let words = if words.is_empty() { "(none yet)".to_owned() } else { words.join(", ") };
    format!(
        "You are a warm, patient 1-on-1 {language} speaking tutor for a Vietnamese learner at {level} level, talking with \
         them by voice in real time. The learner's speech reaches you through speech recognition, which writes the words \
         it heard: a word that makes no sense where it stands was most likely mispronounced (\"bitch\" for \"beach\"), \
         so treat it as a pronunciation mistake. The app may also say how well they said the sentence you asked them to \
         repeat. They may speak Vietnamese to ask something, such as how to say a phrase; help with that too. \
         Each turn, kindly correct the one or two mistakes that matter most (pronunciation, grammar or word choice), \
         then keep the conversation going with a question. Speak simple, natural {language} at their level. SAY is read \
         aloud: at most three short sentences, no lists, markdown, emoji, IPA or Vietnamese. Bring in some of the \
         learner's words when they fit: {words}. Reply in exactly this format, without anything else:\n\
         SAY: <what you say aloud, in {language}>\n\
         TRY: <one short sentence for the learner to say, usually their sentence corrected; leave this line out when \
         there is nothing to practise>\n\
         FIX: <in Vietnamese: each mistake as \"wrong\" -> \"right\": why, with how to make a hard sound (tongue, lips, \
         tone); leave this line out when there was no mistake>"
    )
}

/// What the AI is asked: the talk so far and what the learner just said.
fn question(history: &[(String, String)], heard: &str, target: &str, check: Option<&speech::Score>) -> String {
    let mut q = String::new();
    let recent = &history[history.len().saturating_sub(12)..];
    if recent.is_empty() {
        q.push_str("This is the start of the lesson.\n");
    } else {
        q.push_str("Conversation so far:\n");
        for (role, text) in recent {
            q.push_str(&format!("{}: {}\n", if role == "user" { "Learner" } else { "Tutor" }, ai::cut(text, 300)));
        }
    }
    if let Some(check) = check {
        q.push_str(&format!("You asked the learner to say: \"{target}\". The app scored it {}/100", check.score));
        if !check.missed.is_empty() {
            q.push_str(&format!(", and did not hear: {}", check.missed.join(", ")));
        }
        q.push_str(".\n");
    }
    q.push_str(&format!("The learner just said: \"{}\"", ai::cut(heard, 500)));
    q
}

/// The parts of an answer: what was said aloud, the sentence to repeat and the notes in Vietnamese.
#[derive(Debug, Default, PartialEq)]
struct Reply {
    say: String,
    repeat: String,
    fix: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Part {
    #[default]
    Say,
    Try,
    Fix,
}

/// The label a line begins with, if any, and the rest of it; a model may wrap the label in markdown.
fn label(line: &str) -> (Option<Part>, &str) {
    let bare = line.trim_start_matches(['*', '#', '-', ' ']);
    for (name, part) in [("SAY:", Part::Say), ("TRY:", Part::Try), ("FIX:", Part::Fix)] {
        if let Some(rest) = bare.strip_prefix(name) {
            return (Some(part), rest.trim_start_matches(['*', ' ']));
        }
    }
    (None, line)
}

fn parse(text: &str) -> Reply {
    let mut reply = Reply::default();
    let mut part = Part::Say;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (found, rest) = label(line);
        part = found.unwrap_or(part);
        let slot = match part {
            Part::Say => &mut reply.say,
            Part::Try => &mut reply.repeat,
            Part::Fix => &mut reply.fix,
        };
        if !rest.is_empty() {
            if !slot.is_empty() {
                slot.push('\n');
            }
            slot.push_str(rest.trim());
        }
    }
    reply
}

/// Cuts the answer, as it streams in, into the sentences to read aloud: those of SAY, then TRY (marked to be read
/// slower). FIX is only shown.
#[derive(Default)]
struct Reader {
    rest: String,
    part: Part,
}

impl Reader {
    fn push(&mut self, piece: &str) -> Vec<(String, bool)> {
        self.rest.push_str(piece);
        let mut out = Vec::new();
        while let Some(end) = sentence_end(&self.rest) {
            let sentence: String = self.rest.drain(..end).collect();
            out.extend(self.take(&sentence));
        }
        out
    }

    fn finish(&mut self) -> Vec<(String, bool)> {
        let last = std::mem::take(&mut self.rest);
        self.take(&last).into_iter().collect()
    }

    fn take(&mut self, sentence: &str) -> Option<(String, bool)> {
        let (found, rest) = label(sentence.trim());
        self.part = found.unwrap_or(self.part);
        let rest = rest.trim();
        let spoken = rest.chars().any(char::is_alphanumeric) && self.part != Part::Fix;
        spoken.then(|| (rest.to_owned(), self.part == Part::Try))
    }
}

/// Where the first finished sentence of `text` ends: after a line break, or after a full stop, question or exclamation
/// mark (also the Chinese ones) that is followed by a space.
fn sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let end = c == '\n' || matches!(c, '。' | '！' | '？') || (matches!(c, '.' | '!' | '?') && next.is_some_and(char::is_whitespace));
        if end {
            return Some(at + c.len_utf8());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_read_a_sentence_at_a_time_as_it_streams() {
        let mut r = Reader::default();
        let mut said = Vec::new();
        for piece in ["SA", "Y: Nice", "! I went", " there too. Who", " did you go with?\nTR", "Y: I went to the", " beach.\nFIX: \"I go\" -> \"I went\". Thêm"] {
            said.extend(r.push(piece));
        }
        said.extend(r.finish());
        assert_eq!(
            said,
            [("Nice!".into(), false), ("I went there too.".into(), false), ("Who did you go with?".into(), false), ("I went to the beach.".into(), true)]
        );
        // The numbers in a sentence stay whole, and Chinese ends its sentences without a space.
        let mut r = Reader::default();
        assert_eq!(r.push("It costs 3.50 dollars. "), [("It costs 3.50 dollars.".to_owned(), false)]);
        assert_eq!(Reader::default().push("**SAY:** 你好！我们"), [("你好！".to_owned(), false)]);
    }

    /// Hears Vietnamese, English and both mixed, said by the system's voices. Run it by hand with the model in a data
    /// folder: `TALK_DIR=<data folder> cargo test hears_vietnamese -- --ignored --nocapture`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the downloaded model"]
    fn hears_vietnamese_and_english() {
        let dir = PathBuf::from(std::env::var_os("TALK_DIR").expect("TALK_DIR is the data folder"));
        assert!(ready(&dir));
        let say = |voice: &str, text: &str| {
            let wav = std::env::temp_dir().join(format!("card-english-talk-{}.wav", text.len()));
            let said = std::process::Command::new("/usr/bin/say")
                .args(["-v", voice, "--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
                .arg(&wav)
                .arg(text)
                .status()
                .unwrap();
            assert!(said.success());
            let bytes = std::fs::read(&wav).unwrap();
            let at = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
            bytes[at..].chunks_exact(2).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect::<Vec<f32>>()
        };
        warm_up(&dir, "en", None);
        for (voice, text, want) in [
            ("Samantha", "I want to improve my English pronunciation.", "pronunciation"),
            ("Linh", "Làm sao để nói tôi đói bằng tiếng Anh?", "tiếng anh"),
            ("Linh", "Hôm nay tôi muốn luyện nói về du lịch.", "du lịch"),
        ] {
            let pcm = say(voice, text);
            let start = std::time::Instant::now();
            let heard = hear(&pcm, &dir).unwrap();
            println!("{text} -> {heard} in {:?}", start.elapsed());
            assert!(heard.to_lowercase().contains(want), "{heard}");
        }
    }

    /// Times a real answer: its first sentence, ready to be read, and Kokoro's speech of it. Uses the AI and the voice
    /// set up in a data folder: `TALK_DATA=<data folder> TALK_AI=codex cargo test answers_quickly -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks the AI of a signed-in account"]
    fn answers_quickly() {
        let dir = PathBuf::from(std::env::var_os("TALK_DATA").expect("TALK_DATA is the data folder"));
        let settings = ai::Settings { provider: std::env::var("TALK_AI").unwrap_or("codex".into()), ..Default::default() };
        let system = system("A2", &[], false);
        let question = question(&[], "Hello teacher, I want practice speak English about my job.", "", None);
        let prompt = ai::Prompt { system: &system, example: SHOT, question: &question, max_tokens: 220, cjk: false };
        let start = std::time::Instant::now();
        let reader = RefCell::new(Reader::default());
        let first = RefCell::new(None);
        let on_text = |piece: &str| {
            if let Some(line) = reader.borrow_mut().push(piece).into_iter().next() {
                first.borrow_mut().get_or_insert((line.0, start.elapsed()));
            }
        };
        let ai = ai::Ai::new(&settings, &dir).streaming(&on_text);
        let (text, by) = ai.chat(&prompt).unwrap();
        let (line, at) = first.borrow_mut().take().expect("a sentence before the end");
        println!("{}\nfirst sentence {line:?} after {at:?}, all after {:?}\n{text}", by.name(), start.elapsed());
        // Once to load it, as the page does when it opens, then the sentence.
        for _ in 0..2 {
            let start = std::time::Instant::now();
            let (samples, rate) = voice::generate(&dir, "en", 3, &line, 1.0).unwrap();
            println!("Kokoro: {:.1} s of speech in {:?}", samples.len() as f32 / rate as f32, start.elapsed());
        }
        assert!(!parse(&text).say.is_empty());
    }

    #[test]
    fn the_parts_of_an_answer_parse() {
        let reply = parse(SHOT.1);
        assert!(reply.say.starts_with("The beach sounds lovely!") && reply.say.ends_with("Who did you go with?"));
        assert_eq!(reply.repeat, "I went to the beach with my family.");
        assert!(reply.fix.starts_with("\"I go\" -> \"I went\""));
        // A model that drops the labels still said something.
        assert_eq!(parse("Great job!").say, "Great job!");
    }

    #[test]
    fn the_question_tells_how_the_sentence_to_repeat_went() {
        let check = speech::score("I went to the beach.", "I want to the bitch", 0.7);
        let q = question(&[("ai".into(), "Hi!".into())], "I want to the bitch", "I went to the beach.", Some(&check));
        assert!(q.contains("Tutor: Hi!") && q.contains("did not hear: went, beach"), "{q}");
        assert!(question(&[], "xin chào", "", None).starts_with("This is the start"));
    }
}
