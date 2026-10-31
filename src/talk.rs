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
/// The same model told the language of the course, made only when the one that finds the language for itself wrote
/// down a script the learner cannot have spoken (Whisper hears Korean or Hawaiian in an accent or a noisy room).
static FORCED: Mutex<Option<(String, OfflineRecognizer)>> = Mutex::new(None);
/// One download at a time, and no delete during one.
static DOWNLOADING: Mutex<()> = Mutex::new(());

/// The turn under way; a newer turn or a stop makes the older one fall silent.
static TURN: AtomicU64 = AtomicU64::new(0);
/// What the learner had said when they paused, and its writing down under way (see `hear_early`).
type Early = (Vec<f32>, thread::JoinHandle<Result<String, String>>);
static EARLY: Mutex<Option<Early>> = Mutex::new(None);

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
    /// A line typed instead of said.
    pub typed: String,
    /// The voice chosen for the course, else the best one downloaded; the system's voice when there is none.
    pub voice: Option<voice::Voice>,
    /// The Vietnamese voice, for what the tutor says in Vietnamese; the system's when there is none.
    pub vi_voice: Option<voice::Voice>,
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
    let heard = if typed.is_empty() { hear_said(dir, &t.course)? } else { typed.to_owned() };
    post(json!({ "kind": "heard", "text": heard }));

    let question = question(&t.history, &heard);
    let system = system(&ai::level(&t.words, zh), &ai::practice_words(&t.words, 12), zh);
    let prompt = ai::Prompt { system: &system, example: if zh { SHOT_ZH } else { SHOT }, question: &question, max_tokens: 260, cjk: zh };

    let (to_say, said) = mpsc::channel::<(String, Part)>();
    let speaker = speak_all(dir.to_path_buf(), (t.voice, t.vi_voice), t.accent.clone(), t.rate, said, me);
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
    if reply.say.is_empty() && reply.vi.is_empty() {
        return Err("AI chưa trả lời được, hãy nói lại".into());
    }
    Ok(json!({ "heard": heard, "say": reply.say, "vi": reply.vi, "note": reply.note, "by": by, "spoken": spoken.err() }))
}

/// Reads the lines sent on `said` one after another, those of VI in the Vietnamese voice and the others in the voice
/// of the course, until the turn is no longer the current one. The voice writes the next sentence while the one before
/// is playing. A line with no voice of the app goes to the system's once what came before has been played. A sentence
/// that cannot be read is skipped; the first such failure is the answer.
type Player = (mpsc::Sender<(Vec<f32>, u32)>, thread::JoinHandle<Result<(), String>>);
fn speak_all(
    dir: PathBuf,
    (course, vi): (Option<voice::Voice>, Option<voice::Voice>),
    accent: String,
    rate: f32,
    said: mpsc::Receiver<(String, Part)>,
    me: u64,
) -> thread::JoinHandle<Result<(), String>> {
    let current = move || TURN.load(Ordering::Relaxed) == me;
    let finish = |player: Option<Player>| {
        player.map_or(Ok(()), |(to_play, playing)| {
            drop(to_play);
            playing.join().unwrap_or_else(|_| Err("loa bị dừng đột ngột".into()))
        })
    };
    thread::spawn(move || {
        let mut failed = Ok(());
        let mut player: Option<Player> = None;
        for (line, part) in said.iter() {
            if !current() {
                break;
            }
            let chosen = if part == Part::Vi { vi } else { course };
            match chosen.map(|v| voice::render(&dir, &v, &line, rate)) {
                Some(Ok(speech)) => {
                    let (to_play, _) = player.get_or_insert_with(|| {
                        let (to_play, pieces) = mpsc::channel();
                        (to_play, thread::spawn(move || voice::play_stream(pieces, &|| !current())))
                    });
                    let _ = to_play.send(speech);
                }
                Some(Err(e)) if e != voice::UNKNOWN_WORD => failed = failed.and(Err(e)),
                _ => {
                    failed = failed.and(finish(player.take()));
                    if current() {
                        let accent = if part == Part::Vi { "vi-VN" } else { &accent };
                        failed = failed.and(crate::speak(&line, false, accent, f64::from(rate)));
                    }
                }
            }
        }
        failed.and(finish(player.take()))
    })
}

/// Starts writing down what the learner has said so far, while the page waits out their pause to be sure they
/// have finished: Whisper takes about half a second, which the wait then hides.
pub fn hear_early(dir: &Path, course: &str) -> Result<(), String> {
    let said = speech::peek()?;
    let (dir, copy, course) = (dir.to_path_buf(), said.clone(), course.to_owned());
    let done = thread::spawn(move || hear(&copy, &dir, &course));
    *EARLY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((said, done));
    Ok(())
}

