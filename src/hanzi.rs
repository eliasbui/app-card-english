//! Chinese characters: how each is written, stroke by stroke, and what it is made of. The strokes are those of
//! hanzi-writer-data (Make Me a Hanzi, Arphic Public License: assets/hanzi-writer-LICENSE), fetched once per
//! character and kept in the data dir; the parts and radicals come from Make Me a Hanzi's dictionary (LGPL:
//! assets/hanzi-parts-LICENSE).

use crate::{net, pron};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// `字<tab>parts<tab>radical` per line, made by scripts/make-hanzi-data.py; `？` is a part nobody knows.
const PARTS: &str = include_str!("../assets/hanzi-parts.txt");
/// Where `<character>.json` is; the version is pinned so that a file kept in the cache never goes stale.
const STROKES_URL: &str = "https://cdn.jsdelivr.net/npm/hanzi-writer-data@2.0.1/";
/// The most characters one request draws, and the most bytes one character's file may have (they have a few KB).
const MAX_CHARS: usize = 16;
const MAX_FILE: usize = 64 << 10;
/// Forms of radicals with no reading of their own, read as the character they stand for: 讠 as 言, ngôn.
const FORMS: [(char, char); 16] = [
    ('钅', '金'), ('讠', '言'), ('纟', '糸'), ('饣', '食'), ('⺼', '肉'), ('⺮', '竹'), ('罒', '网'), ('丷', '八'),
    ('⺈', '刀'), ('⺊', '卜'), ('⺌', '小'), ('⺍', '小'), ('氺', '水'), ('丬', '爿'), ('覀', '襾'), ('⻊', '足'),
];

/// A character or a part of one, with its pinyin and Sino-Vietnamese reading, either of them empty when unknown.
#[derive(Debug, PartialEq, Serialize)]
pub struct Reading {
    pub char: String,
    pub pinyin: String,
    pub hanviet: String,
}

/// What a character of a word is made of.
#[derive(Debug, Serialize)]
pub struct Character {
    #[serde(flatten)]
    pub reading: Reading,
    /// The radical the character is filed under.
    pub radical: Option<Reading>,
    /// Its components, the unknown ones left out.
    pub parts: Vec<Reading>,
    /// Whether a component was unknown, so that `parts` is not the whole character.
    pub partial: bool,
}

fn table() -> &'static HashMap<char, (&'static str, Option<char>)> {
    static TABLE: OnceLock<HashMap<char, (&str, Option<char>)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        PARTS
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let c = fields.next()?.chars().next()?;
                Some((c, (fields.next().unwrap_or_default(), fields.next().and_then(|r| r.chars().next()))))
            })
            .collect()
    })
}

fn reading(c: char) -> Reading {
    let base = FORMS.iter().find(|f| f.0 == c).map_or(c, |f| f.1);
    let read = |f: fn(&str) -> String| Some(f(&c.to_string())).filter(|r| !r.is_empty()).unwrap_or_else(|| f(&base.to_string()));
    Reading { char: c.to_string(), pinyin: read(pron::pinyin), hanviet: read(pron::hanviet) }
}

/// The radical and components of each Han character of a word, the characters read as they are in the word: 好 in
/// 爱好 is hào.
pub fn parts(word: &str) -> Vec<Character> {
    let chars: Vec<char> = word.chars().filter(|&c| pron::is_han(c)).take(MAX_CHARS).collect();
    let syllables = pron::syllables(word);
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let mut own = reading(c);
            if syllables.len() == chars.len() {
                own.pinyin = pron::mark(&syllables[i]);
            }
            let (parts, radical) = table().get(&c).copied().unwrap_or(("", None));
            Character {
                reading: own,
                radical: radical.map(reading),
                parts: parts.chars().filter(|&p| p != '？').map(reading).collect(),
                partial: parts.contains('？'),
            }
        })
        .collect()
}

