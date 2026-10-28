//! The words and the settings, in one SQLite file in the data folder, the FSRS schedule of review and the log of
//! answers that undo and the statistics read.

use crate::{fsrs, pron};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

pub const DAY: i64 = 86_400;
/// A forgotten word comes back within the same session.
const RELEARN_SECONDS: i64 = 600;
/// The version of the tables in `PRAGMA user_version`; `migrate` brings an older file up to it.
const SCHEMA: i64 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Word {
    pub id: i64,
    pub word: String,
    pub ipa: String,
    pub pos: String,
    pub meaning: String,
    pub explain: String,
    pub example: String,
    pub example_vi: String,
    pub note: String,
    pub synonyms: String,
    pub antonyms: String,
    pub collocations: String,
    /// The CEFR level A1 to C2, HSK1 to HSK9 for Chinese, or empty.
    pub level: String,
    pub topic: String,
    /// "en" or "zh", from the script of the word; each is a course of its own.
    pub lang: String,
    /// Chinese: the Sino-Vietnamese reading of the word.
    pub hanviet: String,
    /// Chinese: the pinyin of the example.
    pub example_reading: String,
    pub created: i64,
    pub due: i64,
    /// Days until the next review, 0 while a forgotten word is learned again.
    pub interval: f64,
    /// FSRS: the days until the chance of recall falls to 90%, 0 before the first review.
    pub stability: f64,
    /// FSRS: 1 to 10.
    pub difficulty: f64,
    pub last_review: i64,
    pub reps: i64,
    pub lapses: i64,
    /// When the word was first reviewed, 0 while it is still new.
    pub learned_at: i64,
    /// The last pronunciation score, -1 before any.
    pub score: i64,
    /// Left out of reviews, exercises and the counts: the learner does not want to study it again.
    pub skipped: bool,
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let mut db = Connection::open(path)?;
    // The file holds API keys and the ChatGPT tokens: readable by its owner only.
    #[cfg(unix)]
    for suffix in ["", "-wal", "-shm"] {
        use std::os::unix::fs::PermissionsExt;
        let file = format!("{}{suffix}", path.display());
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    init(&mut db)?;
    Ok(db)
}

/// The tables of the first version, then what later versions added.
fn init(db: &mut Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 3000;
         CREATE TABLE IF NOT EXISTS words (
             id INTEGER PRIMARY KEY,
             word TEXT NOT NULL UNIQUE COLLATE NOCASE,
             ipa TEXT NOT NULL DEFAULT '',
             pos TEXT NOT NULL DEFAULT '',
             meaning TEXT NOT NULL DEFAULT '',
             explain TEXT NOT NULL DEFAULT '',
             example TEXT NOT NULL DEFAULT '',
             example_vi TEXT NOT NULL DEFAULT '',
             note TEXT NOT NULL DEFAULT '',
             created INTEGER NOT NULL,
             due INTEGER NOT NULL,
             interval REAL NOT NULL DEFAULT 0,
             ease REAL NOT NULL DEFAULT 2.5,
             reps INTEGER NOT NULL DEFAULT 0,
             lapses INTEGER NOT NULL DEFAULT 0,
             learned_at INTEGER NOT NULL DEFAULT 0,
             score INTEGER NOT NULL DEFAULT -1
         );
         CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS activity (
             day TEXT PRIMARY KEY,
             xp INTEGER NOT NULL DEFAULT 0,
             answers INTEGER NOT NULL DEFAULT 0,
             correct INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS usage (
             day TEXT NOT NULL,
             provider TEXT NOT NULL,
             model TEXT NOT NULL,
             requests INTEGER NOT NULL DEFAULT 0,
             failures INTEGER NOT NULL DEFAULT 0,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (day, provider, model)
         );",
    )?;
    migrate(db)
}

/// Version 1: FSRS in place of SM-2, the longer explanation, and the log of answers.
const V1: &str = "
    ALTER TABLE words ADD COLUMN stability REAL NOT NULL DEFAULT 0;
    ALTER TABLE words ADD COLUMN difficulty REAL NOT NULL DEFAULT 0;
    ALTER TABLE words ADD COLUMN last_review INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE words ADD COLUMN synonyms TEXT NOT NULL DEFAULT '';
    ALTER TABLE words ADD COLUMN antonyms TEXT NOT NULL DEFAULT '';
    ALTER TABLE words ADD COLUMN collocations TEXT NOT NULL DEFAULT '';
    ALTER TABLE words ADD COLUMN level TEXT NOT NULL DEFAULT '';
    ALTER TABLE words ADD COLUMN topic TEXT NOT NULL DEFAULT '';
    CREATE TABLE reviews (
        id INTEGER PRIMARY KEY,
        word_id INTEGER NOT NULL,
        at INTEGER NOT NULL,
        rating INTEGER NOT NULL,
        -- The schedule of the word before the answer, which undo puts back.
        prev TEXT NOT NULL
    );
    CREATE INDEX reviews_word ON reviews (word_id, id);
    CREATE INDEX reviews_at ON reviews (at);";

/// Version 2: Chinese, a course of its own.
const V2: &str = "
    ALTER TABLE words ADD COLUMN lang TEXT NOT NULL DEFAULT 'en';
    ALTER TABLE words ADD COLUMN hanviet TEXT NOT NULL DEFAULT '';
    ALTER TABLE words ADD COLUMN example_reading TEXT NOT NULL DEFAULT '';
    CREATE INDEX words_lang ON words (lang, learned_at, due);";

/// Version 3: each course keeps its own experience, so the activity of a day is per course. What was there
/// before is English, the only course for most of that time.
const V3: &str = "
    CREATE TABLE activity_v3 (
        day TEXT NOT NULL,
        lang TEXT NOT NULL,
        xp INTEGER NOT NULL DEFAULT 0,
        answers INTEGER NOT NULL DEFAULT 0,
        correct INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, lang)
    );
    INSERT INTO activity_v3 (day, lang, xp, answers, correct) SELECT day, 'en', xp, answers, correct FROM activity;
    DROP TABLE activity;
    ALTER TABLE activity_v3 RENAME TO activity;";

/// Version 4: words the learner set aside.
const V4: &str = "ALTER TABLE words ADD COLUMN skipped INTEGER NOT NULL DEFAULT 0;";

