//! What the tutor does rather than says: requests the app recognises by their wording and answers with its own
//! features, as cards the chat draws (suggested words, a quiz, buttons, numbers). A small model on this machine picks
//! the wrong tool too often to be asked which one; it only does the AI steps inside, such as suggesting words.

use crate::{
    ai::{self, Ai, By},
    db, pron, roadmap,
};
use rusqlite::Connection;
use serde_json::{json, Value};

/// The most words of one card; a larger request gets the rest through "Gợi ý thêm".
const BATCH: usize = 10;

/// A request the app answers with a feature of its own.
#[derive(Debug, PartialEq)]
pub enum Intent {
    Suggest { count: usize, topic: String },
    /// `source` as the practice page names its sources: all, hard, recent, topic:<name>.
    Quiz { count: usize, source: String },
    /// An exercise of the practice page, or several to choose from when `exercise` is empty.
    Practice { exercise: &'static str, source: String },
    Exam,
    Import(Vec<String>),
    Extract(String),
    Progress,
    Roadmap,
}

/// The answer to a request: a line for the chat, the cards under it, and who answered when the AI took part.
pub struct Act {
    pub text: String,
    pub cards: Vec<Value>,
    pub by: Option<By>,
}

/// The exercises of the practice page by the names a learner calls them; a name that holds another comes first, so
/// that "luyện nói đuổi" is taken for "nói đuổi" and not "luyện nói".
const EXERCISES: [(&str, &str, &str); 13] = [
    ("sap xep cau", "order", "Sắp xếp câu"),
    ("viet chu han", "hanzi", "Viết chữ Hán"),
    ("hoc thong minh", "smart", "Học thông minh"),
    ("chon dap an", "choice", "Chọn đáp án"),
    ("thanh dieu", "tone", "Luyện thanh điệu"),
    ("trac nghiem", "choice", "Chọn đáp án"),
    ("nghe chep", "dict", "Nghe chép"),
    ("flashcard", "flash", "Flashcard"),
    ("noi duoi", "shadow", "Nói đuổi"),
    ("luyen noi", "speak", "Luyện nói"),
    ("dien tu", "fill", "Điền từ"),
    ("noi tu", "match", "Nối từ"),
    ("dat cau", "write", "Đặt câu"),
];

/// Whether `q` speaks of words ("từ"): with marks, "từ" and not "tự"; typed without them, the bare "tu".
fn says_words(q: &str, f: &str) -> bool {
    let marked = q.chars().any(|c| ai::fold(&c.to_string()) != c.to_lowercase().to_string());
    if marked { q.to_lowercase().contains("từ") } else { f.split(|c: char| !c.is_alphanumeric()).any(|w| w == "tu") }
}

/// The first whole number of `f`, if any.
fn number(f: &str) -> Option<usize> {
    f.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok())
}

/// What follows the first of `markers` in `q`, folded or not, without the polite words that end a request.
fn after(q: &str, f: &str, markers: &[&str]) -> Option<String> {
    let at = markers.iter().filter_map(|m| f.find(m).map(|i| i + m.len())).min()?;
    // The folded text keeps the characters of the original one for one, so the place is the same.
    let rest: String = q.chars().skip(f[..at].chars().count()).collect();
    let mut words: Vec<&str> = rest.split_whitespace().collect();
    while let Some(last) = words.last() {
        let l = ai::fold(last.trim_matches(|c: char| !c.is_alphanumeric()));
        if ["nhe", "nha", "nhen", "di", "giup", "voi", "minh", "toi", "please", "a", "ạ", ""].contains(&l.as_str()) {
            words.pop();
        } else {
            break;
        }
    }
    let topic = words.join(" ").trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
    (!topic.is_empty()).then(|| topic.chars().take(40).collect())
}

/// The topic a request names: after "chủ đề", "về", "lĩnh vực", or the words following the count.
fn topic_of(q: &str, f: &str) -> String {
    if let Some(t) = after(q, f, &["chu de ", "linh vuc ", "topic ", "about ", "nganh "]) {
        return t;
    }
    if let Some(t) = after(q, f, &[" ve "]) {
        return t;
    }
    // "gợi ý 20 từ công nghệ": what follows "từ" or "từ vựng" after the count.
    let words: Vec<&str> = f.split_whitespace().collect();
    match words.iter().position(|w| *w == "tu") {
        Some(i) => {
            let skip = if words.get(i + 1) == Some(&"vung") { i + 2 } else { i + 1 };
            let marker = words[..skip].join(" ");
            after(q, f, &[&marker]).unwrap_or_default()
        }
        None => String::new(),
    }
}

