//! Topics the learner makes, and goals with a roadmap of stages towards them: one active roadmap per course. A
//! stage counts the learned words of its topics against its target, and the goal compares the words still to
//! learn before the deadline with the learner's recent pace.

use crate::{db::{self, DAY}, exam};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// The colours a topic may take; the page draws each in both themes.
pub const COLORS: [&str; 7] = ["blue", "green", "amber", "red", "purple", "teal", "gray"];
/// The exercises of the app a stage can point to, by the names `startPractice` in the page takes.
pub const EXERCISES: [&str; 7] = ["flash", "fill", "choice", "dict", "speak", "write", "story"];
pub const MIN_STEPS: usize = 4;
pub const MAX_STEPS: usize = 12;
const MAX_TARGET: i64 = 5000;
/// How far back the learner's pace is measured.
const PACE_DAYS: i64 = 14;

/// Stages that fill up a roadmap the AI left short, or start one the learner makes by hand: the title, the topic,
/// the skills and the exercises.
const TEMPLATE_EN: [(&str, &str, &str, [&str; 2]); 5] = [
    ("Giao tiếp hằng ngày", "Giao tiếp hằng ngày", "Nghe, Phát âm", ["flash", "speak"]),
    ("Công việc văn phòng", "Công việc văn phòng", "Đọc, Viết", ["fill", "write"]),
    ("Du lịch", "Du lịch", "Nghe, Nói", ["dict", "choice"]),
    ("Sức khỏe", "Sức khỏe", "Đọc, Từ vựng", ["choice", "story"]),
    ("Mua sắm", "Mua sắm", "Nghe, Đọc", ["flash", "fill"]),
];
const TEMPLATE_ZH: [(&str, &str, &str, [&str; 2]); 5] = [
    ("Chào hỏi", "Chào hỏi", "Nghe, Phát âm", ["flash", "speak"]),
    ("Gia đình", "Gia đình", "Nghe, Nói", ["choice", "dict"]),
    ("Ăn uống", "Ăn uống", "Đọc, Viết", ["fill", "write"]),
    ("Mua sắm", "Mua sắm", "Nghe, Đọc", ["dict", "story"]),
    ("Du lịch", "Du lịch", "Đọc, Từ vựng", ["flash", "choice"]),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Topic {
    /// Empty for a topic only words have, with no row of its own yet.
    pub uid: String,
    pub name: String,
    pub color: String,
    pub description: String,
    /// Its words and how many of them are learned, without the ones set aside.
    pub words: i64,
    pub learned: i64,
}

/// A stage of a roadmap.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Step {
    /// 16 hex digits given as the roadmap is saved, which the stage's tests refer to.
    pub uid: String,
    pub title: String,
    pub topics: Vec<String>,
    /// Words of its topics to learn.
    pub target: i64,
    pub skills: String,
    /// Names from `EXERCISES`.
    pub exercises: Vec<String>,
    /// How long it lasts; the stages follow one another from the start of the roadmap.
    pub days: i64,
    /// Marked done by the learner, whatever the count says.
    pub done: bool,
}

/// A goal and its roadmap.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Roadmap {
    pub uid: String,
    /// What it is for, such as "IELTS 6.5".
    pub purpose: String,
    /// The level the learner starts from: A1 to C2, HSK1 to HSK9, or empty.
    pub level: String,
    /// The day it was made and the day it is for, `YYYY-MM-DD`.
    pub start: String,
    pub deadline: String,
    pub minutes: i64,
    pub steps: Vec<Step>,
    pub active: bool,
}

/// Where the learner stands against the deadline.
#[derive(Debug, Serialize, PartialEq)]
pub struct Pace {
    /// Words of the stages not done yet still to learn.
    pub remaining: i64,
    pub days_left: i64,
    /// New words a day that meet the deadline.
    pub needed: i64,
    /// New words a day learned lately.
    pub recent: f64,
    /// "done", "late", "ahead", "on_track" or "behind"; "test" when every word is learned but a stage test is not
    /// passed yet.
    pub verdict: &'static str,
}

/// The pace for `remaining` words in `days_left` days, from `learned` words over the last `days` days.
pub fn pace(remaining: i64, days_left: i64, learned: i64, days: i64) -> Pace {
    let recent = learned as f64 / days.max(1) as f64;
    let (needed, verdict) = if remaining <= 0 {
        (0, "done")
    } else if days_left <= 0 {
        (remaining, "late")
    } else {
        let exact = remaining as f64 / days_left as f64;
        let verdict = if recent >= exact * 1.25 {
            "ahead"
        } else if recent >= exact * 0.9 {
            "on_track"
        } else {
            "behind"
        };
        ((remaining + days_left - 1) / days_left, verdict)
    };
    Pace { remaining: remaining.max(0), days_left, needed, recent, verdict }
}