/// Version 5: the talks with the AI, the tutor's and each role-play's. Rows carry a uid, the time of their last
/// change and a deleted mark instead of being deleted, so that a sync can merge them row by row.
const V5: &str = "
    CREATE TABLE chat (
        id INTEGER PRIMARY KEY,
        uid TEXT NOT NULL UNIQUE,
        lang TEXT NOT NULL,
        thread TEXT NOT NULL,
        role TEXT NOT NULL,
        text TEXT NOT NULL,
        vi TEXT NOT NULL DEFAULT '',
        reading TEXT NOT NULL DEFAULT '',
        source TEXT NOT NULL DEFAULT '',
        created INTEGER NOT NULL,
        updated INTEGER NOT NULL,
        deleted INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX chat_thread ON chat (lang, thread, created);";

/// The settings each course keeps for itself; the rest are shared.
const SCOPED: [&str; 5] = ["palette", "new_per_day", "daily_goal", "voice", "achievements"];

/// The key a setting is stored under for a course: English keeps the plain key it always had.
pub fn scoped(key: &str, lang: &str) -> String {
    if lang == "zh" && SCOPED.contains(&key) { format!("{key}@zh") } else { key.to_owned() }
}

fn user_version(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
}

fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    if user_version(db)? >= SCHEMA {
        return Ok(());
    }
    // Under the write lock and checked again inside it: every thread opens its own connection.
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if user_version(&tx)? < 1 {
        tx.execute_batch(V1)?;
        let old: Vec<(i64, f64, f64, i64)> = {
            let mut stmt = tx.prepare("SELECT id, interval, ease, due FROM words WHERE learned_at > 0")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?
        };
        for (id, interval, ease, due) in old {
            let m = fsrs::from_sm2(interval, ease);
            let last = if interval >= 1.0 { due - (interval * DAY as f64) as i64 } else { due - RELEARN_SECONDS };
            tx.execute(
                "UPDATE words SET stability = ?2, difficulty = ?3, last_review = ?4 WHERE id = ?1",
                params![id, m.stability, m.difficulty, last],
            )?;
        }
    }
    if user_version(&tx)? < 2 {
        tx.execute_batch(V2)?;
        // Chinese words added before there was a course for them move to it, with their readings.
        // The columns of this version only: `select` reads ones that later versions add.
        let old: Vec<Word> = {
            let mut stmt = tx.prepare("SELECT id, word, ipa, example FROM words")?;
            let word = |r: &Row| Ok(Word { id: r.get(0)?, word: r.get(1)?, ipa: r.get(2)?, example: r.get(3)?, ..Word::default() });
            stmt.query_map([], word)?.collect::<rusqlite::Result<_>>()?
        };
        for mut w in old.into_iter().filter(|w| pron::is_chinese(&w.word)) {
            derive(&mut w);
            tx.execute(
                "UPDATE words SET lang = ?2, ipa = ?3, hanviet = ?4, example_reading = ?5 WHERE id = ?1",
                params![w.id, w.lang, w.ipa, w.hanviet, w.example_reading],
            )?;
        }
    }
    if user_version(&tx)? < 3 {
        tx.execute_batch(V3)?;
    }
    if user_version(&tx)? < 4 {
        tx.execute_batch(V4)?;
    }
    if user_version(&tx)? < 5 {
        tx.execute_batch(V5)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA)?;
    tx.commit()
}

const COLUMNS: &str = "id, word, ipa, pos, meaning, explain, example, example_vi, note, synonyms, antonyms, collocations, \
                       level, topic, created, due, interval, stability, difficulty, last_review, reps, lapses, learned_at, score, \
                       lang, hanviet, example_reading, skipped";

fn row(r: &Row) -> rusqlite::Result<Word> {
    Ok(Word {
        id: r.get(0)?,
        word: r.get(1)?,
        ipa: r.get(2)?,
        pos: r.get(3)?,
        meaning: r.get(4)?,
        explain: r.get(5)?,
        example: r.get(6)?,
        example_vi: r.get(7)?,
        note: r.get(8)?,
        synonyms: r.get(9)?,
        antonyms: r.get(10)?,
        collocations: r.get(11)?,
        level: r.get(12)?,
        topic: r.get(13)?,
        created: r.get(14)?,
        due: r.get(15)?,
        interval: r.get(16)?,
        stability: r.get(17)?,
        difficulty: r.get(18)?,
        last_review: r.get(19)?,
        reps: r.get(20)?,
        lapses: r.get(21)?,
        learned_at: r.get(22)?,
        score: r.get(23)?,
        lang: r.get(24)?,
        hanviet: r.get(25)?,
        example_reading: r.get(26)?,
        skipped: r.get(27)?,
    })
}

