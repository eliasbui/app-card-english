//! How words are said. English: the IPA and the phonemes the speaking score compares, from the CMU Pronouncing
//! Dictionary (assets/cmudict-LICENSE). Chinese: pinyin from CC-CEDICT (assets/cedict-LICENSE) and the
//! Sino-Vietnamese reading of each character (assets/hanviet-LICENSE).

use std::{collections::HashMap, sync::OnceLock};

const CMUDICT: &str = include_str!("../assets/cmudict.dict");
/// `traditional simplified [pin1 yin1]` per line, made by scripts/make-zh-data.py.
const CEDICT: &str = include_str!("../assets/cedict.txt");
/// `字 tự` per line, made by scripts/make-zh-data.py.
const HANVIET: &str = include_str!("../assets/hanviet.txt");
/// The longest word looked for when Chinese text is cut into words, in characters.
const LONGEST: usize = 8;
/// Characters whose everyday reading is not the one CC-CEDICT happens to list first.
const USUAL: [(&str, &str); 24] = [
    ("吗", "ma5"), ("吧", "ba5"), ("啊", "a5"), ("么", "me5"), ("那", "na4"), ("个", "ge4"), ("上", "shang4"), ("看", "kan4"),
    ("得", "de5"), ("着", "zhe5"), ("行", "xing2"), ("重", "zhong4"), ("要", "yao4"), ("只", "zhi3"), ("干", "gan4"), ("乐", "le4"),
    ("假", "jia3"), ("累", "lei4"), ("量", "liang4"), ("落", "luo4"), ("正", "zheng4"), ("转", "zhuan3"), ("觉", "jue2"), ("为", "wei4"),
];

fn dict() -> &'static HashMap<&'static str, Vec<&'static str>> {
    static DICT: OnceLock<HashMap<&str, Vec<&str>>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut map = HashMap::new();
        for line in CMUDICT.lines() {
            let line = line.split(" #").next().unwrap_or_default();
            let mut parts = line.split_whitespace();
            let Some(word) = parts.next() else { continue };
            // `read(2)` and the like are other ways to say it; the first one is kept.
            if !word.ends_with(')') {
                map.entry(word).or_insert_with(|| parts.collect());
            }
        }
        map
    })
}

/// The letters, digits and apostrophes of each word, in lower case.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The phonemes of a word or phrase without stress, when every word is in the dictionary.
pub fn phones(text: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for word in words(text) {
        let p = dict().get(word.as_str())?;
        out.extend(p.iter().map(|p| p.trim_end_matches(['0', '1', '2']).to_owned()));
    }
    (!out.is_empty()).then_some(out)
}

/// How to say a word: American IPA such as `/həˈloʊ/`, or pinyin such as `nǐhǎo` for Chinese; an empty string
/// when an English word is unknown.
pub fn ipa(text: &str) -> String {
    if is_chinese(text) {
        return pinyin(text);
    }
    let mut out = Vec::new();
    for word in words(text) {
        let Some(p) = dict().get(word.as_str()) else {
            return String::new();
        };
        out.push(ipa_of(p));
    }
    if out.is_empty() { String::new() } else { format!("/{}/", out.join(" ")) }
}

fn ipa_of(phones: &[&str]) -> String {
    let vowel = |p: &str| p.ends_with(['0', '1', '2']);
    let vowels = phones.iter().filter(|p| vowel(p)).count();
    let mut marks = vec![""; phones.len()];
    for (i, p) in phones.iter().enumerate() {
        let mark = match p.chars().last() {
            Some('1') => "ˈ",
            Some('2') => "ˌ",
            _ => continue,
        };
        // Before the consonant that opens the syllable, or before all of them at the start of the word.
        let mut start = i;
        while start > 0 && !vowel(phones[start - 1]) {
            start -= 1;
        }
        let at = if start == 0 { 0 } else { i - usize::from(i > start) };
        if vowels > 1 {
            marks[at] = mark;
        }
    }
    phones
        .iter()
        .zip(marks)
        .map(|(p, mark)| {
            let base = p.trim_end_matches(['0', '1', '2']);
            let sym = match (base, p.ends_with('0')) {
                ("AH", true) => "ə",
                ("ER", true) => "ɚ",
                _ => symbol(base),
            };
            format!("{mark}{sym}")
        })
        .collect()
}