/// The source of words a quiz or an exercise names.
fn source_of(q: &str, f: &str) -> String {
    if f.contains("hay quen") || f.contains("hay sai") {
        "hard".into()
    } else if f.contains("moi hoc") || f.contains("vua hoc") {
        "recent".into()
    } else if let Some(t) = after(q, f, &["chu de "]) {
        format!("topic:{t}")
    } else {
        "all".into()
    }
}

/// The feature `question` asks for, or `None` for a question to talk about.
pub fn route(question: &str) -> Option<Intent> {
    let q = question.trim();
    let f = ai::fold(q);
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    // A long text to pick words from, or a list of words to add.
    if q.chars().count() > 80 && has(&["chon tu", "lay tu", "hoc tu trong", "tim tu trong", "loc tu"]) {
        let text = q.split_once(':').map_or(q, |(_, t)| t).trim();
        return Some(Intent::Extract(text.to_owned()));
    }
    if has(&["nhap tu", "nhap cac tu", "nhap nhung tu", "import", "them cac tu", "them nhung tu", "them tu"]) {
        let list = match q.split_once(':') {
            Some((_, rest)) => rest.to_owned(),
            None => after(q, &f, &["them tu ", "nhap tu ", "import "]).unwrap_or_default(),
        };
        let words: Vec<String> = list.split([',', ';', '\n', '，', '、']).map(|w| w.trim().trim_matches('.').to_owned())
            .filter(|w| !w.is_empty() && w.chars().count() <= 40).take(50).collect();
        if !words.is_empty() {
            return Some(Intent::Import(words));
        }
    }
    if has(&["thi thu", "bai thi", "de thi", "kiem tra trinh do", "mock test"]) || (has(&["ielts", "toeic", "vstep", "hsk"]) && has(&["thi", "kiem tra"])) {
        return Some(Intent::Exam);
    }
    let exercise = EXERCISES.iter().find(|(name, ..)| f.contains(name)).map(|e| e.1);
    if has(&["bai tap", "luyen tap"]) || (exercise.is_some() && has(&["tao", "luyen", "choi", "lam", "mo", "bat dau"])) {
        return Some(Intent::Practice { exercise: exercise.unwrap_or(""), source: source_of(q, &f) });
    }
    if q.to_lowercase().contains("đố") || has(&["do toi", "do minh", "quiz", "kiem tra toi", "kiem tra tu", "on tap", "on lai"]) {
        return Some(Intent::Quiz { count: number(&f).unwrap_or(5).clamp(3, 10), source: source_of(q, &f) });
    }
    if has(&["goi y", "de xuat", "suggest", "liet ke", "cho toi", "cho minh"]) && says_words(q, &f) {
        return Some(Intent::Suggest { count: number(&f).unwrap_or(BATCH).clamp(1, 200), topic: topic_of(q, &f) });
    }
    if has(&["tien do", "thong ke", "hom nay toi hoc", "hom nay minh hoc", "chuoi ngay", "streak", "da hoc duoc", "hoc duoc bao nhieu"]) {
        return Some(Intent::Progress);
    }
    if has(&["lo trinh", "ke hoach hoc", "muc tieu cua toi", "muc tieu cua minh"]) {
        return Some(Intent::Roadmap);
    }
    None
}

/// A small random sequence for quizzes, seeded by the time: shuffling needs nothing better.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// A word as a line of a `words` card.
fn word_line(word: &str, meaning: &str, reading: &str, level: &str, in_list: bool) -> Value {
    json!({ "word": word, "meaning": meaning, "reading": reading, "level": level, "in_list": in_list })
}

/// The reading of a word: pinyin for Chinese, IPA for English.
fn reading(word: &str) -> String {
    if pron::is_chinese(word) { pron::pinyin(word) } else { pron::ipa(word) }
}

