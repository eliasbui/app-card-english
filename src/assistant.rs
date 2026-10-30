//! What the tutor does rather than says: requests the app recognises by their wording and answers with its own
//! features, as cards the chat draws (suggested words, a quiz, buttons, numbers). A small model on this machine picks
//! the wrong tool too often to be asked which one; it only does the AI steps inside, such as suggesting words.

use crate::{
    ai::{self, Ai, By},
    db, pron, roadmap,
};
use rusqlite::Connection;
use serde_json::{json, Value};

/// The most words of one card; a larger request gets the rest through "Gợi ý thêm".
const BATCH: usize = 10;

/// A request the app answers with a feature of its own.
#[derive(Debug, PartialEq)]
pub enum Intent {
    Suggest { count: usize, topic: String },
    /// `source` as the practice page names its sources: all, hard, recent, topic:<name>.
    Quiz { count: usize, source: String },
    /// An exercise of the practice page, or several to choose from when `exercise` is empty.
    Practice { exercise: &'static str, source: String },
    Exam,
    Import(Vec<String>),
    Extract(String),
    Progress,
    Roadmap,
    /// Set the reminder: a clock time (empty for none), every so many hours, or off.
    Remind { at: String, every_hours: Option<u32>, off: bool },
    /// Change one setting; `label` is what the button says.
    Setting { key: &'static str, value: String, label: String },
    Switch(&'static str),
    StartStudy(String),
    /// A page by its name in the contract, or "packs" for the built-in word packs.
    OpenPage(&'static str),
    /// A roadmap to make; what is missing is asked for.
    CreateRoadmap { purpose: String, days: Option<i64>, minutes: Option<i64> },
    /// The plan for today.
    Today,
    /// How to use a feature: an index into `GUIDES`, the first being the overview.
    Guide(usize),
    SkipWord(String),
    DeleteWord(String),
}

/// The answer to a request: a line for the chat, the cards under it, and who answered when the AI took part.
pub struct Act {
    pub text: String,
    pub cards: Vec<Value>,
    pub by: Option<By>,
}

/// The exercises of the practice page by the names a learner calls them; a name that holds another comes first, so
/// that "luyện nói đuổi" is taken for "nói đuổi" and not "luyện nói".
const EXERCISES: [(&str, &str, &str); 13] = [
    ("sap xep cau", "order", "Sắp xếp câu"),
    ("viet chu han", "hanzi", "Viết chữ Hán"),
    ("hoc thong minh", "smart", "Học thông minh"),
    ("chon dap an", "choice", "Chọn đáp án"),
    ("thanh dieu", "tone", "Luyện thanh điệu"),
    ("trac nghiem", "choice", "Chọn đáp án"),
    ("nghe chep", "dict", "Nghe chép"),
    ("flashcard", "flash", "Flashcard"),
    ("noi duoi", "shadow", "Nói đuổi"),
    ("luyen noi", "speak", "Luyện nói"),
    ("dien tu", "fill", "Điền từ"),
    ("noi tu", "match", "Nối từ"),
    ("dat cau", "write", "Đặt câu"),
];

/// Whether `q` speaks of words ("từ"): with marks, "từ" and not "tự"; typed without them, the bare "tu".
fn says_words(q: &str, f: &str) -> bool {
    let marked = q.chars().any(|c| ai::fold(&c.to_string()) != c.to_lowercase().to_string());
    if marked { q.to_lowercase().contains("từ") } else { f.split(|c: char| !c.is_alphanumeric()).any(|w| w == "tu") }
}

/// The first whole number of `f`, if any.
fn number(f: &str) -> Option<usize> {
    f.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok())
}

/// What follows the first of `markers` in `q`, folded or not, without the polite words that end a request.
fn after(q: &str, f: &str, markers: &[&str]) -> Option<String> {
    let at = markers.iter().filter_map(|m| f.find(m).map(|i| i + m.len())).min()?;
    // The folded text keeps the characters of the original one for one, so the place is the same.
    let rest: String = q.chars().skip(f[..at].chars().count()).collect();
    let mut words: Vec<&str> = rest.split_whitespace().collect();
    while let Some(last) = words.last() {
        let l = ai::fold(last.trim_matches(|c: char| !c.is_alphanumeric()));
        if ["nhe", "nha", "nhen", "di", "giup", "voi", "minh", "toi", "please", "a", "ạ", ""].contains(&l.as_str()) {
            words.pop();
        } else {
            break;
        }
    }
    let topic = words.join(" ").trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
    (!topic.is_empty()).then(|| topic.chars().take(40).collect())
}

/// The topic a request names: after "chủ đề", "về", "lĩnh vực", or the words following the count.
fn topic_of(q: &str, f: &str) -> String {
    if let Some(t) = after(q, f, &["chu de ", "linh vuc ", "topic ", "about ", "nganh "]) {
        return t;
    }
    if let Some(t) = after(q, f, &[" ve "]) {
        return t;
    }
    // "gợi ý 20 từ công nghệ": what follows "từ" or "từ vựng" after the count.
    let words: Vec<&str> = f.split_whitespace().collect();
    match words.iter().position(|w| *w == "tu") {
        Some(i) => {
            let skip = if words.get(i + 1) == Some(&"vung") { i + 2 } else { i + 1 };
            let marker = words[..skip].join(" ");
            after(q, f, &[&marker]).unwrap_or_default()
        }
        None => String::new(),
    }
}

/// The source of words a quiz or an exercise names.
fn source_of(q: &str, f: &str) -> String {
    if f.contains("hay quen") || f.contains("hay sai") {
        "hard".into()
    } else if f.contains("moi hoc") || f.contains("vua hoc") {
        "recent".into()
    } else if let Some(t) = after(q, f, &["chu de "]) {
        format!("topic:{t}")
    } else {
        "all".into()
    }
}

/// The words of `f` as separate tokens, without the punctuation around them ("20:00" and "6.5" stay whole).
fn tokens(f: &str) -> Vec<&str> {
    f.split_whitespace().map(|t| t.trim_matches(|c: char| !c.is_alphanumeric())).filter(|t| !t.is_empty()).collect()
}

fn digits(t: &str) -> Option<u32> {
    (!t.is_empty() && t.len() <= 4 && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok()).flatten()
}

/// Every so many hours ("mỗi 2 giờ", "2 tiếng một lần"), and the tokens it took.
fn every_hours(t: &[&str]) -> Option<(u32, Vec<usize>)> {
    for i in 0..t.len() {
        let unit = |j: usize| matches!(t.get(j), Some(&"gio") | Some(&"tieng"));
        if t[i] == "moi" {
            if let Some(n) = t.get(i + 1).and_then(|x| digits(x)) {
                if unit(i + 2) {
                    return Some((n, vec![i, i + 1, i + 2]));
                }
            }
            if let Some(n) = t.get(i + 1).and_then(|x| x.strip_suffix('h')).and_then(digits) {
                return Some((n, vec![i, i + 1]));
            }
        }
        if let Some(n) = digits(t[i]) {
            if unit(i + 1) && t.get(i + 2) == Some(&"mot") && t.get(i + 3) == Some(&"lan") {
                return Some((n, vec![i, i + 1, i + 2, i + 3]));
            }
        }
    }
    None
}

/// A time of day in the tokens, as `HH:MM`: "20:00", "20h", "8h30", "8 giờ 30", "8 giờ tối" (the evening hours count
/// from noon).
fn clock_of(t: &[&str]) -> Option<String> {
    for (i, tok) in t.iter().enumerate() {
        let mut used = i + 1;
        let (h, m) = if let Some((a, b)) = tok.split_once(':') {
            (digits(a), digits(b))
        } else if let Some((a, b)) = tok.split_once('h') {
            (digits(a), if b.is_empty() { Some(0) } else { digits(b) })
        } else if digits(tok).is_some() && matches!(t.get(i + 1), Some(&"gio") | Some(&"h")) {
            used = i + 2;
            let m = t.get(used).and_then(|x| digits(x));
            if m.is_some() {
                used += 1;
                if t.get(used) == Some(&"phut") {
                    used += 1;
                }
            }
            (digits(tok), Some(m.unwrap_or(0)))
        } else {
            continue;
        };
        let (Some(mut h), Some(m)) = (h, m) else { continue };
        if matches!(t.get(used), Some(&"chieu") | Some(&"toi") | Some(&"dem")) && (1..12).contains(&h) {
            h += 12;
        }
        if h < 24 && m < 60 {
            return Some(format!("{h:02}:{m:02}"));
        }
    }
    None
}

/// A reminder to set, or to ask the time of.
fn remind_of(f: &str) -> Option<Intent> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if !has(&["nhac hoc", "nhac toi", "nhac minh", "nhac nho", "dat nhac", "bat nhac", "tat nhac", "huy nhac", "ngung nhac", "remind"]) {
        return None;
    }
    if has(&["tat nhac", "huy nhac", "ngung nhac", "khong nhac", "dung nhac", "bo nhac"]) {
        return Some(Intent::Remind { at: String::new(), every_hours: Some(0), off: true });
    }
    let t = tokens(f);
    let every = every_hours(&t);
    let masked: Vec<&str> = t.iter().enumerate().map(|(i, x)| if every.as_ref().is_some_and(|(_, used)| used.contains(&i)) { "" } else { *x }).collect();
    let at = clock_of(&masked).unwrap_or_default();
    let every = every.map(|(n, _)| n).filter(|n| (1..=24).contains(n));
    if at.is_empty() && every.is_none() && !has(&["nhac hoc", "nhac nho", "dat nhac"]) {
        return None;
    }
    Some(Intent::Remind { at, every_hours: every, off: false })
}

/// A setting the wording asks to change.
fn setting_of(q: &str, f: &str) -> Option<Intent> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    let verb = has(&["doi", "chuyen", "bat ", "dat ", "chon", "dung", "set ", "change", "switch", "thiet lap", "chinh", "muon", "hoc ", "doc cham", "doc nhanh"]);
    let t = tokens(f);
    let setting = |key: &'static str, value: &str, label: &str| Some(Intent::Setting { key, value: value.into(), label: label.into() });
    if has(&["tat ngay nghi", "bo ngay nghi", "huy ngay nghi", "khong ngay nghi"]) {
        return setting("streak_freeze", "0", "Tắt ngày nghỉ giữ chuỗi");
    }
    if has(&["bat ngay nghi", "mo ngay nghi", "dung ngay nghi"]) {
        return setting("streak_freeze", "1", "Bật ngày nghỉ giữ chuỗi");
    }
    if !verb {
        return None;
    }
    if has(&["khoa "]) && has(&["sang tieng", "khoa tieng", "khoa trung", "khoa zh", "khoa en"]) {
        if has(&["tieng trung", "khoa trung", "khoa zh"]) {
            return Some(Intent::Switch("zh"));
        }
        if has(&["tieng anh", "khoa en"]) {
            return Some(Intent::Switch("en"));
        }
    }
    if has(&["giao dien", "che do", "theme", "dark mode", "light mode", "nen "]) {
        if has(&["theo he thong", "tu dong", "system"]) {
            return setting("theme", "system", "Giao diện theo hệ thống");
        }
        if has(&["dark", "giao dien toi", "che do toi", "nen toi", "theme toi", "giao dien den", "che do den"]) {
            return setting("theme", "dark", "Đổi sang giao diện tối");
        }
        if has(&["light", "giao dien sang", "che do sang", "nen sang", "theme sang", "giao dien trang"]) {
            return setting("theme", "light", "Đổi sang giao diện sáng");
        }
    }
    if let Some(i) = t.iter().position(|x| *x == "mau") {
        let colors = [("xanh", "blue", "xanh"), ("hong", "pink", "hồng"), ("tim", "purple", "tím"), ("do", "red", "đỏ")];
        let found = t[i + 1..t.len().min(i + 5)].iter().find_map(|x| colors.iter().find(|c| c.0 == *x));
        if let Some((_, value, name)) = found {
            return setting("palette", value, &format!("Đổi màu chủ đạo sang {name}"));
        }
    }
    if has(&["giong"]) {
        let us = has(&["giong my", "anh my", "anh-my", "american"]) || t.contains(&"us");
        let gb = has(&["giong anh", "anh anh", "anh-anh", "british"]) || t.contains(&"uk");
        if us {
            return setting("accent", "en-US", "Dùng giọng Anh-Mỹ");
        }
        if gb {
            return setting("accent", "en-GB", "Dùng giọng Anh-Anh");
        }
    }
    if has(&["toc do doc", "doc cham", "doc nhanh", "toc do giong"]) {
        let exact = t.iter().find_map(|x| x.parse::<f64>().ok().filter(|n| (0.5..=1.5).contains(n) && x.contains('.')));
        let (value, label) = match exact {
            Some(n) => (n.to_string(), format!("Đọc với tốc độ {n}")),
            None if has(&["nhanh"]) => ("1.2".into(), "Đọc nhanh hơn".into()),
            None if has(&["cham"]) => ("0.8".into(), "Đọc chậm hơn".into()),
            None if has(&["binh thuong"]) => ("1".into(), "Đọc với tốc độ bình thường".into()),
            None => return None,
        };
        return Some(Intent::Setting { key: "speech_rate", value, label });
    }
    if has(&["ti le nho"]) {
        let n = number(f).filter(|n| (70..=97).contains(n))?;
        return setting("desired_retention", &(n as f64 / 100.0).to_string(), &format!("Đặt tỉ lệ nhớ mong muốn {n}%"));
    }
    if has(&["ngay on nhe", "ngay on it", "ngay ban"]) {
        let mut days: Vec<u32> = Vec::new();
        for (i, x) in t.iter().enumerate() {
            if *x == "thu" {
                if let Some(n) = t.get(i + 1).and_then(|d| digits(d)).filter(|n| (2..=7).contains(n)) {
                    days.push(n - 1);
                }
            }
        }
        if f.contains("chu nhat") {
            days.push(0);
        }
        if f.contains("cuoi tuan") {
            days.extend([6, 0]);
        }
        days.sort();
        days.dedup();
        if days.is_empty() {
            return None;
        }
        let list: Vec<String> = days.iter().map(u32::to_string).collect();
        let names: Vec<String> = days.iter().map(|d| if *d == 0 { "Chủ nhật".to_owned() } else { format!("Thứ {}", d + 1) }).collect();
        return setting("easy_days", &list.join(","), &format!("Ngày ôn nhẹ: {}", names.join(", ")));
    }
    if has(&["tu moi"]) && has(&["moi ngay", "mot ngay", "hang ngay", "/ngay"]) && !has(&["goi y"]) {
        let n = number(f).filter(|n| (1..=200).contains(n))?;
        return setting("new_per_day", &n.to_string(), &format!("Học {n} từ mới mỗi ngày"));
    }
    if has(&["muc tieu xp", "muc tieu moi ngay", "muc tieu hang ngay", "xp moi ngay", "xp hang ngay", "daily goal"]) {
        let n = number(f)?.clamp(10, 1000);
        return setting("daily_goal", &n.to_string(), &format!("Mục tiêu {n} XP mỗi ngày"));
    }
    let _ = q;
    None
}

/// The word a request to skip or delete names.
fn word_of(q: &str, f: &str, markers: &[&str]) -> Option<String> {
    let rest = after(q, f, markers)?;
    // "xóa từ abc khỏi danh sách": the word ends where the rest of the request begins.
    let folded = ai::fold(&rest);
    let end = [" khoi ", " ra khoi ", " di ", " giup "].iter().filter_map(|m| folded.find(m)).min().unwrap_or(folded.len());
    let word: String = rest.chars().take(folded[..end].chars().count()).collect();
    let word = word.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
    let skip = ["vung", "nay", "do", "nao", "kia", "tren", "dang hoc"].contains(&ai::fold(&word).as_str());
    (!word.is_empty() && !skip && word.chars().count() <= 40).then_some(word)
}

/// A roadmap to create: its purpose, how long, and minutes a day.
fn roadmap_of(q: &str, f: &str) -> Option<Intent> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if !has(&["lo trinh"]) || !has(&["tao", "lap", "len ", "xay dung", "them", "dat ", "thiet ke", "lam giup", "make", "create"]) {
        return None;
    }
    let t = tokens(f);
    let mut days = None;
    let mut minutes = None;
    for i in 0..t.len() {
        let (Some(n), Some(unit)) = (digits(t[i]), t.get(i + 1)) else { continue };
        let n = n as i64;
        match *unit {
            "thang" => days = days.or(Some(n * 30)),
            "tuan" => days = days.or(Some(n * 7)),
            "nam" => days = days.or(Some(n * 365)),
            "ngay" if i == 0 || t[i - 1] != "moi" => days = days.or(Some(n)),
            "phut" => minutes = minutes.or(Some(n)),
            "tieng" | "gio" if has(&["moi ngay", "mot ngay", "hang ngay"]) => minutes = minutes.or(Some(n * 60)),
            _ => continue,
        }
    }
    let rest = after(q, f, &["lo trinh "]).unwrap_or_default();
    let folded = ai::fold(&rest);
    let end = [" trong ", " moi ngay", " den ", " voi ", " khoang ", ",", ";"].iter().filter_map(|m| folded.find(m)).min().unwrap_or(folded.len());
    let head: String = rest.chars().take(folded[..end].chars().count()).collect();
    let skip = ["hoc", "cho", "de", "thi", "luyen", "dat", "ve", "la", "voi", "muc", "tieu", "trong", "moi", "mot", "ngay", "thang", "tuan", "nam", "phut", "gio", "tieng"];
    let mut words: Vec<&str> = head.split_whitespace().collect();
    // A duration that came before the purpose, such as "lộ trình 3 tháng IELTS", is not part of it.
    let key: Vec<String> = words.iter().map(|w| ai::fold(w.trim_matches(|c: char| !c.is_alphanumeric()))).collect();
    let units = ["thang", "tuan", "nam", "ngay", "phut", "gio", "tieng"];
    let drop: Vec<bool> = (0..words.len()).map(|i| {
        (digits(&key[i]).is_some() && key.get(i + 1).is_some_and(|u| units.contains(&u.as_str())))
            || (i > 0 && digits(&key[i - 1]).is_some() && units.contains(&key[i].as_str()))
    }).collect();
    let mut at = 0;
    words.retain(|_| { at += 1; !drop[at - 1] });
    while words.first().is_some_and(|w| skip.contains(&ai::fold(w.trim_matches(|c: char| !c.is_alphanumeric())).as_str())) {
        words.remove(0);
    }
    let purpose: String = words.join(" ").trim_matches(|c: char| !c.is_alphanumeric()).chars().take(60).collect();
    Some(Intent::CreateRoadmap { purpose, days, minutes })
}

