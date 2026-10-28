//! Ready-made word packs and the vocabulary size test. English: the New General Service List, the New Academic
//! Word List and the TOEIC Service List, by frequency (assets/ngsl-LICENSE); Chinese: the words of each HSK 3.0
//! level, in the order of the standard (assets/hsk30-LICENSE).

use crate::{ai, db};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

const NGSL: &str = include_str!("../assets/ngsl.txt");
const NAWL: &str = include_str!("../assets/nawl.txt");
const TSL: &str = include_str!("../assets/tsl.txt");
/// `level word` per line, the advanced levels 7 to 9 as 7, made by scripts/make-packs.py.
const HSK: &str = include_str!("../assets/hsk30.txt");
/// The most words one run adds.
const MAX_ADD: usize = 50;
/// A band is known, and a pack may start after it, when this share of it is known after the correction for guessing.
const KNOWN: f64 = 0.8;
/// An HSK level counts as reached at this share.
const REACHED: f64 = 0.6;
/// Made-up words that look real, to see how often a "yes" is a guess. None is in CMUdict or CC-CEDICT.
const FAKE_EN: [&str; 8] = ["frodicate", "slenture", "glamber", "rendicle", "carbitude", "morplish", "dorbance", "trelative"];
const FAKE_ZH: [&str; 7] = ["书饭", "门笑", "茶跑", "鱼读", "花睡", "雨桌", "灯听"];

static CANCEL: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct Pack {
    pub id: &'static str,
    /// "en" or "zh".
    pub lang: &'static str,
    pub name: String,
    pub about: String,
    /// The words in the order they are learned: by frequency, or as the standard lists them.
    pub words: Vec<&'static str>,
    /// The level a word of the pack is at, when the pack says, such as `HSK3`.
    pub level: &'static str,
}

/// Every pack of both courses.
fn all() -> &'static [Pack] {
    static PACKS: OnceLock<Vec<Pack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        let list = |text: &'static str| text.lines().map(str::trim).filter(|w| !w.is_empty()).collect::<Vec<_>>();
        let mut packs = vec![
            Pack {
                id: "ngsl",
                lang: "en",
                name: "Từ thông dụng (NGSL)".into(),
                about: "Những từ hay gặp nhất của tiếng Anh, từ dễ đến khó theo tần suất.".into(),
                words: list(NGSL),
                level: "",
            },
            Pack {
                id: "nawl",
                lang: "en",
                name: "Từ học thuật (NAWL)".into(),
                about: "Từ hay gặp trong sách, bài giảng và bài báo khoa học. Nên học sau NGSL.".into(),
                words: list(NAWL),
                level: "",
            },
            Pack {
                id: "tsl",
                lang: "en",
                name: "Từ TOEIC (TSL)".into(),
                about: "Từ hay gặp trong bài thi TOEIC và công việc văn phòng. Nên học sau NGSL.".into(),
                words: list(TSL),
                level: "",
            },
        ];
        const LEVELS: [(&str, &str); 7] =
            [("hsk1", "HSK1"), ("hsk2", "HSK2"), ("hsk3", "HSK3"), ("hsk4", "HSK4"), ("hsk5", "HSK5"), ("hsk6", "HSK6"), ("hsk7", "HSK7")];
        for (id, level) in LEVELS {
            let n = &level[3..];
            let words = HSK.lines().filter_map(|l| l.split_once(' ')).filter(|(l, _)| *l == n).map(|(_, w)| w.trim()).collect();
            let (name, about) = if n != "7" {
                (format!("HSK {n}"), format!("Từ vựng cấp {n} theo chuẩn HSK 3.0 (2021)."))
            } else {
                ("HSK 7-9".into(), "Từ vựng cấp cao, từ cấp 7 đến cấp 9 theo chuẩn HSK 3.0 (2021).".into())
            };
            packs.push(Pack { id, lang: "zh", name, about, words, level });
        }
        packs
    })
}

pub fn find(id: &str) -> Option<&'static Pack> {
    all().iter().find(|p| p.id == id)
}

