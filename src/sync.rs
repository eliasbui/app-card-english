//! Google Drive sync. One document in the app's hidden Drive folder holds everything learned; each machine
//! downloads it, merges it into its own database row by row (by `uid`, the later `updated` winning, deletions
//! carried by tombstones) and uploads the result. A row the document lacks is never deleted here: only a tombstone
//! deletes. Pictures are files of their own beside the document, named by their hash.

use crate::{db, drive, images};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params_from_iter, types::Value as Sql, types::ValueRef};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// The document's name in the app's folder.
pub const DOC: &str = "card-english-sync.json.gz";

/// Tables of user data that follow the sync convention of the database (`uid`, `updated`, `deleted`), each with
/// the columns that travel, never `id`. A row is merged by its `uid`, the later `updated` winning; a deletion is
/// `deleted = 1` with a later `updated`, so it travels like any change.
pub const TABLES: &[(&str, &[&str])] = &[
    ("chat", &["uid", "updated", "deleted", "lang", "thread", "role", "text", "vi", "reading", "source", "created", "cards"]),
    ("topics", &["uid", "updated", "deleted", "lang", "name", "color", "description"]),
    ("roadmaps", &["uid", "updated", "deleted", "lang", "purpose", "level", "start", "deadline", "minutes", "steps", "active"]),
    ("exams", &["uid", "updated", "deleted", "lang", "roadmap", "stage", "kind", "format", "level", "pass", "paper", "answers", "result",
                "score", "passed", "created", "finished"]),
];

/// The words' columns that travel; `id` stays on each machine and `ease` belongs to the old SM-2 schedule.
const WORD_COLUMNS: &[&str] = &[
    "uid", "updated", "word", "ipa", "pos", "meaning", "explain", "example", "example_vi", "note", "synonyms", "antonyms",
    "collocations", "level", "topic", "lang", "hanviet", "example_reading", "created", "due", "interval", "stability",
    "difficulty", "last_review", "reps", "lapses", "learned_at", "score", "skipped", "image",
];

/// Settings that describe the learning rather than the machine; never keys, tokens, paths, the hotkey or reminders.
/// The ones a course keeps for itself travel for both courses.
const SETTINGS: [&str; 7] = ["new_per_day", "daily_goal", "desired_retention", "palette", "streak_freeze", "accent", "achievements"];

/// A decompressed document larger than this is not read.
const MAX_DOC: u64 = 200 << 20;
const NEWER: &str = "Dữ liệu đồng bộ trên Google Drive do bản Card English mới hơn tạo ra. Hãy cập nhật app trên máy này rồi đồng bộ lại.";

static RUNNING: AtomicBool = AtomicBool::new(false);

/// What a merge brought.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    /// Words and answers that came from the other machines.
    pub words: usize,
    pub reviews: usize,
    /// Rows changed or deleted to match newer copies, and other machines' activity taken in.
    pub changed: usize,
    pub settings: bool,
    /// Pictures of rows changed or deleted here, for `images::release` once the merge is committed.
    pub released: Vec<String>,
}

impl Merged {
    fn add(&mut self, other: Merged) {
        self.words += other.words;
        self.reviews += other.reviews;
        self.changed += other.changed;
        self.settings |= other.settings;
        self.released.extend(other.released);
    }
}

fn synced_settings() -> Vec<String> {
    let mut keys: Vec<String> = SETTINGS.iter().map(|k| k.to_string()).collect();
    keys.extend(SETTINGS.iter().map(|k| db::scoped(k, "zh")).filter(|k| k.contains('@')));
    keys
}

fn to_json(value: ValueRef) -> Value {
    match value {
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
        ValueRef::Null | ValueRef::Blob(_) => Value::Null,
    }
}

fn to_sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0)), Sql::Integer),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

/// The rows of a table as objects of `columns`, each with its local rowid beside it.
fn rows(db: &Connection, table: &str, columns: &[&str], tail: &str, args: &[&str]) -> rusqlite::Result<Vec<(i64, Map<String, Value>)>> {
    let mut stmt = db.prepare(&format!("SELECT rowid, {} FROM {table} {tail}", columns.join(", ")))?;
    stmt.query_map(params_from_iter(args), |r| {
        let mut row = Map::new();
        for (i, column) in columns.iter().enumerate() {
            row.insert(column.to_string(), to_json(r.get_ref(i + 1)?));
        }
        Ok((r.get(0)?, row))
    })?
    .collect()
}

/// Writes the columns of `row` that the table has: over the row of that rowid, or as a new row.
fn write(db: &Connection, table: &str, columns: &[&str], row: &Map<String, Value>, id: Option<i64>) -> rusqlite::Result<()> {
    let present: Vec<&str> = columns.iter().copied().filter(|c| row.contains_key(*c)).collect();
    let mut values: Vec<Sql> = present.iter().map(|c| to_sql(&row[*c])).collect();
    let sql = match id {
        Some(id) => {
            values.push(Sql::Integer(id));
            let set: Vec<String> = present.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 1)).collect();
            format!("UPDATE {table} SET {} WHERE rowid = ?{}", set.join(", "), present.len() + 1)
        }
        None => {
            let marks: Vec<String> = (1..=present.len()).map(|i| format!("?{i}")).collect();
            format!("INSERT INTO {table} ({}) VALUES ({})", present.join(", "), marks.join(", "))
        }
    };
    db.execute(&sql, params_from_iter(values)).map(|_| ())
}