/// How to use a feature, from the README: the cues that ask for it, a title, and the steps, each with what a button
/// does ("page:home", "study", "packs", "vocab_test", "sync", or nothing).
struct Guide {
    cues: &'static [&'static str],
    title: &'static str,
    items: &'static [(&'static str, &'static str)],
}

/// The first guide is the overview; it is asked for by what it is, the others by their feature.
const GUIDES: [Guide; 15] = [
    Guide {
        cues: &["huong dan su dung", "huong dan dung", "dung app", "su dung app", "app nay lam duoc gi", "app lam duoc gi", "tro ly lam duoc gi",
                "ban lam duoc gi", "ban giup duoc gi", "ban co the lam gi", "lam duoc nhung gi", "co nhung tinh nang"],
        title: "Cách dùng Card English",
        items: &[
            ("Học hằng ngày: mở Học hôm nay và bấm Bắt đầu để ôn các từ đến hạn và học từ mới", "page:home"),
            ("Thêm và quản lý từ ở trang Từ vựng; AI giải thích nghĩa, ví dụ và phiên âm", "page:words"),
            ("Luyện bằng nhiều dạng bài ở trang Bài tập", "page:practice"),
            ("Đặt mục tiêu như IELTS, TOEIC, HSK ở trang Lộ trình để AI chia thành từng chặng", "page:roadmap"),
            ("Xem XP, chuỗi ngày học và từ hay quên ở trang Thống kê", "page:stats"),
            ("Nói chuyện 1 kèm 1 bằng giọng nói với gia sư AI ở mục Gia sư nói", "page:talk"),
            ("Chỉnh nhắc học, giọng đọc, nguồn AI và đồng bộ ở Cài đặt", "page:settings"),
            ("Ngay trong khung chat này, gõ các yêu cầu như \"Gợi ý 20 từ chủ đề du lịch\", \"Nhắc tôi học lúc 20:00\" hay \"Kế hoạch hôm nay\"", ""),
        ],
    },
    Guide {
        cues: &["them tu", "nhap tu", "them nhieu tu", "import", "bo tu co san", "ngsl", "nawl", "tu co san"],
        title: "Cách thêm từ",
        items: &[
            ("Mở trang Từ vựng và gõ từ muốn học; app kiểm tra chính tả, AI giải thích từ loại, nghĩa, ví dụ và từ đồng nghĩa", "page:words"),
            ("Gõ vào khung chat này \"Nhập các từ: a, b, c\" hoặc \"Gợi ý 20 từ chủ đề du lịch\", chọn từ rồi bấm Thêm", ""),
            ("Thêm cả bộ từ có sẵn (NGSL, NAWL, TSL, HSK 3.0): bấm Thêm 10, 20 hoặc 50 từ", "packs"),
            ("Nhập từ file CSV, file text xuất ra của Anki hoặc bản sao JSON; từ đã có thì giữ nguyên", ""),
            ("Phím tắt toàn cục Cmd hoặc Ctrl + Shift + E thêm từ vừa copy, và nếu là cả đoạn văn thì để AI chọn từ đáng học", ""),
        ],
    },
    Guide {
        cues: &["hoc tu moi", "on tap", "lich on", "fsrs", "hoc hang ngay", "hoc hom nay", "hoc theo chu de", "hoan tu", "on truoc", "buoi hoc"],
        title: "Cách học và ôn từ",
        items: &[
            ("Mở Học hôm nay và bấm Bắt đầu: buổi học gồm từ đến hạn ôn và từ mới, trong giới hạn số từ mới mỗi ngày", "study"),
            ("Khi ôn, phím tắt là Space và 1-4; phím Z hoàn tác câu trả lời vừa rồi", ""),
            ("Chọn chủ đề ngay dưới số từ cần ôn để học riêng một chủ đề", "page:home"),
            ("Quá tải thì bấm Hoãn từ cần ôn để dời bài sang các ngày tới, hoặc Ôn trước để học sớm các từ sắp đến hạn", "page:home"),
            ("Chỉnh số từ mới mỗi ngày và tỉ lệ nhớ mong muốn trong Cài đặt", "page:settings"),
        ],
    },
    Guide {
        cues: &["bai tap", "luyen tap", "flashcard", "dien tu", "nghe chep", "nghe viet", "chon dap an", "hoc thong minh", "noi tu", "sap xep cau"],
        title: "Cách làm bài tập",
        items: &[
            ("Mở trang Bài tập và chọn dạng bài: flashcard, điền từ, chọn đáp án, nghe chép, nói, đặt câu, nối từ, sắp xếp câu, nói đuổi, học thông minh", "page:practice"),
            ("Chọn nguồn từ: tất cả các từ, từ hay quên, từ phát âm còn yếu hoặc một chủ đề", ""),
            ("Học thông minh cho mỗi từ một dạng bài vừa sức; trả lời sai thì bấm Vì sao sai? để AI giải thích bằng tiếng Việt", ""),
            ("Gõ vào chat \"Tạo bài tập nối từ chủ đề du lịch\" hoặc \"Đố tôi 5 từ hay quên\" là có ngay", ""),
        ],
    },
    Guide {
        cues: &["thi thu", "bai thi", "von tu", "kiem tra trinh do", "mock test", "kiem tra chang"],
        title: "Cách thi thử và kiểm tra vốn từ",
        items: &[
            ("Kiểm tra vốn từ: khoảng 56 câu Biết hoặc Không biết, ước lượng vốn từ và trình độ của bạn", "vocab_test"),
            ("Đề thi thử đủ bốn kỹ năng Nghe, Đọc, Viết, Nói nằm ở chặng cuối của lộ trình có mục tiêu IELTS, TOEIC, VSTEP, HSK hoặc trình độ từ B2 trở lên", "page:roadmap"),
            ("Mỗi chặng của lộ trình khó kết thúc bằng một bài kiểm tra chặng; cần Qwen 1.5B hoặc một nguồn AI trên mạng", ""),
        ],
    },
    Guide {
        cues: &["lo trinh", "muc tieu hoc", "ke hoach hoc"],
        title: "Cách lập lộ trình",
        items: &[
            ("Mở trang Lộ trình, đặt mục tiêu (IELTS, TOEIC, HSK, du lịch...), trình độ, hạn chót hoặc ngày thi và số phút mỗi ngày", "page:roadmap"),
            ("AI chia thành 4 đến 12 chặng gồm chủ đề, số từ, kỹ năng và bài tập nên làm; bạn cũng có thể tự lập và sửa mọi thứ", ""),
            ("App cho biết bạn nhanh hay chậm so với hạn chót và gợi ý số từ mới mỗi ngày; trang Học hôm nay hiện chặng đang học với nút Học tiếp", "page:home"),
            ("Hoặc nói với mình: \"Tạo lộ trình IELTS 6.5 trong 3 tháng, mỗi ngày 30 phút\"", ""),
        ],
    },
    Guide {
        cues: &["nhac hoc", "nhac nho", "gio yen lang", "khoi dong cung may", "thong bao nhac"],
        title: "Cách đặt nhắc học",
        items: &[
            ("Trong Cài đặt có nhắc mỗi N giờ, nhắc vào một giờ cố định mỗi ngày và giờ yên lặng", "page:settings"),
            ("Bấm vào thông báo là mở cửa sổ Đố nhanh để ôn các từ đến hạn", ""),
            ("Số từ cần học hiện cạnh icon trên thanh menu; có thể bật khởi động cùng máy tính để app chạy trong khay", ""),
            ("Hoặc nói với mình: \"Nhắc tôi học lúc 20:00\" hay \"Nhắc tôi mỗi 2 giờ\"", ""),
        ],
    },
    Guide {
        cues: &["luyen phat am", "cham phat am", "cham diem phat am", "ghi am", "luyen noi", "noi duoi", "thanh dieu"],
        title: "Cách luyện phát âm",
        items: &[
            ("Mở cửa sổ của một từ ở trang Từ vựng để ghi âm và chấm điểm một từ hoặc câu ví dụ; app chỉ ra những từ chưa nghe rõ", "page:words"),
            ("Bài Luyện nói và Nói đuổi ở trang Bài tập; Nói đuổi chấm theo từng từ, kèm đường lên xuống giọng đặt cạnh giọng mẫu", "page:practice"),
            ("Chấm điểm chạy trên máy bằng model nhận giọng tải trong Cài đặt", "page:settings"),
        ],
    },
    Guide {
        cues: &["gia su noi", "noi chuyen voi gia su", "hoi thoai", "nhap vai", "noi chuyen bang giong", "luyen giao tiep"],
        title: "Cách nói chuyện với gia sư",
        items: &[
            ("Mở Gia sư nói (Beta), bấm Bắt đầu nói rồi cứ nói; ngừng một chút là câu được gửi đi, ngắt lời gia sư bất cứ lúc nào hoặc gõ câu thay cho nói", "page:talk"),
            ("Mỗi lượt gia sư sửa lỗi quan trọng nhất, đưa một câu để bạn nói theo và ghi chú lỗi bằng tiếng Việt", ""),
            ("Hội thoại nhập vai ở trang Bài tập (Học cùng AI): chọn tình huống như phỏng vấn, sân bay, gọi món; Kết thúc để nhận nhận xét lỗi", "page:practice"),
        ],
    },
    Guide {
        cues: &["bo qua tu", "xoa tu", "chu de", "chon nhieu tu", "hoc lai tu", "loc tu", "sua tu", "quan ly tu"],
        title: "Cách quản lý từ và chủ đề",
        items: &[
            ("Bỏ qua một từ không muốn học nữa: từ ra khỏi lịch ôn, bài tập và thống kê, bật lại được bất cứ lúc nào; nhờ mình bằng \"Bỏ qua từ X\"", ""),
            ("Chọn nhiều từ: tick ô đầu mỗi dòng (giữ Shift để chọn một dãy) rồi xóa, bỏ qua, học lại hoặc gán chủ đề cho cả nhóm", "page:words"),
            ("Tự tạo chủ đề với màu và mô tả; đổi tên thì mọi từ đổi theo, xóa chủ đề thì từ vẫn còn", "page:words"),
            ("Lọc danh sách theo trạng thái và theo chủ đề", ""),
        ],
    },
    Guide {
        cues: &["doi khoa", "khoa tieng trung", "chuyen khoa", "chu han", "viet chu han"],
        title: "Khóa tiếng Trung",
        items: &[
            ("Đổi khóa ở thanh bên; mỗi khóa là một không gian riêng với lịch ôn, XP, thống kê, màu giao diện và mục tiêu riêng", ""),
            ("Từ tiếng Trung có pinyin, âm Hán Việt, trình độ HSK và câu ví dụ kèm pinyin", ""),
            ("Cửa sổ của mỗi từ có Thứ tự nét và bộ thủ; bài Viết chữ Hán cho bạn tự viết từng nét; bài Luyện thanh điệu nghe và chọn thanh", "page:practice"),
        ],
    },
    Guide {
        cues: &["thong ke", " xp", "chuoi ngay", "thanh tich", "cap do"],
        title: "Cách xem tiến độ",
        items: &[
            ("Mở Thống kê để xem tổng XP, cấp độ, chuỗi ngày học hiện tại và dài nhất, độ chính xác 30 ngày", "page:stats"),
            ("Có lịch học 12 tuần, dự báo lượt ôn 7 ngày tới, danh sách từ hay quên và 12 thành tích", ""),
            ("Có thể bật một ngày nghỉ mỗi tuần mà chuỗi ngày học vẫn giữ", ""),
        ],
    },
    Guide {
        cues: &["dong bo", "sao luu", "backup", "xuat ", "google drive", "csv", "anki"],
        title: "Đồng bộ, sao lưu và xuất dữ liệu",
        items: &[
            ("Đồng bộ Google Drive: Cài đặt, Đăng nhập Google, cho phép trong trình duyệt; dữ liệu nằm trong thư mục ẩn của app trên Drive của bạn", "page:settings"),
            ("Bấm Đồng bộ ngay để đồng bộ tức thì", "sync"),
            ("Xuất CSV (cho Excel, cho Anki) hoặc xuất toàn bộ sang JSON; file xuất không bao giờ có cài đặt và API key", ""),
            ("App tự sao lưu mỗi ngày vào thư mục backups, giữ lại 7 bản gần nhất", ""),
        ],
    },
    Guide {
        cues: &["nguon ai", "chon model", "api key", "gemini", "chatgpt", "codex", "deepseek", "qwen", "model tren may"],
        title: "Cách chọn nguồn AI",
        items: &[
            ("Chọn nguồn AI trong Cài đặt: model Qwen chạy trên máy (tải về, chạy offline), ChatGPT, ChatGPT qua Codex, hoặc Gemini và DeepSeek bằng API key", "page:settings"),
            ("Khi nguồn đang chọn lỗi hoặc hết hạn mức, app tự chuyển sang nguồn khác đã sẵn sàng và ghi rõ ai đã trả lời", ""),
            ("Trong khung chat này có thể chọn model ngay tại chỗ; lựa chọn áp dụng cho cả Hội thoại nhập vai", ""),
        ],
    },
    Guide {
        cues: &["giong doc", "goi giong", "kokoro", "supertonic", "piper", "toc do doc"],
        title: "Cách cài giọng đọc",
        items: &[
            ("Vào Cài đặt, tải một gói giọng: Supertonic 3 cho máy yếu và trung bình, Kokoro cho máy mạnh, Piper Xiao Ya cho tiếng Trung nhẹ", "page:settings"),
            ("Chỉnh tốc độ đọc trong Cài đặt; gói giọng đã tải xóa được để lấy lại dung lượng", ""),
            ("Chưa tải gói nào thì app dùng giọng của hệ điều hành", ""),
        ],
    },
];

