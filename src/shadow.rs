//! Shadowing: the learner repeats a reading right after hearing it. The pitch of each voice is traced with YIN, put
//! in semitones around the speaker's own middle so that a man and a woman compare fairly, and the two tunes are laid
//! on one time axis, with a sentence on how they differ.

use crate::{pron, speech::Score};
use serde::Serialize;

/// The points of a contour, evenly spread from its first voiced moment to its last.
pub const POINTS: usize = 120;
/// Pitch is looked for in this range, from deep men's voices to children's.
const MIN_HZ: f32 = 70.0;
const MAX_HZ: f32 = 500.0;
/// YIN's threshold on the normalised difference: the first dip below it is the period, or else the deepest dip.
const DIP: f32 = 0.15;
/// A frame whose period dips no lower than this is too far from periodic to be voiced. Looser than the YIN paper,
/// since a question's rise and a creaky ending blur the period of a real voice through a laptop microphone.
const VOICED: f32 = 0.35;
/// Frames quieter than this (root mean square), or than a tenth of the loudest, have no voice.
const QUIET: f32 = 0.005;
/// Fewer voiced frames (10 ms each) than this are too little to draw a tune from.
const MIN_VOICED: usize = 20;

/// The fundamental frequency of every 10 ms of mono sound, `None` where it has no pitch: silence, noise, or
/// consonants such as s and t. YIN (de Cheveigné and Kawahara, 2002), then a median over five frames.
pub fn track(pcm: &[f32], rate: u32) -> Vec<Option<f32>> {
    let hz = rate as f32;
    // The window holds the longest period, short enough to follow a pitch that moves fast.
    let (hop, window) = (rate as usize / 100, rate as usize / 40);
    let (shortest, longest) = ((hz / MAX_HZ) as usize, (hz / MIN_HZ) as usize);
    // A frame compares its first `window` samples with those up to one past the longest period later.
    let span = window + longest + 1;
    if pcm.len() < span {
        return Vec::new();
    }
    let frames: Vec<&[f32]> = (0..=(pcm.len() - span) / hop).map(|i| &pcm[i * hop..i * hop + span]).collect();
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    let loudest = frames.iter().map(|f| rms(&f[..window])).fold(0.0, f32::max);
    let mut d = vec![0f32; longest + 2];
    let raw: Vec<Option<f32>> = frames
        .iter()
        .map(|x| {
            if rms(&x[..window]) < QUIET.max(loudest / 10.0) {
                return None;
            }
            // The cumulative mean normalised difference between the frame and itself `lag` samples later.
            let mut total = 0.0;
            d[0] = 1.0;
            for lag in 1..d.len() {
                let diff: f32 = x[..window].iter().zip(&x[lag..lag + window]).map(|(a, b)| (a - b) * (a - b)).sum();
                total += diff;
                d[lag] = if total > 0.0 { diff * lag as f32 / total } else { 1.0 };
            }
            // The first dip under the threshold, followed down to its bottom, or else the deepest one.
            let deepest = || (shortest..=longest).min_by(|&a, &b| d[a].total_cmp(&d[b])).unwrap_or(shortest);
            let mut lag = (shortest..=longest).find(|&l| d[l] < DIP).unwrap_or_else(deepest);
            while lag < longest && d[lag + 1] < d[lag] {
                lag += 1;
            }
            if d[lag] >= VOICED {
                return None;
            }
            // A parabola through the bottom and its neighbours puts the period between two samples.
            let (a, b, c) = (d[lag - 1], d[lag], d[lag + 1]);
            let bend = a - 2.0 * b + c;
            let shift = if bend > 0.0 { 0.5 * (a - c) / bend } else { 0.0 };
            Some(hz / (lag as f32 + shift))
        })
        .collect();
    smooth(&raw)
}