/// The stroke data of a character when `text` has the shape Hanzi Writer draws: an SVG path per stroke made of
/// nothing but path commands and numbers, the line through the middle of each stroke as points, and the numbers of
/// the strokes of the radical. Anything else in it is dropped.
fn valid(text: &str) -> Option<Value> {
    if text.len() > MAX_FILE {
        return None;
    }
    let data: Value = serde_json::from_str(text).ok()?;
    let strokes = data["strokes"].as_array()?;
    let medians = data["medians"].as_array()?;
    if strokes.is_empty() || strokes.len() > 64 || medians.len() != strokes.len() {
        return None;
    }
    let path = |s: &Value| s.as_str().is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b"MLQCZ0123456789 .-".contains(&b)));
    let point = |p: &Value| p.as_array().is_some_and(|p| p.len() == 2 && p.iter().all(Value::is_number));
    let median = |m: &Value| m.as_array().is_some_and(|m| !m.is_empty() && m.len() <= 256 && m.iter().all(point));
    if !strokes.iter().all(path) || !medians.iter().all(median) {
        return None;
    }
    let radical: Vec<u64> = match &data["radStrokes"] {
        Value::Null => Vec::new(),
        r => r.as_array()?.iter().map(|n| n.as_u64().filter(|&n| n < strokes.len() as u64)).collect::<Option<_>>()?,
    };
    Some(json!({ "strokes": strokes, "medians": medians, "radStrokes": radical }))
}

fn offline(c: char) -> String {
    format!("Chưa có nét viết của chữ {c} trên máy. Hãy kết nối mạng để tải về một lần, lần sau không cần mạng nữa.")
}