/// The guide a how-to question asks for.
fn guide_of(f: &str) -> Option<usize> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if has(GUIDES[0].cues) {
        return Some(0);
    }
    let how = ["lam sao", "lam the nao", "lam cach nao", "cach nao", "huong dan", "o dau", "dung the nao", "chi minh", "chi toi", "chi cach", "how do i", "how to", "cach "];
    if !has(&how) {
        return None;
    }
    GUIDES.iter().position(|g| has(g.cues))
}

/// What a guide step's button runs.
fn guide_run(spec: &str) -> Option<Value> {
    let run = |tool: &str, args: Value| Some(json!({ "tool": tool, "args": args }));
    match spec {
        "" => None,
        "study" => run("start_study", json!({})),
        "packs" => run("open_packs", json!({})),
        "vocab_test" => run("open_vocab_test", json!({})),
        "sync" => run("sync_now", json!({})),
        page => run("open_page", json!({ "page": page.strip_prefix("page:").unwrap_or(page) })),
    }
}

/// The page a request names, if it asks to open one.
fn page_of(f: &str) -> Option<&'static str> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if !has(&["mo ", "vao ", "den ", "chuyen ", "di toi", "open", "go to"]) {
        return None;
    }
    let pages: [(&str, &[&str]); 8] = [
        ("packs", &["bo tu co san", "bo tu vung co san"]),
        ("talk", &["gia su noi"]),
        ("settings", &["cai dat"]),
        ("stats", &["thong ke"]),
        ("roadmap", &["lo trinh"]),
        ("home", &["hoc hom nay", "trang chu"]),
        ("practice", &["trang bai tap", "muc bai tap", "man hinh bai tap"]),
        ("words", &["trang tu vung", "danh sach tu", "muc tu vung", "mo tu vung", "vao tu vung"]),
    ];
    pages.iter().find(|(_, cues)| has(cues)).map(|p| p.0)
}