/// Takes the median of five frames along each voiced run, which removes the odd frame that jumps an octave, then
/// drops pieces shorter than 40 ms between gaps or leaps of more than four semitones in one frame, which no voice
/// makes: those are clicks, breaths and the pitch errors of a creaky ending.
fn smooth(f0: &[Option<f32>]) -> Vec<Option<f32>> {
    let mut out = f0.to_vec();
    for run in runs(f0, |_, _| false) {
        let v: Vec<f32> = f0[run.clone()].iter().flatten().copied().collect();
        for (i, slot) in out[run].iter_mut().enumerate() {
            *slot = Some(median(&v[i.saturating_sub(2)..(i + 3).min(v.len())]));
        }
    }
    for run in runs(&out, |a, b| (a / b).max(b / a) > 1.26) {
        if run.len() < 4 {
            out[run].fill(None);
        }
    }
    out
}

/// The stretches of consecutive voiced frames, also cut between two that `cut` finds too far apart.
fn runs(f0: &[Option<f32>], cut: impl Fn(f32, f32) -> bool) -> Vec<std::ops::Range<usize>> {
    let (mut out, mut start, mut last) = (Vec::new(), None, 0.0);
    for i in 0..=f0.len() {
        let here = f0.get(i).copied().flatten();
        if let Some(s) = start
            && here.is_none_or(|h| cut(last, h))
        {
            out.push(s..i);
            start = None;
        }
        if let Some(h) = here {
            start = start.or(Some(i));
            last = h;
        }
    }
    out
}

fn median(values: &[f32]) -> f32 {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Each pitch in semitones above or below the speaker's median, so that a man and a woman saying the same tune
/// draw the same line.
pub fn semitones(f0: &[Option<f32>]) -> Vec<Option<f32>> {
    let voiced: Vec<f32> = f0.iter().flatten().copied().collect();
    if voiced.is_empty() {
        return vec![None; f0.len()];
    }
    let middle = median(&voiced);
    f0.iter().map(|f| f.map(|f| 12.0 * (f / middle).log2())).collect()
}

/// The contour from its first voiced frame to its last at `POINTS` evenly spread times, so that the pauses before
/// and after the voice do not count and a slow speaker lines up with a fast one.
pub fn normalize(st: &[Option<f32>]) -> Vec<Option<f32>> {
    let (Some(first), Some(last)) = (st.iter().position(Option::is_some), st.iter().rposition(Option::is_some)) else {
        return vec![None; POINTS];
    };
    let span = (last - first) as f32;
    (0..POINTS)
        .map(|i| {
            let at = first as f32 + span * i as f32 / (POINTS - 1) as f32;
            let (j, frac) = (at as usize, at.fract());
            match (st[j], st.get(j + 1).copied().flatten()) {
                (Some(a), Some(b)) => Some(a + (b - a) * frac),
                (a, b) => if frac < 0.5 { a } else { b },
            }
        })
        .collect()
}

/// The tune of a voice on the common axis, or `None` when too little of it is voiced to draw.
pub fn contour(pcm: &[f32], rate: u32) -> Option<Vec<Option<f32>>> {
    let f0 = track(pcm, rate);
    (f0.iter().flatten().count() >= MIN_VOICED).then(|| normalize(&semitones(&f0)))
}

/// A word of the sentence (a character for Chinese) and where it roughly falls on the time axis.
#[derive(Debug, Serialize)]
pub struct Word {
    pub text: String,
    /// The pinyin of a Chinese character, with its tone mark.
    pub reading: String,
    /// Its stretch of the axis, from 0 to 1.
    pub from: f32,
    pub to: f32,
    /// The scoring did not hear it.
    pub missed: bool,
}

/// The words along the axis in proportion to their syllables, which is about where they fall once a contour spans
/// just the voiced part, each marked when the scoring did not hear it. `missed` is in the order of the sentence.
fn words(sentence: &str, missed: &[String]) -> Vec<Word> {
    let mut left = missed.iter().peekable();
    let mut take = |key: &str| left.next_if(|m| *m == key).is_some();
    let mut out: Vec<(String, String, usize, bool)> = Vec::new();
    if pron::is_chinese(sentence) {
        let han: Vec<char> = sentence.chars().filter(|&c| pron::is_han(c)).collect();
        let syllables = pron::syllables(sentence);
        for (i, c) in han.iter().enumerate() {
            // A character's own syllable only when the dictionary read every one, so that none is shifted.
            let reading = if syllables.len() == han.len() { pron::mark(&syllables[i]) } else { String::new() };
            let missed = !reading.is_empty() && take(&reading);
            out.push((c.to_string(), reading, 1, missed));
        }
    } else {
        for token in sentence.split_whitespace() {
            let parts = pron::words(token);
            if parts.is_empty() {
                continue;
            }
            // ponytail: vowel groups count "price" as two syllables; CMUdict's stress marks would be exact.
            let vowels = parts.concat().split(|c: char| !"aeiouy".contains(c)).filter(|s| !s.is_empty()).count();
            let missed = parts.iter().fold(false, |m, p| take(p) || m);
            out.push((token.to_owned(), String::new(), vowels.max(1), missed));
        }
    }
    let total = out.iter().map(|w| w.2).sum::<usize>().max(1) as f32;
    let mut at = 0;
    out.into_iter()
        .map(|(text, reading, weight, missed)| {
            let from = at as f32 / total;
            at += weight;
            Word { text, reading, from, to: at as f32 / total, missed }
        })
        .collect()
}

/// The mean of the voiced points between two times of the axis.
fn mean(c: &[Option<f32>], from: f32, to: f32) -> Option<f32> {
    let last = (POINTS - 1) as f32;
    let v: Vec<f32> = c.iter().enumerate().filter(|(i, _)| (from..=to).contains(&(*i as f32 / last))).filter_map(|(_, v)| *v).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
}

/// How far the pitch moves over a stretch of the axis, in semitones: the mean of its last third less that of its first.
fn movement(c: &[Option<f32>], from: f32, to: f32) -> Option<f32> {
    let third = (to - from) / 3.0;
    Some(mean(c, to - third, to)? - mean(c, from, from + third)?)
}

/// How much the pitch varies: the standard deviation of the voiced points.
fn spread(c: &[Option<f32>]) -> f32 {
    let v: Vec<f32> = c.iter().flatten().copied().collect();
    let n = v.len().max(1) as f32;
    let m = v.iter().sum::<f32>() / n;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / n).sqrt()
}