/// What the learner said: what was written down in their pause if the recording is the same one and they said
/// nothing more after it, or else the whole recording, written down now.
pub fn hear_said(dir: &Path, course: &str) -> Result<String, String> {
    let pcm = speech::stop()?;
    let early = EARLY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    if let Some((said, done)) = early
        && let Some(after) = pcm.get(said.len()..)
        && same_start(&pcm, &said)
        && speech::silent(after)
        && let Ok(Ok(text)) = done.join()
    {
        return Ok(text);
    }
    hear(&pcm, dir, course)
}

/// Whether `pcm` begins with `start`: both were made from one recording at 16 kHz, where only the last two samples
/// of the shorter one can differ.
fn same_start(pcm: &[f32], start: &[f32]) -> bool {
    let n = start.len().saturating_sub(2);
    n > 0 && pcm.get(..n) == Some(&start[..n])
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
    *FORCED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    match std::fs::remove_dir_all(file(dir, "")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Không xóa được model: {e}")),
        _ => Ok(()),
    }
}

/// Loads the models ahead of the first sentence, which would otherwise wait a second or two for each: the one that
/// hears, and the voice of the course (`chosen`) when there is one.
pub fn warm_up(dir: &Path, course: &str, chosen: Option<voice::Voice>) {
    if ready(dir)
        && let Err(e) = recognizer(dir).map(drop)
    {
        eprintln!("talk warm up: {e}");
    }
    if let Some(chosen) = chosen
        && let Err(e) = voice::render(dir, &chosen, if course == "zh" { "你好。" } else { "Hi." }, 1.0)
    {
        eprintln!("talk warm up: {e}");
    }
}

fn recognizer(dir: &Path) -> Result<std::sync::MutexGuard<'static, Option<OfflineRecognizer>>, String> {
    let mut guard = RECOGNIZER.lock().map_err(|_| "model nghe giọng nói đang lỗi")?;
    if guard.is_none() {
        // No language: Whisper finds it for each sentence. Told "vi", it turns English into made-up Vietnamese.
        *guard = Some(open(dir, None)?);
    }
    Ok(guard)
}