fn select(db: &Connection, tail: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<Word>> {
    let mut stmt = db.prepare(&format!("SELECT {COLUMNS} FROM words {tail}"))?;
    stmt.query_map(args, row)?.collect()
}

pub fn list(db: &Connection, query: &str) -> rusqlite::Result<Vec<Word>> {
    let like = format!("%{}%", query.trim());
    select(
        db,
        "WHERE word LIKE ?1 OR meaning LIKE ?1 OR topic LIKE ?1 ORDER BY created DESC",
        [like],
    )
}

pub fn get(db: &Connection, id: i64) -> rusqlite::Result<Option<Word>> {
    Ok(select(db, "WHERE id = ?1", [id])?.pop())
}

pub fn find(db: &Connection, word: &str) -> rusqlite::Result<Option<Word>> {
    Ok(select(db, "WHERE word = ?1", [word.trim()])?.pop())
}

/// What a word takes from its text: its language, and the readings it was given none of.
fn derive(w: &mut Word) {
    w.word = w.word.trim().to_owned();
    let chinese = pron::is_chinese(&w.word);
    w.lang = if chinese { "zh" } else { "en" }.into();
    if w.ipa.trim().is_empty() {
        w.ipa = pron::ipa(&w.word);
    }
    if chinese {
        if w.hanviet.trim().is_empty() {
            w.hanviet = pron::hanviet(&w.word);
        }
        if w.example_reading.trim().is_empty() && pron::is_chinese(&w.example) {
            w.example_reading = pron::pinyin(&w.example);
        }
    }
}

/// Inserts a new word or, with an id, changes the text of one; the schedule is left as it is.
pub fn save(db: &Connection, w: &Word, now: i64) -> rusqlite::Result<i64> {
    let mut w = w.clone();
    derive(&mut w);
    if w.id == 0 {
        db.execute(
            "INSERT INTO words (word, ipa, pos, meaning, explain, example, example_vi, note, synonyms, antonyms,
                                collocations, level, topic, lang, hanviet, example_reading, created, due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
            params![w.word, w.ipa, w.pos, w.meaning, w.explain, w.example, w.example_vi, w.note, w.synonyms, w.antonyms,
                    w.collocations, w.level, w.topic, w.lang, w.hanviet, w.example_reading, now],
        )?;
        Ok(db.last_insert_rowid())
    } else {
        db.execute(
            "UPDATE words SET word = ?2, ipa = ?3, pos = ?4, meaning = ?5, explain = ?6, example = ?7,
             example_vi = ?8, note = ?9, synonyms = ?10, antonyms = ?11, collocations = ?12, level = ?13, topic = ?14,
             lang = ?15, hanviet = ?16, example_reading = ?17
             WHERE id = ?1",
            params![w.id, w.word, w.ipa, w.pos, w.meaning, w.explain, w.example, w.example_vi, w.note, w.synonyms,
                    w.antonyms, w.collocations, w.level, w.topic, w.lang, w.hanviet, w.example_reading],
        )?;
        Ok(w.id)
    }
}

/// Inserts a word with its schedule as it is, for a restored copy.
pub fn insert(db: &Connection, w: &Word) -> rusqlite::Result<i64> {
    let mut w = w.clone();
    derive(&mut w);
    db.execute(
        "INSERT INTO words (word, ipa, pos, meaning, explain, example, example_vi, note, synonyms, antonyms, collocations,
                            level, topic, created, due, interval, stability, difficulty, last_review, reps, lapses,
                            learned_at, score, lang, hanviet, example_reading, skipped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                 ?24, ?25, ?26, ?27)",
        params![w.word, w.ipa, w.pos, w.meaning, w.explain, w.example, w.example_vi, w.note, w.synonyms,
                w.antonyms, w.collocations, w.level, w.topic, w.created, w.due, w.interval, w.stability, w.difficulty,
                w.last_review, w.reps, w.lapses, w.learned_at, w.score, w.lang, w.hanviet, w.example_reading, w.skipped],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn delete(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM reviews WHERE word_id = ?1", [id])?;
    db.execute("DELETE FROM words WHERE id = ?1", [id]).map(|_| ())
}

pub fn set_score(db: &Connection, id: i64, score: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE words SET score = ?2 WHERE id = ?1", [id, score]).map(|_| ())
}

/// Sets a word aside, or brings it back, and gives it as it is now.
pub fn set_skipped(db: &Connection, id: i64, on: bool) -> rusqlite::Result<Option<Word>> {
    db.execute("UPDATE words SET skipped = ?2 WHERE id = ?1", params![id, on])?;
    get(db, id)
}

/// What a session of a course ("en" or "zh") works through: the reviews that are due, then as many new words as
/// today still allows.
pub fn session(db: &Connection, now: i64, day_start: i64, lang: &str) -> rusqlite::Result<Vec<Word>> {
    let mut words = select(db, "WHERE learned_at > 0 AND due <= ?1 AND lang = ?2 AND NOT skipped ORDER BY due", params![now, lang])?;
    let left = new_left(db, day_start, lang)?;
    words.extend(select(db, "WHERE learned_at = 0 AND lang = ?2 AND NOT skipped ORDER BY created LIMIT ?1", params![left, lang])?);
    Ok(words)
}

/// How many new words of a course may still be started today; each course has `new_per_day` of its own.
fn new_left(db: &Connection, day_start: i64, lang: &str) -> rusqlite::Result<i64> {
    let per_day: i64 = setting(db, &scoped("new_per_day", lang))?
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let today: i64 = db.query_row(
        "SELECT count(*) FROM words WHERE learned_at > 0 AND learned_at >= ?1 AND NOT skipped AND (?2 = '' OR lang = ?2)",
        params![day_start, lang],
        |r| r.get(0),
    )?;
    Ok((per_day - today).max(0))
}

#[derive(Serialize)]
pub struct Stats {
    pub total: i64,
    pub due: i64,
    pub new_today: i64,
    pub new_waiting: i64,
    pub learned: i64,
    /// Answers ever given, and since `day_start`.
    pub reviews: i64,
    pub reviews_today: i64,
    /// Words whose last pronunciation scored 90 or more.
    pub perfect: i64,
    /// Words set aside, which none of the other counts include.
    pub skipped: i64,
}

/// The counts of a course, or of both when `lang` is empty, without the words set aside; `new_today` only means
/// something for one course.
pub fn stats(db: &Connection, now: i64, day_start: i64, lang: &str) -> rusqlite::Result<Stats> {
    let every = |filter: &str, arg: i64| {
        let sql = format!("SELECT count(*) FROM words WHERE (?2 = '' OR lang = ?2) AND {filter}");
        db.query_row(&sql, params![arg, lang], |r| r.get::<_, i64>(0))
    };
    let count = |filter: &str, arg: i64| every(&format!("NOT skipped AND {filter}"), arg);
    let reviews = |since: i64| {
        let sql = "SELECT count(*) FROM reviews r JOIN words w ON w.id = r.word_id WHERE r.at >= ?1 AND (?2 = '' OR w.lang = ?2)";
        db.query_row(sql, params![since, lang], |r| r.get::<_, i64>(0))
    };
    let waiting = count("learned_at = 0 AND ?1 = ?1", 0)?;
    Ok(Stats {
        total: count("?1 = ?1", 0)?,
        due: count("learned_at > 0 AND due <= ?1", now)?,
        new_today: new_left(db, day_start, lang)?.min(waiting),
        new_waiting: waiting,
        learned: count("learned_at > 0 AND ?1 = ?1", 0)?,
        reviews: reviews(i64::MIN)?,
        reviews_today: reviews(day_start)?,
        perfect: count("score >= ?1", 90)?,
        skipped: every("skipped AND ?1 = ?1", 0)?,
    })
}

/// The reviews of a course due on each of the next `days` days, the first also counting those that are overdue;
/// `day_start` is the learner's midnight.
pub fn forecast(db: &Connection, day_start: i64, days: usize, lang: &str) -> rusqlite::Result<Vec<i64>> {
    let mut out = vec![0; days.max(1)];
    let end = day_start + out.len() as i64 * DAY;
    let mut stmt = db.prepare("SELECT due FROM words WHERE learned_at > 0 AND NOT skipped AND due < ?1 AND (?2 = '' OR lang = ?2)")?;
    for due in stmt.query_map(params![end, lang], |r| r.get::<_, i64>(0))? {
        let day = ((due? - day_start).max(0) / DAY) as usize;
        let last = out.len() - 1;
        out[day.min(last)] += 1;
    }
    Ok(out)
}

/// The words of a course answered since `since`, most recent first; when there are none, the words learned last,
/// and before anything is learned, the words added last.
pub fn recent_words(db: &Connection, since: i64, limit: i64, lang: &str) -> rusqlite::Result<Vec<String>> {
    let words = |sql: &str| -> rusqlite::Result<Vec<String>> {
        let mut stmt = db.prepare(sql)?;
        stmt.query_map(params![since, limit, lang], |r| r.get(0))?.collect()
    };
    let today = words(
        "SELECT word FROM words WHERE id IN (SELECT word_id FROM reviews WHERE at >= ?1) AND lang = ?3 AND NOT skipped
         ORDER BY last_review DESC LIMIT ?2",
    )?;
    if !today.is_empty() {
        return Ok(today);
    }
    let learned = words("SELECT word FROM words WHERE learned_at > 0 AND lang = ?3 AND NOT skipped AND ?1 = ?1 ORDER BY last_review DESC LIMIT ?2")?;
    if !learned.is_empty() {
        return Ok(learned);
    }
    words("SELECT word FROM words WHERE lang = ?3 AND NOT skipped AND ?1 = ?1 ORDER BY created DESC, id DESC LIMIT ?2")
}

/// Every answer as a copy keeps it: the word, when, and how it went.
pub fn review_log(db: &Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut stmt = db.prepare("SELECT w.word, r.at, r.rating FROM reviews r JOIN words w ON w.id = r.word_id ORDER BY r.id")?;
    stmt.query_map([], |r| Ok(json!({ "word": r.get::<_, String>(0)?, "at": r.get::<_, i64>(1)?, "rating": r.get::<_, i64>(2)? })))?
        .collect()
}

/// Records an answer, 1 forgot to 4 easy, and keeps the schedule it replaces so that the answer can be undone.
pub fn grade(db: &Connection, id: i64, rating: u8, now: i64, retention: f64) -> rusqlite::Result<Option<Word>> {
    let Some(mut w) = get(db, id)? else {
        return Ok(None);
    };
    let prev = json!({
        "due": w.due, "interval": w.interval, "stability": w.stability, "difficulty": w.difficulty,
        "last_review": w.last_review, "reps": w.reps, "lapses": w.lapses, "learned_at": w.learned_at,
    });
    schedule(&mut w, rating, now, retention);
    let tx = db.unchecked_transaction()?;
    write_schedule(&tx, &w)?;
    tx.execute(
        "INSERT INTO reviews (word_id, at, rating, prev) VALUES (?1, ?2, ?3, ?4)",
        params![w.id, now, rating.clamp(1, 4), prev.to_string()],
    )?;
    tx.commit()?;
    Ok(Some(w))
}

/// Puts a word back as it was before its last answer, and forgets that answer.
pub fn undo(db: &Connection, id: i64) -> rusqlite::Result<Option<Word>> {
    let last: Option<(i64, String)> = db
        .query_row("SELECT id, prev FROM reviews WHERE word_id = ?1 ORDER BY id DESC LIMIT 1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let (Some((review, prev)), Some(mut w)) = (last, get(db, id)?) else {
        return Ok(None);
    };
    let p: serde_json::Value = serde_json::from_str(&prev).unwrap_or_default();
    let int = |key: &str| p[key].as_i64().unwrap_or(0);
    let real = |key: &str| p[key].as_f64().unwrap_or(0.0);
    (w.due, w.last_review, w.reps, w.lapses, w.learned_at) = (int("due"), int("last_review"), int("reps"), int("lapses"), int("learned_at"));
    (w.interval, w.stability, w.difficulty) = (real("interval"), real("stability"), real("difficulty"));
    let tx = db.unchecked_transaction()?;
    write_schedule(&tx, &w)?;
    tx.execute("DELETE FROM reviews WHERE id = ?1", [review])?;
    tx.commit()?;
    Ok(Some(w))
}

fn write_schedule(db: &Connection, w: &Word) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE words SET due = ?2, interval = ?3, stability = ?4, difficulty = ?5, last_review = ?6, reps = ?7,
         lapses = ?8, learned_at = ?9 WHERE id = ?1",
        params![w.id, w.due, w.interval, w.stability, w.difficulty, w.last_review, w.reps, w.lapses, w.learned_at],
    )
    .map(|_| ())
}