/// How closely two contours rise and fall together (Pearson's correlation), over the points where both are voiced.
fn correlation(a: &[Option<f32>], b: &[Option<f32>]) -> Option<f32> {
    let pairs: Vec<(f32, f32)> = a.iter().zip(b).filter_map(|(x, y)| Some(((*x)?, (*y)?))).collect();
    if pairs.len() < POINTS / 4 {
        return None;
    }
    let n = pairs.len() as f32;
    let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f32>() / n, pairs.iter().map(|p| p.1).sum::<f32>() / n);
    let (mut xy, mut xx, mut yy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        xy += (x - mx) * (y - my);
        xx += (x - mx) * (x - mx);
        yy += (y - my) * (y - my);
    }
    (xx > 0.0 && yy > 0.0).then(|| xy / (xx * yy).sqrt())
}

/// One plain sentence on the learner's tune against the reading's: for Chinese the character whose tone moves the
/// wrong way first, then the end of the sentence, the range of the voice, and the word that differs most.
fn feedback(reference: Option<&[Option<f32>]>, learner: Option<&[Option<f32>]>, words: &[Word], chinese: bool) -> String {
    let Some(learner) = learner else {
        return "Chưa nghe đủ giọng để vẽ ngữ điệu, hãy nói to và liền hơi hơn.".into();
    };
    let Some(reference) = reference else {
        return "Tải giọng Kokoro của khóa này trong Cài đặt để so ngữ điệu của bạn với giọng mẫu.".into();
    };
    let way = |m: f32| ["xuống giọng", "giữ ngang", "lên giọng"][usize::from(m > -1.5) + usize::from(m > 1.5)];
    if chinese {
        // A neutral tone (its pinyin has no mark) just follows its neighbours, so it is never the one to blame.
        let wrong = words
            .iter()
            .filter(|w| !w.reading.is_ascii())
            .filter_map(|w| Some((w, movement(reference, w.from, w.to)?, movement(learner, w.from, w.to)?)))
            .filter(|(_, r, l)| way(*r) != way(*l) && way(*r) != way(0.0) && (r - l).abs() > 3.0)
            .max_by(|a, b| (a.1 - a.2).abs().total_cmp(&(b.1 - b.2).abs()));
        if let Some((w, r, l)) = wrong {
            let reading = if w.reading.is_empty() { String::new() } else { format!(" ({})", w.reading) };
            return format!("Chữ {}{reading}: giọng mẫu {}, còn bạn {}; chú ý thanh điệu của chữ này.", w.text, way(r), way(l));
        }
    } else {
        // How high the voice ends against its middle: a question that rises ends high, a statement low.
        let end = |c| mean(c, 0.88, 1.0).unwrap_or(0.0) - mean(c, 0.0, 1.0).unwrap_or(0.0);
        let (r, l) = (end(reference), end(learner));
        if r > 2.5 && l < 0.5 {
            return "Cuối câu giọng mẫu lên cao như khi hỏi, còn bạn lại thấp giọng: hãy lên giọng ở từ cuối.".into();
        }
        if r < -1.5 && l > 1.5 {
            return "Cuối câu giọng mẫu hạ thấp, còn bạn lại lên giọng: hãy hạ giọng ở từ cuối.".into();
        }
    }
    let (r, l) = (spread(reference), spread(learner));
    if l < r * 0.55 {
        return if chinese {
            "Giọng bạn khá đều nên thanh điệu chưa rõ, hãy lên xuống giọng rõ hơn theo từng chữ.".into()
        } else {
            "Giọng bạn khá đều, còn mẫu lên xuống rõ hơn: hãy nhấn giọng vào những từ chính.".into()
        };
    }
    if l > r * 1.8 {
        return "Giọng bạn lên xuống nhiều hơn mẫu, hãy nói tự nhiên và đều hơn một chút.".into();
    }
    if correlation(reference, learner).is_some_and(|c| c >= 0.6) {
        return format!("{} của bạn khá giống mẫu, giữ vững nhé!", if chinese { "Thanh điệu" } else { "Ngữ điệu" });
    }
    let gap = |w: &Word| Some((mean(reference, w.from, w.to)? - mean(learner, w.from, w.to)?).abs());
    match words.iter().filter_map(|w| Some((w, gap(w)?))).max_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((w, _)) => format!(
            "Chỗ khác mẫu nhiều nhất là {} \"{}\", nghe lại rồi bắt chước cao độ ở đó.",
            if chinese { "chữ" } else { "từ" },
            w.text.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        ),
        None => "Ngữ điệu còn khác mẫu, nghe lại rồi bắt chước chỗ lên xuống giọng.".into(),
    }
}