fn symbol(arpabet: &str) -> &'static str {
    match arpabet {
        "AA" => "ɑ",
        "AE" => "æ",
        "AH" => "ʌ",
        "AO" => "ɔ",
        "AW" => "aʊ",
        "AY" => "aɪ",
        "B" => "b",
        "CH" => "tʃ",
        "D" => "d",
        "DH" => "ð",
        "EH" => "ɛ",
        "ER" => "ɝ",
        "EY" => "eɪ",
        "F" => "f",
        "G" => "ɡ",
        "HH" => "h",
        "IH" => "ɪ",
        "IY" => "i",
        "JH" => "dʒ",
        "K" => "k",
        "L" => "l",
        "M" => "m",
        "N" => "n",
        "NG" => "ŋ",
        "OW" => "oʊ",
        "OY" => "ɔɪ",
        "P" => "p",
        "R" => "r",
        "S" => "s",
        "SH" => "ʃ",
        "T" => "t",
        "TH" => "θ",
        "UH" => "ʊ",
        "UW" => "u",
        "V" => "v",
        "W" => "w",
        "Y" => "j",
        "Z" => "z",
        "ZH" => "ʒ",
        _ => "",
    }
}

/// How alike two sequences are, from 0 to 1, by edit distance.
pub fn similarity<T: PartialEq>(a: &[T], b: &[T]) -> f32 {
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(x != y)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f32 / longest as f32
}

/// A Han character: the unified ideographs, their extensions and the compatibility ones.
pub fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{3134f}')
}

/// Whether text is Chinese: it has a Han character in it.
pub fn is_chinese(text: &str) -> bool {
    text.chars().any(is_han)
}

/// The numbered pinyin of every word, under its simplified and its traditional form.
fn cedict() -> &'static HashMap<&'static str, &'static str> {
    static DICT: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut map: HashMap<&str, &str> = HashMap::new();
        for line in CEDICT.lines() {
            let Some((heads, pinyin)) = line.split_once(" [") else { continue };
            let Some((traditional, simplified)) = heads.split_once(' ') else { continue };
            let pinyin = pinyin.trim_end_matches(']');
            for key in [simplified, traditional] {
                // The first reading stays, unless it is a name (capitalised) and this one is not.
                let name = |p: &str| p.starts_with(char::is_uppercase);
                if map.get(key).is_none_or(|old| name(old) && !name(pinyin)) {
                    map.insert(key, pinyin);
                }
            }
        }
        map.extend(USUAL);
        map
    })
}

/// A piece of Chinese text: a word of the dictionary, as its numbered pinyin, or any other character.
enum Piece {
    Word(&'static str),
    Other(char),
}

/// Text cut into words, the longest that the dictionary knows first.
fn pieces(text: &str) -> Vec<Piece> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let found = is_han(c).then(|| {
            (1..=LONGEST.min(chars.len() - i)).rev().find_map(|n| {
                let end = chars.get(i + n).map_or(text.len(), |c| c.0);
                cedict().get(&text[start..end]).map(|p| (n, *p))
            })
        });
        match found.flatten() {
            Some((n, pinyin)) => {
                out.push(Piece::Word(pinyin));
                i += n;
            }
            None => {
                out.push(Piece::Other(c));
                i += 1;
            }
        }
    }
    out
}