/// Every word in the list, of both courses, in lower case.
fn listed(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = db.prepare("SELECT word FROM words")?;
    stmt.query_map([], |r| r.get::<_, String>(0))?.map(|w| w.map(|w| w.to_lowercase())).collect()
}

/// Where the next words of a pack start: the words before it were added, or known when the learner took the test.
pub fn position(db: &Connection, pack: &Pack) -> usize {
    db::setting(db, &format!("pack_{}", pack.id)).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0).min(pack.words.len())
}

pub fn set_position(db: &Connection, id: &str, at: usize) -> Result<(), String> {
    let pack = find(id).ok_or("Không có bộ từ này")?;
    db::set_setting(db, &format!("pack_{}", pack.id), &at.min(pack.words.len()).to_string()).map_err(|e| e.to_string())
}

/// The next `n` words of a pack from `from` on that are not in the list yet, with where each is in the pack.
fn next(pack: &Pack, from: usize, listed: &HashSet<String>, n: usize) -> Vec<(usize, &'static str)> {
    pack.words.iter().copied().enumerate().skip(from).filter(|(_, w)| !listed.contains(&w.to_lowercase())).take(n).collect()
}

/// The packs of a course and how far the learner is in each: the words already in the list, where the next ones
/// start, how many are left and the first few of them.
pub fn status(db: &Connection, lang: &str) -> rusqlite::Result<Value> {
    let listed = listed(db)?;
    let packs: Vec<Value> = all()
        .iter()
        .filter(|p| p.lang == lang)
        .map(|p| {
            let at = position(db, p);
            json!({
                "id": p.id,
                "name": p.name,
                "about": p.about,
                "size": p.words.len(),
                "have": p.words.iter().filter(|w| listed.contains(&w.to_lowercase())).count(),
                "pos": at,
                "left": next(p, at, &listed, usize::MAX).len(),
                "next": next(p, at, &listed, 6).into_iter().map(|(_, w)| w).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "packs": packs, "running": RUNNING.load(Ordering::SeqCst) }))
}

/// Stops a run after the word it is on.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Adds the next `n` words of a pack to the list one after another, each explained by the AI, and tells `progress`
/// before each. A word the AI cannot explain is left out and reported; the pack then starts at it next time, so
/// that it is tried again.
pub fn add(dir: &Path, settings: &ai::Settings, id: &str, n: usize, day: &str, progress: impl Fn(Value)) -> Result<Value, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Đang thêm từ của một bộ khác, hãy chờ xong hoặc bấm Dừng".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let result = run(dir, settings, id, n.clamp(1, MAX_ADD), day, progress);
    RUNNING.store(false, Ordering::SeqCst);
    result
}

fn run(dir: &Path, settings: &ai::Settings, id: &str, n: usize, day: &str, progress: impl Fn(Value)) -> Result<Value, String> {
    let pack = find(id).ok_or("Không có bộ từ này")?;
    let db = db::open(&dir.join("words.db")).map_err(|e| e.to_string())?;
    let from = position(&db, pack);
    let todo = next(pack, from, &listed(&db).map_err(|e| e.to_string())?, n);
    let ai = ai::Ai::new(settings, dir);
    let (mut added, mut failed) = (Vec::new(), Vec::new());
    let mut cancelled = false;
    // Past the last word done, unless the run stopped early or a word failed.
    let mut until = todo.last().map_or(from, |(i, _)| i + 1);
    for (done, &(i, word)) in todo.iter().enumerate() {
        if CANCEL.load(Ordering::SeqCst) {
            cancelled = true;
            until = until.min(i);
            break;
        }
        progress(json!({ "pack": pack.id, "done": done, "total": todo.len(), "word": word }));
        // The learner may have added it meanwhile.
        if db::find(&db, word).ok().flatten().is_some() {
            continue;
        }
        match ai.explain(word).and_then(|e| save(&db, pack, word, e)) {
            Ok(()) => added.push(word),
            Err(error) => {
                until = until.min(i);
                failed.push(json!({ "word": word, "error": error }));
            }
        }
    }
    progress(json!({ "pack": pack.id, "done": todo.len(), "total": todo.len(), "word": "" }));
    crate::record_usage(dir, day, &ai.attempts.borrow());
    set_position(&db, pack.id, until.max(from))?;
    Ok(json!({ "pack": pack.id, "added": added, "failed": failed, "cancelled": cancelled }))
}

