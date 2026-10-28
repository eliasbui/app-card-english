//! How words are said. English: the IPA and the phonemes the speaking score compares, from the CMU Pronouncing
//! Dictionary (assets/cmudict-LICENSE). Chinese: pinyin from CC-CEDICT (assets/cedict-LICENSE) and the
//! Sino-Vietnamese reading of each character (assets/hanviet-LICENSE), with a short English gloss of each word from
//! CC-CEDICT.

use std::{collections::HashMap, io::Read, sync::OnceLock};

const CMUDICT: &str = include_str!("../assets/cmudict.dict");
/// `traditional simplified [pin1 yin1]` per line, made by scripts/make-zh-data.py.
const CEDICT: &str = include_str!("../assets/cedict.txt");
/// A short English gloss for each line of `CEDICT`, in the same order, made by scripts/make-zh-data.py. Packed, it
/// keeps the binary small; it is unpacked the first time a gloss is asked for.
const GLOSSES: &[u8] = include_bytes!("../assets/cedict-gloss.txt.gz");
/// `字 tự` per line, made by scripts/make-zh-data.py.
const HANVIET: &str = include_str!("../assets/hanviet.txt");
/// The longest word looked for when Chinese text is cut into words, in characters.
const LONGEST: usize = 8;
/// Characters whose everyday reading is not the one CC-CEDICT happens to list first.
const USUAL: [(&str, &str); 25] = [
    ("吗", "ma5"), ("吧", "ba5"), ("啊", "a5"), ("么", "me5"), ("那", "na4"), ("个", "ge4"), ("上", "shang4"), ("看", "kan4"),
    ("得", "de5"), ("着", "zhe5"), ("行", "xing2"), ("重", "zhong4"), ("要", "yao4"), ("只", "zhi3"), ("干", "gan4"), ("乐", "le4"),
    ("假", "jia3"), ("累", "lei4"), ("量", "liang4"), ("落", "luo4"), ("正", "zheng4"), ("转", "zhuan3"), ("觉", "jue2"), ("为", "wei4"),
    ("说", "shuo1"),
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

/// The longest word the spell check looks for corrections of; a longer one gets none.
const SPELL_LONGEST: usize = 30;

/// The letters a word has, one bit each: a word one edit away differs in at most two of them.
fn letter_set(word: impl Iterator<Item = char>) -> u32 {
    word.fold(0, |set, c| set | if c.is_ascii_lowercase() { 1 << (c as u8 - b'a') } else { 1 << 26 })
}

/// The words of the dictionary that can be suggested (letters and apostrophes, no abbreviations), by length, each
/// with its letter set.
fn by_length() -> &'static Vec<Vec<(u32, &'static str)>> {
    static WORDS: OnceLock<Vec<Vec<(u32, &str)>>> = OnceLock::new();
    WORDS.get_or_init(|| {
        let mut out = vec![Vec::new(); SPELL_LONGEST + 3];
        let mut words: Vec<&str> = dict().keys().copied().filter(|w| w.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')).collect();
        words.sort_unstable();
        for w in words {
            if let Some(bucket) = out.get_mut(w.len()) {
                bucket.push((letter_set(w.chars()), w));
            }
        }
        out
    })
}

/// The edit distance of `a` and `b` with a swap of two neighbours as one edit, or None once it is beyond `max`.
fn distance_within(a: &[char], b: &[u8], max: usize) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max || n > SPELL_LONGEST || m > SPELL_LONGEST + 2 {
        return None;
    }
    let (mut before, mut prev, mut cur) = ([0u8; SPELL_LONGEST + 3], [0u8; SPELL_LONGEST + 3], [0u8; SPELL_LONGEST + 3]);
    for (j, cell) in prev.iter_mut().enumerate().take(m + 1) {
        *cell = j as u8;
    }
    for i in 1..=n {
        cur[0] = i as u8;
        let mut least = i as u8;
        for j in 1..=m {
            let same = a[i - 1] == char::from(b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + u8::from(!same));
            if i > 1 && j > 1 && a[i - 1] == char::from(b[j - 2]) && a[i - 2] == char::from(b[j - 1]) {
                v = v.min(before[j - 2] + 1);
            }
            cur[j] = v;
            least = least.min(v);
        }
        if usize::from(least) > max {
            return None;
        }
        (before, prev) = (prev, cur);
    }
    let d = usize::from(prev[m]);
    (d <= max).then_some(d)
}