/// A `words` card of suggested words, and what is left to ask for.
pub fn words_card(title: &str, topic: &str, items: Vec<Value>, left: usize) -> Value {
    let more = if left > 0 { json!({ "topic": topic, "count": left }) } else { Value::Null };
    json!({ "type": "words", "title": title, "topic": topic, "words": items, "more": more })
}

/// Suggests `count` words on `topic` (at most a card's worth), none of the list or of `shown`.
pub fn suggest(ai: &Ai, db: &Connection, lang: &str, topic: &str, count: usize, shown: &[String]) -> Result<Act, String> {
    let words = db::list(db, "").map_err(|e| e.to_string())?;
    let mut known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
    known.extend(shown.iter().cloned());
    let learned: Vec<String> = words.iter().filter(|w| w.learned_at > 0 && w.lang == lang).map(|w| w.word.clone()).collect();
    let s = ai.suggest(topic, &learned, &known, lang == "zh", count.min(BATCH))?;
    let items: Vec<Value> = s.items.iter().map(|w| word_line(&w.word, &w.meaning, &reading(&w.word), "", false)).collect();
    let left = count.saturating_sub(items.len());
    let about = if topic.is_empty() { String::new() } else { format!(" chủ đề {topic}") };
    let text = format!("Đây là {} từ{about}. Chọn những từ bạn muốn học rồi bấm Thêm, AI sẽ giải thích từng từ.", items.len());
    Ok(Act { text, cards: vec![words_card(&format!("Từ gợi ý{about}"), topic, items, left)], by: Some(s.by) })
}

/// A quiz of `count` of the learner's own words from `source`, built here: choices of meanings or words, and the
/// word missing from its example.
fn quiz(words: &[db::Word], count: usize, source: &str, rng: &mut Rng) -> Option<Value> {
    let usable: Vec<&db::Word> = words.iter().filter(|w| !w.skipped && !w.meaning.is_empty()).collect();
    if usable.len() < 4 {
        return None;
    }
    let mut pool: Vec<&db::Word> = match source {
        "hard" => usable.iter().copied().filter(|w| w.lapses > 0 || w.difficulty >= 7.0).collect(),
        "recent" => {
            let mut v = usable.clone();
            v.sort_by_key(|w| std::cmp::Reverse(w.created));
            v.truncate(count * 2);
            v
        }
        s if s.starts_with("topic:") => usable.iter().copied().filter(|w| w.topic == s[6..]).collect(),
        _ => usable.clone(),
    };
    if pool.is_empty() {
        pool = usable.clone();
    }
    rng.shuffle(&mut pool);
    pool.truncate(count);
    let cut = |t: &str| t.chars().take(60).collect::<String>();
    let items: Vec<Value> = pool.iter().enumerate().map(|(i, w)| {
        let blank = (i % 3 == 2).then(|| blanked(&w.example, &w.word)).flatten();
        if let Some(prompt) = blank {
            return json!({ "id": w.id, "word": w.word, "meaning": cut(&w.meaning), "learned": w.learned_at > 0, "kind": "fill",
                          "prompt": prompt, "hint": cut(&w.meaning), "answer": w.word });
        }
        let to_meaning = i % 2 == 0;
        let base = json!({ "id": w.id, "word": w.word, "meaning": cut(&w.meaning), "learned": w.learned_at > 0 });
        let mut others: Vec<&db::Word> = usable.iter().copied().filter(|o| o.id != w.id && cut(&o.meaning) != cut(&w.meaning)).collect();
        rng.shuffle(&mut others);
        let pick = |o: &db::Word| if to_meaning { cut(&o.meaning) } else { o.word.clone() };
        let answer = pick(w);
        let mut options: Vec<String> = vec![answer.clone()];
        for o in others {
            if options.len() == 4 {
                break;
            }
            let option = pick(o);
            if !options.contains(&option) {
                options.push(option);
            }
        }
        rng.shuffle(&mut options);
        let prompt = if to_meaning { format!("\"{}\" nghĩa là gì?", w.word) } else { format!("Từ nào có nghĩa \"{}\"?", cut(&w.meaning)) };
        let mut item = base;
        item.as_object_mut().unwrap().extend([("kind".into(), json!("choice")), ("prompt".into(), json!(prompt)),
                                              ("options".into(), json!(options)), ("answer".into(), json!(answer))]);
        item
    }).collect();
    Some(json!({ "type": "quiz", "title": format!("Đố {} từ", items.len()), "items": items }))
}