fn text<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn updated(row: &Map<String, Value>) -> i64 {
    row.get("updated").and_then(Value::as_i64).unwrap_or(0)
}

/// Whether `theirs` replaces `ours`: it changed later, or in the same second with content that sorts higher, so
/// that every machine picks the same copy.
fn newer(theirs: &Map<String, Value>, ours: &Map<String, Value>) -> bool {
    let (a, b) = (updated(theirs), updated(ours));
    a > b || (a == b && Value::Object(theirs.clone()).to_string() > Value::Object(ours.clone()).to_string())
}

fn tombstone(db: &Connection, uid: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row("SELECT updated FROM tombstones WHERE uid = ?1", [uid], |r| r.get(0)).optional()
}

fn bury(db: &Connection, uid: &str, at: i64) -> rusqlite::Result<()> {
    db.execute("INSERT INTO tombstones (uid, updated) VALUES (?1, ?2) ON CONFLICT(uid) DO UPDATE SET updated = max(updated, ?2)", rusqlite::params![uid, at])
        .map(|_| ())
}

/// Everything the sync carries, as the document uploads it; `me` is this machine's device id. Rows come in a fixed
/// order, so that the same data always gives the same text.
pub fn export(db: &Connection, me: &str) -> rusqlite::Result<Value> {
    let objects = |list: Vec<(i64, Map<String, Value>)>| list.into_iter().map(|(_, row)| Value::Object(row)).collect::<Vec<_>>();
    let words = objects(rows(db, "words", WORD_COLUMNS, "ORDER BY uid", &[])?);
    let mut stmt = db.prepare("SELECT uid, updated FROM tombstones ORDER BY uid")?;
    let tombstones: Vec<Value> = stmt.query_map([], |r| Ok(json!({ "uid": r.get::<_, String>(0)?, "updated": r.get::<_, i64>(1)? })))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = db.prepare("SELECT r.uid, w.uid, r.at, r.rating, r.prev FROM reviews r JOIN words w ON w.id = r.word_id ORDER BY r.uid")?;
    let reviews: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(json!({ "uid": r.get::<_, String>(0)?, "word": r.get::<_, String>(1)?, "at": r.get::<_, i64>(2)?, "rating": r.get::<_, i64>(3)?,
                       "prev": r.get::<_, String>(4)? }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Each machine's days under its own id, dated by its last change; this machine's rows have no device.
    let mut activity: BTreeMap<String, (i64, Vec<Value>)> = BTreeMap::new();
    let mut stmt = db.prepare("SELECT device, day, lang, xp, answers, correct, updated FROM activity ORDER BY device, day, lang")?;
    let mut found = stmt.query([])?;
    while let Some(r) = found.next()? {
        let device: String = r.get(0)?;
        let entry = activity.entry(if device.is_empty() { me.to_owned() } else { device }).or_default();
        entry.0 = entry.0.max(r.get(6)?);
        entry.1.push(json!({ "day": r.get::<_, String>(1)?, "lang": r.get::<_, String>(2)?, "xp": r.get::<_, i64>(3)?,
                             "answers": r.get::<_, i64>(4)?, "correct": r.get::<_, i64>(5)? }));
    }
    let activity: Map<String, Value> = activity.into_iter().map(|(device, (at, days))| (device, json!({ "updated": at, "days": days }))).collect();
    let mut settings = Map::new();
    for key in synced_settings() {
        let row: Option<(String, i64)> = db.query_row("SELECT value, updated FROM settings WHERE key = ?1", [&key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((value, at)) = row {
            settings.insert(key, json!({ "value": value, "updated": at }));
        }
    }
    let mut tables = Map::new();
    for (name, columns) in TABLES {
        tables.insert(name.to_string(), Value::Array(objects(rows(db, name, columns, "ORDER BY uid", &[])?)));
    }
    Ok(json!({ "app": "card-english", "schema": db::SCHEMA, "words": words, "tombstones": tombstones, "reviews": reviews,
               "activity": activity, "settings": settings, "tables": tables }))
}

fn list<'a>(doc: &'a Value, key: &str) -> impl Iterator<Item = &'a Map<String, Value>> {
    doc[key].as_array().into_iter().flatten().filter_map(Value::as_object)
}

/// Merges a downloaded document into the database, in one transaction. `me` is this machine's device id, whose
/// activity here is the true one; `valid` checks a setting's value as the settings page would.
pub fn merge(db: &Connection, doc: &Value, me: &str, valid: &dyn Fn(&str, &str) -> bool) -> rusqlite::Result<Merged> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let mut m = Merged::default();
    for t in list(doc, "tombstones") {
        merge_tombstone(&tx, text(t, "uid"), updated(t), &mut m)?;
    }
    // Ids of words that became another, so that their answers follow.
    let mut alias: HashMap<String, String> = HashMap::new();
    for w in list(doc, "words") {
        merge_word(&tx, w, &mut alias, &mut m)?;
    }
    for r in list(doc, "reviews") {
        let uid = text(r, "uid");
        let word = alias.get(text(r, "word")).map_or(text(r, "word"), String::as_str);
        if uid.is_empty() || tombstone(&tx, uid)?.is_some() {
            continue;
        }
        let Some(id) = tx.query_row("SELECT id FROM words WHERE uid = ?1", [word], |r| r.get::<_, i64>(0)).optional()? else { continue };
        let (at, rating) = (r.get("at").and_then(Value::as_i64).unwrap_or(0), r.get("rating").and_then(Value::as_i64).unwrap_or(3).clamp(1, 4));
        let prev = r.get("prev").and_then(Value::as_str).unwrap_or("{}");
        // The same answer under another id is one a copy restored on both machines already holds.
        m.reviews += tx.execute(
            "INSERT INTO reviews (word_id, at, rating, prev, uid) SELECT ?1, ?2, ?3, ?4, ?5
             WHERE NOT EXISTS (SELECT 1 FROM reviews WHERE uid = ?5 OR (word_id = ?1 AND at = ?2 AND rating = ?3))",
            rusqlite::params![id, at, rating, prev, uid],
        )?;
    }
    for (device, block) in doc["activity"].as_object().into_iter().flatten() {
        if device == me || device.is_empty() {
            continue;
        }
        let at = block["updated"].as_i64().unwrap_or(0);
        let ours: Option<i64> = tx.query_row("SELECT max(updated) FROM activity WHERE device = ?1", [device], |r| r.get(0))?;
        if ours.is_some_and(|ours| ours >= at) {
            continue;
        }
        tx.execute("DELETE FROM activity WHERE device = ?1", [device])?;
        for d in block["days"].as_array().into_iter().flatten() {
            let (day, lang) = (d["day"].as_str().unwrap_or_default(), d["lang"].as_str().unwrap_or_default());
            if db::is_day(day) && matches!(lang, "en" | "zh") {
                let n = |key: &str| d[key].as_i64().unwrap_or(0).max(0);
                tx.execute(
                    "INSERT OR REPLACE INTO activity (device, day, lang, xp, answers, correct, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![device, day, lang, n("xp"), n("answers"), n("correct"), at],
                )?;
            }
        }
        m.changed += 1;
    }
    let synced = synced_settings();
    for (key, entry) in doc["settings"].as_object().into_iter().flatten() {
        let (value, at) = (entry["value"].as_str().unwrap_or_default(), entry["updated"].as_i64().unwrap_or(0));
        if !synced.contains(key) || !valid(key.split('@').next().unwrap_or_default(), value) {
            continue;
        }
        let ours: Option<i64> = tx.query_row("SELECT updated FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        if ours.is_none_or(|ours| ours < at) {
            tx.execute(
                "INSERT INTO settings (key, value, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated",
                rusqlite::params![key, value, at],
            )?;
            m.settings = true;
        }
    }
    for (name, columns) in TABLES {
        let rows: Vec<&Map<String, Value>> = doc["tables"][name].as_array().into_iter().flatten().filter_map(Value::as_object).collect();
        m.changed += merge_rows(&tx, name, columns, &rows)?;
    }
    tx.commit()?;
    Ok(m)
}

/// A deletion from another machine: the word goes with its answers unless it changed after, an undone answer goes.
fn merge_tombstone(tx: &Connection, uid: &str, at: i64, m: &mut Merged) -> rusqlite::Result<()> {
    if uid.is_empty() || tombstone(tx, uid)?.is_some_and(|ours| ours >= at) {
        return Ok(());
    }
    bury(tx, uid, at)?;
    if let Some((id, word)) = rows(tx, "words", &["updated", "image"], "WHERE uid = ?1", &[uid])?.pop() {
        if updated(&word) <= at {
            tx.execute("DELETE FROM reviews WHERE word_id = ?1", [id])?;
            tx.execute("DELETE FROM words WHERE id = ?1", [id])?;
            m.released.push(text(&word, "image").to_owned());
            m.changed += 1;
        }
    }
    m.changed += tx.execute("DELETE FROM reviews WHERE uid = ?1", [uid])?;
    Ok(())
}

fn merge_word(tx: &Connection, row: &Map<String, Value>, alias: &mut HashMap<String, String>, m: &mut Merged) -> rusqlite::Result<()> {
    let mut theirs: Map<String, Value> = row.iter().filter(|(k, _)| WORD_COLUMNS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    // A row lacking a column would fail the whole merge, every time.
    if theirs.len() < WORD_COLUMNS.len() {
        return Ok(());
    }
    // A picture is only ever a file of the images folder.
    if !images::valid(text(&theirs, "image")) {
        theirs.insert("image".into(), "".into());
    }
    let uid = text(&theirs, "uid").to_owned();
    let word = text(&theirs, "word").trim().to_owned();
    if uid.is_empty() || uid.len() > 64 || word.is_empty() || tombstone(tx, &uid)?.is_some_and(|at| at >= updated(&theirs)) {
        return Ok(());
    }
    let mine = rows(tx, "words", WORD_COLUMNS, "WHERE uid = ?1", &[uid.as_str()])?.pop();
    if mine.as_ref().is_some_and(|(_, ours)| !newer(&theirs, ours)) {
        return Ok(());
    }
    // Written as a live row, it is no longer deleted.
    tx.execute("DELETE FROM tombstones WHERE uid = ?1", [&uid])?;
    match rows(tx, "words", WORD_COLUMNS, "WHERE word = ?1 AND uid != ?2", &[word.as_str(), uid.as_str()])?.pop() {
        Some((other_id, other)) => {
            // The same word under two ids, added on two machines or renamed into one another: it keeps the smaller
            // id and the newer content, its answers from both, and the larger id becomes a tombstone.
            let other_uid = text(&other, "uid").to_owned();
            let (keep, gone) = if uid < other_uid { (uid.clone(), other_uid) } else { (other_uid, uid.clone()) };
            let mut merged = if newer(&theirs, &other) { theirs.clone() } else { other.clone() };
            merged.insert("uid".into(), keep.clone().into());
            if let Some((mine_id, ours)) = mine {
                tx.execute("UPDATE reviews SET word_id = ?1 WHERE word_id = ?2", [other_id, mine_id])?;
                tx.execute("DELETE FROM words WHERE id = ?1", [mine_id])?;
                m.released.push(text(&ours, "image").to_owned());
            }
            m.released.push(text(&other, "image").to_owned());
            write(tx, "words", WORD_COLUMNS, &merged, Some(other_id))?;
            bury(tx, &gone, updated(&theirs).max(updated(&other)))?;
            alias.insert(gone, keep);
            m.changed += 1;
        }
        None => match mine {
            Some((id, ours)) => {
                m.released.push(text(&ours, "image").to_owned());
                write(tx, "words", WORD_COLUMNS, &theirs, Some(id))?;
                m.changed += 1;
            }
            None => {
                write(tx, "words", WORD_COLUMNS, &theirs, None)?;
                m.words += 1;
            }
        },
    }
    Ok(())
}

/// Merges the rows of a table that follows the sync convention; answers how many changed. A row another
/// constraint of the table refuses is left out, not the whole sync.
fn merge_rows(tx: &Connection, table: &str, columns: &[&str], rows_in: &[&Map<String, Value>]) -> rusqlite::Result<usize> {
    let mut changed = 0;
    for theirs in rows_in {
        let uid = text(theirs, "uid");
        if uid.is_empty() {
            continue;
        }
        let mine = rows(tx, table, columns, "WHERE uid = ?1", &[uid])?.pop();
        if mine.as_ref().is_some_and(|(_, ours)| !newer(theirs, ours)) {
            continue;
        }
        match write(tx, table, columns, theirs, mine.map(|(id, _)| id)) {
            Ok(()) => changed += 1,
            Err(rusqlite::Error::SqliteFailure(f, message)) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                eprintln!("sync: {table} row {uid} left out: {}", message.unwrap_or_default());
            }
            Err(e) => return Err(e),
        }
    }
    Ok(changed)
}

/// The pictures the words show.
fn pictures(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = db.prepare("SELECT DISTINCT image FROM words WHERE image != ''")?;
    stmt.query_map([], |r| r.get(0))?.collect()
}

fn sha(text: &[u8]) -> String {
    Sha256::digest(text).iter().map(|b| format!("{b:02x}")).collect()
}

fn gzip(text: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).and_then(|_| encoder.finish()).map_err(|e| e.to_string())
}

