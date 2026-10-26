//! The words and the settings, in one SQLite file in the data folder, and the SM-2 schedule of review.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

const DAY: i64 = 86_400;
/// A forgotten word comes back within the same session.
const RELEARN_SECONDS: i64 = 600;

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
    pub created: i64,
    pub due: i64,
    pub interval: f64,
    pub ease: f64,
    pub reps: i64,
    pub lapses: i64,
    /// When the word was first reviewed, 0 while it is still new.
    pub learned_at: i64,
    /// The last pronunciation score, -1 before any.
    pub score: i64,
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    // The file holds API keys and the ChatGPT tokens: readable by its owner only.
    #[cfg(unix)]
    for suffix in ["", "-wal", "-shm"] {
        use std::os::unix::fs::PermissionsExt;
        let file = format!("{}{suffix}", path.display());
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
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
    Ok(db)
}

const COLUMNS: &str = "id, word, ipa, pos, meaning, explain, example, example_vi, note, created, due, \
                       interval, ease, reps, lapses, learned_at, score";

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
        created: r.get(9)?,
        due: r.get(10)?,
        interval: r.get(11)?,
        ease: r.get(12)?,
        reps: r.get(13)?,
        lapses: r.get(14)?,
        learned_at: r.get(15)?,
        score: r.get(16)?,
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
        "WHERE word LIKE ?1 OR meaning LIKE ?1 ORDER BY created DESC",
        [like],
    )
}

pub fn get(db: &Connection, id: i64) -> rusqlite::Result<Option<Word>> {
    Ok(select(db, "WHERE id = ?1", [id])?.pop())
}

/// Inserts a new word or, with an id, changes the text of one; the schedule is left as it is.
pub fn save(db: &Connection, w: &Word, now: i64) -> rusqlite::Result<i64> {
    let word = w.word.trim();
    if w.id == 0 {
        db.execute(
            "INSERT INTO words (word, ipa, pos, meaning, explain, example, example_vi, note, created, due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![word, w.ipa, w.pos, w.meaning, w.explain, w.example, w.example_vi, w.note, now],
        )?;
        Ok(db.last_insert_rowid())
    } else {
        db.execute(
            "UPDATE words SET word = ?2, ipa = ?3, pos = ?4, meaning = ?5, explain = ?6, example = ?7,
             example_vi = ?8, note = ?9 WHERE id = ?1",
            params![w.id, word, w.ipa, w.pos, w.meaning, w.explain, w.example, w.example_vi, w.note],
        )?;
        Ok(w.id)
    }
}

pub fn delete(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM words WHERE id = ?1", [id]).map(|_| ())
}

pub fn set_score(db: &Connection, id: i64, score: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE words SET score = ?2 WHERE id = ?1", [id, score]).map(|_| ())
}

/// What a session works through: the reviews that are due, then as many new words as today still allows.
pub fn session(db: &Connection, now: i64, day_start: i64) -> rusqlite::Result<Vec<Word>> {
    let mut words = select(db, "WHERE learned_at > 0 AND due <= ?1 ORDER BY due", [now])?;
    let left = new_left(db, day_start)?;
    words.extend(select(db, "WHERE learned_at = 0 ORDER BY created LIMIT ?1", [left])?);
    Ok(words)
}

/// How many new words may still be started today.
fn new_left(db: &Connection, day_start: i64) -> rusqlite::Result<i64> {
    let per_day: i64 = setting(db, "new_per_day")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let today: i64 = db.query_row(
        "SELECT count(*) FROM words WHERE learned_at > 0 AND learned_at >= ?1",
        [day_start],
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
}

pub fn stats(db: &Connection, now: i64, day_start: i64) -> rusqlite::Result<Stats> {
    let count = |sql: &str, arg: i64| db.query_row(sql, [arg], |r| r.get::<_, i64>(0));
    let waiting = count("SELECT count(*) FROM words WHERE learned_at = 0 AND ?1 = ?1", 0)?;
    Ok(Stats {
        total: count("SELECT count(*) FROM words WHERE ?1 = ?1", 0)?,
        due: count("SELECT count(*) FROM words WHERE learned_at > 0 AND due <= ?1", now)?,
        new_today: new_left(db, day_start)?.min(waiting),
        new_waiting: waiting,
        learned: count("SELECT count(*) FROM words WHERE learned_at > 0 AND ?1 = ?1", 0)?,
    })
}

/// Records an answer: 1 forgot, 3 hard, 4 good, 5 easy.
pub fn grade(db: &Connection, id: i64, quality: u8, now: i64) -> rusqlite::Result<Option<Word>> {
    let Some(mut w) = get(db, id)? else {
        return Ok(None);
    };
    sm2(&mut w, quality, now);
    db.execute(
        "UPDATE words SET due = ?2, interval = ?3, ease = ?4, reps = ?5, lapses = ?6, learned_at = ?7
         WHERE id = ?1",
        params![w.id, w.due, w.interval, w.ease, w.reps, w.lapses, w.learned_at],
    )?;
    Ok(Some(w))
}

/// SuperMemo 2: the gap grows by the ease after the first two days and six days, a lapse starts over.
pub fn sm2(w: &mut Word, quality: u8, now: i64) {
    let q = f64::from(quality.clamp(0, 5));
    if w.learned_at == 0 {
        w.learned_at = now;
    }
    if quality < 3 {
        w.reps = 0;
        w.lapses += 1;
        w.interval = 0.0;
        w.due = now + RELEARN_SECONDS;
    } else {
        w.reps += 1;
        w.interval = match w.reps {
            1 => 1.0,
            2 => 6.0,
            _ => (w.interval * w.ease).round(),
        };
        if quality == 3 {
            w.interval = (w.interval * 0.8).max(1.0).round();
        }
        w.due = now + (w.interval * DAY as f64) as i64;
    }
    w.ease = (w.ease + 0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02)).max(1.3);
}