fn save(db: &Connection, pack: &Pack, word: &str, e: ai::Explain) -> Result<(), String> {
    let level = if pack.level.is_empty() { crate::valid_level(&e.level) } else { pack.level.to_owned() };
    let w = db::Word {
        word: word.to_owned(),
        pos: e.pos,
        meaning: e.meaning,
        explain: e.explain,
        example: e.example,
        example_vi: e.example_vi,
        synonyms: e.synonyms,
        antonyms: e.antonyms,
        collocations: e.collocations,
        level,
        topic: e.topic,
        hanviet: e.hanviet,
        example_reading: e.example_reading,
        ..db::Word::default()
    };
    db::save(db, &w, crate::now()).map(|_| ()).map_err(|e| e.to_string())
}

/// A part of a pack the test samples.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Band {
    pack: &'static str,
    start: usize,
    end: usize,
    items: usize,
}

/// The bands of a course: the general list in bands of 500 by frequency, each other list and each HSK level a band.
fn bands(lang: &str) -> Vec<Band> {
    let mut out = Vec::new();
    for p in all().iter().filter(|p| p.lang == lang) {
        let step = if p.id == "ngsl" { 500 } else { p.words.len() };
        let mut start = 0;
        while start < p.words.len() {
            let end = (start + step).min(p.words.len());
            out.push(Band { pack: p.id, start, end, items: if lang == "zh" { 7 } else { 6 } });
            start = end;
        }
    }
    out
}

/// A small random generator for picking the words of a test, seeded by the system.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let mut b = [0u8; 8];
        let seed = getrandom::fill(&mut b).map(|_| u64::from_le_bytes(b)).unwrap_or_else(|_| crate::now() as u64);
        Self(seed | 1)
    }

    /// xorshift64*: below `n`.
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n.max(1)
    }
}

/// The words of a test of a course, band after band: each with the index of its band, or none for a made-up word.
/// The page asks them in a shuffled order.
pub fn test(lang: &str) -> Vec<Value> {
    let mut rng = Rng::new();
    let mut items = Vec::new();
    for (b, band) in bands(lang).iter().enumerate() {
        let Some(pack) = find(band.pack) else { continue };
        let mut pool: Vec<usize> = (band.start..band.end).collect();
        for _ in 0..band.items.min(pool.len()) {
            let i = pool.swap_remove(rng.below(pool.len()));
            items.push(json!({ "word": pack.words[i], "band": b }));
        }
    }
    let fakes: &[&str] = if lang == "zh" { &FAKE_ZH } else { &FAKE_EN };
    items.extend(fakes.iter().map(|w| json!({ "word": w, "band": null })));
    items
}

#[derive(Debug, Serialize, PartialEq)]
pub struct BandResult {
    pub pack: &'static str,
    pub start: usize,
    pub end: usize,
    /// The share of the band known, corrected for guessing.
    pub known: f64,
}

/// Where a pack can start after the bands the learner knows.
#[derive(Debug, Serialize, PartialEq)]
pub struct Move {
    pub pack: &'static str,
    pub to: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Estimate {
    pub lang: String,
    pub bands: Vec<BandResult>,
    /// The share of the made-up words answered "known".
    pub guessed: f64,
    /// English: the words of the NGSL known; Chinese: the words of HSK 1 to 9.
    pub words: usize,
    /// "A1" to "C1", or "HSK 1" to "HSK 7-9", or empty below HSK 1.
    pub level: String,
    /// English: the shares of the academic and the TOEIC words known.
    pub academic: f64,
    pub toeic: f64,
    pub moves: Vec<Move>,
}

/// The share of a band known: the hits less what guessing explains, as in yes/no tests, where the false alarms on
/// made-up words estimate how often a "yes" is a guess.
fn corrected(hits: f64, false_alarms: f64) -> f64 {
    if false_alarms >= 1.0 { 0.0 } else { ((hits - false_alarms) / (1.0 - false_alarms)).clamp(0.0, 1.0) }
}

/// Rough CEFR levels by how many of the NGSL's 2,800 words are known, after the vocabulary sizes usually given for
/// each level; C1 also takes most of the academic words.
fn cefr(words: usize, academic: f64) -> &'static str {
    match words {
        _ if words >= 2500 && academic >= 0.7 => "C1",
        2300.. => "B2",
        1500.. => "B1",
        800.. => "A2",
        _ => "A1",
    }
}

