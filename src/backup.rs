//! The word list in and out of the app: CSV for spreadsheets and Anki, a JSON copy of all that was learned (never
//! the keys and tokens kept with the settings), and the copy made each day in the data folder.

use crate::db::{self, Word};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const COLUMNS: [&str; 15] = [
    "word", "ipa", "pos", "meaning", "explain", "example", "example_vi", "note", "synonyms", "antonyms", "collocations",
    "level", "topic", "hanviet", "example_reading",
];
/// The largest file the page may send, and the most words taken from one.
pub const MAX_BYTES: usize = 20 << 20;
const MAX_WORDS: usize = 50_000;
/// The daily copies kept in the data folder.
const KEEP_DAILY: usize = 7;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Imported {
    pub added: usize,
    /// Already in the list, so left as they are.
    pub skipped: usize,
    /// Rows without a word, or with one longer than a word can be.
    pub invalid: usize,
}

fn cells(w: &Word) -> [&str; 15] {
    [
        &w.word, &w.ipa, &w.pos, &w.meaning, &w.explain, &w.example, &w.example_vi, &w.note, &w.synonyms, &w.antonyms,
        &w.collocations, &w.level, &w.topic, &w.hanviet, &w.example_reading,
    ]
}

fn set_cell(w: &mut Word, column: usize, value: String) {
    let slot = match column {
        0 => &mut w.word,
        1 => &mut w.ipa,
        2 => &mut w.pos,
        3 => &mut w.meaning,
        4 => &mut w.explain,
        5 => &mut w.example,
        6 => &mut w.example_vi,
        7 => &mut w.note,
        8 => &mut w.synonyms,
        9 => &mut w.antonyms,
        10 => &mut w.collocations,
        11 => &mut w.level,
        12 => &mut w.topic,
        13 => &mut w.hanviet,
        14 => &mut w.example_reading,
        _ => return,
    };
    *slot = value;
}

/// CSV with a header row, in UTF-8 with the mark Excel needs to read it so.
pub fn csv(words: &[Word]) -> String {
    let line = |cells: &[&str]| cells.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",") + "\r\n";
    let mut out = String::from("\u{feff}");
    out.push_str(&line(&COLUMNS));
    for w in words {
        out.push_str(&line(&cells(w)));
    }
    out
}

fn quote(cell: &str) -> String {
    if cell.contains([',', '"', '\n', '\r']) || cell.trim() != cell {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
    }
}

/// The rows of separated text, whose quoted cells may hold the separator, quotes and line breaks.
fn rows(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => cell.push(c),
            }
        } else if c == '"' && cell.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|c| !c.is_empty()) {
                out.push(std::mem::take(&mut row));
            }
            row.clear();
        } else {
            cell.push(c);
        }
    }
    row.push(cell);
    if row.iter().any(|c| !c.is_empty()) {
        out.push(row);
    }
    out
}