/// A downloaded document and the hash of its text; refused when a newer app made it.
fn read_doc(bytes: &[u8]) -> Result<(Value, String), String> {
    let unreadable = "Dữ liệu đồng bộ trên Google Drive không đọc được. Hãy cập nhật Card English lên bản mới nhất rồi đồng bộ lại.";
    let mut text = String::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(bytes).take(MAX_DOC).read_to_string(&mut text).map_err(|_| unreadable.to_owned())?;
    } else {
        text = String::from_utf8(bytes.to_vec()).map_err(|_| unreadable.to_owned())?;
    }
    let doc: Value = serde_json::from_str(&text).map_err(|_| unreadable.to_owned())?;
    if doc["app"] != "card-english" {
        return Err(unreadable.into());
    }
    if doc["schema"].as_i64().unwrap_or(0) > db::SCHEMA {
        return Err(NEWER.into());
    }
    Ok((doc, sha(text.as_bytes())))
}

fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("jpg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "image/webp",
    }
}

/// Syncs now and gives what the page shows of it, or a sentence saying what went wrong and what to do. The time
/// and the error of the last attempt are kept for the settings page.
pub fn run(dir: &Path, valid: fn(&str, &str) -> bool) -> Result<Value, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Đang đồng bộ, chờ một chút rồi thử lại".into());
    }
    let opened = db::open(&dir.join("words.db")).map_err(|e| e.to_string());
    let result = opened.as_ref().map_err(Clone::clone).and_then(|db| sync(db, dir, &valid));
    RUNNING.store(false, Ordering::SeqCst);
    let db = opened?;
    let now = crate::now().to_string();
    let _ = match &result {
        Ok(_) => db::set_setting(&db, "sync_at", &now).and_then(|_| db::set_setting(&db, "sync_error", "")),
        Err(e) => db::set_setting(&db, "sync_error", e),
    };
    let m = result?;
    Ok(json!({ "words": m.words, "reviews": m.reviews, "changed": m.words + m.reviews + m.changed > 0 || m.settings, "status": status(&db) }))
}