/// Whether a request is worded as a command to the app, which is when the planner is worth its extra call.
pub fn may_command(question: &str) -> bool {
    let f = ai::fold(question);
    ["mo ", "bat dau", "vao ", "dat ", "doi ", "chuyen ", "tao ", "xoa", "bo qua", "them ", "nhac", "tat ", "bat ", "cai dat", "lo trinh", "dong bo",
     "khoa ", "cho toi hoc", "cho minh hoc", "luyen", "thi ", "ke hoach", "hom nay", "kich hoat", "huy ", "gui ", "open", "start", "set ", "remind"]
        .iter().any(|c| f.contains(c))
}

/// The rules that need no model: every request the app can recognise by its wording.
fn control(q: &str, f: &str) -> Option<Intent> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if let Some(i) = guide_of(f) {
        return Some(Intent::Guide(i));
    }
    if has(&["xoa tu ", "delete word "]) {
        return word_of(q, f, &["xoa tu ", "delete word "]).map(Intent::DeleteWord);
    }
    if has(&["bo qua tu ", "skip word "]) {
        return word_of(q, f, &["bo qua tu ", "skip word "]).map(Intent::SkipWord);
    }
    remind_of(f).or_else(|| setting_of(q, f)).or_else(|| roadmap_of(q, f)).or_else(|| {
        let today = has(&["hom nay", "bua nay"]) && has(&["hoc gi", "nen hoc", "ke hoach", "lam gi", "on gi"]) && !has(&["da hoc"]);
        (today || has(&["lich hoc hom nay"])).then_some(Intent::Today)
    })
}

/// Opening a page or starting a session, which wait until the practice and exam wordings have had their turn.
fn navigation(q: &str, f: &str) -> Option<Intent> {
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    if let Some(page) = page_of(f) {
        return Some(Intent::OpenPage(page));
    }
    let study = has(&["bat dau hoc", "hoc ngay", "hoc di", "vao hoc", "hoc bai", "bat dau on", "on bai", "mo buoi hoc", "hoc thoi"]);
    if study && !has(&["tu dau", "tieng anh", "tieng trung", "lam sao", "nhu the nao"]) {
        let topic = after(q, f, &["chu de "]).unwrap_or_default();
        return Some(Intent::StartStudy(topic));
    }
    None
}

/// The feature `question` asks for, or `None` for a question to talk about.
pub fn route(question: &str) -> Option<Intent> {
    let q = question.trim();
    let f = ai::fold(q);
    let has = |cues: &[&str]| cues.iter().any(|c| f.contains(c));
    // A long text to pick words from, or a list of words to add.
    if q.chars().count() > 80 && has(&["chon tu", "lay tu", "hoc tu trong", "tim tu trong", "loc tu"]) {
        let text = q.split_once(':').map_or(q, |(_, t)| t).trim();
        return Some(Intent::Extract(text.to_owned()));
    }
    if let Some(intent) = control(q, &f) {
        return Some(intent);
    }
    if has(&["nhap tu", "nhap cac tu", "nhap nhung tu", "import", "them cac tu", "them nhung tu", "them tu"]) {
        let list = match q.split_once(':') {
            Some((_, rest)) => rest.to_owned(),
            None => after(q, &f, &["them tu ", "nhap tu ", "import "]).unwrap_or_default(),
        };
        let words: Vec<String> = list.split([',', ';', '\n', '，', '、']).map(|w| w.trim().trim_matches('.').to_owned())
            .filter(|w| !w.is_empty() && w.chars().count() <= 40).take(50).collect();
        if !words.is_empty() {
            return Some(Intent::Import(words));
        }
    }
    if has(&["thi thu", "bai thi", "de thi", "kiem tra trinh do", "mock test"]) || (has(&["ielts", "toeic", "vstep", "hsk"]) && has(&["thi", "kiem tra"])) {
        return Some(Intent::Exam);
    }
    if let Some(intent) = navigation(q, &f) {
        return Some(intent);
    }
    let exercise = EXERCISES.iter().find(|(name, ..)| f.contains(name)).map(|e| e.1);
    if has(&["bai tap", "luyen tap"]) || (exercise.is_some() && has(&["tao", "luyen", "choi", "lam", "mo", "bat dau"])) {
        return Some(Intent::Practice { exercise: exercise.unwrap_or(""), source: source_of(q, &f) });
    }
    if q.to_lowercase().contains("đố") || has(&["do toi", "do minh", "quiz", "kiem tra toi", "kiem tra tu", "on tap", "on lai"]) {
        return Some(Intent::Quiz { count: number(&f).unwrap_or(5).clamp(3, 10), source: source_of(q, &f) });
    }
    if has(&["goi y", "de xuat", "suggest", "liet ke", "cho toi", "cho minh"]) && says_words(q, &f) {
        return Some(Intent::Suggest { count: number(&f).unwrap_or(BATCH).clamp(1, 200), topic: topic_of(q, &f) });
    }
    if has(&["tien do", "thong ke", "hom nay toi hoc", "hom nay minh hoc", "chuoi ngay", "streak", "da hoc duoc", "hoc duoc bao nhieu"]) {
        return Some(Intent::Progress);
    }
    if has(&["lo trinh", "ke hoach hoc", "muc tieu cua toi", "muc tieu cua minh"]) {
        return Some(Intent::Roadmap);
    }
    None
}

/// A small random sequence for quizzes, seeded by the time: shuffling needs nothing better.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// A word as a line of a `words` card.
fn word_line(word: &str, meaning: &str, reading: &str, level: &str, in_list: bool) -> Value {
    json!({ "word": word, "meaning": meaning, "reading": reading, "level": level, "in_list": in_list })
}

/// The reading of a word: pinyin for Chinese, IPA for English.
fn reading(word: &str) -> String {
    if pron::is_chinese(word) { pron::pinyin(word) } else { pron::ipa(word) }
}

/// A `words` card of suggested words, and what is left to ask for.
pub fn words_card(title: &str, topic: &str, items: Vec<Value>, left: usize) -> Value {
    let more = if left > 0 { json!({ "topic": topic, "count": left }) } else { Value::Null };
    json!({ "type": "words", "title": title, "topic": topic, "words": items, "more": more })
}

/// Suggests `count` words on `topic` (at most a card's worth), none of the list or of `shown`.
pub fn suggest(ai: &Ai, db: &Connection, lang: &str, topic: &str, count: usize, shown: &[String]) -> Result<Act, String> {
    let words = db::list(db, "").map_err(|e| e.to_string())?;
    let mut known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
    known.extend(shown.iter().cloned());
    let learned: Vec<String> = words.iter().filter(|w| w.learned_at > 0 && w.lang == lang).map(|w| w.word.clone()).collect();
    let s = ai.suggest(topic, &learned, &known, lang == "zh", count.min(BATCH))?;
    let items: Vec<Value> = s.items.iter().map(|w| word_line(&w.word, &w.meaning, &reading(&w.word), "", false)).collect();
    let left = count.saturating_sub(items.len());
    let about = if topic.is_empty() { String::new() } else { format!(" chủ đề {topic}") };
    let text = format!("Đây là {} từ{about}. Chọn những từ bạn muốn học rồi bấm Thêm, AI sẽ giải thích từng từ.", items.len());
    Ok(Act { text, cards: vec![words_card(&format!("Từ gợi ý{about}"), topic, items, left)], by: Some(s.by) })
}

/// A quiz of `count` of the learner's own words from `source`, built here: choices of meanings or words, and the
/// word missing from its example.
fn quiz(words: &[db::Word], count: usize, source: &str, rng: &mut Rng) -> Option<Value> {
    let usable: Vec<&db::Word> = words.iter().filter(|w| !w.skipped && !w.meaning.is_empty()).collect();
    if usable.len() < 4 {
        return None;
    }
    let mut pool: Vec<&db::Word> = match source {
        "hard" => usable.iter().copied().filter(|w| w.lapses > 0 || w.difficulty >= 7.0).collect(),
        "recent" => {
            let mut v = usable.clone();
            v.sort_by_key(|w| std::cmp::Reverse(w.created));
            v.truncate(count * 2);
            v
        }
        s if s.starts_with("topic:") => usable.iter().copied().filter(|w| w.topic == s[6..]).collect(),
        _ => usable.clone(),
    };
    if pool.is_empty() {
        pool = usable.clone();
    }
    rng.shuffle(&mut pool);
    pool.truncate(count);
    let cut = |t: &str| t.chars().take(60).collect::<String>();
    let items: Vec<Value> = pool.iter().enumerate().map(|(i, w)| {
        let blank = (i % 3 == 2).then(|| blanked(&w.example, &w.word)).flatten();
        if let Some(prompt) = blank {
            return json!({ "id": w.id, "word": w.word, "meaning": cut(&w.meaning), "learned": w.learned_at > 0, "kind": "fill",
                          "prompt": prompt, "hint": cut(&w.meaning), "answer": w.word });
        }
        let to_meaning = i % 2 == 0;
        let base = json!({ "id": w.id, "word": w.word, "meaning": cut(&w.meaning), "learned": w.learned_at > 0 });
        let mut others: Vec<&db::Word> = usable.iter().copied().filter(|o| o.id != w.id && cut(&o.meaning) != cut(&w.meaning)).collect();
        rng.shuffle(&mut others);
        let pick = |o: &db::Word| if to_meaning { cut(&o.meaning) } else { o.word.clone() };
        let answer = pick(w);
        let mut options: Vec<String> = vec![answer.clone()];
        for o in others {
            if options.len() == 4 {
                break;
            }
            let option = pick(o);
            if !options.contains(&option) {
                options.push(option);
            }
        }
        rng.shuffle(&mut options);
        let prompt = if to_meaning { format!("\"{}\" nghĩa là gì?", w.word) } else { format!("Từ nào có nghĩa \"{}\"?", cut(&w.meaning)) };
        let mut item = base;
        item.as_object_mut().unwrap().extend([("kind".into(), json!("choice")), ("prompt".into(), json!(prompt)),
                                              ("options".into(), json!(options)), ("answer".into(), json!(answer))]);
        item
    }).collect();
    Some(json!({ "type": "quiz", "title": format!("Đố {} từ", items.len()), "items": items }))
}

/// `example` with `word` hidden, when it holds the word.
fn blanked(example: &str, word: &str) -> Option<String> {
    let at = example.to_lowercase().find(&word.to_lowercase())?;
    Some(format!("{}____{}", &example[..at], &example[at + word.len()..]))
}

/// A button of an `actions` card.
fn button(label: &str, run: Value) -> Value {
    json!({ "label": label, "run": run })
}

fn actions(buttons: Vec<Value>) -> Value {
    json!({ "type": "actions", "buttons": buttons })
}