/// FSRS: the memory of the word after the answer, then the day it is next due. A forgotten word comes back in ten
/// minutes, within the same session.
pub fn schedule(w: &mut Word, rating: u8, now: i64, retention: f64) {
    let rating = rating.clamp(1, 4);
    let memory = (w.stability > 0.0).then_some(fsrs::Memory { stability: w.stability, difficulty: w.difficulty });
    let days = if w.last_review > 0 { (now - w.last_review).max(0) / DAY } else { 0 };
    let m = fsrs::next(memory, days, rating);
    (w.stability, w.difficulty) = (m.stability, m.difficulty);
    if w.learned_at == 0 {
        w.learned_at = now;
    }
    w.reps += 1;
    w.last_review = now;
    if rating == 1 {
        // Only a word that had been remembered can be forgotten; a new one is still being learned.
        if w.interval >= 1.0 {
            w.lapses += 1;
        }
        w.interval = 0.0;
        w.due = now + RELEARN_SECONDS;
    } else {
        w.interval = fsrs::interval(m.stability, retention);
        w.due = now + (w.interval * DAY as f64) as i64;
    }
}

/// One day of study, by the local date of the learner (`2026-10-26`).
#[derive(Debug, Serialize, PartialEq)]
pub struct Day {
    pub day: String,
    pub xp: i64,
    pub answers: i64,
    pub correct: i64,
}