fn open(dir: &Path, language: Option<&str>) -> Result<OfflineRecognizer, String> {
    if !ready(dir) {
        return Err("Chưa tải model nghe giọng nói của Gia sư nói".into());
    }
    let path = |name: &str| Some(voice::narrow(&file(dir, name)));
    let config = OfflineRecognizerConfig {
        model_config: OfflineModelConfig {
            whisper: OfflineWhisperModelConfig {
                encoder: path(WHISPER[0].0),
                decoder: path(WHISPER[1].0),
                language: language.map(str::to_owned),
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
    OfflineRecognizer::create(&config).ok_or_else(|| "Không mở được model nghe giọng nói".into())
}

fn decode(whisper: &OfflineRecognizer, pcm: &[f32]) -> String {
    let stream = whisper.create_stream();
    stream.accept_waveform(16_000, pcm);
    whisper.decode(&stream);
    stream.get_result().map(|r| r.text.trim().to_owned()).unwrap_or_default()
}

/// What was said, in Vietnamese or the language of the course, from 16 kHz samples.
fn hear(pcm: &[f32], dir: &Path, course: &str) -> Result<String, String> {
    if !pcm.is_empty() && pcm.iter().all(|&s| s == 0.0) {
        return Err("Micro không gửi âm thanh nào, hãy cho phép Card English dùng micro trong phần quyền riêng tư của máy rồi thử lại".into());
    }
    if speech::silent(pcm) {
        return Err("Không nghe thấy giọng, hãy nói to và gần micro hơn".into());
    }
    // Half a second of silence after the voice, or Whisper drops the last word.
    let mut padded = speech::louder(&focus(pcm));
    padded.resize(padded.len() + 8_000, 0.0);
    let mut text = {
        let guard = recognizer(dir)?;
        decode(guard.as_ref().ok_or("model chưa nạp")?, &padded)
    };
    let zh = course == "zh";
    if !text.is_empty() && !spoken_script(&text, zh) {
        let language = if zh { "zh" } else { "en" };
        let mut forced = FORCED.lock().map_err(|_| "model nghe giọng nói đang lỗi")?;
        if forced.as_ref().is_none_or(|(l, _)| l != language) {
            *forced = Some((language.to_owned(), open(dir, Some(language))?));
        }
        text = forced.as_ref().map(|(_, whisper)| decode(whisper, &padded)).unwrap_or_default();
    }
    let text = without_made_up(&text);
    if text.is_empty() || !spoken_script(&text, zh) { Err("Chưa nghe rõ, hãy nói lại".into()) } else { Ok(text) }
}

/// The learner's voice brought forward over other people talking further away: the learner is the one nearest the
/// microphone, so the loud part of the recording. Each fiftieth of a second well under the learner's level, and not
/// within a moment of their voice, is turned down to a tenth, so that Whisper writes down the learner and not the room.
fn focus(pcm: &[f32]) -> Vec<f32> {
    const FRAME: usize = 320;
    /// A quarter of a second on each side of the voice, for the soft starts and ends of words.
    const AROUND: usize = 12;
    let rms: Vec<f32> = pcm.chunks(FRAME).map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let Some(&floor) = sorted.get(sorted.len() / 10) else { return pcm.to_vec() };
    let voiced: Vec<f32> = sorted.iter().copied().filter(|&r| r > (floor * 3.0).max(0.004)).collect();
    if voiced.len() < 5 {
        return pcm.to_vec();
    }
    // The learner's level: most of their voice is under it, a shout or a cough over it.
    let learner = voiced[voiced.len() * 4 / 5];
    let loud: Vec<bool> = rms.iter().map(|&r| r >= learner * 0.15).collect();
    let gains: Vec<f32> = (0..rms.len())
        .map(|i| if loud[i.saturating_sub(AROUND)..(i + AROUND + 1).min(rms.len())].contains(&true) { 1.0 } else { 0.1 })
        .collect();
    // Gains change gradually across each frame, so that no click is made.
    pcm.iter()
        .enumerate()
        .map(|(n, &s)| {
            let (i, at) = (n / FRAME, (n % FRAME) as f32 / FRAME as f32);
            let next = gains.get(i + 1).copied().unwrap_or(gains[i]);
            let prev = if i > 0 { gains[i - 1] } else { gains[i] };
            let g = if at < 0.5 { prev + (gains[i] - prev) * (at + 0.5) } else { gains[i] + (next - gains[i]) * (at - 0.5) };
            s * g
        })
        .collect()
}

/// Whether text is in a script the learner speaks: Latin letters, Vietnamese ones among them, and for the Chinese
/// course Chinese characters too.
fn spoken_script(text: &str, zh: bool) -> bool {
    text.chars().filter(|c| c.is_alphabetic()).all(|c| {
        matches!(c as u32, 0x41..=0x24F | 0x1E00..=0x1EFF) || (zh && matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF))
    })
}

/// Lines Whisper writes on noise or on the end of a recording, learned from the subtitles of videos: never what a
/// learner says to a tutor.
const MADE_UP: [&str; 14] = [
    "thank you for watching",
    "thanks for watching",
    "please subscribe",
    "like and subscribe",
    "subtitles by",
    "amara.org",
    "hãy subscribe",
    "đăng ký kênh",
    "ghiền mì gõ",
    "cảm ơn các bạn đã theo dõi",
    "các bạn đã xem video",
    "请不吝点赞",
    "字幕",
    "订阅",
];

/// The text without the sentences that hold one of MADE_UP.
fn without_made_up(text: &str) -> String {
    let lower = text.to_lowercase();
    if !MADE_UP.iter().any(|m| lower.contains(m)) {
        return text.trim().to_owned();
    }
    let mut kept = Vec::new();
    let mut sentence = String::new();
    for c in text.chars() {
        sentence.push(c);
        if matches!(c, '.' | '!' | '?' | '。' | '！' | '？') {
            kept.push(std::mem::take(&mut sentence));
        }
    }
    kept.push(sentence);
    kept.into_iter()
        .filter(|s| !MADE_UP.iter().any(|m| s.to_lowercase().contains(m)))
        .collect::<String>()
        .trim()
        .to_owned()
}

const SHOT: (&str, &str) = (
    "Conversation so far:\nTutor: Hi! What did you do last weekend?\nThe learner just said: \"I go to the bitch with my family.\"",
    "SAY: Oh nice, you went to the beach with your family! I love the beach too. Was the water warm?\n\
     NOTE: \"I go\" -> \"I went\": chuyện đã xảy ra thì dùng quá khứ.\n\
     NOTE: \"bitch\" -> \"beach\": âm /iː/ của beach kéo dài, căng môi như đang cười.",
);
const SHOT_ZH: (&str, &str) = (
    "Conversation so far:\nTutor: 你好！你周末做了什么？\nThe learner just said: \"我去海边了，跟我的家人。\"",
    "SAY: 哇，你跟家人去海边了！真不错。海水凉不凉？\n\
     NOTE: \"去海边了，跟我的家人\" -> \"跟家人去海边了\": trạng ngữ \"跟 ai\" đứng trước động từ.",
);

fn system(level: &str, words: &[String], zh: bool) -> String {
    let language = if zh { "Mandarin Chinese (simplified characters)" } else { "English" };
    let words = if words.is_empty() { "(none yet)".to_owned() } else { words.join(", ") };
    format!(
        "You are a warm, easy-going 1-on-1 {language} speaking tutor for a Vietnamese learner at {level} level, talking \
         with them by voice in real time. Talk the way a friendly native speaker who teaches would: a relaxed, natural \
         back-and-forth conversation, never a drill or a test. React to what they actually meant, share a little of your \
         own (an opinion, a short story), now and then sum up or rephrase their idea in your own words to show you \
         understood, and end with one open question that keeps them talking. Vary your reactions and do not praise every \
         line. Stay on the topic they chose, or bring up a new one when the talk runs dry.\n\
         Do not be strict about pronunciation or grammar, and never stop the conversation to correct them. Never ask \
         them to repeat a sentence, say it again or say it after you. When something they said was wrong or unnatural, \
         simply use the right way of saying it in your own reply, as people do in a real conversation (\"Oh, you went to \
         the beach!\"), and go on; ignore small slips. Their speech reaches you through speech recognition, which writes \
         the words it heard: a word that makes no sense where it stands was most likely mispronounced (\"bitch\" for \
         \"beach\"); understand what they meant and answer that.\n\
         Every mistake worth learning from goes, silently, in NOTE lines: they are not read aloud and not shown during \
         the talk, only in the report at the end of the lesson.\n\
         The learner may speak Vietnamese: to ask how to say something, because they are stuck, or just to chat. Then \
         help them in VI, a short reply in Vietnamese, give them the {language} words they need in SAY, and carry on \
         with the conversation; do not make them say it. When they want to stop or say goodbye, wrap up warmly in a \
         sentence or two.\n\
         SAY is read aloud by a {language} voice: one to three short sentences of natural spoken {language} at their \
         level, no lists, markdown, emoji, IPA or Vietnamese. Bring in some of the learner's words when they fit: {words}.\n\
         Reply in exactly this format, without anything else:\n\
         VI: <only when it helps, as above: one or two short sentences in Vietnamese only, read aloud by a Vietnamese \
         voice, so with no {language} words in it, those go in SAY; leave this line out otherwise>\n\
         SAY: <what you say aloud, in {language}>\n\
         NOTE: <in Vietnamese, one line per mistake: \"wrong\" -> \"right\": why, with how to make a hard sound (tongue, \
         lips, tone) for a pronunciation mistake; leave these lines out when there was none>\n\
         For example, to \"Mình muốn nói là mình bị lạc đường thì nói sao?\" you answer:\n\
         VI: Bạn nói thế này là được nè.\n\
         SAY: {}"
        ,
        if zh { "我迷路了。你是在哪里迷路的？" } else { "I'm lost. Or, I think I'm lost. Where were you going when you got lost?" }
    )
}

/// What the AI is asked: the talk so far and what the learner just said.
fn question(history: &[(String, String)], heard: &str) -> String {
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
    q.push_str(&format!("The learner just said: \"{}\"", ai::cut(heard, 500)));
    q
}

/// The parts of an answer: what was said in Vietnamese, what was said in the language of the course, and the notes
/// kept for the report.
#[derive(Debug, Default, PartialEq)]
struct Reply {
    vi: String,
    say: String,
    note: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Part {
    #[default]
    Say,
    Vi,
    Note,
}

/// The label a line begins with, if any, and the rest of it; a model may wrap the label in markdown, and one used to
/// the earlier format may still write FIX for its notes or a TRY line, kept among the notes.
fn label(line: &str) -> (Option<Part>, &str) {
    let bare = line.trim_start_matches(['*', '#', '-', ' ']);
    for (name, part) in [("SAY:", Part::Say), ("VI:", Part::Vi), ("NOTE:", Part::Note), ("FIX:", Part::Note), ("TRY:", Part::Note), ("TIP:", Part::Note)] {
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
        let (slot, joint) = match part {
            Part::Say => (&mut reply.say, ' '),
            Part::Vi => (&mut reply.vi, ' '),
            Part::Note => (&mut reply.note, '\n'),
        };
        if !rest.is_empty() {
            if !slot.is_empty() {
                slot.push(joint);
            }
            slot.push_str(rest.trim());
        }
    }
    reply
}

/// Cuts the answer, as it streams in, into the sentences to read aloud: those of VI and SAY, each with its part so
/// that it gets its voice. NOTE is kept for the report. The first line read ends at its first comma: the voice makes a
/// short line sooner, and starts on it while the rest of the answer is still coming.
#[derive(Default)]
struct Reader {
    rest: String,
    part: Part,
    started: bool,
}

impl Reader {
    fn push(&mut self, piece: &str) -> Vec<(String, Part)> {
        self.rest.push_str(piece);
        let mut out = Vec::new();
        while let Some(end) = sentence_end(&self.rest, !self.started) {
            let sentence: String = self.rest.drain(..end).collect();
            out.extend(self.take(&sentence));
            self.started |= !out.is_empty();
        }
        out
    }

    fn finish(&mut self) -> Vec<(String, Part)> {
        let last = std::mem::take(&mut self.rest);
        self.take(&last).into_iter().collect()
    }

    fn take(&mut self, sentence: &str) -> Option<(String, Part)> {
        let (found, rest) = label(sentence.trim());
        self.part = found.unwrap_or(self.part);
        let rest = rest.trim();
        let spoken = rest.chars().any(char::is_alphanumeric) && self.part != Part::Note;
        // The AI writes curly quotes, which the voices' dictionaries do not have: "That’s" was a word Kokoro cannot read.
        spoken.then(|| (rest.replace(['\u{2018}', '\u{2019}'], "'").replace(['\u{201C}', '\u{201D}'], "\""), self.part))
    }
}

/// The report at the end of a talk, in Vietnamese: what it was about, what went well, the mistakes with the right way
/// to say them, words and phrases worth keeping, sounds to work on, and a last speaking exercise of a few sentences
/// the learner can try aloud if they like.
pub fn summary(dir: &Path, settings: &ai::Settings, course: &str, history: &[(String, String)], notes: &[String], day: &str) -> Result<Value, String> {
    let zh = course == "zh";
    if !history.iter().any(|(role, _)| role == "user") {
        return Err("Hãy nói ít nhất một câu rồi mới có tổng kết".into());
    }
    let mut q = String::from("The lesson:\n");
    for (role, text) in &history[history.len().saturating_sub(40)..] {
        q.push_str(&format!("{}: {}\n", if role == "user" { "Learner" } else { "Tutor" }, ai::cut(text, 400)));
    }
    if !notes.is_empty() {
        q.push_str("Notes the tutor took during the lesson:\n");
        for note in notes.iter().rev().take(30).rev() {
            q.push_str(&format!("- {}\n", ai::cut(note, 300)));
        }
    }
    let system = summary_system(zh);
    let prompt = ai::Prompt { system: &system, example: if zh { SUMMARY_SHOT_ZH } else { SUMMARY_SHOT }, question: &q, max_tokens: 600, cjk: zh };
    let ai = ai::Ai::new(settings, dir);
    let result = ai.chat(&prompt);
    crate::record_usage(dir, day, &ai.attempts.borrow());
    let (text, by) = result?;
    let mut out = json!({ "topic": "", "good": "", "mistakes": [], "phrases": [], "sounds": [], "practice": [], "next": "", "by": by });
    for line in text.lines().map(str::trim) {
        let bare = line.trim_start_matches(['*', '#', '-', ' ']);
        let Some((key, rest)) = bare.split_once(':') else { continue };
        let rest = rest.trim_start_matches('*').trim();
        let list = || Value::from(rest.split(" ; ").map(str::trim).filter(|s| !s.is_empty() && *s != "-").collect::<Vec<_>>());
        match key.trim_end_matches('*').trim() {
            "CHUDE" => out["topic"] = rest.into(),
            "TOT" => out["good"] = rest.into(),
            "LOI" => out["mistakes"] = list(),
            "CAU" => out["phrases"] = list(),
            "PHATAM" => out["sounds"] = list(),
            "LUYEN" => out["practice"] = list(),
            "TIEP" => out["next"] = rest.into(),
            _ => {}
        }
    }
    if out["good"] == "" && out["mistakes"].as_array().is_none_or(Vec::is_empty) {
        return Err("AI chưa tổng kết được, hãy thử lại".into());
    }
    Ok(out)
}

fn summary_system(zh: bool) -> String {
    let language = if zh { "Chinese (simplified characters)" } else { "English" };
    format!(
        "You are a kind {language} speaking tutor for a Vietnamese learner. You get a voice lesson you just had with \
         them and the notes you took during it. Write the report you give at the end, in Vietnamese except for the \
         {language} words, with exactly these lines and nothing else:\n\
         CHUDE: <what the talk was about, a few words>\n\
         TOT: <two sentences on what they did well, concrete, from the lesson>\n\
         LOI: <the mistakes that matter most, at most six, grammar, word choice and pronunciation, each as: wrong words \
         -> right words (short reason), separated by ' ; ', or - if there were none>\n\
         CAU: <three to five useful {language} words or phrases that came up or that they needed, each as: phrase = \
         Vietnamese meaning, separated by ' ; '>\n\
         PHATAM: <sounds to work on, each with how to make it (tongue, lips, tone), separated by ' ; ', or - if none>\n\
         LUYEN: <the last speaking exercise: three to five natural {language} sentences they could have said in this \
         talk, with their mistakes put right, each a single sentence in {language} only, separated by ' ; '>\n\
         TIEP: <one sentence suggesting what to talk about next time>"
    )
}

const SUMMARY_SHOT: (&str, &str) = (
    "The lesson:\nTutor: Hi! What did you do last weekend?\nLearner: I go to the bitch with my family.\n\
     Tutor: Oh nice, you went to the beach with your family! Was the water warm?\nLearner: Yes, very warm. We eat many sea food.\n\
     Notes the tutor took during the lesson:\n- \"I go\" -> \"I went\": chuyện đã xảy ra thì dùng quá khứ.\n\
     - \"bitch\" -> \"beach\": âm /iː/ kéo dài.\n- \"eat many sea food\" -> \"ate a lot of seafood\": seafood không đếm được.",
    "CHUDE: Kỳ nghỉ cuối tuần ở biển\n\
     TOT: Bạn kể chuyện tự nhiên và trả lời đúng ý câu hỏi. Bạn dám nói câu dài, đó là điều rất tốt.\n\
     LOI: I go -> I went (chuyện đã qua dùng quá khứ) ; eat many sea food -> ate a lot of seafood (seafood không đếm được, dùng a lot of)\n\
     CAU: seafood = hải sản ; the water was warm = nước ấm ; spend the weekend = dành cuối tuần\n\
     PHATAM: beach /iː/: kéo dài âm i, căng môi như đang cười, khác hẳn bitch /ɪ/ ngắn\n\
     LUYEN: I went to the beach with my family. ; The water was really warm. ; We ate a lot of seafood.\n\
     TIEP: Lần tới thử kể về món ăn bạn thích nhất ở biển.",
);
const SUMMARY_SHOT_ZH: (&str, &str) = (
    "The lesson:\nTutor: 你好！你周末做了什么？\nLearner: 我去海边了，跟我的家人。\nTutor: 哇，你跟家人去海边了！海水凉不凉？\nLearner: 很凉。我们吃很多海鲜。\n\
     Notes the tutor took during the lesson:\n- \"去海边了，跟我的家人\" -> \"跟家人去海边了\": trạng ngữ đứng trước động từ.",
    "CHUDE: Đi biển cuối tuần\n\
     TOT: Bạn trả lời rõ ràng và dùng đúng 了 cho chuyện đã qua. Bạn nói tự tin, đủ ý.\n\
     LOI: 去海边了，跟我的家人 -> 跟家人去海边了 (\"跟 ai\" đứng trước động từ) ; 吃很多海鲜 -> 吃了很多海鲜 (việc đã xong thêm 了)\n\
     CAU: 海鲜 = hải sản ; 海水 = nước biển ; 周末 = cuối tuần\n\
     PHATAM: 海 hǎi thanh 3: hạ giọng xuống thấp rồi mới lên\n\
     LUYEN: 我跟家人去海边了。 ; 海水很凉。 ; 我们吃了很多海鲜。\n\
     TIEP: Lần tới thử kể về một chuyến du lịch bạn nhớ nhất.",
);

/// Where the first finished sentence of `text` ends: after a line break, or after a full stop, question or exclamation
/// mark (also the Chinese ones) that is followed by a space; with `comma`, after a comma too.
fn sentence_end(text: &str, comma: bool) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let stops: &[char] = if comma { &['.', '!', '?', ','] } else { &['.', '!', '?'] };
        let end = c == '\n'
            || matches!(c, '。' | '！' | '？')
            || (comma && c == '，')
            || (stops.contains(&c) && next.is_some_and(char::is_whitespace));
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
        for piece in ["VI: Bạn nói", " thế này nhé.\nSA", "Y: Nice", "! I went", " there too. Who", " did you go with?\nNO", "TE: \"I go\" -> \"I went\". Thêm"] {
            said.extend(r.push(piece));
        }
        said.extend(r.finish());
        assert_eq!(
            said,
            [
                ("Bạn nói thế này nhé.".into(), Part::Vi),
                ("Nice!".into(), Part::Say),
                ("I went there too.".into(), Part::Say),
                ("Who did you go with?".into(), Part::Say)
            ]
        );
        // The numbers in a sentence stay whole, and Chinese ends its sentences without a space.
        let mut r = Reader::default();
        assert_eq!(r.push("It costs 3.50 dollars. "), [("It costs 3.50 dollars.".to_owned(), Part::Say)]);
        assert_eq!(Reader::default().push("**SAY:** 你好！我们"), [("你好！".to_owned(), Part::Say)]);
        // The first line ends at a comma, the next ones do not.
        let mut r = Reader::default();
        assert_eq!(r.push("SAY: Well done, you said 1,000 words, great. "), [("Well done,".to_owned(), Part::Say), ("you said 1,000 words, great.".to_owned(), Part::Say)]);
        assert_eq!(Reader::default().push("SAY: 很好，我们"), [("很好，".to_owned(), Part::Say)]);
        assert_eq!(Reader::default().push("SAY: That\u{2019}s \u{201C}fine\u{201D}. "), [("That's \"fine\".".to_owned(), Part::Say)]);
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
            let heard = hear(&pcm, &dir, "en").unwrap();
            println!("{text} -> {heard} in {:?}", start.elapsed());
            assert!(heard.to_lowercase().contains(want), "{heard}");
        }
        // The learner near the microphone while someone further away talks over them.
        let near = say("Samantha", "I usually drink coffee with my friends in the morning.");
        let far = say("Daniel", "The weather report says it will rain all day tomorrow in the north.");
        let mixed: Vec<f32> = (0..near.len().max(far.len()))
            .map(|i| near.get(i).copied().unwrap_or(0.0) + 0.12 * far.get(i).copied().unwrap_or(0.0))
            .collect();
        let heard = hear(&mixed, &dir, "en").unwrap();
        println!("with someone else talking -> {heard}");
        assert!(heard.to_lowercase().contains("coffee") && !heard.to_lowercase().contains("weather"), "{heard}");
    }

    /// Times a real answer: its first sentence, ready to be read, and Kokoro's speech of it. Uses the AI and the voice
    /// set up in a data folder: `TALK_DATA=<data folder> TALK_AI=codex cargo test answers_quickly -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks the AI of a signed-in account"]
    fn answers_quickly() {
        let dir = PathBuf::from(std::env::var_os("TALK_DATA").expect("TALK_DATA is the data folder"));
        let settings = ai::Settings { provider: std::env::var("TALK_AI").unwrap_or("codex".into()), ..Default::default() };
        let system = system("A2", &[], false);
        let said = std::env::var("TALK_SAID").unwrap_or("Hello teacher, I want practice speak English about my job.".into());
        let question = question(&[], &said);
        let prompt = ai::Prompt { system: &system, example: SHOT, question: &question, max_tokens: 220, cjk: false };
        let start = std::time::Instant::now();
        let reader = RefCell::new(Reader::default());
        let first = RefCell::new(None);
        let on_text = |piece: &str| {
            if let Some(line) = reader.borrow_mut().push(piece).into_iter().find(|l| l.1 == Part::Say) {
                first.borrow_mut().get_or_insert((line.0, start.elapsed()));
            }
        };
        let ai = ai::Ai::new(&settings, &dir).streaming(&on_text);
        let (text, by) = ai.chat(&prompt).unwrap();
        // A short answer may have its only sentence of SAY at the end.
        let last = || reader.borrow_mut().finish().into_iter().find(|l| l.1 == Part::Say).map(|l| (l.0, start.elapsed()));
        let (line, at) = first.borrow_mut().take().or_else(last).expect("a sentence to read");
        println!("{}\nfirst sentence {line:?} after {at:?}, all after {:?}\n{text}", by.name(), start.elapsed());
        // Once to load it, as the page does when it opens, then the sentence. A word Kokoro has no pronunciation for
        // sends the sentence to the system's voice in the app; here it is only said.
        for _ in 0..2 {
            let start = std::time::Instant::now();
            match voice::generate(&dir, "en", 3, &line, 1.0) {
                Ok((samples, rate)) => println!("Kokoro: {:.1} s of speech in {:?}", samples.len() as f32 / rate as f32, start.elapsed()),
                Err(e) => println!("Kokoro: {e}"),
            }
        }
        assert!(!parse(&text).say.is_empty());
    }

    /// The report on a short talk, from the AI set up in a data folder:
    /// `TALK_DATA=<data folder> TALK_AI=codex cargo test reports_on_a_talk -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks the AI of a signed-in account"]
    fn reports_on_a_talk() {
        let dir = PathBuf::from(std::env::var_os("TALK_DATA").expect("TALK_DATA is the data folder"));
        let settings = ai::Settings { provider: std::env::var("TALK_AI").unwrap_or("codex".into()), ..Default::default() };
        let history: Vec<(String, String)> = [
            ("ai", "Hi! What do you do for work?"),
            ("user", "I am work at a bank, I am accountant."),
            ("ai", "Oh, you work at a bank as an accountant! Do you like it?"),
            ("user", "Yes, but very busy. Every day I must to check many report."),
        ]
        .map(|(r, t)| (r.to_owned(), t.to_owned()))
        .into();
        let notes = ["\"I am work\" -> \"I work\": không dùng am với động từ thường.".to_owned()];
        let report = summary(&dir, &settings, "en", &history, &notes, "").unwrap();
        println!("{report:#}");
        assert!(!report["practice"].as_array().unwrap().is_empty());
    }

    #[test]
    fn what_was_heard_in_the_pause_is_the_start_of_the_recording() {
        for rate in [48_000, 44_100] {
            let full: Vec<f32> = (0..rate as usize).map(|i| (i as f32 * 0.01).sin()).collect();
            let all = speech::resample(&full, rate, 16_000);
            assert!(same_start(&all, &speech::resample(&full[..30_001], rate, 16_000)));
            assert!(!same_start(&all, &speech::resample(&full[7..30_001], rate, 16_000)));
        }
        assert!(!same_start(&[0.1; 10], &[]));
    }

    #[test]
    fn the_parts_of_an_answer_parse() {
        let reply = parse(SHOT.1);
        assert!(reply.say.starts_with("Oh nice") && reply.say.ends_with("Was the water warm?"));
        assert_eq!(reply.note.lines().count(), 2);
        assert!(reply.note.starts_with("\"I go\" -> \"I went\""));
        assert!(reply.vi.is_empty());
        // A model that drops the labels still said something, and one that keeps the old labels keeps its notes.
        assert_eq!(parse("Great job!").say, "Great job!");
        let old = parse("SAY: Nice.\nTRY: I went there.\nFIX: \"go\" -> \"went\"");
        assert_eq!((old.say.as_str(), old.note.as_str()), ("Nice.", "I went there.\n\"go\" -> \"went\""));
    }

    #[test]
    fn the_question_holds_the_talk_so_far() {
        let q = question(&[("ai".into(), "Hi!".into())], "I want to the bitch");
        assert!(q.contains("Tutor: Hi!") && q.ends_with("\"I want to the bitch\""), "{q}");
        assert!(question(&[], "xin chào").starts_with("This is the start"));
    }

    #[test]
    fn the_learner_is_heard_over_the_room() {
        // A second of the learner, loud, then a second of someone talking further away.
        let voice = |amp: f32, hz: f32| (0..16_000).map(move |i| amp * (i as f32 * hz / 16_000.0 * std::f32::consts::TAU).sin());
        let pcm: Vec<f32> = voice(0.3, 220.0).chain(voice(0.02, 180.0)).collect();
        let out = focus(&pcm);
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        assert!((rms(&out[..12_000]) - rms(&pcm[..12_000])).abs() < 1e-4);
        assert!(rms(&out[24_000..]) < rms(&pcm[24_000..]) * 0.15);
        // A recording of one voice stays as it was.
        let one: Vec<f32> = voice(0.1, 200.0).collect();
        assert!((rms(&focus(&one)) - rms(&one)).abs() < 1e-4);
    }

    #[test]
    fn what_whisper_makes_up_and_scripts_nobody_spoke_are_left_out() {
        assert_eq!(without_made_up("I like coffee. Thank you for watching!"), "I like coffee.");
        assert_eq!(without_made_up("Hãy subscribe cho kênh Ghiền Mì Gõ để không bỏ lỡ"), "");
        assert_eq!(without_made_up("Tôi thích cà phê."), "Tôi thích cà phê.");
        assert!(spoken_script("Làm sao để nói tôi đói bằng tiếng Anh?", false));
        assert!(!spoken_script("안녕하세요", false) && !spoken_script("我饿了", false) && spoken_script("我饿了, ok", true));
    }
}
