//! Pronunciations from the CMU Pronouncing Dictionary (assets/cmudict-LICENSE): the IPA shown with a word, and
//! the phonemes the speaking score compares.

use std::{collections::HashMap, sync::OnceLock};

const CMUDICT: &str = include_str!("../assets/cmudict.dict");

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

/// American IPA such as `/həˈloʊ/`, or an empty string when a word is unknown.
pub fn ipa(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