/// Adds experience to a day of a course, and an answer when `correct` says how it went.
pub fn add_xp(db: &Connection, day: &str, lang: &str, xp: i64, correct: Option<bool>) -> rusqlite::Result<()> {
    let (answers, right) = match correct {
        Some(ok) => (1, i64::from(ok)),
        None => (0, 0),
    };
    db.execute(
        "INSERT INTO activity (day, lang, xp, answers, correct) VALUES (?1, ?5, ?2, ?3, ?4)
         ON CONFLICT(day, lang) DO UPDATE SET xp = xp + ?2, answers = answers + ?3, correct = correct + ?4",
        params![day, xp, answers, right, lang],
    )
    .map(|_| ())
}

/// Takes back the experience and the answer of an undone review.
pub fn take_back_xp(db: &Connection, day: &str, lang: &str, xp: i64, correct: bool) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE activity SET xp = max(0, xp - ?2), answers = max(0, answers - 1), correct = max(0, correct - ?3)
         WHERE day = ?1 AND lang = ?4",
        params![day, xp, i64::from(correct), lang],
    )
    .map(|_| ())
}

/// The last `days` days of a course that saw any study, newest first, and its experience of all time.
pub fn activity(db: &Connection, days: i64, lang: &str) -> rusqlite::Result<(Vec<Day>, i64)> {
    let mut stmt = db.prepare("SELECT day, xp, answers, correct FROM activity WHERE lang = ?2 ORDER BY day DESC LIMIT ?1")?;
    let list = stmt
        .query_map(params![days, lang], |r| Ok(Day { day: r.get(0)?, xp: r.get(1)?, answers: r.get(2)?, correct: r.get(3)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let total = db.query_row("SELECT coalesce(sum(xp), 0) FROM activity WHERE lang = ?1", [lang], |r| r.get(0))?;
    Ok((list, total))
}

/// Counts one call to an AI provider on a day; tokens only come with an answer.
pub fn add_usage(db: &Connection, day: &str, provider: &str, model: &str, ok: bool, input: u64, output: u64) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO usage (day, provider, model, requests, failures, input_tokens, output_tokens)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)
         ON CONFLICT(day, provider, model) DO UPDATE SET requests = requests + 1, failures = failures + ?4,
             input_tokens = input_tokens + ?5, output_tokens = output_tokens + ?6",
        params![day, provider, model, i64::from(!ok), input as i64, output as i64],
    )
    .map(|_| ())
}

/// Per provider: today, the days since `since7` and since `since30` (requests, failures, tokens), with the busiest
/// day and the number of days in use over the latter. Days are the learner's local dates.
pub fn usage_summary(db: &Connection, today: &str, since7: &str, since30: &str) -> rusqlite::Result<serde_json::Value> {
    let mut out = serde_json::Map::new();
    let mut stmt = db.prepare(
        "SELECT provider,
                sum(CASE WHEN day = ?1 THEN requests ELSE 0 END), sum(CASE WHEN day = ?1 THEN failures ELSE 0 END),
                sum(CASE WHEN day = ?1 THEN input_tokens + output_tokens ELSE 0 END),
                sum(CASE WHEN day >= ?2 THEN requests ELSE 0 END), sum(CASE WHEN day >= ?2 THEN input_tokens + output_tokens ELSE 0 END),
                sum(requests), sum(input_tokens + output_tokens)
         FROM usage WHERE day >= ?3 GROUP BY provider",
    )?;
    let rows = stmt.query_map([today, since7, since30], |r| {
        let n = |i| r.get::<_, i64>(i);
        Ok((r.get::<_, String>(0)?, serde_json::json!({
            "today": { "requests": n(1)?, "failures": n(2)?, "tokens": n(3)? },
            "week": { "requests": n(4)?, "tokens": n(5)? },
            "month": { "requests": n(6)?, "tokens": n(7)? },
        })))
    })?;
    for row in rows {
        let (provider, summary) = row?;
        out.insert(provider, summary);
    }
    let mut stmt = db.prepare(
        "SELECT provider, max(n), count(*) FROM
             (SELECT provider, day, sum(requests) AS n FROM usage WHERE day >= ?1 GROUP BY provider, day)
         GROUP BY provider",
    )?;
    let rows = stmt.query_map([since30], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
    for row in rows {
        let (provider, peak, days) = row?;
        if let Some(month) = out.get_mut(&provider).and_then(|p| p.get_mut("month")) {
            month["peak"] = peak.into();
            month["active_days"] = days.into();
        }
    }
    Ok(serde_json::Value::Object(out))
}

/// A date as the page sends it, `YYYY-MM-DD`.
pub fn is_day(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

pub fn setting(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

pub fn set_setting(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .map(|_| ())
}

/// One line of a talk with the AI.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Message {
    pub uid: String,
    /// "tutor", or "roleplay:<scenario>:<uid>" for one role-play.
    pub thread: String,
    /// "user", "ai", or "feedback" for the notes that end a role-play.
    pub role: String,
    pub text: String,
    /// A role-play line in Vietnamese.
    pub vi: String,
    /// Chinese: the pinyin of a role-play line.
    pub reading: String,
    /// Who answered, as the page names it.
    pub source: String,
    pub created: i64,
}

const CHAT_COLUMNS: &str = "uid, thread, role, text, vi, reading, source, created";

fn message(r: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        uid: r.get(0)?,
        thread: r.get(1)?,
        role: r.get(2)?,
        text: r.get(3)?,
        vi: r.get(4)?,
        reading: r.get(5)?,
        source: r.get(6)?,
        created: r.get(7)?,
    })
}

/// 16 random hex digits, the uid a row of user data is known by in every copy.
pub fn new_uid(db: &Connection) -> rusqlite::Result<String> {
    db.query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))
}