/// `example` with `word` hidden, when it holds the word.
fn blanked(example: &str, word: &str) -> Option<String> {
    let at = example.to_lowercase().find(&word.to_lowercase())?;
    Some(format!("{}____{}", &example[..at], &example[at + word.len()..]))
}

/// A button of an `actions` card.
fn button(label: &str, run: Value) -> Value {
    json!({ "label": label, "run": run })
}

fn actions(buttons: Vec<Value>) -> Value {
    json!({ "type": "actions", "buttons": buttons })
}

/// Does what `intent` asks; `day_start` is the start of the learner's day.
pub fn act(intent: &Intent, ai: &Ai, db: &Connection, lang: &str, now: i64, day_start: i64) -> Result<Act, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let words: Vec<db::Word> = db::list(db, "").map_err(e)?.into_iter().filter(|w| w.lang == lang).collect();
    let plain = |text: String, cards: Vec<Value>| Ok(Act { text, cards, by: None });
    match intent {
        Intent::Suggest { count, topic } => suggest(ai, db, lang, topic, *count, &[]),
        Intent::Quiz { count, source } => match quiz(&words, *count, source, &mut Rng(now as u64 | 1)) {
            Some(card) => plain("Làm thử nhé, chọn đáp án hoặc gõ từ còn thiếu.".into(), vec![card]),
            None => plain("Cần ít nhất 4 từ có nghĩa để đố. Hãy thêm từ trước, ví dụ từ một bộ từ có sẵn.".into(),
                          vec![actions(vec![button("Mở bộ từ có sẵn", json!({ "kind": "packs" }))])]),
        },
        Intent::Practice { exercise, source } => {
            let named: Vec<&(&str, &str, &str)> = if exercise.is_empty() {
                EXERCISES.iter().filter(|e| ["smart", "match", "fill", "choice"].contains(&e.1)).collect()
            } else {
                EXERCISES.iter().filter(|e| e.1 == *exercise).take(1).collect()
            };
            let buttons = named.iter().map(|(_, key, name)| button(name, json!({ "kind": "practice", "exercise": key, "source": source }))).collect();
            let from = match source.as_str() {
                "hard" => " với các từ hay quên".to_owned(),
                s if s.starts_with("topic:") => format!(" với chủ đề {}", &s[6..]),
                _ => String::new(),
            };
            plain(format!("Bấm để bắt đầu{from}."), vec![actions(buttons)])
        }
        Intent::Exam => {
            let active = roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|r| r.active);
            let text = match &active {
                Some(r) => format!("Lộ trình của bạn là \"{}\". Bài thi thử theo đúng dạng đề của mục tiêu này, đủ bốn kỹ năng.", r.purpose),
                None => "Bạn chưa có lộ trình. Tạo một lộ trình có mục tiêu như IELTS, TOEIC, VSTEP hay HSK để có bài thi thử theo đúng dạng đề; còn bây giờ có thể kiểm tra vốn từ.".into(),
            };
            let mut buttons = vec![button("Kiểm tra vốn từ", json!({ "kind": "vocab_test" }))];
            buttons.insert(0, match &active {
                Some(r) => button("Làm bài thi thử", json!({ "kind": "exam", "roadmap": r.uid, "stage": -1 })),
                None => button("Tạo lộ trình", json!({ "kind": "page", "page": "roadmap" })),
            });
            plain(text, vec![actions(buttons)])
        }
        Intent::Import(list) => {
            let items: Vec<Value> = list.iter().map(|w| match words.iter().find(|x| x.word.eq_ignore_ascii_case(w)) {
                Some(x) => word_line(&x.word, &x.meaning, &x.ipa, &x.level, true),
                None => word_line(w, "", &reading(w), "", false),
            }).collect();
            let have = items.iter().filter(|i| i["in_list"] == true).count();
            let text = format!("{} từ, {have} từ đã có trong danh sách. Chọn rồi bấm Thêm, AI sẽ giải thích từng từ mới.", items.len());
            plain(text, vec![words_card("Từ để nhập", "", items, 0)])
        }
        Intent::Extract(text) => {
            let known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
            let s = ai.extract(text, &known)?;
            let items: Vec<Value> = s.items.iter().map(|w| word_line(&w.word, &w.meaning, &reading(&w.word), "", false)).collect();
            let line = format!("Mình chọn được {} từ đáng học trong đoạn này.", items.len());
            Ok(Act { text: line, cards: vec![words_card("Từ trong đoạn văn", "", items, 0)], by: Some(s.by) })
        }
        Intent::Progress => {
            let s = db::stats(db, now, day_start, lang).map_err(e)?;
            let (days, xp) = db::activity(db, 60, lang).map_err(e)?;
            let streak = days.iter().take_while(|d| d.xp > 0).count();
            let mut forgotten: Vec<&db::Word> = words.iter().filter(|w| w.lapses > 0 && !w.skipped).collect();
            forgotten.sort_by_key(|w| std::cmp::Reverse(w.lapses));
            let hard: Vec<&str> = forgotten.iter().take(3).map(|w| w.word.as_str()).collect();
            let items = vec![
                json!({ "label": "Số từ", "value": s.total.to_string() }),
                json!({ "label": "Đã thuộc", "value": s.learned.to_string() }),
                json!({ "label": "Cần ôn hôm nay", "value": s.due.to_string() }),
                json!({ "label": "Lượt ôn hôm nay", "value": s.reviews_today.to_string() }),
                json!({ "label": "Chuỗi ngày học", "value": format!("{streak} ngày") }),
                json!({ "label": "Tổng XP", "value": xp.to_string() }),
            ];
            let note = if hard.is_empty() { "Chưa có từ nào hay quên, giữ vững nhé.".to_owned() } else { format!("Từ hay quên nhất: {}.", hard.join(", ")) };
            let text = if s.due > 0 { format!("Hôm nay bạn còn {} từ cần ôn.", s.due) } else { "Hôm nay bạn đã ôn xong các từ đến hạn.".to_owned() };
            plain(text, vec![json!({ "type": "stats", "items": items, "note": note }),
                             actions(vec![button("Học ngay", json!({ "kind": "page", "page": "home" })), button("Xem thống kê", json!({ "kind": "page", "page": "stats" }))])])
        }
        Intent::Roadmap => match roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|r| r.active) {
            Some(r) => {
                let current = r.steps.iter().position(|s| !s.done).unwrap_or(r.steps.len().saturating_sub(1));
                let stage = r.steps.get(current).map(|s| format!(" Bạn đang ở chặng {}/{}: {}.", current + 1, r.steps.len(), s.title)).unwrap_or_default();
                plain(format!("Mục tiêu: {}, hạn {}.{stage}", r.purpose, r.deadline), vec![actions(vec![button("Mở lộ trình", json!({ "kind": "page", "page": "roadmap" }))])])
            }
            None => plain("Bạn chưa có lộ trình. Đặt mục tiêu và AI sẽ chia thành từng chặng.".into(),
                          vec![actions(vec![button("Tạo lộ trình", json!({ "kind": "page", "page": "roadmap" }))])]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_recognised_however_they_are_typed() {
        let suggest = |count, topic: &str| Some(Intent::Suggest { count, topic: topic.into() });
        assert_eq!(route("Gợi ý 100 từ chủ đề công nghệ nhé"), suggest(100, "công nghệ"));
        assert_eq!(route("goi y 20 tu cong nghe"), suggest(20, "cong nghe"));
        assert_eq!(route("Đề xuất từ vựng về du lịch"), suggest(10, "du lịch"));
        assert_eq!(route("cho tôi 15 từ vựng IELTS"), suggest(15, "IELTS"));
        assert_eq!(route("Đố tôi 5 từ hay quên"), Some(Intent::Quiz { count: 5, source: "hard".into() }));
        assert_eq!(route("đố mình chủ đề Du lịch"), Some(Intent::Quiz { count: 5, source: "topic:Du lịch".into() }));
        assert_eq!(route("Tạo bài tập nối từ"), Some(Intent::Practice { exercise: "match", source: "all".into() }));
        assert_eq!(route("luyện nói đuổi với từ hay quên"), Some(Intent::Practice { exercise: "shadow", source: "hard".into() }));
        assert_eq!(route("tạo bài tập"), Some(Intent::Practice { exercise: "", source: "all".into() }));
        assert_eq!(route("Cho mình làm bài thi thử IELTS"), Some(Intent::Exam));
        assert_eq!(route("Nhập các từ: resilient, cope with, abundant"), Some(Intent::Import(vec!["resilient".into(), "cope with".into(), "abundant".into()])));
        assert_eq!(route("thêm từ serendipity"), Some(Intent::Import(vec!["serendipity".into()])));
        assert_eq!(route("Tiến độ của tôi"), Some(Intent::Progress));
        assert_eq!(route("Lộ trình của mình thế nào?"), Some(Intent::Roadmap));
        let long = format!("Chọn từ đáng học trong đoạn này: {}", "Technology changes how we work and live every single day. ".repeat(2));
        assert!(matches!(route(&long), Some(Intent::Extract(t)) if t.starts_with("Technology")));
        // Questions to talk about stay with the tutor.
        for q in ["deadline nghĩa là gì?", "Dịch sang tiếng Anh: Tôi rất vui được gặp bạn.", "Viết email xin nghỉ một ngày",
                  "Khác nhau giữa affect và effect?", "IELTS writing task 2 là gì?", "Làm sao để tự học tiếng Anh?", "how do I say xin chào"] {
            assert_eq!(route(q), None, "{q}");
        }
    }

    /// `cargo test --release qwen_suggests -- --ignored --nocapture` with the downloaded models.
    #[test]
    #[ignore = "needs the downloaded Qwen models"]
    fn qwen_suggests_on_a_topic() {
        let dir = crate::models_only();
        let db = db::open(std::path::Path::new(":memory:")).unwrap();
        for local_model in ["llm", "llm15"] {
            let settings = ai::Settings { provider: "local".into(), local_model: local_model.into(), ..ai::Settings::default() };
            let ai = Ai::new(&settings, &dir);
            let intent = route("Gợi ý 100 từ chủ đề công nghệ nhé").unwrap();
            let start = std::time::Instant::now();
            let act = act(&intent, &ai, &db, "en", 1_000, 0).unwrap();
            println!("{local_model} {:?}: {}\n{}", start.elapsed(), act.text, act.cards[0]["words"]);
        }
    }

    fn word(id: i64, word: &str, meaning: &str, example: &str) -> db::Word {
        db::Word { id, word: word.into(), meaning: meaning.into(), example: example.into(), lang: "en".into(), ..db::Word::default() }
    }

    #[test]
    fn a_quiz_is_made_from_the_learners_words() {
        let words = [
            word(1, "deadline", "hạn chót", "The deadline is Friday."),
            word(2, "souvenir", "quà lưu niệm", "I bought a souvenir."),
            word(3, "itinerary", "lịch trình", "Check the itinerary."),
            word(4, "negotiate", "đàm phán", "We negotiate prices."),
            word(5, "passport", "hộ chiếu", ""),
        ];
        let card = quiz(&words, 5, "all", &mut Rng(42)).unwrap();
        let items = card["items"].as_array().unwrap();
        assert_eq!(items.len(), 5);
        for item in items {
            match item["kind"].as_str().unwrap() {
                "choice" => {
                    let options: Vec<&str> = item["options"].as_array().unwrap().iter().map(|o| o.as_str().unwrap()).collect();
                    assert_eq!(options.len(), 4);
                    assert!(options.contains(&item["answer"].as_str().unwrap()));
                    let mut distinct = options.clone();
                    distinct.dedup();
                    distinct.sort();
                    distinct.dedup();
                    assert_eq!(distinct.len(), 4, "{options:?}");
                }
                "fill" => assert!(item["prompt"].as_str().unwrap().contains("____")),
                other => panic!("{other}"),
            }
        }
        assert!(quiz(&words[..3], 5, "all", &mut Rng(1)).is_none(), "fewer than four words");
        assert_eq!(blanked("The deadline is Friday.", "Deadline").as_deref(), Some("The ____ is Friday."));
        assert_eq!(blanked("No word here.", "deadline"), None);
    }
}