/// A syllable with its tone mark: `hao3` as `hǎo`, `lu:4` as `lǜ`, `ma5` as `ma`; anything else as it is.
pub fn mark(syllable: &str) -> String {
    let Some(tone) = syllable.chars().last().and_then(|c| c.to_digit(10)).filter(|t| (1..=5).contains(t)) else {
        return syllable.to_owned();
    };
    let base = syllable[..syllable.len() - 1].replace("u:", "ü").replace("U:", "Ü").replace('v', "ü");
    let lower = base.to_lowercase();
    // The mark goes on a or e, on the o of ou, or else on the last vowel.
    let at = lower.find('a').or_else(|| lower.find('e')).or_else(|| lower.find("ou")).or_else(|| lower.rfind(['i', 'o', 'u', 'ü']));
    let Some(at) = at.filter(|_| tone < 5) else { return base };
    const MARKS: [(char, [char; 4]); 6] = [
        ('a', ['ā', 'á', 'ǎ', 'à']),
        ('e', ['ē', 'é', 'ě', 'è']),
        ('i', ['ī', 'í', 'ǐ', 'ì']),
        ('o', ['ō', 'ó', 'ǒ', 'ò']),
        ('u', ['ū', 'ú', 'ǔ', 'ù']),
        ('ü', ['ǖ', 'ǘ', 'ǚ', 'ǜ']),
    ];
    let vowel = lower[at..].chars().next().unwrap_or('a');
    let marked = MARKS.iter().find(|m| m.0 == vowel).map_or(vowel, |m| m.1[tone as usize - 1]);
    let rest = &base[at..];
    let mut out = base[..at].to_owned();
    if rest.starts_with(char::is_uppercase) {
        out.extend(marked.to_uppercase());
    } else {
        out.push(marked);
    }
    out.push_str(&rest[vowel.len_utf8()..]);
    out
}

/// The pinyin of a dictionary word, its syllables written together as Hanyu Pinyin wants: `xuéxí`, `Xī'ān`.
fn word_pinyin(numbered: &str) -> String {
    let mut out = String::new();
    for syllable in numbered.split_whitespace() {
        let marked = mark(syllable);
        // A syllable that starts with a, o or e is set off from the one before; letters such as the OK of 卡拉OK are not.
        let toned = syllable.ends_with(|c: char| c.is_ascii_digit());
        if toned && !out.is_empty() && marked.to_lowercase().starts_with(['a', 'o', 'e', 'ā', 'á', 'ǎ', 'à', 'ō', 'ó', 'ǒ', 'ò', 'ē', 'é', 'ě', 'è']) {
            out.push('\'');
        }
        out.push_str(&marked);
    }
    out
}

/// Chinese punctuation as its ASCII form, so that it sits in pinyin.
fn halfwidth(c: char) -> char {
    match c {
        '。' => '.',
        '、' => ',',
        '“' | '”' | '「' | '」' | '『' | '』' => '"',
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
        c => c,
    }
}

/// Chinese text in pinyin, word by word: `wǒmen zǒu le.`. Characters the dictionary does not know are left out.
pub fn pinyin(text: &str) -> String {
    let mut tokens: Vec<String> = Vec::new();
    // Whether the last token is plain text still being written, such as a word in Latin letters.
    let mut plain = false;
    for piece in pieces(text) {
        match piece {
            Piece::Word(numbered) => {
                tokens.push(word_pinyin(numbered));
                plain = false;
            }
            Piece::Other(c) => {
                let c = halfwidth(c);
                if c.is_whitespace() || is_han(c) {
                    plain = false;
                } else if c.is_alphanumeric() || c == '\'' || c == '-' {
                    match tokens.last_mut() {
                        Some(last) if plain => last.push(c),
                        _ => tokens.push(c.to_string()),
                    }
                    plain = true;
                } else if let Some(last) = tokens.last_mut() {
                    // Punctuation stays with the word before it.
                    last.push(c);
                    plain = false;
                }
            }
        }
    }
    tokens.join(" ")
}