/// Adds a line to a thread of a course ("en" or "zh") and gives it back with its uid.
pub fn add_message(db: &Connection, lang: &str, m: &Message, now: i64) -> rusqlite::Result<Message> {
    db.execute(
        "INSERT INTO chat (uid, lang, thread, role, text, vi, reading, source, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![new_uid(db)?, lang, m.thread, m.role, m.text, m.vi, m.reading, m.source, now],
    )?;
    db.query_row(&format!("SELECT {CHAT_COLUMNS} FROM chat WHERE id = ?1"), [db.last_insert_rowid()], message)
}

/// The last `limit` lines of a thread of a course, oldest first.
pub fn messages(db: &Connection, lang: &str, thread: &str, limit: i64) -> rusqlite::Result<Vec<Message>> {
    let mut stmt = db.prepare(&format!(
        "SELECT * FROM (SELECT {CHAT_COLUMNS}, id FROM chat WHERE lang = ?1 AND thread = ?2 AND deleted = 0
                        ORDER BY created DESC, id DESC LIMIT ?3)
         ORDER BY created, id"
    ))?;
    stmt.query_map(params![lang, thread, limit], message)?.collect()
}

/// Deletes the lines of a thread of a course by marking them, so that a sync takes the deletion to other copies.
pub fn clear_thread(db: &Connection, lang: &str, thread: &str, now: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE chat SET deleted = 1, updated = ?3 WHERE lang = ?1 AND thread = ?2 AND deleted = 0",
        params![lang, thread, now],
    )
    .map(|_| ())
}

/// A thread as the list of past role-plays shows it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Thread {
    pub thread: String,
    pub started: i64,
    /// The lines the learner wrote or said.
    pub turns: i64,
    /// Whether it ended with notes.
    pub ended: bool,
}