/// The days from 1970-01-01 to a `YYYY-MM-DD` date (the days-from-civil algorithm of Howard Hinnant).
pub fn day_number(date: &str) -> Option<i64> {
    if !db::is_day(date) {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (date[..4].parse().ok()?, date[5..7].parse().ok()?, date[8..].parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let year = y - era * 400;
    let day_of_year = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    Some(era * 146_097 + year * 365 + year / 4 - year / 100 + day_of_year - 719_468)
}

/// Trimmed, on one line, at most `max` characters.
pub fn cut(text: &str, max: usize) -> String {
    let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let line: String = line.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect();
    line.trim().to_owned()
}

/// Topic names match whatever their case.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The topics of a course: the ones in the table, and the ones only words have.
pub fn topics(db: &Connection, lang: &str) -> rusqlite::Result<Vec<Topic>> {
    let mut stmt = db.prepare("SELECT uid, name, color, description FROM topics WHERE lang = ?1 AND deleted = 0")?;
    let row = |r: &rusqlite::Row| Ok(Topic { uid: r.get(0)?, name: r.get(1)?, color: r.get(2)?, description: r.get(3)?, ..Topic::default() });
    let mut list: Vec<Topic> = stmt.query_map([lang], row)?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = db.prepare("SELECT topic, count(*), sum(learned_at > 0) FROM words WHERE lang = ?1 AND topic != '' AND NOT skipped GROUP BY topic")?;
    for row in stmt.query_map([lang], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (name, words, learned) = row?;
        match list.iter_mut().find(|t| t.name == name) {
            Some(t) => (t.words, t.learned) = (words, learned),
            None => list.push(Topic { name, words, learned, ..Topic::default() }),
        }
    }
    list.sort_by_key(|t| t.name.to_lowercase());
    Ok(list)
}

/// Makes a topic or changes one; a new name goes to its words and to the stages of the course's roadmaps. A topic
/// only words had (`uid` empty, `old` its name) gets a row of its own.
pub fn save_topic(db: &Connection, lang: &str, topic: &Topic, old: &str, now: i64) -> Result<(), String> {
    let name = cut(&topic.name, 40);
    if name.is_empty() {
        return Err("Hãy đặt tên cho chủ đề".into());
    }
    let color = if COLORS.contains(&topic.color.as_str()) { topic.color.as_str() } else { "" };
    let description = cut(&topic.description, 200);
    let e = |e: rusqlite::Error| e.to_string();
    let rows = topic_rows(db, lang).map_err(e)?;
    if rows.iter().any(|(uid, other)| *uid != topic.uid && same(other, &name)) {
        return Err(format!("Chủ đề \"{name}\" đã có"));
    }
    let tx = db.unchecked_transaction().map_err(e)?;
    let before = if topic.uid.is_empty() {
        tx.execute(
            "INSERT INTO topics (lang, name, color, description, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![lang, name, color, description, now],
        )
        .map_err(e)?;
        cut(old, 40)
    } else {
        let (_, before) = rows.into_iter().find(|(uid, _)| *uid == topic.uid).ok_or("Không tìm thấy chủ đề này")?;
        tx.execute(
            "UPDATE topics SET name = ?2, color = ?3, description = ?4, updated = ?5 WHERE uid = ?1",
            params![topic.uid, name, color, description, now],
        )
        .map_err(e)?;
        before
    };
    if !before.is_empty() && before != name {
        rename(&tx, lang, &before, &name, now).map_err(e)?;
    }
    tx.commit().map_err(e)
}

/// Deletes a topic: its words keep no topic, and the stages of the roadmaps drop it.
pub fn delete_topic(db: &Connection, lang: &str, uid: &str, name: &str, now: i64) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let name = match topic_rows(&tx, lang)?.into_iter().find(|(u, _)| !uid.is_empty() && u == uid) {
        Some((_, stored)) => stored,
        None => name.to_owned(),
    };
    if !uid.is_empty() {
        tx.execute("UPDATE topics SET deleted = 1, updated = ?3 WHERE uid = ?1 AND lang = ?2", params![uid, lang, now])?;
    }
    if !name.is_empty() {
        rename(&tx, lang, &name, "", now)?;
    }
    tx.commit()
}

fn topic_rows(db: &Connection, lang: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = db.prepare("SELECT uid, name FROM topics WHERE lang = ?1 AND deleted = 0")?;
    stmt.query_map([lang], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// A topic's new name on its words and in the stages of the roadmaps; an empty name takes it away.
fn rename(db: &Connection, lang: &str, old: &str, new: &str, now: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE words SET topic = ?3, updated = ?4 WHERE lang = ?1 AND topic = ?2", params![lang, old, new, now])?;
    for mut r in roadmaps(db, lang)? {
        let before = r.steps.clone();
        for s in &mut r.steps {
            s.topics = s.topics.iter().filter(|t| !(new.is_empty() && *t == old)).map(|t| if t == old { new.to_owned() } else { t.clone() }).collect();
        }
        if r.steps != before {
            db.execute("UPDATE roadmaps SET steps = ?2, updated = ?3 WHERE uid = ?1", params![r.uid, json!(r.steps).to_string(), now])?;
        }
    }
    Ok(())
}

/// The roadmaps of a course, the active one first.
pub fn roadmaps(db: &Connection, lang: &str) -> rusqlite::Result<Vec<Roadmap>> {
    let mut stmt = db.prepare(
        "SELECT uid, purpose, level, start, deadline, minutes, steps, active FROM roadmaps
         WHERE lang = ?1 AND deleted = 0 ORDER BY active DESC, updated DESC",
    )?;
    let mut list: Vec<Roadmap> = stmt
        .query_map([lang], |r| {
            Ok(Roadmap {
                uid: r.get(0)?,
                purpose: r.get(1)?,
                level: r.get(2)?,
                start: r.get(3)?,
                deadline: r.get(4)?,
                minutes: r.get(5)?,
                steps: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                active: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Two copies merged by the sync may both have one active: the one changed last wins.
    for (i, r) in list.iter_mut().enumerate() {
        r.active &= i == 0;
    }
    Ok(list)
}

/// A roadmap from the page made safe to keep: lengths cut, numbers within bounds, unknown exercises dropped.
pub fn sanitize(r: &mut Roadmap, today: &str) -> Result<(), String> {
    r.purpose = cut(&r.purpose, 80);
    if r.purpose.is_empty() {
        return Err("Hãy cho biết bạn học để làm gì".into());
    }
    if day_number(&r.deadline).is_none() {
        return Err("Hãy chọn hạn chót cho mục tiêu".into());
    }
    if day_number(&r.start).is_none() {
        r.start = today.to_owned();
    }
    r.minutes = r.minutes.clamp(5, 600);
    r.steps.truncate(MAX_STEPS);
    if r.steps.is_empty() {
        return Err("Lộ trình cần ít nhất một chặng".into());
    }
    for (i, s) in r.steps.iter_mut().enumerate() {
        tidy(s, i);
        s.target = s.target.max(1);
        s.days = s.days.max(1);
    }
    Ok(())
}

fn tidy(s: &mut Step, i: usize) {
    s.uid = cut(&s.uid, 32);
    s.title = cut(&s.title, 80);
    if s.title.is_empty() {
        s.title = format!("Chặng {}", i + 1);
    }
    let mut topics: Vec<String> = Vec::new();
    for t in s.topics.iter().map(|t| cut(t, 40)) {
        if !t.is_empty() && !topics.iter().any(|x| same(x, &t)) {
            topics.push(t);
        }
    }
    topics.truncate(4);
    if topics.is_empty() {
        topics.push(cut(&s.title, 40));
    }
    s.topics = topics;
    s.skills = cut(&s.skills, 80);
    let mut exercises: Vec<String> = Vec::new();
    for e in &s.exercises {
        if EXERCISES.contains(&e.as_str()) && !exercises.contains(e) {
            exercises.push(e.clone());
        }
    }
    if exercises.is_empty() {
        exercises = vec!["flash".into(), "choice".into()];
    }
    s.exercises = exercises;
    s.target = s.target.min(MAX_TARGET);
    s.days = s.days.min(3650);
}

/// The template's stages, for a roadmap made by hand.
pub fn template(zh: bool) -> Vec<Step> {
    let rows = if zh { TEMPLATE_ZH } else { TEMPLATE_EN };
    rows.iter()
        .map(|(title, topic, skills, exercises)| Step {
            title: (*title).into(),
            topics: vec![(*topic).into()],
            skills: (*skills).into(),
            exercises: exercises.iter().map(|e| (*e).into()).collect(),
            ..Step::default()
        })
        .collect()
}

/// The words a learner can take on in `days` days at `minutes` a day: a new word for every four minutes.
fn budget(days: i64, minutes: i64) -> i64 {
    (days.max(1) * (minutes / 4).max(2)).clamp(20, MAX_TARGET)
}

/// Stages as a model wrote them made into a whole roadmap of `days` days: at most twelve, at least four (the
/// template fills up), every stage with a topic, a target and its share of the days, which add up to `days`.
pub fn fit(mut steps: Vec<Step>, days: i64, minutes: i64, zh: bool) -> Vec<Step> {
    steps.truncate(MAX_STEPS);
    for t in template(zh) {
        if steps.len() >= MIN_STEPS {
            break;
        }
        if !steps.iter().any(|s| same(&s.title, &t.title)) {
            steps.push(t);
        }
    }
    let n = steps.len() as i64;
    let share = (budget(days, minutes) / n).clamp(5, 500);
    for (i, s) in steps.iter_mut().enumerate() {
        tidy(s, i);
        if s.target <= 0 {
            s.target = share;
        }
        if s.days <= 0 {
            s.days = (days / n).max(1);
        }
    }
    // The ends of the stages are rounded rather than their lengths, so that the lengths add up to `days`; each end
    // leaves at least a day for every stage around it.
    let total: i64 = steps.iter().map(|s| s.days).sum();
    if days >= n && total != days {
        let (mut sum, mut prev) = (0, 0);
        for (i, s) in steps.iter_mut().enumerate() {
            sum += s.days;
            let end = ((sum * days) as f64 / total as f64).round() as i64;
            let end = end.clamp(prev + 1, days - (n - 1 - i as i64));
            (s.days, prev) = (end - prev, end);
        }
    }
    steps
}

/// The days a plan for `goal` has from `today`, or what to fix first.
pub fn plan_days(goal: &Roadmap, today: &str) -> Result<i64, String> {
    if cut(&goal.purpose, 80).is_empty() {
        return Err("Hãy cho biết bạn học để làm gì".into());
    }
    match (day_number(&goal.deadline), day_number(today)) {
        (Some(end), Some(now)) if end > now => Ok(end - now),
        _ => Err("Hãy chọn hạn chót sau hôm nay".into()),
    }
}

/// Keeps a roadmap of a course and makes it the active one; its topics get rows of their own. Answers its uid.
pub fn save_roadmap(db: &Connection, lang: &str, r: &Roadmap, now: i64) -> Result<String, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let tx = db.unchecked_transaction().map_err(e)?;
    let mut steps = r.steps.clone();
    for s in steps.iter_mut().filter(|s| s.uid.is_empty()) {
        s.uid = db::new_uid(&tx).map_err(e)?;
    }
    let steps = json!(steps).to_string();
    let uid = if r.uid.is_empty() {
        tx.query_row(
            "INSERT INTO roadmaps (lang, purpose, level, start, deadline, minutes, steps, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) RETURNING uid",
            params![lang, r.purpose, r.level, r.start, r.deadline, r.minutes, steps, now],
            |row| row.get(0),
        )
        .map_err(e)?
    } else {
        let changed = tx
            .execute(
                "UPDATE roadmaps SET purpose = ?3, level = ?4, start = ?5, deadline = ?6, minutes = ?7, steps = ?8, updated = ?9
                 WHERE uid = ?1 AND lang = ?2 AND deleted = 0",
                params![r.uid, lang, r.purpose, r.level, r.start, r.deadline, r.minutes, steps, now],
            )
            .map_err(e)?;
        if changed == 0 {
            return Err("Không tìm thấy lộ trình này, có thể nó đã bị xóa".into());
        }
        r.uid.clone()
    };
    activate(&tx, lang, &uid, now).map_err(e)?;
    let mut known: Vec<String> = topic_rows(&tx, lang).map_err(e)?.into_iter().map(|(_, name)| name).collect();
    for name in r.steps.iter().flat_map(|s| &s.topics) {
        if !known.iter().any(|k| same(k, name)) {
            let color = COLORS[known.len() % (COLORS.len() - 1)];
            tx.execute("INSERT INTO topics (lang, name, color, updated) VALUES (?1, ?2, ?3, ?4)", params![lang, name, color, now]).map_err(e)?;
            known.push(name.clone());
        }
    }
    tx.commit().map_err(e)?;
    Ok(uid)
}

/// Makes one roadmap of a course the active one; the others are kept.
pub fn activate(db: &Connection, lang: &str, uid: &str, now: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE roadmaps SET active = (uid = ?2), updated = ?3 WHERE lang = ?1 AND deleted = 0 AND active != (uid = ?2)",
        params![lang, uid, now],
    )
    .map(|_| ())
}

/// Deletes a roadmap; when it was the active one, the one changed last takes its place.
pub fn delete_roadmap(db: &Connection, lang: &str, uid: &str, now: i64) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute("UPDATE roadmaps SET deleted = 1, active = 0, updated = ?3 WHERE uid = ?2 AND lang = ?1", params![lang, uid, now])?;
    tx.execute(
        "UPDATE roadmaps SET active = 1, updated = ?2
         WHERE id = (SELECT id FROM roadmaps WHERE lang = ?1 AND deleted = 0 ORDER BY updated DESC LIMIT 1)
           AND NOT EXISTS (SELECT 1 FROM roadmaps WHERE lang = ?1 AND deleted = 0 AND active)",
        params![lang, now],
    )?;
    tx.commit()
}

/// Per stage: the learned words of its topics, and all its words, without the ones set aside.
fn counts(db: &Connection, lang: &str, steps: &[Step]) -> rusqlite::Result<Vec<(i64, i64)>> {
    let mut by_topic: HashMap<String, (i64, i64)> = HashMap::new();
    let mut stmt = db.prepare("SELECT topic, sum(learned_at > 0), count(*) FROM words WHERE lang = ?1 AND topic != '' AND NOT skipped GROUP BY topic")?;
    for row in stmt.query_map([lang], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (topic, learned, words) = row?;
        let entry = by_topic.entry(topic.to_lowercase()).or_default();
        (entry.0, entry.1) = (entry.0 + learned, entry.1 + words);
    }
    Ok(steps
        .iter()
        .map(|s| {
            let names: BTreeSet<String> = s.topics.iter().map(|t| t.to_lowercase()).collect();
            names.iter().filter_map(|t| by_topic.get(t)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
        })
        .collect())
}

/// Whether a stage is behind the learner: its target is reached and, on a hard roadmap (`tested`), its test passed;
/// or it was marked done.
fn finished(step: &Step, learned: i64, tested: bool) -> bool {
    step.done || (learned >= step.target.max(1) && tested)
}

/// Per stage, whether its test lets it finish: always on an easy roadmap, once passed on a hard one.
fn tested(tests: &Option<Vec<exam::StageTest>>, n: usize) -> Vec<bool> {
    (0..n).map(|i| tests.as_ref().is_none_or(|t| t[i].passed)).collect()
}

/// The topics of the current stage of the course's active roadmap, whose new words a session takes first.
pub fn focus_topics(db: &Connection, lang: &str) -> rusqlite::Result<Vec<String>> {
    let Some(r) = roadmaps(db, lang)?.into_iter().find(|r| r.active) else { return Ok(Vec::new()) };
    let counts = counts(db, lang, &r.steps)?;
    let tested = tested(&exam::stage_tests(db, lang, &r)?, r.steps.len());
    Ok(r.steps.into_iter().zip(counts).zip(tested).find(|((s, c), t)| !finished(s, c.0, *t)).map(|((s, _), _)| s.topics).unwrap_or_default())
}

/// What the page shows of a course: the active roadmap with the progress of each stage and the pace, and the list
/// of all of them.
pub fn view(db: &Connection, lang: &str, today: &str, now: i64) -> rusqlite::Result<Value> {
    let list = roadmaps(db, lang)?;
    let all: Vec<Value> = list.iter().map(|r| json!({ "uid": r.uid, "purpose": r.purpose, "deadline": r.deadline, "active": r.active })).collect();
    let Some(r) = list.into_iter().find(|r| r.active) else { return Ok(json!({ "active": null, "list": all })) };
    let counts = counts(db, lang, &r.steps)?;
    let tests = exam::stage_tests(db, lang, &r)?;
    let ok = tested(&tests, r.steps.len());
    let current = (0..r.steps.len()).position(|i| !finished(&r.steps[i], counts[i].0, ok[i]));
    let remaining = (0..r.steps.len()).filter(|&i| !finished(&r.steps[i], counts[i].0, ok[i])).map(|i| (r.steps[i].target - counts[i].0).max(0)).sum();
    let days_left = match (day_number(&r.deadline), day_number(today)) {
        (Some(end), Some(now)) => end - now,
        _ => 0,
    };
    // A learner who started less than two weeks ago is measured over the days since.
    let (learned, first): (Option<i64>, Option<i64>) = db.query_row(
        "SELECT sum(learned_at >= ?2), min(learned_at) FROM words WHERE lang = ?1 AND learned_at > 0 AND NOT skipped",
        params![lang, now - PACE_DAYS * DAY],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let days = first.map_or(PACE_DAYS, |f| ((now - f + DAY - 1) / DAY).clamp(1, PACE_DAYS));
    let steps: Vec<Value> = r
        .steps
        .iter()
        .zip(&counts)
        .enumerate()
        .map(|(i, (s, &(learned, words)))| {
            let mut v = json!(s);
            v["learned"] = learned.into();
            v["words"] = words.into();
            v["state"] = if finished(s, learned, ok[i]) { "done" } else if Some(i) == current { "current" } else { "next" }.into();
            if let Some(t) = tests.as_ref().map(|t| &t[i]) {
                v["test"] = t.info.clone();
                v["test"]["pending"] = (!s.done && !t.passed && learned >= s.target.max(1)).into();
            }
            v
        })
        .collect();
    let mut active = json!(r);
    active["steps"] = steps.into();
    active["current"] = json!(current);
    active["hard"] = tests.is_some().into();
    let mut pace = pace(remaining, days_left, learned.unwrap_or(0), days);
    if pace.verdict == "done" && current.is_some() {
        pace.verdict = "test";
    }
    Ok(json!({ "active": active, "list": all, "pace": pace }))
}

/// The topics and roadmaps of both courses, as a copy keeps them.
pub fn export(db: &Connection) -> rusqlite::Result<Value> {
    let (mut all_topics, mut all_roadmaps) = (Vec::new(), Vec::new());
    for lang in ["en", "zh"] {
        for t in topics(db, lang)?.into_iter().filter(|t| !t.uid.is_empty()) {
            all_topics.push(json!({ "lang": lang, "uid": t.uid, "name": t.name, "color": t.color, "description": t.description }));
        }
        for r in roadmaps(db, lang)? {
            let mut v = json!(r);
            v["lang"] = lang.into();
            all_roadmaps.push(v);
        }
    }
    Ok(json!({ "topics": all_topics, "roadmaps": all_roadmaps }))
}

/// Puts back the topics and roadmaps of a copy that this file lacks, known by their uid; a topic whose name is
/// taken stays out, and a roadmap only becomes the active one of a course that has none.
pub fn restore(db: &Connection, copy: &Value, now: i64) -> rusqlite::Result<()> {
    let has = |table: &str, uid: &str| -> rusqlite::Result<bool> {
        db.query_row(&format!("SELECT count(*) FROM {table} WHERE uid = ?1"), [uid], |r| r.get::<_, i64>(0)).map(|n| n > 0)
    };
    let uid_of = |v: &Value| cut(v["uid"].as_str().unwrap_or_default(), 32);
    let lang_of = |v: &Value| if v["lang"] == "zh" { "zh" } else { "en" };
    for t in copy["topics"].as_array().into_iter().flatten() {
        let (uid, lang, name) = (uid_of(t), lang_of(t), cut(t["name"].as_str().unwrap_or_default(), 40));
        if uid.is_empty() || name.is_empty() || has("topics", &uid)? || topic_rows(db, lang)?.iter().any(|(_, n)| same(n, &name)) {
            continue;
        }
        let color = t["color"].as_str().filter(|c| COLORS.contains(c)).unwrap_or_default();
        let description = cut(t["description"].as_str().unwrap_or_default(), 200);
        db.execute(
            "INSERT INTO topics (uid, lang, name, color, description, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![uid, lang, name, color, description, now],
        )?;
    }
    for v in copy["roadmaps"].as_array().into_iter().flatten() {
        let (uid, lang) = (uid_of(v), lang_of(v));
        let Ok(mut r) = serde_json::from_value::<Roadmap>(v.clone()) else { continue };
        if uid.is_empty() || has("roadmaps", &uid)? || sanitize(&mut r, "").is_err() {
            continue;
        }
        db.execute(
            "INSERT INTO roadmaps (uid, lang, purpose, level, start, deadline, minutes, steps, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![uid, lang, r.purpose, cut(&r.level, 4), r.start, r.deadline, r.minutes, json!(r.steps).to_string(), now],
        )?;
        if r.active && !roadmaps(db, lang)?.iter().any(|x| x.active) {
            activate(db, lang, &uid, now)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Word;
    use std::path::Path;

    fn memory() -> Connection {
        db::open(Path::new(":memory:")).unwrap()
    }

    fn word(db: &Connection, word: &str, topic: &str, learned_at: i64) -> i64 {
        let id = db::save(db, &Word { word: word.into(), topic: topic.into(), ..Word::default() }, 0).unwrap();
        db.execute("UPDATE words SET learned_at = ?2 WHERE id = ?1", [id, learned_at]).unwrap();
        id
    }

    fn step(title: &str, topics: &[&str], target: i64, days: i64) -> Step {
        Step { title: title.into(), topics: topics.iter().map(|t| (*t).into()).collect(), target, days, ..Step::default() }
    }

    #[test]
    fn the_pace_says_ahead_on_track_behind_late_or_done() {
        assert_eq!(pace(100, 50, 28, 14), Pace { remaining: 100, days_left: 50, needed: 2, recent: 2.0, verdict: "on_track" });
        assert_eq!(pace(100, 50, 42, 14).verdict, "ahead");
        assert_eq!(pace(100, 50, 14, 14).verdict, "behind");
        assert_eq!(pace(101, 50, 0, 14).needed, 3, "a part of a word is a whole word more");
        assert_eq!(pace(30, 0, 70, 14), Pace { remaining: 30, days_left: 0, needed: 30, recent: 5.0, verdict: "late" });
        assert_eq!(pace(0, -3, 0, 14).verdict, "done");
        assert_eq!(pace(10, 5, 4, 0).recent, 4.0, "no days counts as one");
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2026-10-28").unwrap() - day_number("2026-10-01").unwrap(), 27);
        assert_eq!(day_number("2028-03-01").unwrap() - day_number("2028-02-28").unwrap(), 2, "2028 is a leap year");
        assert_eq!(day_number("2027-01-01").unwrap() - day_number("2026-12-31").unwrap(), 1);
        assert_eq!([day_number("2026-13-01"), day_number("2026-1-01"), day_number("")], [None, None, None]);
    }

    #[test]
    fn a_plan_fits_its_days_and_fills_its_gaps() {
        let steps = fit(vec![step("Công sở", &["Công việc"], 60, 10), step("Email", &[], 0, 30)], 90, 20, false);
        assert_eq!(steps.len(), MIN_STEPS, "the template fills up to four stages");
        assert_eq!(steps.iter().map(|s| s.days).sum::<i64>(), 90);
        assert_eq!(steps[1].topics, ["Email"], "a stage with no topic takes its title");
        assert_eq!((steps[0].target, steps[1].target), (60, 112), "450 words for 90 days at 20 minutes, in four");
        assert!(steps[1].days > steps[0].days, "the spans keep their proportions");
        assert!(steps.iter().all(|s| !s.exercises.is_empty() && s.days >= 1));
        let many = fit((0..20).map(|i| step(&format!("S{i}"), &["x"], 10, 0)).collect(), 30, 20, true);
        assert_eq!(many.len(), MAX_STEPS);
        assert_eq!(many.iter().map(|s| s.days).sum::<i64>(), 30);
        // Fewer days than stages: one day each, whatever the deadline.
        assert!(fit(vec![], 2, 20, false).iter().all(|s| s.days == 1));
        assert_eq!(template(true)[0].topics, ["Chào hỏi"]);
        let today = "2026-10-28";
        let goal = Roadmap { purpose: "IELTS 6.5".into(), deadline: "2027-01-26".into(), ..Roadmap::default() };
        assert_eq!(plan_days(&goal, today), Ok(90));
        assert!(plan_days(&Roadmap { deadline: today.into(), ..goal.clone() }, today).is_err());
        assert!(plan_days(&Roadmap { purpose: " ".into(), ..goal }, today).is_err());
    }

    #[test]
    fn topics_are_made_renamed_and_deleted_with_their_words() {
        let db = memory();
        let a = word(&db, "hotel", "Du lịch", 5);
        word(&db, "luggage", "Du lịch", 0);
        word(&db, "deadline", "Công việc", 0);
        save_topic(&db, "en", &Topic { name: "Sức khỏe".into(), color: "green".into(), ..Topic::default() }, "", 1).unwrap();
        let names = |db: &Connection| topics(db, "en").unwrap().into_iter().map(|t| (t.name, t.words, t.learned, t.uid.is_empty())).collect::<Vec<_>>();
        assert_eq!(names(&db), [("Công việc".into(), 1, 0, true), ("Du lịch".into(), 2, 1, true), ("Sức khỏe".into(), 0, 0, false)]);
        assert!(topics(&db, "zh").unwrap().is_empty());
        assert_eq!(save_topic(&db, "en", &Topic { name: "sức KHỎE".into(), ..Topic::default() }, "", 2), Err("Chủ đề \"sức KHỎE\" đã có".into()));
        assert!(save_topic(&db, "en", &Topic { name: "  ".into(), ..Topic::default() }, "", 2).is_err());
        // The topics of a roadmap get rows, and its stages follow a topic's new name.
        let r = Roadmap { purpose: "Du lịch".into(), deadline: "2027-01-01".into(), steps: vec![step("Đi chơi", &["Du lịch", "Sức khỏe"], 10, 30)], ..Roadmap::default() };
        save_roadmap(&db, "en", &r, 3).unwrap();
        let trip = topics(&db, "en").unwrap().into_iter().find(|t| t.name == "Du lịch").unwrap();
        assert!(!trip.uid.is_empty() && trip.words == 2);
        save_topic(&db, "en", &Topic { name: "Travel".into(), color: "blue".into(), ..trip }, "", 4).unwrap();
        assert_eq!(db::get(&db, a).unwrap().unwrap().topic, "Travel");
        assert_eq!(roadmaps(&db, "en").unwrap()[0].steps[0].topics, ["Travel", "Sức khỏe"]);
        let travel = topics(&db, "en").unwrap().into_iter().find(|t| t.name == "Travel").unwrap();
        assert_eq!((travel.words, travel.color.as_str()), (2, "blue"));
        assert!(!topics(&db, "en").unwrap().iter().any(|t| t.name == "Du lịch"), "renamed, not doubled");
        // A topic only words had gets a row of its own as it is renamed.
        save_topic(&db, "en", &Topic { name: "Văn phòng".into(), ..Topic::default() }, "Công việc", 5).unwrap();
        let office = topics(&db, "en").unwrap().into_iter().find(|t| t.name == "Văn phòng").unwrap();
        assert_eq!((office.words, office.uid.is_empty()), (1, false));
        delete_topic(&db, "en", &travel.uid, "", 6).unwrap();
        assert_eq!(db::get(&db, a).unwrap().unwrap().topic, "");
        assert_eq!(roadmaps(&db, "en").unwrap()[0].steps[0].topics, ["Sức khỏe"]);
        // A topic only words have is deleted by its name.
        word(&db, "menu", "Ẩm thực", 0);
        delete_topic(&db, "en", "", "Ẩm thực", 7).unwrap();
        delete_topic(&db, "en", &office.uid, "", 7).unwrap();
        assert_eq!(names(&db), [("Sức khỏe".into(), 0, 0, false)]);
    }

    #[test]
    fn a_roadmap_counts_its_stages_and_its_pace() {
        let db = memory();
        let (now, today) = (100 * DAY, "1970-04-11");
        assert_eq!(view(&db, "en", today, now).unwrap()["active"], Value::Null);
        word(&db, "older", "", 0);
        for i in 0..3 {
            word(&db, &format!("trip{i}"), "Du lịch", now - DAY);
        }
        word(&db, "hotel", "du lịch", now - 20 * DAY);
        word(&db, "meeting", "Công việc", 0);
        word(&db, "contract", "Công việc", 0);
        let mut r = Roadmap {
            purpose: "Đi làm".into(),
            deadline: "1970-04-21".into(),
            minutes: 20,
            steps: vec![step("Du lịch", &["Du lịch"], 4, 10), step("Công sở", &["Công việc"], 10, 10), step("Họp", &["Họp"], 5, 10)],
            ..Roadmap::default()
        };
        r.uid = save_roadmap(&db, "en", &r, now).unwrap();
        let v = view(&db, "en", today, now).unwrap();
        let steps = &v["active"]["steps"];
        assert_eq!((steps[0]["learned"].as_i64(), steps[0]["words"].as_i64(), steps[0]["state"].as_str()), (Some(4), Some(4), Some("done")));
        assert_eq!((steps[1]["state"].as_str(), steps[2]["state"].as_str(), v["active"]["current"].as_i64()), (Some("current"), Some("next"), Some(1)));
        // 15 words left in 10 days; 3 learned in the last 14 days, out of 20 days since the first.
        assert_eq!(v["pace"]["remaining"], 15);
        assert_eq!((v["pace"]["needed"].as_i64(), v["pace"]["verdict"].as_str()), (Some(2), Some("behind")));
        assert!((v["pace"]["recent"].as_f64().unwrap() - 3.0 / 14.0).abs() < 1e-9);
        // The topics of the stages got rows, and the session takes the current stage's new words first.
        assert_eq!(topics(&db, "en").unwrap().iter().filter(|t| !t.uid.is_empty()).count(), 3);
        assert_eq!(focus_topics(&db, "en").unwrap(), ["Công việc"]);
        db::set_setting(&db, "new_per_day", "1").unwrap();
        assert_eq!(db::session(&db, now, now, "en", "").unwrap().last().unwrap().word, "meeting", "not the older word");
        // A session on one topic: its reviews and its new words only, still within today's new words.
        let travel: Vec<_> = db::session(&db, now, now, "en", "Du lịch").unwrap().into_iter().map(|w| w.word).collect();
        assert!(!travel.is_empty() && travel.iter().all(|w| w.starts_with("trip")), "{travel:?}");
        assert_eq!(db::session(&db, now, now, "en", "Họp").unwrap().len(), 0);
        // Marked done, the next stage is current; a second roadmap becomes the active one and the first is kept.
        r.steps[1].done = true;
        save_roadmap(&db, "en", &r, now).unwrap();
        assert_eq!(focus_topics(&db, "en").unwrap(), ["Họp"]);
        let other = save_roadmap(&db, "en", &Roadmap { uid: String::new(), ..r.clone() }, now + 1).unwrap();
        let list = roadmaps(&db, "en").unwrap();
        assert_eq!((list.len(), list[0].uid == other, list[0].active, list[1].active), (2, true, true, false));
        assert_eq!(list[0].uid.len(), 16);
        activate(&db, "en", &r.uid, now + 2).unwrap();
        assert_eq!(roadmaps(&db, "en").unwrap()[0].uid, r.uid);
        delete_roadmap(&db, "en", &r.uid, now + 3).unwrap();
        let left = roadmaps(&db, "en").unwrap();
        assert_eq!((left.len(), left[0].uid == other, left[0].active), (1, true, true), "the other one takes over");
        assert!(roadmaps(&db, "zh").unwrap().is_empty());
        assert!(save_roadmap(&db, "en", &r, now + 4).is_err(), "a deleted roadmap is not saved again");
    }

    #[test]
    fn a_copy_keeps_the_topics_and_roadmaps() {
        let db = memory();
        save_topic(&db, "zh", &Topic { name: "Ăn uống".into(), color: "amber".into(), ..Topic::default() }, "", 1).unwrap();
        let mut r = Roadmap { purpose: "HSK 3".into(), level: "HSK1".into(), deadline: "2027-02-01".into(), minutes: 20, steps: vec![step("Ăn", &["Ăn uống", "Mua sắm"], 60, 30)], ..Roadmap::default() };
        sanitize(&mut r, "2026-10-28").unwrap();
        save_roadmap(&db, "zh", &r, 2).unwrap();
        let copy: Value = serde_json::from_str(&crate::backup::backup(&db, 3).unwrap()).unwrap();
        let other = memory();
        save_topic(&other, "zh", &Topic { name: "ăn uống".into(), ..Topic::default() }, "", 1).unwrap();
        restore(&other, &copy, 4).unwrap();
        restore(&other, &copy, 5).unwrap();
        let names: Vec<String> = topics(&other, "zh").unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Mua sắm", "ăn uống"], "a name already taken is not doubled, a second restore adds nothing");
        let back = roadmaps(&other, "zh").unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].purpose.as_str(), back[0].active, back[0].steps.clone()), ("HSK 3", true, roadmaps(&db, "zh").unwrap()[0].steps.clone()));
        assert_eq!(back[0].uid, roadmaps(&db, "zh").unwrap()[0].uid);
        assert!(roadmaps(&other, "en").unwrap().is_empty());
    }

    #[test]
    fn a_roadmap_from_the_page_is_tidied() {
        let mut r = Roadmap {
            purpose: " TOEIC\n750 ".into(),
            deadline: "2027-05-01".into(),
            minutes: 1,
            steps: vec![Step { title: "".into(), topics: vec!["A".into(), "a".into(), " ".into()], target: -4, days: 0, exercises: vec!["flash".into(), "hack".into(), "flash".into()], ..Step::default() }],
            ..Roadmap::default()
        };
        sanitize(&mut r, "2026-10-28").unwrap();
        assert_eq!((r.purpose.as_str(), r.start.as_str(), r.minutes), ("TOEIC 750", "2026-10-28", 5));
        let s = &r.steps[0];
        assert_eq!((s.title.as_str(), s.topics.clone(), s.target, s.days, s.exercises.clone()), ("Chặng 1", vec!["A".to_owned()], 1, 1, vec!["flash".to_owned()]));
        assert!(sanitize(&mut Roadmap { steps: vec![], ..r.clone() }, "2026-10-28").is_err());
        assert!(sanitize(&mut Roadmap { deadline: "soon".into(), ..r }, "2026-10-28").is_err());
    }
}