/// The stroke data of each Han character of `chars`, as `{ "好": { strokes, medians, radStrokes } }`: from the
/// `hanzi` folder of the data dir, or else downloaded and kept there. A character the data does not have is left out.
pub fn strokes(dir: &Path, chars: &str) -> Result<Value, String> {
    let mut wanted: Vec<char> = Vec::new();
    for c in chars.chars().filter(|&c| pron::is_han(c)) {
        if !wanted.contains(&c) && wanted.len() < MAX_CHARS {
            wanted.push(c);
        }
    }
    let folder = dir.join("hanzi");
    let mut out = Map::new();
    for c in wanted {
        // Named by code point, since some file systems mangle the names of characters.
        let file = folder.join(format!("{:x}.json", c as u32));
        let data = match std::fs::read_to_string(&file).ok().and_then(|t| valid(&t)) {
            Some(data) => data,
            None => {
                let url = format!("{STROKES_URL}{}.json", net::encode(&c.to_string()));
                let (status, body) = net::request(&url, &[], None, 20).map_err(|_| offline(c))?;
                match status {
                    200 => {
                        let data = valid(&body).ok_or_else(|| format!("Nét viết của chữ {c} tải về bị lỗi. Hãy thử lại sau."))?;
                        // Kept for next time; without the copy it still draws now.
                        let _ = std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&file, data.to_string()));
                        data
                    }
                    404 => continue,
                    _ => return Err(offline(c)),
                }
            }
        };
        out.insert(c.to_string(), data);
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"{"strokes":["M 10 20 Q 30 40 50 60 L 7.5 -8 Z"],"medians":[[[10,20],[50,60]]],"radStrokes":[0],"extra":"x"}"#;

    #[test]
    fn stroke_data_is_checked_before_it_is_drawn() {
        let data = valid(ONE).unwrap();
        assert_eq!(data["strokes"][0], "M 10 20 Q 30 40 50 60 L 7.5 -8 Z");
        assert_eq!(data["radStrokes"], json!([0]));
        assert!(data.get("extra").is_none(), "unknown keys are dropped");
        let without_radical = r#"{"strokes":["M 1 2 L 3 4"],"medians":[[[1,2],[3,4]]]}"#;
        assert_eq!(valid(without_radical).unwrap()["radStrokes"], json!([]));
        let bad = [
            "",
            "[]",
            r#"{"strokes":[],"medians":[]}"#,
            r#"{"strokes":["M 1 2\" onload=\"x"],"medians":[[[1,2]]]}"#,
            r#"{"strokes":["M 1 2","M 3 4"],"medians":[[[1,2]]]}"#,
            r#"{"strokes":["M 1 2"],"medians":[[[1]]]}"#,
            r#"{"strokes":["M 1 2"],"medians":[[["a","b"]]]}"#,
            r#"{"strokes":["M 1 2"],"medians":[[[1,2]]],"radStrokes":[1]}"#,
            r#"{"strokes":["M 1 2"],"medians":[[[1,2]]],"radStrokes":"0"}"#,
        ];
        for text in bad {
            assert_eq!(valid(text), None, "{text}");
        }
        let huge = format!(r#"{{"strokes":["M {}"],"medians":[[[1,2]]]}}"#, "1 ".repeat(MAX_FILE));
        assert_eq!(valid(&huge), None);
    }

    #[test]
    fn strokes_come_from_the_cache_without_the_network() {
        let dir = std::env::temp_dir().join(format!("card-english-hanzi-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("hanzi")).unwrap();
        std::fs::write(dir.join("hanzi/597d.json"), ONE).unwrap();
        let data = strokes(&dir, "好 好 hello").unwrap();
        assert_eq!(data.as_object().unwrap().len(), 1);
        assert_eq!(data["好"]["medians"], json!([[[10, 20], [50, 60]]]));
        assert_eq!(strokes(&dir, "abc").unwrap(), json!({}));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "downloads from jsDelivr"]
    fn strokes_are_downloaded_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("card-english-hanzi-net-{}", std::process::id()));
        let first = strokes(&dir, "学习").unwrap();
        assert_eq!(first["学"]["strokes"].as_array().map(Vec::len), Some(8));
        assert!(dir.join("hanzi/5b66.json").exists() && dir.join("hanzi/4e60.json").exists());
        assert_eq!(strokes(&dir, "学习").unwrap(), first);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn characters_break_into_parts_with_their_readings() {
        assert!(table().len() > 9000, "{}", table().len());
        let hao = &parts("好")[0];
        assert_eq!(hao.reading, Reading { char: "好".into(), pinyin: "hǎo".into(), hanviet: "hảo".into() });
        let names: Vec<_> = hao.parts.iter().map(|p| format!("{} {} {}", p.char, p.pinyin, p.hanviet)).collect();
        assert_eq!(names, ["女 nǚ nữ", "子 zǐ tử"]);
        assert_eq!(hao.radical.as_ref().map(|r| r.char.as_str()), Some("女"));
        assert!(!hao.partial);
        // A form of a radical reads as the character it stands for.
        let shuo = &parts("说")[0];
        assert_eq!(shuo.reading.pinyin, "shuō", "the everyday reading, not shuì");
        assert_eq!((shuo.parts[0].char.as_str(), shuo.parts[0].hanviet.as_str()), ("讠", "ngôn"));
        assert_eq!(shuo.radical.as_ref().map(|r| r.pinyin.as_str()), Some("yán"));
        let xuexi = parts("学习");
        assert_eq!(xuexi.iter().map(|c| c.reading.pinyin.as_str()).collect::<Vec<_>>(), ["xué", "xí"]);
        assert_eq!(xuexi[0].parts.iter().map(|p| p.char.as_str()).collect::<String>(), "⺍冖子");
        assert_eq!(xuexi[0].parts[0].hanviet, "tiểu");
        assert!(xuexi[1].partial && xuexi[1].parts.len() == 1, "习 has a part nobody named");
        // A character that is a radical itself has no parts; the reading follows the word.
        assert!(parts("女")[0].parts.is_empty());
        assert_eq!(parts("爱好")[1].reading.pinyin, "hào");
        assert!(parts("hello").is_empty());
    }
}