/// The numbered syllables of Chinese text in lower case, `xue2 xi2` as `["xue2", "xi2"]`, for comparing what was
/// said; simplified and traditional characters read the same.
pub fn syllables(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in pieces(text) {
        if let Piece::Word(numbered) = piece {
            let tones = numbered.split_whitespace().filter(|s| s.ends_with(|c: char| c.is_ascii_digit()));
            out.extend(tones.map(|s| s.to_lowercase().replace("u:", "v")));
        }
    }
    out
}

/// The Sino-Vietnamese reading of each character, such as `học tập` for 学习; empty when a character has none.
pub fn hanviet(text: &str) -> String {
    static TABLE: OnceLock<HashMap<char, &str>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        HANVIET.lines().filter_map(|l| l.split_once(' ')).filter_map(|(c, r)| Some((c.chars().next()?, r))).collect()
    });
    let mut out = Vec::new();
    for c in text.chars().filter(|&c| is_han(c)) {
        let Some(reading) = table.get(&c) else { return String::new() };
        out.push(*reading);
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_marks_go_where_pinyin_puts_them() {
        let cases = [
            ("hao3", "hǎo"), ("lu:4", "lǜ"), ("nu:3", "nǚ"), ("xiu1", "xiū"), ("gui4", "guì"), ("zhou1", "zhōu"),
            ("er2", "ér"), ("ma5", "ma"), ("r5", "r"), ("Bei3", "Běi"), ("A1", "Ā"), ("O", "O"), ("ng2", "ng"),
        ];
        for (numbered, marked) in cases {
            assert_eq!(mark(numbered), marked, "{numbered}");
        }
        assert_eq!(word_pinyin("Xi1 an1"), "Xī'ān");
        assert_eq!(word_pinyin("ka3 la1 O K"), "kǎlāOK");
    }

    #[test]
    fn chinese_reads_word_by_word() {
        assert!(is_chinese("学习 hard") && !is_chinese("hello") && !is_chinese("xuéxí"));
        assert_eq!(pinyin("学习"), "xuéxí");
        assert_eq!(ipa("學習"), "xuéxí");
        assert_eq!(pinyin("你好！"), "nǐhǎo!");
        // 了 and 着 alone take their everyday readings.
        assert_eq!(pinyin("我们走了。"), "wǒmen zǒu le.");
        assert_eq!(pinyin("他看着我"), "tā kàn zhe wǒ");
        assert_eq!(pinyin("我用iPhone。"), "wǒ yòng iPhone.");
        let sentence = pinyin("我很喜欢吃中国菜，你呢？");
        assert!(sentence.contains("xǐhuan") && sentence.ends_with("ne?") && !is_chinese(&sentence), "{sentence}");
        assert_eq!(syllables("學習女兒"), ["xue2", "xi2", "nv3", "er2"]);
        assert_eq!(syllables("hello"), Vec::<String>::new());
    }

    #[test]
    fn hanviet_reads_every_character_or_none() {
        assert_eq!(hanviet("学习"), "học tập");
        assert_eq!(hanviet("越南"), "việt nam");
        // A simplified character reads as its traditional form: 儿 is 兒, not the radical.
        assert_eq!(hanviet("儿子"), "nhi tử");
        assert_eq!(hanviet("电脑"), "điện não");
        assert_eq!(hanviet("hello"), "");
        assert_eq!(hanviet("氧气"), "");
    }

    #[test]
    fn ipa_marks_the_stressed_syllable() {
        assert_eq!(ipa("hello"), "/həˈloʊ/");
        assert_eq!(ipa("apple"), "/ˈæpəl/");
        assert_eq!(ipa("cat"), "/kæt/");
        assert_eq!(ipa("thank you"), "/θæŋk ju/");
        assert_eq!(ipa("qwzxv"), "");
    }

    #[test]
    fn phones_and_similarity() {
        assert_eq!(phones("ship").unwrap(), ["SH", "IH", "P"]);
        let ship = phones("ship").unwrap();
        let sheep = phones("sheep").unwrap();
        assert!((similarity(&ship, &sheep) - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(similarity(&ship, &ship), 1.0);
    }
}