/// One day of study, by the local date of the learner (`2026-10-26`).
#[derive(Debug, Serialize, PartialEq)]
pub struct Day {
    pub day: String,
    pub xp: i64,
    pub answers: i64,
    pub correct: i64,
}

/// Adds experience to a day, and an answer when `correct` says how it went.
pub fn add_xp(db: &Connection, day: &str, xp: i64, correct: Option<bool>) -> rusqlite::Result<()> {
    let (answers, right) = match correct {
        Some(ok) => (1, i64::from(ok)),
        None => (0, 0),
    };
    db.execute(
        "INSERT INTO activity (day, xp, answers, correct) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(day) DO UPDATE SET xp = xp + ?2, answers = answers + ?3, correct = correct + ?4",
        params![day, xp, answers, right],
    )
    .map(|_| ())
}

/// The last `days` days that saw any study, newest first, and the experience of all time.
pub fn activity(db: &Connection, days: i64) -> rusqlite::Result<(Vec<Day>, i64)> {
    let mut stmt = db.prepare("SELECT day, xp, answers, correct FROM activity ORDER BY day DESC LIMIT ?1")?;
    let list = stmt
        .query_map([days], |r| Ok(Day { day: r.get(0)?, xp: r.get(1)?, answers: r.get(2)?, correct: r.get(3)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let total = db.query_row("SELECT coalesce(sum(xp), 0) FROM activity", [], |r| r.get(0))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sm2_grows_the_gap_and_a_lapse_resets_it() {
        let mut w = Word { ease: 2.5, ..Word::default() };
        sm2(&mut w, 4, 100);
        assert_eq!((w.reps, w.interval, w.learned_at), (1, 1.0, 100));
        sm2(&mut w, 4, 200);
        assert_eq!(w.interval, 6.0);
        sm2(&mut w, 5, 300);
        // Two "good" answers leave the ease at 2.5; this "easy" raises it only after it is used.
        assert_eq!((w.interval, w.ease), (15.0, 2.6));
        sm2(&mut w, 1, 400);
        assert_eq!((w.reps, w.lapses, w.due), (0, 1, 400 + RELEARN_SECONDS));
        assert!(w.ease >= 1.3);
        assert_eq!(w.learned_at, 100);
    }

    #[test]
    fn experience_adds_up_by_day() {
        let db = open(Path::new(":memory:")).unwrap();
        add_xp(&db, "2026-10-25", 10, Some(true)).unwrap();
        add_xp(&db, "2026-10-26", 10, Some(false)).unwrap();
        add_xp(&db, "2026-10-26", 3, None).unwrap();
        let (days, total) = activity(&db, 30).unwrap();
        assert_eq!(total, 23);
        assert_eq!(days[0], Day { day: "2026-10-26".into(), xp: 13, answers: 1, correct: 0 });
        assert_eq!(days[1].correct, 1);
        assert!(is_day("2026-10-26") && !is_day("2026-1-26") && !is_day("2026-10-2x"));
    }

    #[test]
    fn usage_is_measured_per_provider_and_period() {
        let db = open(Path::new(":memory:")).unwrap();
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
    fn a_session_holds_due_reviews_and_todays_new_words() {
        let db = open(Path::new(":memory:")).unwrap();
        set_setting(&db, "new_per_day", "1").unwrap();
        for word in ["one", "two", "three"] {
            save(&db, &Word { word: word.into(), ..Word::default() }, 10).unwrap();
        }
        let first = session(&db, 20, 0).unwrap();
        assert_eq!(first.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["one"]);
        grade(&db, first[0].id, 1, 20).unwrap();
        // The forgotten word is due again in ten minutes, and today's one new word is used up.
        assert!(session(&db, 21, 0).unwrap().is_empty());
        let later = session(&db, 20 + RELEARN_SECONDS, 0).unwrap();
        assert_eq!(later.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["one"]);
        let s = stats(&db, 20 + RELEARN_SECONDS, 0).unwrap();
        assert_eq!((s.total, s.due, s.new_today, s.new_waiting), (3, 1, 0, 2));
    }
}