/// Does what `intent` asks; `day_start` is the start of the learner's day.
pub fn act(intent: &Intent, ai: &Ai, db: &Connection, lang: &str, now: i64, day_start: i64) -> Result<Act, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let words: Vec<db::Word> = db::list(db, "").map_err(e)?.into_iter().filter(|w| w.lang == lang).collect();
    let plain = |text: String, cards: Vec<Value>| Ok(Act { text, cards, by: None });
    match intent {
        Intent::Suggest { count, topic } => suggest(ai, db, lang, topic, *count, &[]),
        Intent::Quiz { count, source } => match quiz(&words, *count, source, &mut Rng(now as u64 | 1)) {
            Some(card) => plain("Làm thử nhé, chọn đáp án hoặc gõ từ còn thiếu.".into(), vec![card]),
            None => plain("Cần ít nhất 4 từ có nghĩa để đố. Hãy thêm từ trước, ví dụ từ một bộ từ có sẵn.".into(),
                          vec![actions(vec![button("Mở bộ từ có sẵn", json!({ "kind": "packs" }))])]),
        },
        Intent::Practice { exercise, source } => {
            let named: Vec<&(&str, &str, &str)> = if exercise.is_empty() {
                EXERCISES.iter().filter(|e| ["smart", "match", "fill", "choice"].contains(&e.1)).collect()
            } else {
                EXERCISES.iter().filter(|e| e.1 == *exercise).take(1).collect()
            };
            let buttons = named.iter().map(|(_, key, name)| button(name, json!({ "kind": "practice", "exercise": key, "source": source }))).collect();
            let from = match source.as_str() {
                "hard" => " với các từ hay quên".to_owned(),
                s if s.starts_with("topic:") => format!(" với chủ đề {}", &s[6..]),
                _ => String::new(),
            };
            plain(format!("Bấm để bắt đầu{from}."), vec![actions(buttons)])
        }
        Intent::Exam => {
            let active = roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|r| r.active);
            let text = match &active {
                Some(r) => format!("Lộ trình của bạn là \"{}\". Bài thi thử theo đúng dạng đề của mục tiêu này, đủ bốn kỹ năng.", r.purpose),
                None => "Bạn chưa có lộ trình. Tạo một lộ trình có mục tiêu như IELTS, TOEIC, VSTEP hay HSK để có bài thi thử theo đúng dạng đề; còn bây giờ có thể kiểm tra vốn từ.".into(),
            };
            let mut buttons = vec![button("Kiểm tra vốn từ", json!({ "kind": "vocab_test" }))];
            buttons.insert(0, match &active {
                Some(r) => button("Làm bài thi thử", json!({ "kind": "exam", "roadmap": r.uid, "stage": -1 })),
                None => button("Tạo lộ trình", json!({ "kind": "page", "page": "roadmap" })),
            });
            plain(text, vec![actions(buttons)])
        }
        Intent::Import(list) => {
            let items: Vec<Value> = list.iter().map(|w| match words.iter().find(|x| x.word.eq_ignore_ascii_case(w)) {
                Some(x) => word_line(&x.word, &x.meaning, &x.ipa, &x.level, true),
                None => word_line(w, "", &reading(w), "", false),
            }).collect();
            let have = items.iter().filter(|i| i["in_list"] == true).count();
            let text = format!("{} từ, {have} từ đã có trong danh sách. Chọn rồi bấm Thêm, AI sẽ giải thích từng từ mới.", items.len());
            plain(text, vec![words_card("Từ để nhập", "", items, 0)])
        }
        Intent::Extract(text) => {
            let known: Vec<String> = words.iter().map(|w| w.word.clone()).collect();
            let s = ai.extract(text, &known)?;
            let items: Vec<Value> = s.items.iter().map(|w| word_line(&w.word, &w.meaning, &reading(&w.word), "", false)).collect();
            let line = format!("Mình chọn được {} từ đáng học trong đoạn này.", items.len());
            Ok(Act { text: line, cards: vec![words_card("Từ trong đoạn văn", "", items, 0)], by: Some(s.by) })
        }
        Intent::Progress => {
            let s = db::stats(db, now, day_start, lang).map_err(e)?;
            let (days, xp) = db::activity(db, 60, lang).map_err(e)?;
            let streak = days.iter().take_while(|d| d.xp > 0).count();
            let mut forgotten: Vec<&db::Word> = words.iter().filter(|w| w.lapses > 0 && !w.skipped).collect();
            forgotten.sort_by_key(|w| std::cmp::Reverse(w.lapses));
            let hard: Vec<&str> = forgotten.iter().take(3).map(|w| w.word.as_str()).collect();
            let items = vec![
                json!({ "label": "Số từ", "value": s.total.to_string() }),
                json!({ "label": "Đã thuộc", "value": s.learned.to_string() }),
                json!({ "label": "Cần ôn hôm nay", "value": s.due.to_string() }),
                json!({ "label": "Lượt ôn hôm nay", "value": s.reviews_today.to_string() }),
                json!({ "label": "Chuỗi ngày học", "value": format!("{streak} ngày") }),
                json!({ "label": "Tổng XP", "value": xp.to_string() }),
            ];
            let note = if hard.is_empty() { "Chưa có từ nào hay quên, giữ vững nhé.".to_owned() } else { format!("Từ hay quên nhất: {}.", hard.join(", ")) };
            let text = if s.due > 0 { format!("Hôm nay bạn còn {} từ cần ôn.", s.due) } else { "Hôm nay bạn đã ôn xong các từ đến hạn.".to_owned() };
            plain(text, vec![json!({ "type": "stats", "items": items, "note": note }),
                             actions(vec![button("Học ngay", json!({ "kind": "page", "page": "home" })), button("Xem thống kê", json!({ "kind": "page", "page": "stats" }))])])
        }
        Intent::Roadmap => match roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|r| r.active) {
            Some(r) => {
                let current = r.steps.iter().position(|s| !s.done).unwrap_or(r.steps.len().saturating_sub(1));
                let stage = r.steps.get(current).map(|s| format!(" Bạn đang ở chặng {}/{}: {}.", current + 1, r.steps.len(), s.title)).unwrap_or_default();
                plain(format!("Mục tiêu: {}, hạn {}.{stage}", r.purpose, r.deadline), vec![actions(vec![button("Mở lộ trình", json!({ "kind": "page", "page": "roadmap" }))])])
            }
            None => plain("Bạn chưa có lộ trình. Đặt mục tiêu và AI sẽ chia thành từng chặng.".into(),
                          vec![actions(vec![button("Tạo lộ trình", json!({ "kind": "page", "page": "roadmap" }))])]),
        },
        Intent::Remind { at, every_hours, off } => {
            if *off {
                return plain("Mình tắt nhắc học nhé, bạn đồng ý không?".into(),
                             vec![do_card("Tắt nhắc học", vec![call("set_reminder", json!({ "at": "", "every_hours": 0 }), "Tắt nhắc học", true)])]);
            }
            let mut args = serde_json::Map::new();
            let mut parts = Vec::new();
            if !at.is_empty() {
                args.insert("at".into(), json!(at));
                parts.push(format!("lúc {at} mỗi ngày"));
            }
            if let Some(n) = every_hours {
                args.insert("every_hours".into(), json!(n));
                parts.push(format!("mỗi {n} giờ"));
            }
            if args.is_empty() {
                return plain("Bạn muốn được nhắc học lúc mấy giờ? Ví dụ: \"Nhắc tôi học lúc 20:00\" hoặc \"Nhắc tôi mỗi 2 giờ\".".into(), Vec::new());
            }
            let label = format!("Nhắc học {}", parts.join(", "));
            plain("Mình đặt nhắc học như sau, bạn đồng ý không?".into(), vec![do_card("Đặt nhắc học", vec![call("set_reminder", Value::Object(args), &label, true)])])
        }
        Intent::Setting { key, value, label } => {
            if !crate::valid_setting(key, value) {
                return plain("Giá trị này không hợp lệ cho cài đặt đó.".into(), Vec::new());
            }
            plain("Mình đổi cài đặt này nhé, bạn đồng ý không?".into(),
                  vec![do_card("Đổi cài đặt", vec![call("set_setting", json!({ "key": key, "value": value }), label, true)])])
        }
        Intent::Switch(course) => {
            if *course == lang {
                return plain("Bạn đang ở khóa này rồi.".into(), Vec::new());
            }
            let name = if *course == "zh" { "tiếng Trung" } else { "tiếng Anh" };
            let label = format!("Chuyển sang khóa {name}");
            plain("Mình chuyển khóa nhé, bạn đồng ý không?".into(), vec![do_card("Đổi khóa học", vec![call("switch_course", json!({ "course": course }), &label, true)])])
        }
        Intent::StartStudy(topic) => {
            let args = if topic.is_empty() { json!({}) } else { json!({ "topic": topic }) };
            let label = if topic.is_empty() { "Bắt đầu học".to_owned() } else { format!("Bắt đầu học chủ đề {topic}") };
            plain("Mình mở buổi học cho bạn.".into(), vec![do_card("Bắt đầu học", vec![call("start_study", args, &label, false)])])
        }
        Intent::OpenPage("packs") => plain("Mình mở bộ từ có sẵn cho bạn.".into(), vec![do_card("Bộ từ có sẵn", vec![call("open_packs", json!({}), "Mở bộ từ có sẵn", false)])]),
        Intent::OpenPage(page) => {
            let label = format!("Mở {}", page_name(page));
            plain(format!("Mình mở {} cho bạn.", page_name(page)), vec![do_card("Mở trang", vec![call("open_page", json!({ "page": page }), &label, false)])])
        }
        Intent::CreateRoadmap { purpose, days, minutes } => {
            let (Some(days), false) = (days, purpose.is_empty()) else {
                return plain("Để lập lộ trình mình cần biết mục tiêu và thời hạn. Ví dụ: \"Tạo lộ trình IELTS 6.5 trong 3 tháng, mỗi ngày 30 phút\".".into(), Vec::new());
            };
            let today = crate::clock::now().map(|l| l.date()).unwrap_or_default();
            let Some(deadline) = add_days(&today, (*days).clamp(7, 1100)) else {
                return plain("Mình chưa biết hôm nay là ngày mấy, bạn hãy lập lộ trình ở trang Lộ trình.".into(), Vec::new());
            };
            let minutes = minutes.unwrap_or(30).clamp(5, 600);
            let label = format!("Tạo lộ trình {purpose} đến {deadline}, {minutes} phút mỗi ngày");
            plain("Mình lập lộ trình này nhé, bạn đồng ý không?".into(),
                  vec![do_card("Tạo lộ trình", vec![call("roadmap_create", json!({ "purpose": purpose, "deadline": deadline, "minutes": minutes }), &label, true)])])
        }
        Intent::Today => {
            let s = db::stats(db, now, day_start, lang).map_err(e)?;
            let step = |text: &str, tool: &str, args: Value| json!({ "text": text, "run": { "tool": tool, "args": args } });
            let mut steps = Vec::new();
            if s.due + s.new_today > 0 {
                let text = match (s.due, s.new_today) {
                    (d, 0) => format!("Ôn {d} từ đến hạn"),
                    (0, n) => format!("Học {n} từ mới"),
                    (d, n) => format!("Ôn {d} từ đến hạn và học {n} từ mới"),
                };
                steps.push(step(&text, "start_study", json!({})));
            }
            if let Some(r) = roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|r| r.active) {
                if let Some((i, st)) = r.steps.iter().enumerate().find(|(_, st)| !st.done) {
                    let exercise = st.exercises.iter().map(String::as_str).find(|x| EXERCISES.iter().any(|e| e.1 == *x)).unwrap_or("smart");
                    let source = st.topics.first().map_or("all".to_owned(), |t| format!("topic:{t}"));
                    steps.push(step(&format!("Chặng {}/{} của lộ trình: {}", i + 1, r.steps.len(), st.title), "start_practice", json!({ "exercise": exercise, "source": source })));
                }
            }
            let hard = words.iter().filter(|w| !w.skipped && !w.meaning.is_empty() && w.lapses > 0).count();
            if hard >= 4 {
                steps.push(step("Đố nhanh các từ hay quên", "start_practice", json!({ "exercise": "choice", "source": "hard" })));
            }
            if steps.is_empty() {
                if s.total == 0 {
                    steps.push(step("Thêm những từ đầu tiên từ bộ từ có sẵn", "open_packs", json!({})));
                } else {
                    steps.push(step("Hôm nay không còn từ đến hạn, luyện thêm một dạng bài bạn thích", "open_page", json!({ "page": "practice" })));
                }
            }
            let load = s.due + s.new_today;
            let note = if load > 0 { format!("Khoảng {} phút cho buổi học chính.", (load / 3).max(1)) } else { "Bạn đã xong phần bắt buộc hôm nay.".to_owned() };
            plain("Đây là kế hoạch hôm nay của bạn, bấm Bắt đầu ở từng bước.".into(), vec![json!({ "type": "plan", "title": "Kế hoạch hôm nay", "steps": steps, "note": note })])
        }
        Intent::Guide(i) => {
            let g = &GUIDES[*i];
            let items: Vec<Value> = g.items.iter().map(|(text, run)| match guide_run(run) {
                Some(run) => json!({ "text": text, "run": run }),
                None => json!({ "text": text }),
            }).collect();
            plain("Đây là cách làm, bấm vào từng bước để mở đúng chỗ.".into(), vec![json!({ "type": "guide", "title": g.title, "items": items })])
        }
        Intent::SkipWord(word) | Intent::DeleteWord(word) => {
            let skip = matches!(intent, Intent::SkipWord(_));
            let Some(w) = words.iter().find(|w| w.word.eq_ignore_ascii_case(word)) else {
                return plain(format!("Mình không thấy từ \"{word}\" trong danh sách của khóa này."), Vec::new());
            };
            let (tool, label, title) = if skip {
                ("skip_word", format!("Bỏ qua từ {}", w.word), "Bỏ qua từ")
            } else {
                ("delete_word", format!("Xóa từ {}", w.word), "Xóa từ")
            };
            plain(format!("{} không? Bạn đồng ý thì bấm nút bên dưới.", label), vec![do_card(title, vec![call(tool, json!({ "word": w.word }), &label, true)])])
        }
    }
}