/// The learner's vocabulary from the answers of a test: for each word, the index of its band (none for a made-up
/// word) and whether the learner said they know it.
pub fn estimate(lang: &str, answers: &[(Option<usize>, bool)]) -> Estimate {
    let bands = bands(lang);
    let share = |list: Vec<bool>| if list.is_empty() { 0.0 } else { list.iter().filter(|&&y| y).count() as f64 / list.len() as f64 };
    let guessed = share(answers.iter().filter(|(b, _)| b.is_none()).map(|&(_, y)| y).collect());
    let results: Vec<BandResult> = bands
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let hits = share(answers.iter().filter(|(a, _)| *a == Some(i)).map(|&(_, y)| y).collect());
            BandResult { pack: b.pack, start: b.start, end: b.end, known: corrected(hits, guessed) }
        })
        .collect();
    let size = |r: &BandResult| r.known * (r.end - r.start) as f64;
    let of = |pack: &str| results.iter().filter(|r| r.pack == pack).map(size).sum::<f64>();
    let share_of = |pack: &str| find(pack).map_or(0.0, |p| of(pack) / p.words.len().max(1) as f64);
    let (words, level, academic, toeic) = if lang == "zh" {
        let reached = results.iter().take_while(|r| r.known >= REACHED).count();
        let level = match reached {
            0 => String::new(),
            1..=6 => format!("HSK {reached}"),
            _ => "HSK 7-9".into(),
        };
        (results.iter().map(size).sum::<f64>().round() as usize, level, 0.0, 0.0)
    } else {
        let words = of("ngsl").round() as usize;
        let academic = share_of("nawl");
        (words, cefr(words, academic).into(), academic, share_of("tsl"))
    };
    // Each pack starts after its first bands that are known, one after another from its start.
    let mut moves: Vec<Move> = Vec::new();
    for r in &results {
        let first = !moves.iter().any(|m| m.pack == r.pack) && r.start == 0;
        let chained = moves.last().is_some_and(|m| m.pack == r.pack && m.to == r.start);
        if r.known >= KNOWN && (first || chained) {
            match moves.last_mut() {
                Some(m) if chained => m.to = r.end,
                _ => moves.push(Move { pack: r.pack, to: r.end }),
            }
        }
    }
    Estimate { lang: lang.into(), bands: results, guessed, words, level, academic, toeic, moves }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pron;

    fn pack(id: &str) -> &'static Pack {
        find(id).unwrap()
    }

    #[test]
    fn the_packs_are_whole_and_in_order() {
        assert_eq!(pack("ngsl").words[..3], ["the", "be", "and"]);
        assert_eq!([pack("ngsl").words.len(), pack("nawl").words.len(), pack("tsl").words.len()], [2809, 957, 1250]);
        let hsk: Vec<usize> = (1..=7).map(|n| pack(&format!("hsk{n}")).words.len()).collect();
        assert_eq!(hsk.iter().sum::<usize>(), HSK.lines().count());
        assert!(hsk.iter().all(|&n| n > 400), "{hsk:?}");
        assert_eq!(pack("hsk1").words[0], "爱");
        assert_eq!(pack("hsk3").level, "HSK3");
        // Each word once in a pack, and once in all of HSK, at its lowest level; NAWL and TSL share some words.
        for group in [&["ngsl"][..], &["nawl"], &["tsl"], &["hsk1", "hsk2", "hsk3", "hsk4", "hsk5", "hsk6", "hsk7"]] {
            let words: Vec<String> = group.iter().flat_map(|id| pack(id).words.iter().map(|w| w.to_lowercase())).collect();
            let unique: HashSet<&String> = words.iter().collect();
            assert_eq!(unique.len(), words.len(), "{group:?}");
            assert!(words.iter().all(|w| pron::is_chinese(w) == group[0].starts_with("hsk")), "{group:?}");
        }
        // The made-up words of the test are in neither dictionary.
        assert!(FAKE_EN.iter().chain(&FAKE_ZH).all(|w| !pron::spell(w).0), "a made-up word is real");
    }

    #[test]
    fn the_next_words_skip_the_listed_ones_and_those_before_the_position() {
        let listed: HashSet<String> = ["be", "of"].map(String::from).into();
        assert_eq!(next(pack("ngsl"), 0, &listed, 3), [(0, "the"), (2, "and"), (4, "to")]);
        assert_eq!(next(pack("ngsl"), 2, &listed, 2), [(2, "and"), (4, "to")]);
        assert!(next(pack("tsl"), 1250, &listed, 5).is_empty());
        let db = db::open(Path::new(":memory:")).unwrap();
        db::save(&db, &db::Word { word: "The".into(), ..db::Word::default() }, 0).unwrap();
        set_position(&db, "ngsl", 1).unwrap();
        assert_eq!(position(&db, pack("ngsl")), 1);
        set_position(&db, "ngsl", 99_999).unwrap();
        assert_eq!(position(&db, pack("ngsl")), 2809);
        assert!(set_position(&db, "nope", 1).is_err());
        let s = status(&db, "en").unwrap();
        assert_eq!((s["packs"][0]["have"].as_u64(), s["packs"][0]["left"].as_u64()), (Some(1), Some(0)));
        assert_eq!(s["packs"][1]["next"][0], "repertoire");
        assert_eq!(s["packs"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_run_leaves_out_what_the_ai_cannot_explain_and_stops_when_asked() {
        let dir = std::env::temp_dir().join(format!("card-english-packs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = db::open(&dir.join("words.db")).unwrap();
        db::save(&db, &db::Word { word: "be".into(), ..db::Word::default() }, 0).unwrap();
        // No model downloaded: every explanation fails.
        let settings = ai::Settings { provider: "local".into(), ..ai::Settings::default() };
        let events = std::cell::RefCell::new(Vec::new());
        let r = add(&dir, &settings, "ngsl", 3, "", |e| events.borrow_mut().push(e)).unwrap();
        assert_eq!(r["added"], json!([]));
        let failed: Vec<&str> = r["failed"].as_array().unwrap().iter().map(|f| f["word"].as_str().unwrap()).collect();
        assert_eq!(failed, ["the", "and", "of"], "the word in the list is not asked about");
        let done: Vec<u64> = events.borrow().iter().map(|e| e["done"].as_u64().unwrap()).collect();
        assert_eq!(done, [0, 1, 2, 3]);
        // The next run tries them again.
        assert_eq!(position(&db, pack("ngsl")), 0);
        let r = add(&dir, &settings, "nawl", 5, "", |_| cancel()).unwrap();
        assert_eq!((r["cancelled"].as_bool(), r["failed"].as_array().unwrap().len()), (Some(true), 1));
        assert_eq!(position(&db, pack("nawl")), 0);
        assert!(add(&dir, &settings, "nope", 5, "", |_| {}).is_err());
        assert!(!RUNNING.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_test_samples_every_band_and_ends_with_the_made_up_words() {
        for (lang, real, fakes) in [("en", 48, 8), ("zh", 49, 7)] {
            let items = test(lang);
            assert_eq!(items.len(), real + fakes, "{lang}");
            let words: HashSet<&str> = items.iter().filter_map(|i| i["word"].as_str()).collect();
            assert_eq!(words.len(), items.len(), "no word twice");
            let bands = bands(lang);
            for item in &items[..real] {
                let b = bands[item["band"].as_u64().unwrap() as usize];
                let at = pack(b.pack).words.iter().position(|w| Some(*w) == item["word"].as_str()).unwrap();
                assert!((b.start..b.end).contains(&at));
            }
            assert!(items[real..].iter().all(|i| i["band"].is_null()));
        }
    }

    /// Answers that know the first `known` bands of a course, and say yes to `fakes` of the made-up words.
    fn answers(lang: &str, known: impl Fn(usize) -> bool, fakes: usize) -> Vec<(Option<usize>, bool)> {
        let mut out: Vec<(Option<usize>, bool)> =
            bands(lang).iter().enumerate().flat_map(|(i, b)| vec![(Some(i), known(i)); b.items]).collect();
        let count = if lang == "zh" { FAKE_ZH.len() } else { FAKE_EN.len() };
        out.extend((0..count).map(|i| (None, i < fakes)));
        out
    }

    #[test]
    fn the_estimate_corrects_for_guessing_and_moves_the_packs_past_what_is_known() {
        assert_eq!(corrected(1.0, 0.25), 1.0);
        assert!((corrected(0.5, 0.25) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!((corrected(0.2, 0.25), corrected(1.0, 1.0)), (0.0, 0.0));

        // Everything known, nothing guessed: every word of every pack, and every pack moves to its end.
        let all_known = estimate("en", &answers("en", |_| true, 0));
        assert_eq!((all_known.words, all_known.level.as_str(), all_known.guessed), (2809, "C1", 0.0));
        assert_eq!(all_known.moves, [Move { pack: "ngsl", to: 2809 }, Move { pack: "nawl", to: 957 }, Move { pack: "tsl", to: 1250 }]);

        // Yes to everything, the made-up words too: the guessing explains it all.
        let guesser = estimate("en", &answers("en", |_| true, 8));
        assert_eq!((guesser.words, guesser.level.as_str(), guesser.moves.len()), (0, "A1", 0));

        // The first 1,500 words known: B1, and the NGSL starts after them.
        let b1 = estimate("en", &answers("en", |i| i < 3, 0));
        assert_eq!((b1.words, b1.level.as_str()), (1500, "B1"));
        assert_eq!(b1.moves, [Move { pack: "ngsl", to: 1500 }]);
        assert_eq!((b1.academic, b1.toeic), (0.0, 0.0));

        // A known band after an unknown one does not move the pack past the unknown one.
        let gap = estimate("en", &answers("en", |i| i == 0 || i == 2, 0));
        assert_eq!(gap.moves, [Move { pack: "ngsl", to: 500 }]);
        assert_eq!(gap.words, 1000);

        // One false alarm in eight: 6 of 6 still counts whole, 4 of 6 less than it seems.
        let mut some = answers("en", |i| i == 0, 1);
        for a in some.iter_mut().filter(|a| a.0 == Some(1)).take(4) {
            a.1 = true;
        }
        let r = estimate("en", &some);
        assert_eq!(r.bands[0].known, 1.0);
        assert!((r.bands[1].known - (4.0 / 6.0 - 0.125) / 0.875).abs() < 1e-9);
        assert_eq!(r.moves, [Move { pack: "ngsl", to: 500 }]);
        assert_eq!((r.words, r.level.as_str()), (810, "A2"));

        // Chinese: HSK 1 to 3 known.
        let zh = estimate("zh", &answers("zh", |i| i < 3, 0));
        let size = |n: usize| pack(&format!("hsk{n}")).words.len();
        assert_eq!((zh.level.as_str(), zh.words), ("HSK 3", size(1) + size(2) + size(3)));
        assert_eq!(zh.moves, [Move { pack: "hsk1", to: size(1) }, Move { pack: "hsk2", to: size(2) }, Move { pack: "hsk3", to: size(3) }]);
        assert_eq!(estimate("zh", &answers("zh", |_| false, 0)).level, "");
        assert_eq!(estimate("zh", &answers("zh", |_| true, 0)).level, "HSK 7-9");
    }
}