/// The text of an Anki field: tags dropped, line breaks as spaces, entities decoded.
fn strip_html(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim_start_matches('/').to_ascii_lowercase();
        if tag.starts_with("br") || tag.starts_with("div") || tag.starts_with('p') || tag.starts_with("li") {
            out.push(' ');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let mut text = out;
    for (entity, c) in [("&nbsp;", " "), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&apos;", "'"), ("&amp;", "&")] {
        text = text.replace(entity, c);
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a file's own first lines say about it: Anki writes `#separator:tab`, `#html:true`, `#columns:...` and which
/// columns hold the note type, the deck, the GUID and the tags.
#[derive(Debug, Default)]
struct Header {
    sep: Option<char>,
    html: bool,
    names: Vec<String>,
    /// Zero-based columns that are not fields of the note.
    meta: Vec<usize>,
}

fn header(text: &str) -> (Header, &str) {
    let mut h = Header::default();
    let mut rest = text;
    while rest.starts_with('#') {
        let (line, tail) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = tail;
        let Some((key, value)) = line[1..].trim_end_matches('\r').split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "separator" => {
                h.sep = match value.to_ascii_lowercase().as_str() {
                    "tab" => Some('\t'),
                    "comma" => Some(','),
                    "semicolon" => Some(';'),
                    "pipe" => Some('|'),
                    "space" => Some(' '),
                    other => other.chars().next(),
                }
            }
            "html" => h.html = value.eq_ignore_ascii_case("true"),
            "columns" => h.names = value.split(['\t', ',', ';']).map(|n| n.trim().to_lowercase()).collect(),
            key if key.ends_with(" column") => {
                if let Ok(n) = value.parse::<usize>() {
                    h.meta.push(n.saturating_sub(1));
                }
            }
            _ => {}
        }
    }
    (h, rest)
}

/// The column of the word each cell of a header row fills, when the row is one.
fn header_row(row: &[String]) -> Option<Vec<Option<usize>>> {
    let names: Vec<String> = row.iter().map(|c| c.trim().to_lowercase()).collect();
    let column = |name: &str| match name {
        "front" | "từ" => Some(0),
        "back" | "nghĩa" | "nghia" | "meaning" | "definition" => Some(3),
        _ => COLUMNS.iter().position(|c| *c == name),
    };
    let map: Vec<Option<usize>> = names.iter().map(|n| column(n)).collect();
    map.contains(&Some(0)).then_some(map)
}

/// Adds the words of a CSV, a tab-separated or Anki text file, or restores a JSON copy. Words already in the list
/// are left as they are, so importing twice changes nothing.
pub fn import(db: &Connection, name: &str, text: &str, now: i64) -> Result<Imported, String> {
    if text.len() > MAX_BYTES {
        return Err("File quá lớn, tối đa 20 MB".into());
    }
    let text = text.trim_start_matches('\u{feff}');
    if name.to_ascii_lowercase().ends_with(".json") || text.trim_start().starts_with('{') {
        let json: Value = serde_json::from_str(text).map_err(|_| "File JSON này không đọc được".to_owned())?;
        return restore(db, &json);
    }
    let (head, body) = header(text);
    let first = body.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let sep = head.sep.unwrap_or_else(|| {
        if first.contains('\t') {
            '\t'
        } else if first.matches(';').count() > first.matches(',').count() {
            ';'
        } else {
            ','
        }
    });
    let mut all = rows(body, sep);
    // Columns by name from a header row or Anki's #columns; otherwise the first field is the word, the second its meaning.
    let named = if head.names.is_empty() { None } else { header_row(&head.names) };
    let map = match named.or_else(|| all.first().and_then(|r| header_row(r))) {
        Some(map) => {
            if head.names.is_empty() {
                all.remove(0);
            }
            map
        }
        None => {
            let width = all.iter().map(Vec::len).max().unwrap_or(0);
            let mut fields = 0;
            (0..width)
                .map(|i| {
                    if head.meta.contains(&i) {
                        return None;
                    }
                    fields += 1;
                    match fields {
                        1 => Some(0),
                        2 => Some(3),
                        _ => None,
                    }
                })
                .collect()
        }
    };
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut out = Imported::default();
    for row in all.into_iter().take(MAX_WORDS) {
        let mut w = Word::default();
        for (i, cell) in row.into_iter().enumerate() {
            if let Some(Some(column)) = map.get(i) {
                let value = if head.html { strip_html(&cell) } else { cell.trim().to_owned() };
                set_cell(&mut w, *column, value);
            }
        }
        add(&tx, w, now, &mut out)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

fn add(db: &Connection, mut w: Word, now: i64, out: &mut Imported) -> Result<(), String> {
    w.word = w.word.trim().to_owned();
    if w.word.is_empty() || w.word.chars().count() > 80 {
        out.invalid += 1;
        return Ok(());
    }
    if db::find(db, &w.word).map_err(|e| e.to_string())?.is_some() {
        out.skipped += 1;
        return Ok(());
    }
    db::save(db, &w, now).map_err(|e| e.to_string())?;
    out.added += 1;
    Ok(())
}

/// Everything learned: the words with their schedules, the days of study and the answers.
pub fn backup(db: &Connection, now: i64) -> rusqlite::Result<String> {
    let words = db::list(db, "")?;
    let mut days = Vec::new();
    for lang in ["en", "zh"] {
        for d in db::activity(db, i64::MAX, lang)?.0 {
            days.push(json!({ "day": d.day, "lang": lang, "xp": d.xp, "answers": d.answers, "correct": d.correct }));
        }
    }
    let reviews = db::review_log(db)?;
    Ok(json!({ "app": "card-english", "version": 1, "exported_at": now, "words": words, "activity": days, "reviews": reviews })
        .to_string())
}

/// Puts back what a copy has and the list lacks: new words with their schedules and answers, and the days of study
/// not already recorded. Nothing already here is changed.
fn restore(db: &Connection, json: &Value) -> Result<Imported, String> {
    if json["app"] != "card-english" || !json["words"].is_array() {
        return Err("Đây không phải bản sao lưu của Card English".into());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut out = Imported::default();
    let mut added: HashMap<String, i64> = HashMap::new();
    for value in json["words"].as_array().into_iter().flatten().take(MAX_WORDS) {
        let Ok(mut w) = serde_json::from_value::<Word>(value.clone()) else {
            out.invalid += 1;
            continue;
        };
        w.word = w.word.trim().to_owned();
        if w.word.is_empty() || w.word.chars().count() > 80 {
            out.invalid += 1;
        } else if db::find(&tx, &w.word).map_err(|e| e.to_string())?.is_some() {
            out.skipped += 1;
        } else {
            w.id = 0;
            let id = db::insert(&tx, &w).map_err(|e| e.to_string())?;
            added.insert(w.word.to_lowercase(), id);
            out.added += 1;
        }
    }
    for d in json["activity"].as_array().into_iter().flatten() {
        let day = d["day"].as_str().unwrap_or_default();
        // A copy from before the courses kept their own experience is English.
        let lang = if d["lang"] == "zh" { "zh" } else { "en" };
        if db::is_day(day) {
            let n = |key: &str| d[key].as_i64().unwrap_or(0).max(0);
            tx.execute(
                "INSERT OR IGNORE INTO activity (day, lang, xp, answers, correct) VALUES (?1, ?5, ?2, ?3, ?4)",
                rusqlite::params![day, n("xp"), n("answers"), n("correct"), lang],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    for r in json["reviews"].as_array().into_iter().flatten() {
        let word = r["word"].as_str().unwrap_or_default().to_lowercase();
        if let Some(id) = added.get(&word) {
            let (at, rating) = (r["at"].as_i64().unwrap_or(0), r["rating"].as_i64().unwrap_or(3).clamp(1, 4));
            tx.execute("INSERT INTO reviews (word_id, at, rating, prev) VALUES (?1, ?2, ?3, '{}')", rusqlite::params![id, at, rating])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

/// A file in the Downloads folder that does not exist yet: `name`, or `name (2)` and so on.
pub fn download_path(name: &str) -> PathBuf {
    let folder = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (1..1000)
        .map(|n| folder.join(if n == 1 { name.to_owned() } else { format!("{stem} ({n}).{ext}") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| folder.join(name))
}

/// Writes `text` whole: into a file beside `to` first, which then takes its name.
pub fn write(to: &Path, text: &str) -> Result<(), String> {
    let part = to.with_extension("part");
    std::fs::write(&part, text).map_err(|e| format!("Không ghi được {}: {e}", to.display()))?;
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Once a day, a copy of everything learned in `backups` of the data folder; the newest few are kept.
pub fn daily(dir: &Path, db: &Connection, date: &str, now: i64) -> Result<(), String> {
    let folder = dir.join("backups");
    let file = folder.join(format!("card-english-{date}.json"));
    if file.exists() || db::stats(db, now, now, "").map_err(|e| e.to_string())?.total == 0 {
        return Ok(());
    }
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    write(&file, &backup(db, now).map_err(|e| e.to_string())?)?;
    let mut copies: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card-english-") && n.ends_with(".json")))
        .collect();
    // The date in the name sorts them, newest last.
    copies.sort();
    for old in copies.iter().rev().skip(KEEP_DAILY) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        db::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn csv_cells_survive_commas_quotes_and_lines() {
        let w = Word { word: "take off".into(), meaning: "cất cánh, cởi".into(), example: "He said \"go\"\nnow".into(), topic: " Du lịch".into(), ..Word::default() };
        let text = csv(std::slice::from_ref(&w));
        assert!(text.starts_with("\u{feff}word,ipa,pos,meaning"));
        let db = memory();
        assert_eq!(import(&db, "list.csv", &text, 5).unwrap(), Imported { added: 1, skipped: 0, invalid: 0 });
        let back = db::find(&db, "take off").unwrap().unwrap();
        assert_eq!((back.meaning.as_str(), back.example.as_str(), back.topic.as_str()), ("cất cánh, cởi", "He said \"go\"\nnow", "Du lịch"));
        assert_eq!(back.ipa, "/teɪk ɔf/");
        // The same file again adds nothing.
        assert_eq!(import(&db, "list.csv", &text, 5).unwrap(), Imported { added: 0, skipped: 1, invalid: 0 });
    }

    #[test]
    fn rows_split_on_quotes_and_line_ends() {
        assert_eq!(rows("a,\"b,c\"\r\n\r\n\"x\"\"y\",z", ','), [vec!["a", "b,c"], vec!["x\"y", "z"]]);
        assert_eq!(rows("one\ttwo\n", '\t'), [vec!["one", "two"]]);
        assert!(rows("\n\n", ',').is_empty());
    }

    #[test]
    fn anki_text_and_plain_lists() {
        let db = memory();
        let anki = "#separator:tab\n#html:true\n#notetype column:1\n#deck column:2\nBasic\tEnglish\t<b>reliable</b>\tđáng&nbsp;tin<br>cậy\nBasic\tEnglish\t\tno word\n";
        assert_eq!(import(&db, "deck.txt", anki, 1).unwrap(), Imported { added: 1, skipped: 0, invalid: 1 });
        assert_eq!(db::find(&db, "reliable").unwrap().unwrap().meaning, "đáng tin cậy");
        // No header: the first column is the word, the second its meaning, whatever the separator.
        assert_eq!(import(&db, "list.txt", "commute;đi làm\nluggage;hành lý", 1).unwrap().added, 2);
        assert_eq!(db::find(&db, "luggage").unwrap().unwrap().meaning, "hành lý");
        assert_eq!(import(&db, "x.csv", "Front,Back\nsouvenir,quà lưu niệm", 1).unwrap().added, 1);
        assert_eq!(strip_html("a<div>b</div>&amp;<i>c</i>"), "a b &c");
        // A Chinese word goes to its course, with its pinyin and Sino-Vietnamese reading.
        assert_eq!(import(&db, "zh.txt", "你好\txin chào\n", 1).unwrap().added, 1);
        let hello = db::find(&db, "你好").unwrap().unwrap();
        assert_eq!((hello.lang.as_str(), hello.ipa.as_str(), hello.meaning.as_str()), ("zh", "nǐhǎo", "xin chào"));
    }

    #[test]
    fn a_copy_restores_what_the_list_lacks() {
        let db = memory();
        let id = db::save(&db, &Word { word: "deadline".into(), meaning: "hạn chót".into(), ..Word::default() }, 1).unwrap();
        db::grade(&db, id, 3, 100, 0.9).unwrap();
        db::add_xp(&db, "2026-10-26", "en", 10, Some(true)).unwrap();
        db::add_xp(&db, "2026-10-26", "zh", 4, Some(true)).unwrap();
        let copy = backup(&db, 200).unwrap();
        assert!(!copy.contains("_key") && !copy.contains("token"));

        let other = memory();
        db::save(&other, &Word { word: "Deadline".into(), meaning: "giữ nguyên".into(), ..Word::default() }, 1).unwrap();
        db::save(&db, &Word { word: "brand new".into(), ..Word::default() }, 2).unwrap();
        let copy = backup(&db, 300).unwrap();
        assert_eq!(import(&other, "card-english.json", &copy, 400).unwrap(), Imported { added: 1, skipped: 1, invalid: 0 });
        assert_eq!(db::find(&other, "deadline").unwrap().unwrap().meaning, "giữ nguyên");
        assert_eq!(db::activity(&other, 10, "en").unwrap().1, 10);
        assert_eq!(db::activity(&other, 10, "zh").unwrap().1, 4);
        assert_eq!(import(&other, "x.json", "{\"app\":\"other\"}", 1).unwrap_err(), "Đây không phải bản sao lưu của Card English");

        let third = memory();
        assert_eq!(import(&third, "copy.json", &copy, 400).unwrap().added, 2);
        let restored = db::find(&third, "deadline").unwrap().unwrap();
        assert_eq!((restored.interval, restored.reps), (2.0, 1));
        assert_eq!(db::stats(&third, 0, 0, "").unwrap().reviews, 1);
    }

    #[test]
    fn the_daily_copy_is_made_once_and_old_ones_go() {
        let dir = std::env::temp_dir().join(format!("card-english-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = memory();
        daily(&dir, &db, "2026-10-01", 0).unwrap();
        assert!(!dir.join("backups").exists(), "an empty list is not copied");
        db::save(&db, &Word { word: "one".into(), ..Word::default() }, 1).unwrap();
        for day in 1..=9 {
            daily(&dir, &db, &format!("2026-10-{day:02}"), 0).unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.join("backups")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names.len(), KEEP_DAILY);
        assert_eq!(names[0], "card-english-2026-10-03.json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