/// A take of shadowing: the score of its words, both tunes and what to change.
#[derive(Debug, Serialize)]
pub struct Shadow {
    pub score: Score,
    /// The learner's tune and the reading's, in semitones around each one's middle at `POINTS` times, `None` where
    /// unvoiced; no learner's when too little was voiced, no reading's when the course has no Kokoro voice.
    pub learner: Option<Vec<Option<f32>>>,
    pub reference: Option<Vec<Option<f32>>>,
    pub words: Vec<Word>,
    pub feedback: String,
    /// How closely the two tunes rise and fall together, from 0 to 100.
    pub similar: Option<u8>,
}

/// Puts the learner's take (`pcm` at 16 kHz, as speech::stop gives it) beside the reading's contour. The score
/// stays the one of the words: a wrong word with the right tune does not pass.
pub fn compare(sentence: &str, score: Score, pcm: &[f32], reference: Option<Vec<Option<f32>>>) -> Shadow {
    let reference = reference.filter(|r| r.len() == POINTS);
    let learner = contour(pcm, 16_000);
    let mut words = words(sentence, &score.missed);
    // A word, or a short Chinese one, gets no list of missed words: under 50 none of it was heard.
    if score.score < 50 && score.missed.is_empty() {
        words.iter_mut().for_each(|w| w.missed = true);
    }
    let feedback = feedback(reference.as_deref(), learner.as_deref(), &words, pron::is_chinese(sentence));
    let similar = learner
        .as_deref()
        .zip(reference.as_deref())
        .and_then(|(l, r)| correlation(r, l))
        .map(|c| (c.max(0.0) * 100.0).round() as u8);
    Shadow { score, learner, reference, words, feedback, similar }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: u32 = 16_000;

    /// One second of a tone whose pitch goes from `from` to `to` Hz in a straight line.
    fn glide(from: f32, to: f32) -> Vec<f32> {
        let mut phase = 0.0;
        (0..RATE)
            .map(|i| {
                phase += TAU * (from + (to - from) * i as f32 / RATE as f32) / RATE as f32;
                0.5 * phase.sin()
            })
            .collect()
    }

    #[test]
    fn a_sine_at_220_hz_reads_220_hz() {
        let f0 = track(&glide(220.0, 220.0), RATE);
        let voiced: Vec<f32> = f0.iter().flatten().copied().collect();
        assert!(voiced.len() > 90, "{} of {}", voiced.len(), f0.len());
        assert!(voiced.iter().all(|f| (f - 220.0).abs() < 1.0), "{voiced:?}");
        let low = track(&glide(110.0, 110.0), RATE);
        assert!(low.iter().flatten().all(|f| (f - 110.0).abs() < 1.0));
    }

    #[test]
    fn silence_and_noise_have_no_pitch() {
        assert!(track(&vec![0.0; RATE as usize], RATE).iter().all(Option::is_none));
        // A fixed pseudo-random hiss.
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..RATE)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect();
        assert!(track(&noise, RATE).iter().all(Option::is_none));
        assert!(track(&[0.1; 100], RATE).is_empty(), "too short for one frame");
        assert!(contour(&vec![0.0; RATE as usize], RATE).is_none());
    }

    #[test]
    fn a_glide_rises() {
        let f0: Vec<f32> = track(&glide(150.0, 300.0), RATE).into_iter().flatten().collect();
        assert!(f0.len() > 90);
        assert!(f0.windows(2).all(|w| w[1] >= w[0] - 1.0), "{f0:?}");
        assert!((f0[0] - 150.0).abs() < 10.0 && (f0[f0.len() - 1] - 300.0).abs() < 15.0, "{f0:?}");
        // On the common axis it climbs an octave.
        let c = contour(&glide(150.0, 300.0), RATE).unwrap();
        let (first, last) = (c[0].unwrap(), c[POINTS - 1].unwrap());
        assert!((last - first - 12.0).abs() < 1.0, "{first} to {last}");
    }

    #[test]
    fn smoothing_removes_octave_slips_and_short_leaps() {
        let hz = |v: &[(f32, usize)]| v.iter().flat_map(|&(f, n)| vec![Some(f); n]).collect::<Vec<_>>();
        // One frame an octave low inside a run is put back; a short burst of a wrong pitch is dropped.
        let slip = smooth(&hz(&[(235.0, 4), (117.0, 1), (235.0, 4)]));
        assert!(slip.iter().all(|f| *f == Some(235.0)), "{slip:?}");
        let burst = smooth(&hz(&[(120.0, 8), (490.0, 3), (75.0, 8)]));
        assert_eq!(burst.iter().filter(|f| f.is_some_and(|f| f > 400.0)).count(), 0, "{burst:?}");
        assert_eq!(burst.iter().flatten().count(), 16);
        assert_eq!(smooth(&hz(&[(200.0, 3)])), [None; 3], "too short to be voice");
    }

    #[test]
    fn semitones_are_counted_from_the_speakers_middle() {
        assert_eq!(semitones(&[Some(100.0), Some(200.0), None, Some(400.0)]), [Some(-12.0), Some(0.0), None, Some(12.0)]);
        // A man and a woman with the same tune an octave apart draw the same line.
        let man = [Some(100.0), Some(120.0), Some(150.0)];
        let woman = man.map(|f| f.map(|f| f * 2.0));
        assert_eq!(semitones(&man), semitones(&woman));
        assert_eq!(semitones(&[None, None]), [None, None]);
    }

    #[test]
    fn the_axis_spans_the_voiced_part() {
        let st = [None, None, Some(0.0), Some(1.0), None, Some(3.0), None];
        let c = normalize(&st);
        assert_eq!(c.len(), POINTS);
        assert_eq!((c[0], c[POINTS - 1]), (Some(0.0), Some(3.0)));
        assert!(c[POINTS / 6].is_some_and(|v| v > 0.0 && v < 1.0), "between the first two");
        assert_eq!(c[POINTS * 2 / 3], None, "the gap stays a gap");
        assert_eq!(normalize(&[None; 3]), vec![None; POINTS]);
    }

    #[test]
    fn words_spread_by_syllables_and_marked_when_missed() {
        let w = words("We need to negotiate, now!", &["negotiate".into()]);
        assert_eq!(w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["We", "need", "to", "negotiate,", "now!"]);
        assert_eq!(w.iter().map(|w| w.missed).collect::<Vec<_>>(), [false, false, false, true, false]);
        assert_eq!((w[0].from, w[4].to), (0.0, 1.0));
        assert!(w.windows(2).all(|p| p[0].to == p[1].from));
        assert!(w[3].to - w[3].from > 3.0 * (w[2].to - w[2].from), "negotiate is longer than to");
        let zh = words("我们明天去北京。", &["míng".into(), "tiān".into()]);
        assert_eq!(zh.iter().map(|w| w.reading.as_str()).collect::<Vec<_>>(), ["wǒ", "men", "míng", "tiān", "qù", "běi", "jīng"]);
        assert_eq!(zh.iter().filter(|w| w.missed).map(|w| w.text.as_str()).collect::<String>(), "明天");
        assert!((zh[1].to - 2.0 / 7.0).abs() < 1e-6);
    }

    /// A contour from a function of the time on the axis.
    fn shape(f: impl Fn(f32) -> f32) -> Vec<Option<f32>> {
        (0..POINTS).map(|i| Some(f(i as f32 / (POINTS - 1) as f32))).collect()
    }

    #[test]
    fn feedback_names_what_differs() {
        let en = words("Are you ready for the trip?", &[]);
        let question = shape(|t| if t > 0.8 { (t - 0.8) * 30.0 } else { (t * 9.0).sin() });
        let flat_end = shape(|t| if t > 0.8 { -(t - 0.8) * 5.0 } else { (t * 9.0).sin() });
        let say = |r: &[Option<f32>], l: &[Option<f32>]| feedback(Some(r), Some(l), &en, false);
        assert!(say(&question, &flat_end).contains("lên giọng ở từ cuối"), "{}", say(&question, &flat_end));
        assert!(say(&question, &question).contains("khá giống mẫu"));
        let (lively, monotone) = (shape(|t| 3.0 * (t * 9.0).sin()), shape(|t| 0.1 * (t * 9.0).sin()));
        assert!(say(&lively, &monotone).contains("khá đều"), "{}", say(&lively, &monotone));
        assert!(feedback(None, Some(&question), &en, false).contains("Cài đặt"));
        assert!(feedback(Some(&question), None, &en, false).contains("nói to"));
        // Chinese: the character said with a falling tone where the reading rises.
        let zh = words("学习", &[]);
        let rise = shape(|t| 6.0 * t - 3.0);
        let fall_second = shape(|t| if t < 0.5 { 6.0 * t - 3.0 } else { 3.0 - 6.0 * (t - 0.5) });
        let said = feedback(Some(&rise), Some(&fall_second), &zh, true);
        assert!(said.starts_with("Chữ 习 (xí): giọng mẫu lên giọng, còn bạn xuống giọng"), "{said}");
    }

    #[test]
    fn the_score_stays_the_words_whatever_the_tune() {
        let pcm = glide(150.0, 300.0);
        let wrong = crate::speech::score("I am ready", "I am steady", 0.9);
        let take = compare("I am ready", wrong, &pcm, Some(contour(&pcm, RATE).unwrap()));
        assert_eq!(take.similar, Some(100));
        assert!(take.score.score < 75, "{:?}", take.score);
        assert!(take.words[2].missed);
        let silent = compare("I am ready", crate::speech::score("I am ready", "", 0.0), &pcm, Some(vec![None; 3]));
        assert_eq!(silent.reference, None, "a reference of the wrong length is dropped");
        assert!(silent.words.iter().all(|w| w.missed), "nothing was heard");
    }

    /// What a system voice says, as samples at 16 kHz.
    #[cfg(target_os = "macos")]
    fn say(voice: &str, text: &str) -> Vec<f32> {
        let wav = std::env::temp_dir().join(format!("card-english-shadow-{voice}.wav"));
        let said = std::process::Command::new("/usr/bin/say")
            .args(["-v", voice, "--file-format=WAVE", "--data-format=LEI16@16000", "-o"])
            .arg(&wav)
            .arg(text)
            .status()
            .unwrap();
        assert!(said.success());
        let bytes = std::fs::read(&wav).unwrap();
        let at = bytes.windows(4).position(|w| w == b"data").expect("a data chunk") + 8;
        bytes[at..].chunks_exact(2).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect()
    }

    /// The whole path with the real models: Kokoro reads the sentence, the system voice plays the learner, Whisper
    /// scores it and the tunes are compared. `cargo test shadowing -- --ignored --nocapture`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs the Kokoro packs and Whisper in the data folder"]
    fn shadowing_a_kokoro_reading_end_to_end() {
        let dir = crate::models_only();
        for (sentence, learner, said) in [
            ("Are you ready for the trip?", "Daniel", "Are you ready for the trip?"),
            ("Are you ready for the trip?", "Daniel", "You are ready for the trip."),
            ("We need to negotiate a better price.", "Samantha", "We need to negotiate a better price."),
        ] {
            let (samples, rate) = crate::voice::generate(&dir, "en", 3, sentence, 1.0).unwrap();
            let reference = contour(&samples, rate);
            assert!(reference.is_some(), "Kokoro's reading has a tune");
            let pcm = say(learner, said);
            let score = crate::speech::assess(sentence, &pcm, &dir).unwrap();
            let take = compare(sentence, score, &pcm, reference);
            println!("{sentence} / {said}: {} {:?} {:?} {}", take.score.score, take.score.missed, take.similar, take.feedback);
            assert!(take.learner.is_some() && take.similar.is_some(), "{take:?}");
        }
        let sentence = "我们明天去北京。";
        let (samples, rate) = crate::voice::generate(&dir, "zh", 3, sentence, 1.0).unwrap();
        let take = compare(sentence, crate::speech::score_chinese(sentence, sentence, 1.0), &say("Tingting", sentence), contour(&samples, rate));
        println!("{sentence}: {:?} {} {:?}", take.similar, take.feedback, take.words.iter().map(|w| &w.reading).collect::<Vec<_>>());
        assert!(take.learner.is_some() && take.similar.is_some(), "{take:?}");
    }
}