/// The date `days` after `date` (`YYYY-MM-DD`), by the days-from-civil algorithm run backwards.
fn add_days(date: &str, days: i64) -> Option<String> {
    let z = roadmap::day_number(date)? + days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The name of a page of the contract as the app shows it.
fn page_name(page: &str) -> &'static str {
    match page {
        "home" => "Học hôm nay",
        "words" => "Từ vựng",
        "practice" => "Bài tập",
        "roadmap" => "Lộ trình",
        "stats" => "Thống kê",
        "settings" => "Cài đặt",
        _ => "Gia sư nói",
    }
}

/// A tool call the chat runs for the learner.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub tool: String,
    pub args: Value,
}

/// A line of a `do` card.
fn call(tool: &str, args: Value, label: &str, confirm: bool) -> Value {
    json!({ "tool": tool, "args": args, "label": label, "confirm": confirm })
}

/// The `do` card: calls the chat runs in order, each with a button of its own.
fn do_card(title: &str, calls: Vec<Value>) -> Value {
    json!({ "type": "do", "title": title, "calls": calls })
}

/// The settings the chat may change, with the tools that write or delete asking for the learner's yes.
const SETTING_KEYS: [&str; 11] = ["new_per_day", "daily_goal", "theme", "palette", "accent", "speech_rate", "desired_retention", "streak_freeze", "easy_days", "course", "tutor_ai"];
const CONFIRMED: [&str; 9] = ["add_words", "skip_word", "delete_word", "set_reminder", "set_setting", "switch_course", "roadmap_create", "roadmap_activate", "roadmap_delete"];

/// A text argument: present, on one line, at most `max` characters.
fn text_arg(args: &Value, key: &str, max: usize) -> Option<String> {
    let t = args.get(key)?.as_str()?.trim();
    (!t.is_empty() && t.chars().count() <= max && !t.chars().any(char::is_control)).then(|| t.to_owned())
}