/// The dictionary words closest to an unknown word, best first: fewest edits, the same first letter, the same set of
/// letters (a swap or a doubled key: `teh`, `offf`), the nearest length, then the alphabet.
// ponytail: CMUdict has no word frequencies, so a common word does not outrank a rare one at the same distance; add a
// frequency list if the order bothers learners.
fn corrections(word: &str, limit: usize) -> Vec<&'static str> {
    let a: Vec<char> = word.chars().collect();
    let set = letter_set(word.chars());
    let mut found: Vec<(usize, bool, bool, usize, &str)> = Vec::new();
    let buckets = by_length();
    for (len, bucket) in buckets.iter().enumerate().take(a.len() + 3).skip(a.len().saturating_sub(2)) {
        for &(letters, w) in bucket {
            if (letters ^ set).count_ones() > 4 {
                continue;
            }
            if let Some(d) = distance_within(&a, w.as_bytes(), 2) {
                found.push((d, w.as_bytes()[0] as char != a[0], letters != set, len.abs_diff(a.len()), w));
            }
        }
    }
    found.sort_unstable();
    found.into_iter().map(|f| f.4).take(limit).collect()
}

/// Whether text is spelt right and, when it is not, up to five corrections of it. English is checked word by word
/// against CMUdict, numbers counting as known; Chinese has to be a word of CC-CEDICT, with no corrections.
pub fn spell(text: &str) -> (bool, Vec<String>) {
    let text = text.trim();
    if text.is_empty() {
        return (true, Vec::new());
    }
    if is_chinese(text) {
        let word = text.trim_matches(|c: char| !is_han(c));
        return (cedict().contains_key(word), Vec::new());
    }
    // The words between spaces and hyphens, each with the punctuation around it kept apart.
    let mut parts: Vec<(String, Vec<String>)> = Vec::new();
    let mut ok = true;
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(|c: char| c.is_whitespace() || c == '-').unwrap_or(rest.len());
        let (token, after) = rest.split_at(end);
        let gap = after.len() - after.trim_start_matches(|c: char| c.is_whitespace() || c == '-').len();
        let (gap, next) = after.split_at(gap);
        rest = next;
        let core = token.trim_matches(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'));
        let lower = core.replace('’', "'").to_lowercase();
        let known = |w: &str| w.is_empty() || w.chars().any(|c| c.is_numeric()) || dict().contains_key(w);
        let mut choices = Vec::new();
        if !known(&lower) && !known(lower.trim_matches('\'')) {
            ok = false;
            let capital = core.starts_with(char::is_uppercase);
            let at = token.find(core).unwrap_or(0);
            choices = corrections(lower.trim_matches('\''), 5)
                .into_iter()
                .map(|w| {
                    let w = if capital { w[..1].to_uppercase() + &w[1..] } else { w.to_owned() };
                    format!("{}{w}{}{gap}", &token[..at], &token[at + core.len()..])
                })
                .collect();
        }
        parts.push((format!("{token}{gap}"), choices));
    }
    if ok {
        return (true, Vec::new());
    }
    let mut out: Vec<String> = Vec::new();
    for rank in 0..5 {
        let whole: String = parts.iter().map(|(as_is, choices)| choices.get(rank).or(choices.last()).unwrap_or(as_is).as_str()).collect();
        let whole = whole.trim_end().to_owned();
        if whole != text && !out.contains(&whole) {
            out.push(whole);
        }
    }
    (false, out)
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

/// The longest word of the dictionary that starts at the character `i` of `text`, as its length in characters and
/// its numbered pinyin; only a Han character starts one.
fn longest(text: &str, chars: &[(usize, char)], i: usize) -> Option<(usize, &'static str)> {
    let (start, c) = chars[i];
    if !is_han(c) {
        return None;
    }
    (1..=LONGEST.min(chars.len() - i)).rev().find_map(|n| {
        let end = chars.get(i + n).map_or(text.len(), |c| c.0);
        cedict().get(&text[start..end]).map(|p| (n, *p))
    })
}

/// Text cut into words, the longest that the dictionary knows first.
fn pieces(text: &str) -> Vec<Piece> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        match longest(text, &chars, i) {
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

/// Chinese text cut into words for reading, by forward maximum matching on CC-CEDICT: each piece with whether it is
/// a word of the dictionary. What lies between words (punctuation, spaces, Latin letters, characters it does not
/// know) comes in runs of its own, so that the pieces put together are the text again.
pub fn segment(text: &str) -> Vec<(String, bool)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some((n, _)) = longest(text, &chars, i) {
            let end = chars.get(i + n).map_or(text.len(), |c| c.0);
            out.push((text[chars[i].0..end].to_owned(), true));
            i += n;
            continue;
        }
        match out.last_mut() {
            Some((run, false)) => run.push(chars[i].1),
            _ => out.push((chars[i].1.to_string(), false)),
        }
        i += 1;
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

/// A short English gloss of a Chinese word from CC-CEDICT, such as `to learn; to study` for 学习: the one of the
/// reading that `pinyin` gives the word.
pub fn gloss(word: &str) -> Option<&'static str> {
    static TABLE: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut text = String::new();
        if flate2::read::GzDecoder::new(GLOSSES).read_to_string(&mut text).is_err() {
            return HashMap::new();
        }
        // Unpacked once and kept as long as the app runs, like the dictionaries built into it.
        let text: &'static str = text.leak();
        let entries: Vec<(&str, &str, &str, &str)> = CEDICT
            .lines()
            .zip(text.lines())
            .filter_map(|(line, gloss)| {
                let (heads, pinyin) = line.split_once(" [")?;
                let (traditional, simplified) = heads.split_once(' ')?;
                Some((traditional, simplified, pinyin.trim_end_matches(']'), gloss))
            })
            .collect();
        // Several traditional forms may share a simplified one and its reading, such as 後 behind and 后 empress for
        // 后; the one more words are written with is the meaning meant.
        let mut uses: HashMap<char, usize> = HashMap::new();
        for c in entries.iter().flat_map(|e| e.0.chars()) {
            *uses.entry(c).or_default() += 1;
        }
        let mut best: HashMap<&str, (usize, &str)> = HashMap::new();
        for &(traditional, simplified, pinyin, gloss) in &entries {
            let weight = traditional.chars().map(|c| uses[&c]).sum();
            for key in [simplified, traditional] {
                let shown = cedict().get(key) == Some(&pinyin);
                if shown && !gloss.is_empty() && best.get(key).is_none_or(|b| weight > b.0) {
                    best.insert(key, (weight, gloss));
                }
            }
        }
        best.into_iter().map(|(key, (_, gloss))| (key, gloss)).collect()
    });
    table.get(word).copied()
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
    fn chinese_is_cut_into_the_longest_words() {
        let words = |text: &str| segment(text).into_iter().map(|(w, ok)| format!("{w}{}", if ok { "" } else { "?" })).collect::<Vec<_>>();
        assert_eq!(words("我们学习中文。"), ["我们", "学习", "中文", "。?"]);
        assert_eq!(words("學習"), ["學習"]);
        assert_eq!(words("我用iPhone, 好吗？"), ["我", "用", "iPhone, ?", "好", "吗", "？?"]);
        assert_eq!(words("hello"), ["hello?"]);
        assert!(segment("").is_empty());
        let text = "他说：“明天见！”\n2026年，北京。";
        assert_eq!(segment(text).into_iter().map(|(w, _)| w).collect::<String>(), text, "the pieces make the text again");
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
    fn chinese_words_have_a_short_english_gloss() {
        assert_eq!(gloss("学习"), Some("to learn; to study"));
        assert_eq!(gloss("學習"), Some("to learn; to study"));
        assert_eq!(gloss("电脑"), Some("computer"), "without the classifier");
        assert_eq!(gloss("苹果"), Some("apple"), "the word, not the company");
        // The reading the app gives: 着 as the particle zhe5, whose own entry only says it is a variant of 著.
        assert_eq!(gloss("着"), Some("aspect particle indicating action in progress or ongoing state"));
        assert_eq!(gloss("吗"), Some("(question particle for \"yes-no\" questions)"));
        // A simplified form shared by several traditional ones means the common one, a traditional form itself.
        assert_eq!(gloss("后"), Some("back; behind; rear"));
        assert_eq!(gloss("年"), Some("year"), "not 秊, an old character for the harvest");
        assert_eq!(gloss("麵"), Some("flour; noodles; (of food) soft (not crunchy)"));
        assert_eq!(gloss("我们学习"), None);
        assert_eq!(gloss("hello"), None);
        assert_eq!(gloss(""), None);
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
    fn spelling_is_checked_word_by_word_with_corrections() {
        assert_eq!(spell("  "), (true, vec![]));
        assert_eq!(spell("Take off!"), (true, vec![]));
        assert_eq!(spell("well-known 3D students'"), (true, vec![]));
        assert_eq!(spell("recieve").1.first().map(String::as_str), Some("receive"));
        assert_eq!(spell("beautifull").1.first().map(String::as_str), Some("beautiful"));
        assert_eq!(spell("teh").1.first().map(String::as_str), Some("the"), "a swap of letters comes first");
        let (ok, fixes) = spell("take offf");
        assert!(!ok && fixes.iter().all(|f| f.starts_with("take ")), "{fixes:?}");
        assert_eq!(fixes.first().map(String::as_str), Some("take off"), "a doubled key");
        let (_, fixes) = spell("Recieve,");
        assert_eq!(fixes.first().map(String::as_str), Some("Receive,"));
        assert!(fixes.len() <= 5 && !fixes.contains(&"Recieve,".to_owned()));
        assert_eq!(spell("qzxqzxqzx"), (false, vec![]));
        assert_eq!(spell("学习"), (true, vec![]));
        assert_eq!(spell("学习习习"), (false, vec![]));
    }

    #[test]
    fn spelling_answers_quickly() {
        spell("warm");
        let start = std::time::Instant::now();
        for word in ["recieve", "beautifull", "acommodation", "enviroment", "definately"] {
            spell(word);
        }
        let each = start.elapsed() / 5;
        println!("spell: {each:?} a word");
        // Debug builds of this crate are not optimised; the app is built for release.
        assert!(cfg!(debug_assertions) || each < std::time::Duration::from_millis(50), "{each:?}");
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