/// Download what is new, merge, upload the result. The document's version is checked again just before the upload
/// and, when another machine wrote it meanwhile, the whole round runs once more; a later write is merged next time.
fn sync(db: &Connection, dir: &Path, valid: &dyn Fn(&str, &str) -> bool) -> Result<Merged, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let token = drive::access_token(db)?;
    let me = db::device(db).map_err(e)?;
    let setting = |key: &str| db::setting(db, key).ok().flatten().unwrap_or_default();
    let part = dir.join("sync.part");
    let read_part = || {
        let bytes = fs::read(&part).map_err(|e| e.to_string());
        let _ = fs::remove_file(&part);
        bytes
    };
    let mut merged = Merged::default();
    for attempt in 0..2 {
        let files = drive::list(&token)?;
        let docs: Vec<&drive::File> = files.iter().filter(|f| f.name == DOC).collect();
        let key = |f: &drive::File| format!("{}:{}", f.id, f.version);
        let changed = docs.len() > 1 || docs.first().is_some_and(|d| key(d) != setting("sync_version"));
        let mut remote = None;
        if changed {
            // Two machines that synced first at the same moment each made one: all are merged, the oldest kept.
            for d in &docs {
                drive::download(&token, &d.id, &part)?;
                let (doc, hash) = read_doc(&read_part()?)?;
                let m = merge(db, &doc, &me, valid).map_err(e)?;
                images::release(db, dir, &m.released);
                merged.add(m);
                remote = Some(hash);
            }
        }
        let on_drive: HashMap<&str, &str> = files.iter().map(|f| (f.name.as_str(), f.id.as_str())).collect();
        for name in pictures(db).map_err(e)? {
            if images::valid(&name) && !images::exists(dir, &name) {
                // Not there yet when its machine has not uploaded it; the next sync brings it.
                let Some(id) = on_drive.get(name.as_str()) else { continue };
                drive::download(&token, id, &part)?;
                let bytes = read_part()?;
                if images::name_of(&bytes).ok().as_deref() == Some(name.as_str()) {
                    images::store(dir, &bytes)?;
                }
            }
        }
        let text = export(db, &me).map_err(e)?.to_string();
        let hash = sha(text.as_bytes());
        let same = if changed { remote.as_deref() == Some(hash.as_str()) } else { setting("sync_hash") == hash };
        if let [doc] = docs[..]
            && same
        {
            db::set_setting(db, "sync_version", &key(doc)).and_then(|_| db::set_setting(db, "sync_hash", &hash)).map_err(e)?;
            return Ok(merged);
        }
        if attempt == 0 && docs.first().is_some_and(|d| drive::version(&token, &d.id).is_ok_and(|v| v != d.version)) {
            continue;
        }
        // The pictures first, so that a document never names one that is not there.
        for name in pictures(db).map_err(e)? {
            if images::valid(&name) && !on_drive.contains_key(name.as_str()) {
                if let Ok(bytes) = fs::read(images::folder(dir).join(&name)) {
                    drive::create(&token, &name, &bytes, mime(&name), &part)?;
                }
            }
        }
        let content = gzip(&text)?;
        let file = match docs.first() {
            Some(d) => drive::update(&token, &d.id, &content, "application/gzip", &part)?,
            None => drive::create(&token, DOC, &content, "application/gzip", &part)?,
        };
        for extra in docs.iter().skip(1) {
            let _ = drive::delete(&token, &extra.id);
        }
        db::set_setting(db, "sync_version", &key(&file)).and_then(|_| db::set_setting(db, "sync_hash", &hash)).map_err(e)?;
        break;
    }
    Ok(merged)
}