/// A whole-number argument given as a number or as digits.
fn int_arg(args: &Value, key: &str) -> Option<i64> {
    match args.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A setting value as the text the settings keep.
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

/// Checks one call the planner asked for against the catalog; the arguments it keeps are the ones the tool knows, and a
/// call that breaks a rule is dropped.
pub fn check_call(raw: &Value) -> Option<Call> {
    let tool = raw.get("tool")?.as_str()?;
    let empty = json!({});
    let a = raw.get("args").filter(|a| a.is_object()).unwrap_or(&empty);
    let uid = |a: &Value| text_arg(a, "uid", 16).filter(|u| u.len() == 16 && u.bytes().all(|b| b.is_ascii_hexdigit()));
    let args = match tool {
        "open_page" => {
            let page = text_arg(a, "page", 10).filter(|p| ["home", "words", "practice", "roadmap", "stats", "settings", "talk"].contains(&p.as_str()))?;
            json!({ "page": page })
        }
        "start_study" => match text_arg(a, "topic", 60) {
            Some(topic) => json!({ "topic": topic }),
            None => json!({}),
        },
        "start_practice" => {
            let exercise = text_arg(a, "exercise", 10).filter(|x| ["flash", "fill", "choice", "dict", "speak", "write", "match", "order", "shadow", "smart", "tone", "hanzi"].contains(&x.as_str()))?;
            let source = text_arg(a, "source", 50).unwrap_or_else(|| "all".into());
            let ok = ["all", "hard", "recent"].contains(&source.as_str()) || source.strip_prefix("topic:").is_some_and(|t| !t.trim().is_empty() && t.chars().count() <= 40);
            if !ok {
                return None;
            }
            json!({ "exercise": exercise, "source": source })
        }
        "open_exam" => match int_arg(a, "stage") {
            Some(n) if (-1..=30).contains(&n) => json!({ "stage": n }),
            Some(_) => return None,
            None => json!({}),
        },
        "open_vocab_test" | "open_packs" | "sync_now" => json!({}),
        "open_word" | "skip_word" | "delete_word" => json!({ "word": text_arg(a, "word", 40)? }),
        "add_words" => {
            let words: Vec<String> = a.get("words")?.as_array()?.iter()
                .filter_map(|w| w.as_str()).map(|w| w.trim().to_owned()).filter(|w| !w.is_empty() && w.chars().count() <= 40 && !w.chars().any(char::is_control)).take(50).collect();
            if words.is_empty() {
                return None;
            }
            match text_arg(a, "topic", 40) {
                Some(topic) => json!({ "words": words, "topic": topic }),
                None => json!({ "words": words }),
            }
        }
        "set_reminder" => {
            let mut out = serde_json::Map::new();
            for key in ["at", "quiet_from", "quiet_to"] {
                if let Some(v) = a.get(key) {
                    let t = v.as_str()?.trim();
                    if !(t.is_empty() || crate::clock::parse_hm(t).is_some()) {
                        return None;
                    }
                    out.insert(key.into(), json!(t));
                }
            }
            if a.get("every_hours").is_some() {
                let n = int_arg(a, "every_hours").filter(|n| (0..=24).contains(n))?;
                out.insert("every_hours".into(), json!(n));
            }
            if out.is_empty() {
                return None;
            }
            Value::Object(out)
        }
        "set_setting" => {
            let key = text_arg(a, "key", 30).filter(|k| SETTING_KEYS.contains(&k.as_str()))?;
            let value = value_text(a.get("value")?)?;
            if !crate::valid_setting(&key, &value) {
                return None;
            }
            json!({ "key": key, "value": value })
        }
        "switch_course" => json!({ "course": text_arg(a, "course", 2).filter(|c| c == "en" || c == "zh")? }),
        "roadmap_create" => {
            let deadline = text_arg(a, "deadline", 10).filter(|d| roadmap::day_number(d).is_some())?;
            let mut out = json!({ "purpose": text_arg(a, "purpose", 80)?, "deadline": deadline, "minutes": int_arg(a, "minutes").filter(|n| (5..=600).contains(n))? });
            if let Some(level) = text_arg(a, "level", 10) {
                out["level"] = json!(level);
            }
            if let Some(ai) = a.get("ai").and_then(Value::as_bool) {
                out["ai"] = json!(ai);
            }
            out
        }
        "roadmap_activate" | "roadmap_delete" => json!({ "uid": uid(a)? }),
        _ => return None,
    };
    Some(Call { tool: tool.to_owned(), args })
}

/// The valid calls of a planner's list, at most six.
pub fn check_calls(raw: &[Value]) -> Vec<Call> {
    raw.iter().filter_map(check_call).take(6).collect()
}

/// What the button of a planned call says.
fn label_of(c: &Call) -> String {
    let s = |k: &str| c.args[k].as_str().unwrap_or_default().to_owned();
    match c.tool.as_str() {
        "open_page" => format!("Mở {}", page_name(&s("page"))),
        "start_study" if c.args.get("topic").is_some() => format!("Bắt đầu học chủ đề {}", s("topic")),
        "start_study" => "Bắt đầu học".into(),
        "start_practice" => {
            let name = EXERCISES.iter().find(|e| e.1 == s("exercise")).map_or("bài tập", |e| e.2);
            format!("Làm bài {name}")
        }
        "open_exam" => "Làm bài thi thử".into(),
        "open_vocab_test" => "Kiểm tra vốn từ".into(),
        "open_packs" => "Mở bộ từ có sẵn".into(),
        "open_word" => format!("Mở từ {}", s("word")),
        "add_words" => format!("Thêm {} từ", c.args["words"].as_array().map_or(0, Vec::len)),
        "skip_word" => format!("Bỏ qua từ {}", s("word")),
        "delete_word" => format!("Xóa từ {}", s("word")),
        "set_reminder" => {
            let mut parts = Vec::new();
            if let Some(at) = c.args["at"].as_str() {
                parts.push(if at.is_empty() { "tắt giờ cố định".to_owned() } else { format!("lúc {at}") });
            }
            if let Some(n) = c.args["every_hours"].as_i64() {
                parts.push(if n == 0 { "tắt nhắc theo giờ".to_owned() } else { format!("mỗi {n} giờ") });
            }
            if let (Some(f), Some(t)) = (c.args["quiet_from"].as_str(), c.args["quiet_to"].as_str()) {
                parts.push(format!("yên lặng {f} đến {t}"));
            }
            format!("Nhắc học {}", parts.join(", "))
        }
        "set_setting" => format!("Đặt {} = {}", s("key"), s("value")),
        "switch_course" => format!("Chuyển sang khóa {}", if s("course") == "zh" { "tiếng Trung" } else { "tiếng Anh" }),
        "roadmap_create" => format!("Tạo lộ trình {} đến {}", s("purpose"), s("deadline")),
        "roadmap_activate" => "Dùng lộ trình này".into(),
        "roadmap_delete" => "Xóa lộ trình này".into(),
        "sync_now" => "Đồng bộ ngay".into(),
        _ => c.tool.clone(),
    }
}

/// The `do` card of planned calls.
fn planned_card(calls: &[Call]) -> Value {
    let lines = calls.iter().map(|c| call(&c.tool, c.args.clone(), &label_of(c), CONFIRMED.contains(&c.tool.as_str()))).collect();
    do_card("Việc mình làm giúp bạn", lines)
}

/// What the planner is told of the learner's app: short, because it is read on every command.
pub fn state_summary(db: &Connection, lang: &str, now: i64, day_start: i64) -> String {
    let set = |key: &str| db::setting(db, &db::scoped(key, lang)).ok().flatten().filter(|v| !v.is_empty());
    let mut out = vec![format!("Today: {}", crate::clock::now().map(|l| l.date()).unwrap_or_default()), format!("Course: {lang}")];
    if let Ok(s) = db::stats(db, now, day_start, lang) {
        out.push(format!("Words: {} (learned {}), due now: {}, new words left today: {}", s.total, s.learned, s.due, s.new_today));
    }
    out.push(format!("Settings: new_per_day={}, daily_goal={}, remind_at={}, remind_hours={}, quiet={}-{}",
                     set("new_per_day").unwrap_or("10".into()), set("daily_goal").unwrap_or("50".into()), set("remind_at").unwrap_or("none".into()),
                     set("remind_hours").unwrap_or("4".into()), set("quiet_from").unwrap_or("none".into()), set("quiet_to").unwrap_or("none".into())));
    if let Ok(list) = roadmap::roadmaps(db, lang) {
        for r in list.iter().take(3) {
            let step = r.steps.iter().position(|s| !s.done).map(|i| format!(", step {}/{}: {}", i + 1, r.steps.len(), r.steps[i].title)).unwrap_or_default();
            out.push(format!("Roadmap uid={} \"{}\" until {}{}{}", r.uid, r.purpose, r.deadline, if r.active { " (active)" } else { "" }, if r.active { step } else { String::new() }));
        }
    }
    if let Ok(topics) = roadmap::topics(db, lang) {
        let names: Vec<String> = topics.iter().take(12).map(|t| t.name.clone()).collect();
        if !names.is_empty() {
            out.push(format!("Topics: {}", names.join(", ")));
        }
    }
    out.push("Pages: home (Học hôm nay), words (Từ vựng), practice (Bài tập), roadmap (Lộ trình), stats (Thống kê), settings (Cài đặt), talk (Gia sư nói)".into());
    out.join("\n")
}

/// The planner's answer to a request the rules did not recognise: a `do` card of its valid calls, or `None` when there
/// are none and the request is for the tutor to talk about.
pub fn plan(ai: &Ai, db: &Connection, lang: &str, question: &str, history: &[db::Message], now: i64, day_start: i64) -> Result<Option<Act>, String> {
    let state = state_summary(db, lang, now, day_start);
    let Some(p) = ai.plan(question, &state, history)? else { return Ok(None) };
    if p.calls.is_empty() {
        return Ok(None);
    }
    let text = if p.say.is_empty() { "Mình làm giúp bạn nhé.".to_owned() } else { p.say };
    Ok(Some(Act { text, cards: vec![planned_card(&p.calls)], by: Some(p.by) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_recognised_however_they_are_typed() {
        let suggest = |count, topic: &str| Some(Intent::Suggest { count, topic: topic.into() });
        assert_eq!(route("Gợi ý 100 từ chủ đề công nghệ nhé"), suggest(100, "công nghệ"));
        assert_eq!(route("goi y 20 tu cong nghe"), suggest(20, "cong nghe"));
        assert_eq!(route("Đề xuất từ vựng về du lịch"), suggest(10, "du lịch"));
        assert_eq!(route("cho tôi 15 từ vựng IELTS"), suggest(15, "IELTS"));
        assert_eq!(route("Đố tôi 5 từ hay quên"), Some(Intent::Quiz { count: 5, source: "hard".into() }));
        assert_eq!(route("đố mình chủ đề Du lịch"), Some(Intent::Quiz { count: 5, source: "topic:Du lịch".into() }));
        assert_eq!(route("Tạo bài tập nối từ"), Some(Intent::Practice { exercise: "match", source: "all".into() }));
        assert_eq!(route("luyện nói đuổi với từ hay quên"), Some(Intent::Practice { exercise: "shadow", source: "hard".into() }));
        assert_eq!(route("tạo bài tập"), Some(Intent::Practice { exercise: "", source: "all".into() }));
        assert_eq!(route("Cho mình làm bài thi thử IELTS"), Some(Intent::Exam));
        assert_eq!(route("Nhập các từ: resilient, cope with, abundant"), Some(Intent::Import(vec!["resilient".into(), "cope with".into(), "abundant".into()])));
        assert_eq!(route("thêm từ serendipity"), Some(Intent::Import(vec!["serendipity".into()])));
        assert_eq!(route("Tiến độ của tôi"), Some(Intent::Progress));
        assert_eq!(route("Lộ trình của mình thế nào?"), Some(Intent::Roadmap));
        let long = format!("Chọn từ đáng học trong đoạn này: {}", "Technology changes how we work and live every single day. ".repeat(2));
        assert!(matches!(route(&long), Some(Intent::Extract(t)) if t.starts_with("Technology")));
        // Questions to talk about stay with the tutor.
        for q in ["deadline nghĩa là gì?", "Dịch sang tiếng Anh: Tôi rất vui được gặp bạn.", "Viết email xin nghỉ một ngày",
                  "Khác nhau giữa affect và effect?", "IELTS writing task 2 là gì?", "Làm sao để tự học tiếng Anh?", "how do I say xin chào"] {
            assert_eq!(route(q), None, "{q}");
        }
    }

    /// `cargo test --release qwen_suggests -- --ignored --nocapture` with the downloaded models.
    #[test]
    #[ignore = "needs the downloaded Qwen models"]
    fn qwen_suggests_on_a_topic() {
        let dir = crate::models_only();
        let db = db::open(std::path::Path::new(":memory:")).unwrap();
        for local_model in ["llm", "llm15"] {
            let settings = ai::Settings { provider: "local".into(), local_model: local_model.into(), ..ai::Settings::default() };
            let ai = Ai::new(&settings, &dir);
            let intent = route("Gợi ý 100 từ chủ đề công nghệ nhé").unwrap();
            let start = std::time::Instant::now();
            let act = act(&intent, &ai, &db, "en", 1_000, 0).unwrap();
            println!("{local_model} {:?}: {}\n{}", start.elapsed(), act.text, act.cards[0]["words"]);
        }
    }

    fn word(id: i64, word: &str, meaning: &str, example: &str) -> db::Word {
        db::Word { id, word: word.into(), meaning: meaning.into(), example: example.into(), lang: "en".into(), ..db::Word::default() }
    }

    #[test]
    fn a_quiz_is_made_from_the_learners_words() {
        let words = [
            word(1, "deadline", "hạn chót", "The deadline is Friday."),
            word(2, "souvenir", "quà lưu niệm", "I bought a souvenir."),
            word(3, "itinerary", "lịch trình", "Check the itinerary."),
            word(4, "negotiate", "đàm phán", "We negotiate prices."),
            word(5, "passport", "hộ chiếu", ""),
        ];
        let card = quiz(&words, 5, "all", &mut Rng(42)).unwrap();
        let items = card["items"].as_array().unwrap();
        assert_eq!(items.len(), 5);
        for item in items {
            match item["kind"].as_str().unwrap() {
                "choice" => {
                    let options: Vec<&str> = item["options"].as_array().unwrap().iter().map(|o| o.as_str().unwrap()).collect();
                    assert_eq!(options.len(), 4);
                    assert!(options.contains(&item["answer"].as_str().unwrap()));
                    let mut distinct = options.clone();
                    distinct.dedup();
                    distinct.sort();
                    distinct.dedup();
                    assert_eq!(distinct.len(), 4, "{options:?}");
                }
                "fill" => assert!(item["prompt"].as_str().unwrap().contains("____")),
                other => panic!("{other}"),
            }
        }
        assert!(quiz(&words[..3], 5, "all", &mut Rng(1)).is_none(), "fewer than four words");
        assert_eq!(blanked("The deadline is Friday.", "Deadline").as_deref(), Some("The ____ is Friday."));
        assert_eq!(blanked("No word here.", "deadline"), None);
    }

    fn setting(key: &'static str, value: &str) -> Option<Intent> {
        Some(Intent::Setting { key, value: value.into(), label: String::new() })
    }

    /// Settings compare without their button text.
    fn plain_intent(i: Option<Intent>) -> Option<Intent> {
        match i {
            Some(Intent::Setting { key, value, .. }) => setting(key, &value),
            other => other,
        }
    }

    #[test]
    fn reminders_are_recognised_with_or_without_marks() {
        let remind = |at: &str, every: Option<u32>| Some(Intent::Remind { at: at.into(), every_hours: every, off: false });
        assert_eq!(route("Nhắc tôi học lúc 20:00"), remind("20:00", None));
        assert_eq!(route("nhac toi hoc luc 8h30 nhe"), remind("08:30", None));
        assert_eq!(route("Nhắc mình học lúc 8 giờ tối"), remind("20:00", None));
        assert_eq!(route("đặt nhắc học 7 giờ 15 sáng"), remind("07:15", None));
        assert_eq!(route("Nhắc tôi học mỗi 2 giờ"), remind("", Some(2)));
        assert_eq!(route("nhac hoc moi 3h"), remind("", Some(3)));
        assert_eq!(route("nhắc tôi 2 tiếng một lần"), remind("", Some(2)));
        assert_eq!(route("Tắt nhắc học đi"), Some(Intent::Remind { at: String::new(), every_hours: Some(0), off: true }));
        assert_eq!(route("đặt nhắc học"), remind("", None));
    }

    #[test]
    fn settings_are_recognised_with_or_without_marks() {
        let r = |q: &str| plain_intent(route(q));
        assert_eq!(r("Đổi sang giao diện tối"), setting("theme", "dark"));
        assert_eq!(r("chuyen sang che do sang"), setting("theme", "light"));
        assert_eq!(r("đặt giao diện theo hệ thống"), setting("theme", "system"));
        assert_eq!(r("Đổi màu chủ đạo sang hồng"), setting("palette", "pink"));
        assert_eq!(r("doi mau tim"), setting("palette", "purple"));
        assert_eq!(r("Mỗi ngày học 15 từ mới"), setting("new_per_day", "15"));
        assert_eq!(r("dat 20 tu moi moi ngay"), setting("new_per_day", "20"));
        assert_eq!(r("Đặt mục tiêu 80 XP mỗi ngày"), setting("daily_goal", "80"));
        assert_eq!(r("Chuyển giọng sang Anh-Anh"), setting("accent", "en-GB"));
        assert_eq!(r("dung giong my"), setting("accent", "en-US"));
        assert_eq!(r("Đọc chậm hơn"), setting("speech_rate", "0.8"));
        assert_eq!(r("đặt tốc độ đọc 1.1"), setting("speech_rate", "1.1"));
        assert_eq!(r("Đặt tỉ lệ nhớ mong muốn 90%"), setting("desired_retention", "0.9"));
        assert_eq!(r("bật ngày nghỉ giữ chuỗi"), setting("streak_freeze", "1"));
        assert_eq!(r("tat ngay nghi"), setting("streak_freeze", "0"));
        assert_eq!(r("đặt ngày ôn nhẹ là thứ 7 và chủ nhật"), setting("easy_days", "0,6"));
        assert_eq!(route("Chuyển sang khóa tiếng Trung"), Some(Intent::Switch("zh")));
        assert_eq!(route("doi sang khoa tieng anh"), Some(Intent::Switch("en")));
    }

    #[test]
    fn navigation_and_study_are_recognised() {
        assert_eq!(route("Mở trang thống kê"), Some(Intent::OpenPage("stats")));
        assert_eq!(route("mo cai dat"), Some(Intent::OpenPage("settings")));
        assert_eq!(route("Vào lộ trình giúp mình"), Some(Intent::OpenPage("roadmap")));
        assert_eq!(route("mở Gia sư nói"), Some(Intent::OpenPage("talk")));
        assert_eq!(route("mở bộ từ có sẵn"), Some(Intent::OpenPage("packs")));
        assert_eq!(route("Bắt đầu học thôi"), Some(Intent::StartStudy(String::new())));
        assert_eq!(route("bat dau hoc chu de du lich"), Some(Intent::StartStudy("du lich".into())));
        assert_eq!(route("Hôm nay tôi học gì?"), Some(Intent::Today));
        assert_eq!(route("ke hoach hom nay cua minh"), Some(Intent::Today));
        assert_eq!(route("Hôm nay tôi nên học gì"), Some(Intent::Today));
    }

    #[test]
    fn words_are_skipped_or_deleted_by_name() {
        assert_eq!(route("Bỏ qua từ serendipity"), Some(Intent::SkipWord("serendipity".into())));
        assert_eq!(route("bo qua tu cope with nhe"), Some(Intent::SkipWord("cope with".into())));
        assert_eq!(route("Xóa từ abundant khỏi danh sách"), Some(Intent::DeleteWord("abundant".into())));
        assert_eq!(route("xoa tu \"resilient\""), Some(Intent::DeleteWord("resilient".into())));
        assert_ne!(route("xóa từ này"), Some(Intent::DeleteWord("này".into())));
    }

    #[test]
    fn a_roadmap_is_made_from_purpose_time_and_minutes() {
        let make = |p: &str, d: Option<i64>, m: Option<i64>| Some(Intent::CreateRoadmap { purpose: p.into(), days: d, minutes: m });
        assert_eq!(route("Tạo lộ trình IELTS 6.5 trong 3 tháng, mỗi ngày 30 phút"), make("IELTS 6.5", Some(90), Some(30)));
        assert_eq!(route("lap lo trinh hoc HSK3 trong 12 tuan"), make("HSK3", Some(84), None));
        assert_eq!(route("tạo lộ trình 2 tháng TOEIC 600"), make("TOEIC 600", Some(60), None));
        assert_eq!(route("Tạo lộ trình giao tiếp"), make("giao tiếp", None, None));
        assert_eq!(route("Lộ trình của mình thế nào?"), Some(Intent::Roadmap));
    }

    #[test]
    fn how_to_questions_get_the_guide_of_the_feature() {
        let title = |q: &str| match route(q) {
            Some(Intent::Guide(i)) => GUIDES[i].title,
            other => panic!("{q}: {other:?}"),
        };
        assert_eq!(title("Làm sao để thêm từ?"), "Cách thêm từ");
        assert_eq!(title("lam the nao de dat nhac hoc"), "Cách đặt nhắc học");
        assert_eq!(title("Hướng dẫn mình dùng app"), "Cách dùng Card English");
        assert_eq!(title("App này làm được gì?"), "Cách dùng Card English");
        assert_eq!(title("Thống kê ở đâu vậy?"), "Cách xem tiến độ");
        assert_eq!(title("cách lập lộ trình"), "Cách lập lộ trình");
        assert_eq!(title("Làm sao để đồng bộ giữa hai máy?"), "Đồng bộ, sao lưu và xuất dữ liệu");
        assert_eq!(title("làm sao để luyện phát âm"), "Cách luyện phát âm");
        // Every step with a button names a tool of the catalog.
        for g in &GUIDES {
            for (_, run) in g.items {
                if let Some(c) = guide_run(run) {
                    assert!(check_call(&c).is_some(), "{} {run}", g.title);
                }
            }
        }
    }

    #[test]
    fn dates_add_days_across_months_and_years() {
        assert_eq!(add_days("2026-10-31", 1).as_deref(), Some("2026-11-01"));
        assert_eq!(add_days("2026-01-31", 30).as_deref(), Some("2026-03-02"));
        assert_eq!(add_days("2026-12-25", 90).as_deref(), Some("2027-03-25"));
        assert_eq!(add_days("2024-02-28", 1).as_deref(), Some("2024-02-29"));
        assert_eq!(add_days("nope", 1), None);
    }

    #[test]
    fn planner_calls_are_checked_against_the_catalog() {
        let ok = |v: Value| check_call(&v);
        assert_eq!(ok(json!({ "tool": "open_page", "args": { "page": "stats" } })).unwrap().args, json!({ "page": "stats" }));
        assert!(ok(json!({ "tool": "open_page", "args": { "page": "admin" } })).is_none());
        assert!(ok(json!({ "tool": "format_disk", "args": {} })).is_none());
        assert!(ok(json!({ "tool": "start_practice", "args": { "exercise": "hack" } })).is_none());
        assert!(ok(json!({ "tool": "start_practice", "args": { "exercise": "match", "source": "topic:" } })).is_none());
        assert_eq!(ok(json!({ "tool": "start_practice", "args": { "exercise": "match" } })).unwrap().args["source"], "all");
        assert_eq!(ok(json!({ "tool": "set_reminder", "args": { "at": "20:00", "every_hours": "2", "extra": 1 } })).unwrap().args, json!({ "at": "20:00", "every_hours": 2 }));
        assert!(ok(json!({ "tool": "set_reminder", "args": { "at": "25:00" } })).is_none());
        assert!(ok(json!({ "tool": "set_reminder", "args": {} })).is_none());
        assert!(ok(json!({ "tool": "add_words", "args": { "words": [] } })).is_none());
        assert_eq!(ok(json!({ "tool": "add_words", "args": { "words": ["a", " ", "b"] } })).unwrap().args, json!({ "words": ["a", "b"] }));
        assert!(ok(json!({ "tool": "switch_course", "args": { "course": "fr" } })).is_none());
        assert!(ok(json!({ "tool": "roadmap_create", "args": { "purpose": "IELTS", "deadline": "2026-13-45", "minutes": 30 } })).is_none());
        assert!(ok(json!({ "tool": "roadmap_create", "args": { "purpose": "IELTS", "deadline": "2026-12-31", "minutes": 30 } })).is_some());
        assert!(ok(json!({ "tool": "roadmap_delete", "args": { "uid": "xyz" } })).is_none());
        assert!(ok(json!({ "tool": "roadmap_delete", "args": { "uid": "0123456789abcdef" } })).is_some());
        assert!(ok(json!("open_page")).is_none());
        let many: Vec<Value> = (0..9).map(|_| json!({ "tool": "open_packs" })).collect();
        assert_eq!(check_calls(&many).len(), 6);
    }

    #[test]
    fn the_setting_whitelist_holds_for_the_planner() {
        let set = |k: &str, v: Value| check_call(&json!({ "tool": "set_setting", "args": { "key": k, "value": v } }));
        assert_eq!(set("new_per_day", json!(15)).unwrap().args, json!({ "key": "new_per_day", "value": "15" }));
        assert_eq!(set("theme", json!("dark")).unwrap().args["value"], "dark");
        assert_eq!(set("streak_freeze", json!(true)).unwrap().args["value"], "1");
        assert_eq!(set("speech_rate", json!(0.9)).unwrap().args["value"], "0.9");
        assert!(set("theme", json!("neon")).is_none());
        assert!(set("speech_rate", json!(3)).is_none());
        assert!(set("desired_retention", json!(0.99)).is_none());
        assert!(set("palette", json!("green")).is_none());
        // Real settings the chat must not touch, however well the value is formed.
        for key in ["gemini_key", "deepseek_key", "ai_provider", "hotkey", "voice", "remind_at", "achievements", "match_best", "icons"] {
            assert!(set(key, json!("1")).is_none(), "{key}");
        }
    }

    #[test]
    fn cards_have_the_shape_the_chat_draws() {
        let card = planned_card(&check_calls(&[json!({ "tool": "set_reminder", "args": { "at": "20:00" } }), json!({ "tool": "open_page", "args": { "page": "stats" } })]));
        assert_eq!(card["type"], "do");
        let calls = card["calls"].as_array().unwrap();
        assert_eq!((calls[0]["confirm"].as_bool(), calls[1]["confirm"].as_bool()), (Some(true), Some(false)));
        assert_eq!(calls[0]["label"], "Nhắc học lúc 20:00");
        assert_eq!(calls[1]["label"], "Mở Thống kê");
        assert!(may_command("mở trang thống kê") && !may_command("deadline nghĩa là gì?"));
    }

    #[test]
    fn acts_build_do_plan_and_guide_cards() {
        let db = db::open(std::path::Path::new(":memory:")).unwrap();
        let settings = ai::Settings::default();
        let ai = Ai::new(&settings, std::path::Path::new("."));
        let run = |q: &str| act(&route(q).unwrap(), &ai, &db, "en", 1_000, 0).unwrap();
        let a = run("Nhắc tôi học lúc 20:00");
        assert_eq!(a.cards[0]["calls"][0], json!({ "tool": "set_reminder", "args": { "at": "20:00" }, "label": "Nhắc học lúc 20:00 mỗi ngày", "confirm": true }));
        let a = run("Hôm nay tôi học gì?");
        assert_eq!(a.cards[0]["type"], "plan");
        assert_eq!(a.cards[0]["steps"][0]["run"]["tool"], "open_packs");
        let a = run("Làm sao để thêm từ?");
        assert_eq!(a.cards[0]["type"], "guide");
        assert!(a.cards[0]["items"][0]["run"]["tool"] == "open_page");
        assert!(a.cards[0]["items"][1].get("run").is_none());
        let a = run("Bỏ qua từ ghost");
        assert!(a.cards.is_empty() && a.text.contains("ghost"));
        let a = run("Tạo lộ trình giao tiếp");
        assert!(a.cards.is_empty());
        let a = run("Mở trang thống kê");
        assert_eq!(a.cards[0]["calls"][0]["confirm"], false);
        let state = state_summary(&db, "en", 1_000, 0);
        assert!(state.contains("Course: en") && state.contains("Pages:"));
    }
}