/// The `limit` threads of a course whose names start with `prefix`, the one talked in last first.
pub fn threads(db: &Connection, lang: &str, prefix: &str, limit: i64) -> rusqlite::Result<Vec<Thread>> {
    let mut stmt = db.prepare(
        "SELECT thread, min(created), sum(role = 'user'), max(role = 'feedback') FROM chat
         WHERE lang = ?1 AND deleted = 0 AND substr(thread, 1, length(?2)) = ?2
         GROUP BY thread ORDER BY max(created) DESC, max(id) DESC LIMIT ?3",
    )?;
    stmt.query_map(params![lang, prefix, limit], |r| Ok(Thread { thread: r.get(0)?, started: r.get(1)?, turns: r.get(2)?, ended: r.get(3)? }))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        open(Path::new(":memory:")).unwrap()
    }

    fn add(db: &Connection, word: &str, now: i64) -> i64 {
        save(db, &Word { word: word.into(), ..Word::default() }, now).unwrap()
    }

    #[test]
    fn fsrs_schedules_and_a_forgotten_word_comes_back_in_ten_minutes() {
        let db = memory();
        let id = add(&db, "negotiate", 0);
        let w = grade(&db, id, 3, 1_000, 0.9).unwrap().unwrap();
        assert_eq!((w.interval, w.due, w.reps, w.learned_at, w.last_review), (2.0, 1_000 + 2 * DAY, 1, 1_000, 1_000));
        assert!((w.stability - 2.3065).abs() < 1e-9);
        let w = grade(&db, id, 3, 1_000 + 2 * DAY, 0.9).unwrap().unwrap();
        assert_eq!(w.interval, 11.0);
        let at = w.due;
        let w = grade(&db, id, 1, at, 0.9).unwrap().unwrap();
        assert_eq!((w.interval, w.due, w.lapses), (0.0, at + RELEARN_SECONDS, 1));
        // Forgotten again while it is being learned again: no second lapse.
        let w = grade(&db, id, 1, at + RELEARN_SECONDS, 0.9).unwrap().unwrap();
        assert_eq!(w.lapses, 1);
        assert_eq!(get(&db, id).unwrap().unwrap(), w);
        // A new word forgotten at once has never been remembered.
        let other = add(&db, "itinerary", 0);
        assert_eq!(grade(&db, other, 1, 5, 0.9).unwrap().unwrap().lapses, 0);
        // A higher retention brings the review sooner.
        let mut keen = Word { stability: 10.0, difficulty: 5.0, last_review: 0, interval: 10.0, ..Word::default() };
        let mut easy = keen.clone();
        schedule(&mut keen, 3, 10 * DAY, 0.95);
        schedule(&mut easy, 3, 10 * DAY, 0.8);
        assert!(keen.interval < easy.interval);
    }

    #[test]
    fn undo_puts_the_word_back_and_forgets_the_answer() {
        let db = memory();
        let id = add(&db, "deadline", 0);
        let before = get(&db, id).unwrap().unwrap();
        grade(&db, id, 4, 100, 0.9).unwrap();
        let first = get(&db, id).unwrap().unwrap();
        grade(&db, id, 1, 100 + 9 * DAY, 0.9).unwrap();
        assert_eq!(undo(&db, id).unwrap().unwrap(), first);
        assert_eq!(get(&db, id).unwrap().unwrap(), first);
        assert_eq!(undo(&db, id).unwrap().unwrap(), before);
        assert_eq!(undo(&db, id).unwrap(), None);
        assert_eq!(stats(&db, 0, 0, "").unwrap().reviews, 0);
        grade(&db, id, 3, 200, 0.9).unwrap();
        delete(&db, id).unwrap();
        assert_eq!(stats(&db, 0, 0, "").unwrap().reviews, 0);
    }

    #[test]
    fn an_sm2_file_moves_to_fsrs() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE words (id INTEGER PRIMARY KEY, word TEXT NOT NULL UNIQUE COLLATE NOCASE, ipa TEXT NOT NULL DEFAULT '',
                 pos TEXT NOT NULL DEFAULT '', meaning TEXT NOT NULL DEFAULT '', explain TEXT NOT NULL DEFAULT '',
                 example TEXT NOT NULL DEFAULT '', example_vi TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '',
                 created INTEGER NOT NULL, due INTEGER NOT NULL, interval REAL NOT NULL DEFAULT 0, ease REAL NOT NULL DEFAULT 2.5,
                 reps INTEGER NOT NULL DEFAULT 0, lapses INTEGER NOT NULL DEFAULT 0, learned_at INTEGER NOT NULL DEFAULT 0,
                 score INTEGER NOT NULL DEFAULT -1);
             INSERT INTO words (word, meaning, created, due, interval, ease, reps, learned_at) VALUES ('old', 'cũ', 1, 700000, 6, 2.5, 2, 5);
             INSERT INTO words (word, created, due) VALUES ('new', 1, 1);
             INSERT INTO words (word, created, due) VALUES ('你好', 1, 1);",
        )
        .unwrap();
        init(&mut db).unwrap();
        init(&mut db).unwrap();
        assert_eq!(user_version(&db).unwrap(), SCHEMA);
        let old = find(&db, "OLD").unwrap().unwrap();
        assert_eq!((old.stability, old.last_review, old.meaning.as_str()), (6.0, 700_000 - 6 * DAY, "cũ"));
        assert!((old.difficulty - 2.118104).abs() < 1e-5);
        let new = find(&db, "new").unwrap().unwrap();
        assert_eq!((new.stability, new.last_review, new.lang.as_str()), (0.0, 0, "en"));
        // A Chinese word added before the course existed moves to it, with its pinyin.
        let chinese = find(&db, "你好").unwrap().unwrap();
        assert_eq!((chinese.lang.as_str(), chinese.ipa.as_str()), ("zh", "nǐhǎo"));
        // The next answer goes on from the converted memory instead of starting over.
        let next = grade(&db, old.id, 3, 700_000, 0.9).unwrap().unwrap();
        assert!(next.interval > 6.0, "{next:?}");
    }

    #[test]
    fn the_forecast_counts_reviews_by_day() {
        let db = memory();
        for (word, due) in [("a", -5), ("b", 10), ("c", DAY + 1), ("d", 3 * DAY), ("e", 30 * DAY)] {
            let id = add(&db, word, 0);
            db.execute("UPDATE words SET learned_at = 1, due = ?2 WHERE id = ?1", [id, due]).unwrap();
        }
        add(&db, "new", 0);
        assert_eq!(forecast(&db, 0, 7, "en").unwrap(), [2, 1, 0, 1, 0, 0, 0]);
        assert_eq!(forecast(&db, 0, 1, "").unwrap(), [2]);
        assert_eq!(forecast(&db, 0, 7, "zh").unwrap(), [0; 7]);
    }

    #[test]
    fn chinese_words_are_a_course_of_their_own() {
        let db = memory();
        set_setting(&db, "new_per_day", "1").unwrap();
        // Each course keeps its own number of new words a day.
        set_setting(&db, &scoped("new_per_day", "zh"), "1").unwrap();
        assert_eq!((scoped("new_per_day", "en"), scoped("theme", "zh")), ("new_per_day".into(), "theme".into()));
        let en = add(&db, "deadline", 0);
        let zh = save(&db, &Word { word: " 学习 ".into(), example: "我每天学习中文。".into(), ..Word::default() }, 0).unwrap();
        add(&db, "苹果", 0);
        let w = get(&db, zh).unwrap().unwrap();
        assert_eq!((w.word.as_str(), w.lang.as_str(), w.ipa.as_str(), w.hanviet.as_str()), ("学习", "zh", "xuéxí", "học tập"));
        assert!(w.example_reading.starts_with("wǒ měitiān xuéxí"), "{}", w.example_reading);
        let english = get(&db, en).unwrap().unwrap();
        assert_eq!((english.lang.as_str(), english.hanviet.as_str(), english.example_reading.as_str()), ("en", "", ""));
        // Each course has its own new word today.
        let words = |lang| session(&db, 10, 0, lang).unwrap().into_iter().map(|w| w.word).collect::<Vec<_>>();
        assert_eq!(words("en"), ["deadline"]);
        assert_eq!(words("zh"), ["学习"]);
        grade(&db, zh, 3, 10, 0.9).unwrap();
        assert!(words("zh").is_empty());
        assert_eq!(words("en"), ["deadline"]);
        let (course, all) = (stats(&db, 10, 0, "zh").unwrap(), stats(&db, 10, 0, "").unwrap());
        assert_eq!((course.total, course.learned, course.reviews, all.total, all.reviews), (2, 1, 1, 3, 1));
        assert_eq!(recent_words(&db, 0, 8, "zh").unwrap(), ["学习"]);
        assert_eq!(recent_words(&db, 0, 8, "en").unwrap(), ["deadline"], "nothing learned yet: the words added");
        assert!(recent_words(&db, 0, 8, "fr").unwrap().is_empty());
    }

    #[test]
    fn a_skipped_word_leaves_the_session_and_the_counts_until_it_comes_back() {
        let db = memory();
        let (a, b) = (add(&db, "alpha", 0), add(&db, "beta", 0));
        grade(&db, a, 3, 10, 0.9).unwrap();
        let due = 10 + 3 * DAY;
        let words = || session(&db, due, due, "en").unwrap().into_iter().map(|w| w.word).collect::<Vec<_>>();
        assert_eq!(words(), ["alpha", "beta"]);
        assert!(set_skipped(&db, a, true).unwrap().unwrap().skipped);
        set_skipped(&db, b, true).unwrap();
        assert!(words().is_empty());
        let s = stats(&db, due, due, "en").unwrap();
        assert_eq!((s.total, s.due, s.new_today, s.new_waiting, s.learned, s.skipped), (0, 0, 0, 0, 0, 2));
        assert_eq!(forecast(&db, 0, 7, "en").unwrap(), [0; 7]);
        assert!(recent_words(&db, 0, 8, "en").unwrap().is_empty());
        assert!(!set_skipped(&db, a, false).unwrap().unwrap().skipped);
        assert_eq!(words(), ["alpha"]);
        assert_eq!(stats(&db, due, due, "en").unwrap().skipped, 1);
        assert_eq!(set_skipped(&db, 999, true).unwrap(), None);
    }

    #[test]
    fn recent_words_prefer_todays_answers() {
        let db = memory();
        let (a, b) = (add(&db, "alpha", 0), add(&db, "beta", 0));
        grade(&db, a, 3, 10, 0.9).unwrap();
        assert_eq!(recent_words(&db, 100, 8, "en").unwrap(), ["alpha"]);
        grade(&db, b, 3, 200, 0.9).unwrap();
        assert_eq!(recent_words(&db, 100, 8, "en").unwrap(), ["beta"]);
        assert_eq!(recent_words(&db, 0, 8, "en").unwrap(), ["beta", "alpha"]);
    }

    #[test]
    fn experience_adds_up_by_day() {
        let db = memory();
        add_xp(&db, "2026-10-25", "en", 10, Some(true)).unwrap();
        add_xp(&db, "2026-10-26", "en", 10, Some(false)).unwrap();
        add_xp(&db, "2026-10-26", "en", 3, None).unwrap();
        add_xp(&db, "2026-10-26", "zh", 7, Some(true)).unwrap();
        let (days, total) = activity(&db, 30, "en").unwrap();
        assert_eq!(total, 23, "the Chinese course keeps its experience apart");
        assert_eq!(days[0], Day { day: "2026-10-26".into(), xp: 13, answers: 1, correct: 0 });
        assert_eq!(days[1].correct, 1);
        assert_eq!(activity(&db, 30, "zh").unwrap(), (vec![Day { day: "2026-10-26".into(), xp: 7, answers: 1, correct: 1 }], 7));
        take_back_xp(&db, "2026-10-25", "en", 12, true).unwrap();
        assert_eq!(activity(&db, 30, "en").unwrap().0[1], Day { day: "2026-10-25".into(), xp: 0, answers: 0, correct: 0 });
        assert!(is_day("2026-10-26") && !is_day("2026-1-26") && !is_day("2026-10-2x"));
    }

    #[test]
    fn usage_is_measured_per_provider_and_period() {
        let db = memory();
        add_usage(&db, "2026-09-01", "gemini", "m", true, 100, 20).unwrap();
        add_usage(&db, "2026-10-20", "gemini", "m", true, 10, 5).unwrap();
        add_usage(&db, "2026-10-26", "gemini", "m", true, 30, 10).unwrap();
        add_usage(&db, "2026-10-26", "gemini", "m", false, 0, 0).unwrap();
        add_usage(&db, "2026-10-26", "gemini", "other", true, 1, 1).unwrap();
        add_usage(&db, "2026-10-26", "chatgpt", "gpt", false, 0, 0).unwrap();
        let u = usage_summary(&db, "2026-10-26", "2026-10-20", "2026-09-27").unwrap();
        assert_eq!(u["gemini"]["today"], serde_json::json!({ "requests": 3, "failures": 1, "tokens": 42 }));
        assert_eq!(u["gemini"]["week"], serde_json::json!({ "requests": 4, "tokens": 57 }));
        // The day in September is outside the thirty days.
        assert_eq!(u["gemini"]["month"], serde_json::json!({ "requests": 4, "tokens": 57, "peak": 3, "active_days": 2 }));
        assert_eq!(u["chatgpt"]["today"]["failures"], 1);
    }

    #[test]
    fn chat_threads_keep_their_lines_per_course_and_delete_by_marking() {
        let db = memory();
        let line = |thread: &str, role: &str, text: &str| Message { thread: thread.into(), role: role.into(), text: text.into(), ..Message::default() };
        let first = add_message(&db, "en", &line("tutor", "user", "What does deadline mean?"), 100).unwrap();
        assert_eq!((first.uid.len(), first.created), (16, 100));
        assert!(first.uid.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        // The same second keeps the order the lines were added in.
        add_message(&db, "en", &line("tutor", "ai", "Hạn chót."), 100).unwrap();
        add_message(&db, "en", &line("tutor", "user", "Thanks"), 101).unwrap();
        add_message(&db, "zh", &line("tutor", "user", "学习"), 102).unwrap();
        let texts = |lang, limit| messages(&db, lang, "tutor", limit).unwrap().into_iter().map(|m| m.text).collect::<Vec<_>>();
        assert_eq!(texts("en", 10), ["What does deadline mean?", "Hạn chót.", "Thanks"]);
        assert_eq!(texts("en", 2), ["Hạn chót.", "Thanks"], "the last ones, oldest first");
        assert_eq!(texts("zh", 10), ["学习"]);
        clear_thread(&db, "en", "tutor", 200).unwrap();
        assert!(texts("en", 10).is_empty());
        assert_eq!(texts("zh", 10), ["学习"]);
        let marked: (i64, i64) = db.query_row("SELECT count(*), min(updated) FROM chat WHERE deleted = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(marked, (3, 200));

        add_message(&db, "en", &line("roleplay:airport:a1", "ai", "Hello"), 300).unwrap();
        add_message(&db, "en", &line("roleplay:airport:a1", "user", "Hi"), 301).unwrap();
        add_message(&db, "en", &line("roleplay:hotel:b2", "ai", "Welcome"), 310).unwrap();
        add_message(&db, "en", &line("roleplay:airport:a1", "feedback", "{}"), 320).unwrap();
        let list = threads(&db, "en", "roleplay:", 5).unwrap();
        assert_eq!(list, [
            Thread { thread: "roleplay:airport:a1".into(), started: 300, turns: 1, ended: true },
            Thread { thread: "roleplay:hotel:b2".into(), started: 310, turns: 0, ended: false },
        ]);
        assert_eq!(threads(&db, "en", "roleplay:", 1).unwrap().len(), 1);
        assert!(threads(&db, "zh", "roleplay:", 5).unwrap().is_empty());
        assert_ne!(new_uid(&db).unwrap(), new_uid(&db).unwrap());
    }

    #[test]
    fn a_session_holds_due_reviews_and_todays_new_words() {
        let db = memory();
        set_setting(&db, "new_per_day", "1").unwrap();
        for word in ["one", "two", "three"] {
            add(&db, word, 10);
        }
        let first = session(&db, 20, 0, "en").unwrap();
        assert_eq!(first.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["one"]);
        grade(&db, first[0].id, 1, 20, 0.9).unwrap();
        // The forgotten word is due again in ten minutes, and today's one new word is used up.
        assert!(session(&db, 21, 0, "en").unwrap().is_empty());
        let later = session(&db, 20 + RELEARN_SECONDS, 0, "en").unwrap();
        assert_eq!(later.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["one"]);
        let s = stats(&db, 20 + RELEARN_SECONDS, 0, "en").unwrap();
        assert_eq!((s.total, s.due, s.new_today, s.new_waiting, s.reviews, s.reviews_today), (3, 1, 0, 2, 1, 1));
    }
}