/// What the settings page shows: the account, the last sync and its error.
pub fn status(db: &Connection) -> Value {
    let mut status = drive::status(db);
    let setting = |key: &str| db::setting(db, key).ok().flatten().unwrap_or_default();
    status["last_sync"] = setting("sync_at").parse::<i64>().unwrap_or(0).into();
    status["error"] = setting("sync_error").into();
    status
}

/// Signs out and forgets what was synced, so that the next account starts with a full merge.
pub fn sign_out(db: &Connection) -> Result<bool, String> {
    let revoked = drive::sign_out(db)?;
    for key in ["sync_version", "sync_hash", "sync_at", "sync_error"] {
        db::set_setting(db, key, "").map_err(|e| e.to_string())?;
    }
    Ok(revoked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Word;

    fn memory() -> Connection {
        db::open(Path::new(":memory:")).unwrap()
    }

    fn any(_: &str, value: &str) -> bool {
        !value.contains('<')
    }

    /// One machine's document merged into another's database.
    fn pass(from: &Connection, from_id: &str, to: &Connection, to_id: &str) -> Merged {
        merge(to, &export(from, from_id).unwrap(), to_id, &any).unwrap()
    }

    fn add(db: &Connection, word: &str, meaning: &str) -> i64 {
        db::save(db, &Word { word: word.into(), meaning: meaning.into(), ..Word::default() }, 100).unwrap()
    }

    fn date(db: &Connection, word: &str, at: i64) {
        db.execute("UPDATE words SET updated = ?2 WHERE word = ?1", rusqlite::params![word, at]).unwrap();
    }

    fn meaning(db: &Connection, word: &str) -> Option<String> {
        db::find(db, word).unwrap().map(|w| w.meaning)
    }

    #[test]
    fn the_chat_topics_and_roadmaps_travel_and_so_do_their_deletions() {
        let (a, b) = (memory(), memory());
        a.execute_batch(
            "INSERT INTO chat (uid, lang, thread, role, text, created, updated) VALUES ('c1', 'en', 'tutor', 'user', 'hi', 5, 5);
             INSERT INTO topics (uid, lang, name, updated) VALUES ('t1', 'en', 'Du lịch', 5);
             INSERT INTO roadmaps (uid, lang, purpose, steps, active, updated) VALUES ('r1', 'zh', 'HSK 4', '[]', 1, 5);",
        )
        .unwrap();
        assert_eq!(pass(&a, "A", &b, "B").changed, 3);
        let count = |db: &Connection, table: &str| -> i64 {
            db.query_row(&format!("SELECT count(*) FROM {table} WHERE deleted = 0"), [], |r| r.get(0)).unwrap()
        };
        assert_eq!((count(&b, "chat"), count(&b, "topics"), count(&b, "roadmaps")), (1, 1, 1));
        a.execute("UPDATE topics SET deleted = 1, updated = 9 WHERE uid = 't1'", []).unwrap();
        pass(&a, "A", &b, "B");
        assert_eq!(count(&b, "topics"), 0);
    }

    #[test]
    fn new_words_from_either_side_meet_and_the_later_edit_wins() {
        let (a, b) = (memory(), memory());
        add(&a, "deadline", "hạn chót");
        add(&b, "itinerary", "lịch trình");
        let id = db::find(&a, "deadline").unwrap().unwrap().id;
        db::grade(&a, id, 3, 1_000, 0.9).unwrap();
        assert_eq!(pass(&a, "A", &b, "B"), Merged { words: 1, reviews: 1, released: vec![], ..Merged::default() });
        pass(&b, "B", &a, "A");
        assert_eq!((meaning(&a, "itinerary"), meaning(&b, "deadline")), (Some("lịch trình".into()), Some("hạn chót".into())));
        let copy = db::find(&b, "deadline").unwrap().unwrap();
        assert_eq!((copy.reps, copy.due, copy.uid.clone()), (1, 1_000 + 2 * db::DAY, db::find(&a, "deadline").unwrap().unwrap().uid));
        // Merging again changes nothing, and both sides export the same document.
        assert_eq!(pass(&a, "A", &b, "B"), Merged::default());
        assert_eq!(export(&a, "X").unwrap(), export(&b, "X").unwrap());

        // Edited on both machines: the later edit wins on both.
        let edit = |db: &Connection, meaning: &str| {
            let w = db::find(db, "deadline").unwrap().unwrap();
            db::save(db, &Word { meaning: meaning.into(), ..w }, 5_000).unwrap();
        };
        edit(&a, "thời hạn");
        edit(&b, "hạn cuối");
        date(&a, "deadline", 5_000);
        date(&b, "deadline", 6_000);
        pass(&a, "A", &b, "B");
        assert_eq!(meaning(&b, "deadline").as_deref(), Some("hạn cuối"), "the older copy does not win");
        assert_eq!(pass(&b, "B", &a, "A").changed, 1);
        assert_eq!(meaning(&a, "deadline").as_deref(), Some("hạn cuối"));
        // Changed in the same second: both machines pick the same copy.
        date(&a, "itinerary", 7_000);
        date(&b, "itinerary", 7_000);
        a.execute("UPDATE words SET note = 'a' WHERE word = 'itinerary'", []).unwrap();
        b.execute("UPDATE words SET note = 'b' WHERE word = 'itinerary'", []).unwrap();
        pass(&a, "A", &b, "B");
        pass(&b, "B", &a, "A");
        assert_eq!(db::find(&a, "itinerary").unwrap().unwrap().note, db::find(&b, "itinerary").unwrap().unwrap().note);
    }

    #[test]
    fn deletions_travel_and_a_later_edit_brings_a_word_back() {
        let (a, b) = (memory(), memory());
        let id = add(&a, "deadline", "hạn chót");
        add(&a, "luggage", "hành lý");
        db::grade(&a, id, 3, 1_000, 0.9).unwrap();
        pass(&a, "A", &b, "B");
        db::delete(&a, id).unwrap();
        let m = pass(&a, "A", &b, "B");
        assert_eq!((m.changed, meaning(&b, "deadline")), (1, None));
        assert_eq!(db::stats(&b, 0, 0, "").unwrap().reviews, 0, "its answers go with it");
        // B's older copy does not bring it back on A, and A's missing words are never deleted from B.
        pass(&b, "B", &a, "A");
        assert_eq!(meaning(&a, "deadline"), None);
        a.execute("DELETE FROM words WHERE word = 'luggage'", []).unwrap();
        pass(&a, "A", &b, "B");
        assert!(meaning(&b, "luggage").is_some(), "a row the document lacks stays");

        // Deleted on A, then edited on B later: the edit wins and the word comes back to A.
        let id = db::find(&b, "luggage").unwrap().unwrap().id;
        pass(&b, "B", &a, "A");
        let id_a = db::find(&a, "luggage").unwrap().unwrap().id;
        db::delete(&a, id_a).unwrap();
        a.execute("UPDATE tombstones SET updated = 8000", []).unwrap();
        db::set_score(&b, id, 88).unwrap();
        date(&b, "luggage", 9_000);
        pass(&b, "B", &a, "A");
        assert_eq!(db::find(&a, "luggage").unwrap().unwrap().score, 88);
        assert!(!export(&a, "A").unwrap()["tombstones"].as_array().unwrap().iter().any(|t| t["uid"] == db::find(&a, "luggage").unwrap().unwrap().uid));
    }

    #[test]
    fn an_undone_answer_is_undone_everywhere_and_answers_are_not_doubled() {
        let (a, b) = (memory(), memory());
        let id = add(&a, "deadline", "hạn chót");
        db::grade(&a, id, 3, 1_000, 0.9).unwrap();
        db::grade(&a, id, 1, 2_000, 0.9).unwrap();
        pass(&a, "A", &b, "B");
        assert_eq!(db::stats(&b, 0, 0, "").unwrap().reviews, 2);
        db::undo(&a, id).unwrap();
        pass(&a, "A", &b, "B");
        assert_eq!(db::stats(&b, 0, 0, "").unwrap().reviews, 1);
        assert_eq!(db::find(&b, "deadline").unwrap().unwrap().reps, 1, "the schedule before the answer");
        // A review restored from a copy on both machines got another id on each: it counts once.
        let doc = export(&a, "A").unwrap();
        let mut copy = doc.clone();
        copy["reviews"][0]["uid"] = "ffffffffffffffff".into();
        assert_eq!(merge(&b, &copy, "B", &any).unwrap().reviews, 0);
    }

    #[test]
    fn the_same_word_added_on_two_machines_becomes_one() {
        let (a, b) = (memory(), memory());
        let ia = add(&a, "deadline", "hạn chót");
        let ib = add(&b, "Deadline", "thời hạn");
        db::grade(&a, ia, 3, 1_000, 0.9).unwrap();
        db::grade(&b, ib, 4, 3_000, 0.9).unwrap();
        date(&a, "deadline", 1_000);
        date(&b, "deadline", 3_000);
        let (ua, ub) = (db::get(&a, ia).unwrap().unwrap().uid, db::get(&b, ib).unwrap().unwrap().uid);
        let keep = ua.clone().min(ub.clone());
        pass(&a, "A", &b, "B");
        pass(&b, "B", &a, "A");
        for db in [&a, &b] {
            let w = db::find(db, "deadline").unwrap().unwrap();
            assert_eq!((w.uid.as_str(), w.meaning.as_str()), (keep.as_str(), "thời hạn"), "the smaller id, the newer content");
            assert_eq!(db::stats(db, 0, 0, "").unwrap().reviews, 2, "the answers of both");
            assert_eq!(db::list(db, "").unwrap().len(), 1);
        }
        // The id that went is a tombstone, so a third machine with the old copy folds it in too.
        let c = memory();
        let ic = add(&c, "deadline", "cũ");
        c.execute("UPDATE words SET uid = ?1, updated = 500 WHERE id = ?2", rusqlite::params![ua.max(ub), ic]).unwrap();
        pass(&a, "A", &c, "C");
        let w = db::find(&c, "deadline").unwrap().unwrap();
        assert_eq!((w.uid.as_str(), w.meaning.as_str(), db::list(&c, "").unwrap().len()), (keep.as_str(), "thời hạn", 1));
    }

    #[test]
    fn each_machine_keeps_its_own_activity_and_the_total_is_the_sum() {
        let (a, b) = (memory(), memory());
        db::add_xp(&a, "2026-10-28", "en", 10, Some(true)).unwrap();
        db::add_xp(&b, "2026-10-28", "en", 6, Some(false)).unwrap();
        db::add_xp(&b, "2026-10-27", "zh", 4, Some(true)).unwrap();
        for _ in 0..3 {
            pass(&a, "A", &b, "B");
            pass(&b, "B", &a, "A");
        }
        for db in [&a, &b] {
            let (days, total) = db::activity(db, 30, "en").unwrap();
            assert_eq!((total, days[0].answers, days[0].correct), (16, 2, 1), "the same day on two machines counts once each");
            assert_eq!(db::activity(db, 30, "zh").unwrap().1, 4);
        }
        // More study on A reaches B as A's block, replacing the older one instead of adding to it.
        db::add_xp(&a, "2026-10-28", "en", 5, None).unwrap();
        pass(&a, "A", &b, "B");
        assert_eq!(db::activity(&b, 30, "en").unwrap().1, 21);
        // A document holding this machine's own block never overrides it, nor an older copy of another's.
        let stale = export(&b, "B").unwrap();
        db::take_back_xp(&a, "2026-10-28", "en", 10, true).unwrap();
        merge(&a, &stale, "A", &any).unwrap();
        assert_eq!(db::activity(&a, 30, "en").unwrap().1, 11);
        pass(&a, "A", &b, "B");
        merge(&b, &stale, "B", &any).unwrap();
        assert_eq!(db::activity(&b, 30, "en").unwrap().1, 11);
    }

    #[test]
    fn pictures_follow_their_words_and_freed_ones_are_named() {
        let (a, b) = (memory(), memory());
        let name = format!("{}.webp", "a".repeat(64));
        let id = add(&a, "apple", "quả táo");
        db::set_image(&a, id, &name).unwrap();
        pass(&a, "A", &b, "B");
        assert_eq!(pictures(&b).unwrap(), [name.clone()]);
        let other = format!("{}.jpg", "b".repeat(64));
        db::set_image(&a, id, &other).unwrap();
        date(&a, "apple", crate::now() + 10);
        let m = pass(&a, "A", &b, "B");
        assert_eq!((m.released, pictures(&b).unwrap()), (vec![name], vec![other.clone()]));
        db::delete(&a, id).unwrap();
        a.execute("UPDATE tombstones SET updated = ?1", [crate::now() + 20]).unwrap();
        assert_eq!(pass(&a, "A", &b, "B").released, [other]);
        // A name that is not a file of the images folder never arrives.
        add(&b, "pear", "quả lê");
        let mut doc = export(&b, "B").unwrap();
        doc["words"][0]["image"] = "../words.db".into();
        merge(&a, &doc, "A", &any).unwrap();
        assert_eq!(db::find(&a, "pear").unwrap().unwrap().image, "");
        // Nor a row without all its columns.
        doc["words"] = json!([{ "uid": "0123456789abcdef", "updated": 1, "word": "plum" }]);
        assert_eq!(merge(&a, &doc, "A", &any).unwrap().words, 0);
    }

    #[test]
    fn learning_settings_travel_and_nothing_else_does() {
        let (a, b) = (memory(), memory());
        db::set_setting(&a, "daily_goal", "80").unwrap();
        db::set_setting(&a, "palette@zh", "pink").unwrap();
        db::set_setting(&a, "gemini_key", "secret").unwrap();
        db::set_setting(&a, "hotkey", "Alt+Shift+E").unwrap();
        let doc = export(&a, "A").unwrap();
        assert!(!doc.to_string().contains("secret") && !doc.to_string().contains("hotkey") && !doc.to_string().contains("device_id"));
        assert!(pass(&a, "A", &b, "B").settings);
        assert_eq!(db::setting(&b, "daily_goal").unwrap().as_deref(), Some("80"));
        assert_eq!(db::setting(&b, "palette@zh").unwrap().as_deref(), Some("pink"));
        assert_eq!(db::setting(&b, "gemini_key").unwrap(), None);
        // A later change here is not undone by the older one there, and a value the page would refuse is left out.
        db::set_setting(&b, "daily_goal", "30").unwrap();
        b.execute("UPDATE settings SET updated = updated + 5 WHERE key = 'daily_goal'", []).unwrap();
        assert!(!pass(&a, "A", &b, "B").settings);
        let mut bad = export(&a, "A").unwrap();
        bad["settings"]["accent"] = json!({ "value": "<b>", "updated": i64::MAX });
        assert!(!merge(&b, &bad, "B", &any).unwrap().settings);
    }

    #[test]
    fn tables_of_the_convention_merge_row_by_row() {
        let (a, b) = (memory(), memory());
        let table = "CREATE TABLE notes (id INTEGER PRIMARY KEY, uid TEXT NOT NULL UNIQUE, updated INTEGER NOT NULL,
                                         deleted INTEGER NOT NULL DEFAULT 0, body TEXT NOT NULL DEFAULT '')";
        let columns = &["uid", "updated", "deleted", "body"];
        for db in [&a, &b] {
            db.execute(table, []).unwrap();
        }
        a.execute("INSERT INTO notes (uid, updated, body) VALUES ('n1', 10, 'first'), ('n2', 10, 'second')", []).unwrap();
        b.execute("INSERT INTO notes (uid, updated, body) VALUES ('n2', 20, 'second, edited'), ('n3', 5, 'third')", []).unwrap();
        let theirs = rows(&a, "notes", columns, "", &[]).unwrap();
        let theirs: Vec<&Map<String, Value>> = theirs.iter().map(|(_, r)| r).collect();
        assert_eq!(merge_rows(&b, "notes", columns, &theirs).unwrap(), 1);
        a.execute("UPDATE notes SET deleted = 1, updated = 30 WHERE uid = 'n1'", []).unwrap();
        let theirs = rows(&a, "notes", columns, "", &[]).unwrap();
        merge_rows(&b, "notes", columns, &theirs.iter().map(|(_, r)| r).collect::<Vec<_>>()).unwrap();
        let mut got: Vec<(String, i64, String)> = b
            .prepare("SELECT uid, deleted, body FROM notes ORDER BY uid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        got.sort();
        assert_eq!(got, [("n1".into(), 1, "first".into()), ("n2".into(), 0, "second, edited".into()), ("n3".into(), 0, "third".into())]);
    }

    #[test]
    fn documents_round_trip_and_newer_ones_are_refused() {
        let a = memory();
        add(&a, "deadline", "hạn chót");
        let text = export(&a, "A").unwrap().to_string();
        let (doc, hash) = read_doc(&gzip(&text).unwrap()).unwrap();
        assert_eq!((doc, hash), (export(&a, "A").unwrap(), sha(text.as_bytes())));
        assert_eq!(read_doc(br#"{"app":"card-english","schema":999}"#).unwrap_err(), NEWER);
        assert!(read_doc(b"<html>").is_err());
        assert!(read_doc(br#"{"app":"other"}"#).is_err());
    }
}
