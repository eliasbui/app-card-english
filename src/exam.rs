//! Mock exams the AI writes on the pattern of the international exams: IELTS, TOEIC, VSTEP and a CEFR four-skill
//! test for English, HSK 3.0 with HSKK for Chinese. The formats are data here, so that the prompts, the timing and
//! the scoring read from one place. A roadmap whose goal names an exam, or a level of B2 or HSK 4 and above, is
//! hard: each of its stages ends with a stage test and the last one with a full mock. A paper is written set by set,
//! kept whole in the `exams` table with the learner's answers and result, and scored on the exam's own scale.

use crate::{ai, db, pron, roadmap, speech};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// How an item is answered, and so how it is scored.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// One of a few options, scored from the key.
    Choice,
    /// One entry of a list the set shares (headings, a word bank), scored from the key.
    Match,
    /// A few words or characters typed in, scored from the key.
    Text,
    /// A writing task, graded by the AI on the exam's criteria.
    Write,
    /// A spoken answer, heard by Whisper and graded by the AI; repeating and reading aloud by its sounds.
    Speak,
}

/// What a set must hold besides its items.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Body {
    None,
    /// A passage, a talk or a story.
    Text,
    /// Lines of two speakers.
    Dialogue,
}

/// An item type of the official exams: how it is answered, what is heard, and what the AI writes for one set.
pub struct Kind {
    pub key: &'static str,
    pub mode: Mode,
    body: Body,
    /// The passage or the dialogue of a set, and each item's `say`, are heard rather than read until the section is
    /// submitted.
    pub heard: bool,
    /// The options are heard too and show only as letters, as in TOEIC parts 1 and 2.
    pub heard_options: bool,
    /// The options every item has, which the AI does not write.
    fixed: &'static [&'static str],
    /// What to do, for the learner.
    pub hint: &'static str,
    /// What one set holds, for the AI.
    brief: &'static str,
    /// A worked example of a set in each course: what the AI is asked and what it answers.
    en: (&'static str, &'static str),
    zh: (&'static str, &'static str),
    /// About how many tokens an answer takes: for the set, and for each item.
    tokens: (usize, usize),
}

const K: Kind = Kind {
    key: "",
    mode: Mode::Choice,
    body: Body::None,
    heard: false,
    heard_options: false,
    fixed: &[],
    hint: "",
    brief: "",
    en: ("", ""),
    zh: ("", ""),
    tokens: (0, 80),
};

/// One part of a section, in the official order: its item type, what the official exam asks in it, and how many
/// sets of how many items a stage test and a full mock take (no sets leaves the part out).
#[derive(Debug)]
pub struct Part {
    pub kind: &'static str,
    pub label: &'static str,
    /// What the official part asks, for the AI.
    pub task: &'static str,
    pub mini: (usize, usize),
    pub full: (usize, usize),
    /// Seconds an item has in the official exam: its section's time over its items. For a spoken answer, the most
    /// it may last.
    pub secs: u32,
    /// Options of a multiple-choice item.
    pub options: usize,
    /// The fewest words (characters in Chinese) an answer to a writing task may have, 0 for none; the CEFR test takes
    /// them from its level.
    pub words: usize,
    /// How much an item counts within its section.
    pub weight: u32,
    /// Seconds to prepare before speaking.
    pub prep: u32,
}

const P: Part = Part { kind: "", label: "", task: "", mini: (0, 0), full: (1, 1), secs: 60, options: 4, words: 0, weight: 1, prep: 0 };

pub struct Section {
    /// "listening", "reading", "writing" or "speaking".
    pub skill: &'static str,
    /// As the official exam names it.
    pub name: &'static str,
    pub parts: &'static [Part],
}

pub struct Format {
    pub id: &'static str,
    pub name: &'static str,
    /// How many times the official exam plays a recording.
    pub plays: u32,
    pub sections: &'static [Section],
    /// The examiner's criteria for writing and for speaking, as the AI is told them and as the learner reads them; the
    /// last speaking one is pronunciation, which the app measures itself.
    pub writing: [(&'static str, &'static str); 4],
    pub speaking: [(&'static str, &'static str); 4],
    /// The top of the scale a criterion is rated on.
    pub rate: f64,
}

/// The CEFR levels, in order.
const CEFR: [&str; 6] = ["A1", "A2", "B1", "B2", "C1", "C2"];

// IELTS Academic as ielts.org describes it: Listening 4 parts, 40 questions, about 30 minutes, heard once; Reading 3
// passages, 40 questions, 60 minutes; Writing Task 1 (150 words, 20 minutes) and Task 2 (250 words, 40 minutes,
// counting double); Speaking Part 1, Part 2 (a cue card, a minute to prepare and up to two to speak) and Part 3.
// Bands 0 to 9 in halves, criteria from the public band descriptors.
const IELTS: Format = Format {
    id: "ielts",
    name: "IELTS",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "Listening", parts: &[
            Part { kind: "l_form", label: "Part 1: Hội thoại đời sống, điền thông tin", task: "a phone call about booking or asking for information, with notes to complete", mini: (1, 4), full: (1, 5), secs: 45, ..P },
            Part { kind: "l_talk", label: "Part 2: Độc thoại về đời sống", task: "one speaker giving information about a local place or event", full: (1, 5), secs: 45, options: 3, ..P },
            Part { kind: "l_dialogue", label: "Part 3: Thảo luận trong học tập", task: "two students discussing a study project", mini: (1, 3), full: (1, 5), secs: 45, options: 3, ..P },
            Part { kind: "l_talk", label: "Part 4: Bài giảng", task: "a short university lecture on an academic subject", full: (1, 5), secs: 45, options: 3, ..P },
        ] },
        Section { skill: "reading", name: "Reading", parts: &[
            Part { kind: "r_tfng", label: "Passage 1: True / False / Not Given", task: "an academic passage of about 250 words with statements to judge", mini: (1, 5), full: (1, 6), secs: 90, options: 3, ..P },
            Part { kind: "r_headings", label: "Passage 2: Chọn tiêu đề cho đoạn văn", task: "an academic passage of four or five lettered paragraphs with a list of headings", full: (1, 5), secs: 90, ..P },
            Part { kind: "r_complete", label: "Passage 3: Hoàn thành câu", task: "an academic passage of about 250 words with sentences to complete in no more than two words", full: (1, 5), secs: 90, ..P },
        ] },
        Section { skill: "writing", name: "Writing", parts: &[
            Part { kind: "w_task", label: "Task 1: Mô tả số liệu", task: "Academic Writing Task 1: a small table of figures given in text, to summarise and compare in at least 150 words", mini: (1, 1), secs: 1200, words: 150, ..P },
            Part { kind: "w_task", label: "Task 2: Bài luận", task: "Academic Writing Task 2: an essay question giving an opinion or a problem, to answer in at least 250 words", secs: 2400, words: 250, weight: 2, ..P },
        ] },
        Section { skill: "speaking", name: "Speaking", parts: &[
            Part { kind: "s_task", label: "Part 1: Giới thiệu và phỏng vấn", task: "Part 1: short questions about the candidate's life, work, study and interests", mini: (1, 3), full: (1, 4), secs: 45, ..P },
            Part { kind: "s_cue", label: "Part 2: Thẻ đề bài (cue card)", task: "Part 2: a cue card with a topic and four points to cover", mini: (1, 1), secs: 120, prep: 60, ..P },
            Part { kind: "s_task", label: "Part 3: Thảo luận", task: "Part 3: abstract discussion questions linked to the Part 2 topic", full: (1, 3), secs: 60, ..P },
        ] },
    ],
    writing: [
        ("Task Achievement / Task Response", "Hoàn thành yêu cầu đề"),
        ("Coherence and Cohesion", "Mạch lạc và liên kết"),
        ("Lexical Resource", "Vốn từ"),
        ("Grammatical Range and Accuracy", "Ngữ pháp đa dạng và chính xác"),
    ],
    speaking: [
        ("Fluency and Coherence", "Trôi chảy và mạch lạc"),
        ("Lexical Resource", "Vốn từ"),
        ("Grammatical Range and Accuracy", "Ngữ pháp đa dạng và chính xác"),
        ("Pronunciation", "Phát âm"),
    ],
    rate: 9.0,
};

// TOEIC as ETS describes it: Listening and Reading, 200 questions in two hours (Listening parts 1 to 4, 100 questions,
// about 45 minutes, heard once; Reading parts 5 to 7, 100 questions, 75 minutes), each scored 5 to 495 and 10 to 990
// in all; Speaking (11 tasks, 20 minutes) and Writing (8 tasks, 60 minutes), each 0 to 200 from task ratings of 0 to 5.
const TOEIC: Format = Format {
    id: "toeic",
    name: "TOEIC",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "Listening", parts: &[
            Part { kind: "l_photo", label: "Part 1: Mô tả tranh", task: "Part 1 Photographs", mini: (1, 1), full: (1, 2), secs: 27, ..P },
            Part { kind: "l_response", label: "Part 2: Hỏi - đáp", task: "Part 2 Question-Response, at work", mini: (1, 3), full: (1, 5), secs: 27, options: 3, ..P },
            Part { kind: "l_dialogue", label: "Part 3: Hội thoại ngắn", task: "Part 3 Conversations: a workplace conversation between a man and a woman", mini: (1, 3), full: (2, 3), secs: 27, ..P },
            Part { kind: "l_talk", label: "Part 4: Bài nói ngắn", task: "Part 4 Talks: an announcement, a voicemail or a short talk at work", full: (2, 3), secs: 27, ..P },
        ] },
        Section { skill: "reading", name: "Reading", parts: &[
            Part { kind: "r_sentence", label: "Part 5: Hoàn thành câu", task: "Part 5 Incomplete Sentences: grammar and business vocabulary", mini: (1, 4), full: (1, 6), secs: 45, ..P },
            Part { kind: "r_text", label: "Part 6: Hoàn thành đoạn văn", task: "Part 6 Text Completion: an email, notice or memo with four blanks", full: (1, 4), secs: 45, ..P },
            Part { kind: "r_choice", label: "Part 7: Đọc hiểu", task: "Part 7 Reading Comprehension: an email, advertisement, article or notice", mini: (1, 3), full: (2, 4), secs: 45, ..P },
        ] },
        Section { skill: "speaking", name: "Speaking", parts: &[
            Part { kind: "s_read", label: "Câu 1-2: Đọc to đoạn văn", task: "Questions 1-2 Read a text aloud: an announcement or advertisement of about 50 words", mini: (1, 1), full: (1, 1), secs: 45, prep: 45, ..P },
            Part { kind: "s_task", label: "Câu 5-7: Trả lời câu hỏi", task: "Questions 5-7 Respond to questions: a market survey about everyday habits", full: (1, 3), secs: 30, prep: 3, ..P },
            Part { kind: "s_cue", label: "Câu 11: Nêu ý kiến", task: "Question 11 Express an opinion: a workplace question to answer with reasons and examples", full: (1, 1), secs: 60, prep: 45, ..P },
        ] },
        Section { skill: "writing", name: "Writing", parts: &[
            Part { kind: "w_task", label: "Câu 6-7: Trả lời email", task: "Questions 6-7 Respond to a written request: an email to answer as asked", mini: (1, 1), full: (1, 1), secs: 600, ..P },
            Part { kind: "w_task", label: "Câu 8: Bài luận nêu ý kiến", task: "Question 8 Write an opinion essay of at least 300 words on a work or life question", secs: 1800, words: 300, weight: 2, ..P },
        ] },
    ],
    writing: [
        ("Grammar", "Ngữ pháp"),
        ("Relevance and completeness", "Đúng và đủ yêu cầu"),
        ("Vocabulary", "Từ vựng"),
        ("Organization", "Bố cục"),
    ],
    speaking: [
        ("Grammar", "Ngữ pháp"),
        ("Vocabulary", "Từ vựng"),
        ("Cohesion and relevance of content", "Mạch lạc và đúng ý"),
        ("Pronunciation, intonation and stress", "Phát âm và ngữ điệu"),
    ],
    rate: 5.0,
};

// VSTEP.3-5 as Decision 729/QĐ-BGDĐT (2015) of the Ministry of Education and Training sets it: Listening 3 parts, 35
// questions, about 40 minutes, heard once; Reading 4 passages, 40 questions, 60 minutes; Writing a letter (120 words, a
// third of the mark) and an essay (250 words, two thirds), 60 minutes; Speaking social interaction, solution
// discussion and topic development, 12 minutes. Each skill 0 to 10, their mean giving B1 (4.0), B2 (6.0) or C1 (8.5).
const VSTEP: Format = Format {
    id: "vstep",
    name: "VSTEP",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "Listening", parts: &[
            Part { kind: "l_short", label: "Part 1: Thông báo, hướng dẫn ngắn", task: "Part 1: short announcements, messages and instructions, one question each", mini: (1, 3), full: (1, 4), secs: 68, ..P },
            Part { kind: "l_dialogue", label: "Part 2: Hội thoại", task: "Part 2: a conversation in everyday or work life", mini: (1, 3), full: (1, 4), secs: 68, ..P },
            Part { kind: "l_talk", label: "Part 3: Bài giảng, bài nói", task: "Part 3: a talk or a lecture", full: (1, 5), secs: 68, ..P },
        ] },
        Section { skill: "reading", name: "Reading", parts: &[
            Part { kind: "r_choice", label: "Bài đọc 1", task: "a passage of about 300 words on a social or general topic", mini: (1, 5), full: (1, 6), secs: 90, ..P },
            Part { kind: "r_choice", label: "Bài đọc 2", task: "a harder passage of about 400 words on an academic topic, with questions on detail, inference and vocabulary", full: (1, 6), secs: 90, ..P },
        ] },
        Section { skill: "writing", name: "Writing", parts: &[
            Part { kind: "w_task", label: "Task 1: Viết thư", task: "Task 1: a letter or email of at least 120 words answering a situation", mini: (1, 1), secs: 1200, words: 120, ..P },
            Part { kind: "w_task", label: "Task 2: Viết luận", task: "Task 2: an essay of at least 250 words on a social issue", secs: 2400, words: 250, weight: 2, ..P },
        ] },
        Section { skill: "speaking", name: "Speaking", parts: &[
            Part { kind: "s_task", label: "Part 1: Tương tác xã hội", task: "Part 1 Social interaction: questions on two familiar topics", mini: (1, 3), full: (1, 4), secs: 45, ..P },
            Part { kind: "s_cue", label: "Part 2: Thảo luận giải pháp", task: "Part 2 Solution discussion: a situation and three options; choose the best one and explain why", full: (1, 1), secs: 120, prep: 60, ..P },
            Part { kind: "s_cue", label: "Part 3: Phát triển chủ đề", task: "Part 3 Topic development: a topic with three ideas to develop, as a mind map", full: (1, 1), secs: 120, prep: 60, ..P },
        ] },
    ],
    writing: [
        ("Task fulfilment", "Hoàn thành yêu cầu"),
        ("Organization", "Bố cục"),
        ("Vocabulary", "Từ vựng"),
        ("Grammar", "Ngữ pháp"),
    ],
    speaking: [
        ("Grammar", "Ngữ pháp"),
        ("Vocabulary", "Từ vựng"),
        ("Fluency and discourse management", "Trôi chảy và tổ chức ý"),
        ("Pronunciation", "Phát âm"),
    ],
    rate: 10.0,
};

// A four-skill test of one CEFR level, on the scales of the CEFR Companion Volume (Council of Europe, 2020), with
// papers like those of the Cambridge English Qualifications of that level; a level is reached from 60%.
const CEFR_TEST: Format = Format {
    id: "cefr",
    name: "CEFR",
    plays: 2,
    sections: &[
        Section { skill: "listening", name: "Listening", parts: &[
            Part { kind: "l_dialogue", label: "Phần 1: Hội thoại", task: "a conversation in everyday life", mini: (1, 3), full: (1, 5), secs: 60, options: 3, ..P },
            Part { kind: "l_talk", label: "Phần 2: Bài nói", task: "a talk, a radio programme or an announcement", full: (1, 5), secs: 60, options: 3, ..P },
        ] },
        Section { skill: "reading", name: "Reading", parts: &[
            Part { kind: "r_choice", label: "Phần 1: Đọc hiểu", task: "an article, a story or an email", mini: (1, 4), full: (1, 5), secs: 75, ..P },
            Part { kind: "r_tfng", label: "Phần 2: Đúng / Sai / Không có thông tin", task: "a short text with statements to judge", full: (1, 5), secs: 75, options: 3, ..P },
        ] },
        Section { skill: "writing", name: "Writing", parts: &[
            Part { kind: "w_task", label: "Bài viết 1: Email", task: "an email to a friend or a colleague answering three points", mini: (1, 1), secs: 1200, ..P },
            Part { kind: "w_task", label: "Bài viết 2: Bài luận ngắn", task: "a short essay or article giving an opinion", secs: 1800, weight: 2, ..P },
        ] },
        Section { skill: "speaking", name: "Speaking", parts: &[
            Part { kind: "s_task", label: "Phần 1: Phỏng vấn", task: "interview questions about everyday life", mini: (1, 3), full: (1, 4), secs: 45, ..P },
            Part { kind: "s_cue", label: "Phần 2: Nói dài", task: "a long turn: a topic with points to talk about", full: (1, 1), secs: 90, prep: 60, ..P },
        ] },
    ],
    writing: [
        ("Task achievement", "Hoàn thành yêu cầu"),
        ("Coherence", "Mạch lạc"),
        ("Range of vocabulary and grammar", "Vốn từ và cấu trúc"),
        ("Accuracy", "Độ chính xác"),
    ],
    speaking: [
        ("Range", "Vốn từ và cấu trúc"),
        ("Accuracy", "Độ chính xác"),
        ("Fluency and coherence", "Trôi chảy và mạch lạc"),
        ("Phonology", "Phát âm"),
    ],
    rate: 10.0,
};

// HSK 3.0 as Chinese Testing International (chinesetest.cn) sets the exams on the GF 0025-2021 standard: levels 1 and
// 2 test listening and reading (200 points, pass 120), levels 3 to 6 add writing (300 points, pass 180), each level
// with its own item types; HSKK speaking at the matching band, beginner for levels 1 and 2, intermediate for 3 and 4,
// advanced for 5 and 6 (100 points, pass 60). Levels 7 to 9 take the level 6 pattern here. Where the exam shows a
// picture, the app describes it in a Vietnamese sentence instead.
const HSK_WRITING: [(&str, &str); 4] = [
    ("Content", "Nội dung"),
    ("Grammar", "Ngữ pháp"),
    ("Vocabulary", "Từ vựng"),
    ("Characters and punctuation", "Chữ Hán và dấu câu"),
];
const HSKK_CRITERIA: [(&str, &str); 4] = [
    ("Content and fluency", "Nội dung và độ trôi chảy"),
    ("Grammar", "Ngữ pháp"),
    ("Vocabulary", "Từ vựng"),
    ("Pronunciation and tones", "Phát âm và thanh điệu"),
];
const HSKK_BEGINNER: Section = Section { skill: "speaking", name: "HSKK 初级", parts: &[
    Part { kind: "s_repeat", label: "第一部分：听后重复", task: "short sentences to repeat after hearing them", mini: (1, 3), full: (1, 5), secs: 15, ..P },
    Part { kind: "s_answer", label: "第二部分：听后回答", task: "short questions heard, to answer in one sentence", mini: (1, 2), full: (1, 3), secs: 20, ..P },
    Part { kind: "s_task", label: "第三部分：回答问题", task: "questions about daily life, to answer in at least five sentences", full: (1, 2), secs: 60, prep: 30, ..P },
] };
const HSKK_INTERMEDIATE: Section = Section { skill: "speaking", name: "HSKK 中级", parts: &[
    Part { kind: "s_repeat", label: "第一部分：听后重复", task: "sentences to repeat after hearing them", mini: (1, 3), full: (1, 4), secs: 15, ..P },
    Part { kind: "s_describe", label: "第二部分：看图说话", task: "a picture, described in Vietnamese, to talk about", full: (1, 1), secs: 90, prep: 45, ..P },
    Part { kind: "s_task", label: "第三部分：回答问题", task: "questions on everyday topics, to answer at length", mini: (1, 1), full: (1, 2), secs: 90, prep: 30, ..P },
] };
const HSKK_ADVANCED: Section = Section { skill: "speaking", name: "HSKK 高级", parts: &[
    Part { kind: "s_retell", label: "第一部分：听后复述", task: "a short story heard, to retell", mini: (1, 1), full: (1, 2), secs: 90, ..P },
    Part { kind: "s_read", label: "第二部分：朗读", task: "a passage of about 120 characters to read aloud", full: (1, 1), secs: 90, prep: 60, ..P },
    Part { kind: "s_task", label: "第三部分：回答问题", task: "questions on society and opinions, to answer at length", mini: (1, 1), full: (1, 2), secs: 120, prep: 60, ..P },
] };
const HSK12: Format = Format {
    id: "hsk12",
    name: "HSK",
    plays: 2,
    sections: &[
        Section { skill: "listening", name: "听力", parts: &[
            Part { kind: "l_judge", label: "第一部分：听句子，判断对错", task: "a sentence heard and a statement to judge true or false", mini: (1, 3), full: (1, 5), secs: 40, options: 2, ..P },
            Part { kind: "l_short", label: "第二部分：听对话，选择正确答案", task: "a short dialogue heard and a question with three options", mini: (1, 3), full: (1, 5), secs: 40, options: 3, ..P },
        ] },
        Section { skill: "reading", name: "阅读", parts: &[
            Part { kind: "r_fill", label: "第一部分：选词填空", task: "sentences with a blank and a bank of words", mini: (1, 3), full: (1, 5), secs: 50, ..P },
            Part { kind: "r_choice", label: "第二部分：阅读理解", task: "a short text with questions", full: (1, 4), secs: 50, options: 3, ..P },
        ] },
        HSKK_BEGINNER,
    ],
    writing: HSK_WRITING,
    speaking: HSKK_CRITERIA,
    rate: 10.0,
};
const HSK3: Format = Format {
    id: "hsk3",
    name: "HSK",
    plays: 2,
    sections: &[
        Section { skill: "listening", name: "听力", parts: &[
            Part { kind: "l_judge", label: "第一部分：听句子，判断对错", task: "a sentence heard and a statement to judge true or false", mini: (1, 3), full: (1, 4), secs: 50, options: 2, ..P },
            Part { kind: "l_short", label: "第二部分：听对话，选择正确答案", task: "a short dialogue heard and a question with three options", mini: (1, 3), full: (1, 5), secs: 50, options: 3, ..P },
        ] },
        Section { skill: "reading", name: "阅读", parts: &[
            Part { kind: "r_fill", label: "第一部分：选词填空", task: "sentences with a blank and a bank of words", mini: (1, 3), full: (1, 4), secs: 60, ..P },
            Part { kind: "r_order", label: "第二部分：排列顺序", task: "three sentences to put in order", full: (1, 3), secs: 60, options: 3, ..P },
            Part { kind: "r_choice", label: "第三部分：阅读理解", task: "a short text with a question", full: (1, 3), secs: 60, options: 3, ..P },
        ] },
        Section { skill: "writing", name: "书写", parts: &[
            Part { kind: "w_order", label: "第一部分：完成句子", task: "words to put in order as a sentence", mini: (1, 3), full: (1, 4), secs: 90, ..P },
            Part { kind: "w_hanzi", label: "第二部分：写汉字", task: "a sentence with one character given in pinyin, to write as a character", full: (1, 4), secs: 60, ..P },
        ] },
        HSKK_INTERMEDIATE,
    ],
    writing: HSK_WRITING,
    speaking: HSKK_CRITERIA,
    rate: 10.0,
};
const HSK4: Format = Format {
    id: "hsk4",
    name: "HSK",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "听力", parts: &[
            Part { kind: "l_judge", label: "第一部分：听句子，判断对错", task: "a short passage heard and a statement to judge true or false", mini: (1, 3), full: (1, 4), secs: 40, options: 2, ..P },
            Part { kind: "l_short", label: "第二部分：听对话，选择正确答案", task: "a short dialogue heard and a question with four options", mini: (1, 3), full: (1, 4), secs: 40, ..P },
            Part { kind: "l_dialogue", label: "第三部分：听长对话，回答问题", task: "a longer dialogue with questions", full: (1, 3), secs: 40, ..P },
        ] },
        Section { skill: "reading", name: "阅读", parts: &[
            Part { kind: "r_fill", label: "第一部分：选词填空", task: "sentences with a blank and a bank of words", mini: (1, 3), full: (1, 4), secs: 60, ..P },
            Part { kind: "r_order", label: "第二部分：排列顺序", task: "three sentences to put in order", full: (1, 3), secs: 60, options: 3, ..P },
            Part { kind: "r_choice", label: "第三部分：阅读理解", task: "a short text with questions", mini: (1, 2), full: (1, 4), secs: 60, ..P },
        ] },
        Section { skill: "writing", name: "书写", parts: &[
            Part { kind: "w_order", label: "第一部分：完成句子", task: "words to put in order as a sentence", mini: (1, 3), full: (1, 4), secs: 100, ..P },
            Part { kind: "w_task", label: "第二部分：看图，用词造句", task: "a picture described in Vietnamese and a word: write one sentence with the word about the picture", full: (1, 2), secs: 140, weight: 2, ..P },
        ] },
        HSKK_INTERMEDIATE,
    ],
    writing: HSK_WRITING,
    speaking: HSKK_CRITERIA,
    rate: 10.0,
};
const HSK5: Format = Format {
    id: "hsk5",
    name: "HSK",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "听力", parts: &[
            Part { kind: "l_short", label: "第一部分：听对话，选择正确答案", task: "a dialogue heard and a question with four options", mini: (1, 4), full: (1, 5), secs: 40, ..P },
            Part { kind: "l_talk", label: "第二部分：听短文，回答问题", task: "a short passage heard with questions", full: (1, 4), secs: 40, ..P },
        ] },
        Section { skill: "reading", name: "阅读", parts: &[
            Part { kind: "r_text", label: "第一部分：选词填空", task: "a passage with numbered blanks and four options each", mini: (1, 3), full: (1, 4), secs: 60, ..P },
            Part { kind: "r_choice", label: "第二部分：阅读理解", task: "a passage of about 300 characters with questions", mini: (1, 2), full: (1, 4), secs: 60, ..P },
        ] },
        Section { skill: "writing", name: "书写", parts: &[
            Part { kind: "w_order", label: "第一部分：完成句子", task: "words to put in order as a sentence", mini: (1, 3), full: (1, 4), secs: 120, ..P },
            Part { kind: "w_task", label: "第二部分：写短文", task: "write a short text of about 80 characters using five given words", full: (1, 1), secs: 1200, words: 80, weight: 4, ..P },
        ] },
        HSKK_ADVANCED,
    ],
    writing: HSK_WRITING,
    speaking: HSKK_CRITERIA,
    rate: 10.0,
};
const HSK6: Format = Format {
    id: "hsk6",
    name: "HSK",
    plays: 1,
    sections: &[
        Section { skill: "listening", name: "听力", parts: &[
            Part { kind: "l_short", label: "第一部分：听短文，选择一致的一项", task: "a short passage heard and a question with four options", mini: (1, 4), full: (1, 4), secs: 40, ..P },
            Part { kind: "l_dialogue", label: "第二部分：听访谈，回答问题", task: "an interview with questions", full: (1, 4), secs: 40, ..P },
            Part { kind: "l_talk", label: "第三部分：听长文，回答问题", task: "a longer passage with questions", full: (1, 3), secs: 40, ..P },
        ] },
        Section { skill: "reading", name: "阅读", parts: &[
            Part { kind: "r_error", label: "第一部分：选出有语病的句子", task: "four sentences, one of them with a grammar error", mini: (1, 2), full: (1, 3), secs: 70, ..P },
            Part { kind: "r_text", label: "第二部分：选词填空", task: "a passage with numbered blanks and four options each", full: (1, 3), secs: 70, ..P },
            Part { kind: "r_choice", label: "第三部分：阅读理解", task: "a passage of about 400 characters with questions", mini: (1, 3), full: (1, 4), secs: 70, ..P },
        ] },
        Section { skill: "writing", name: "书写", parts: &[
            Part { kind: "w_task", label: "缩写", task: "a story of about 500 characters to rewrite as a summary of about 200 characters", mini: (1, 1), full: (1, 1), secs: 1800, words: 150, ..P },
        ] },
        HSKK_ADVANCED,
    ],
    writing: HSK_WRITING,
    speaking: HSKK_CRITERIA,
    rate: 10.0,
};

pub const FORMATS: [&Format; 9] = [&IELTS, &TOEIC, &VSTEP, &CEFR_TEST, &HSK12, &HSK3, &HSK4, &HSK5, &HSK6];

pub fn format(id: &str) -> Option<&'static Format> {
    FORMATS.iter().copied().find(|f| f.id == id)
}

/// The item types, with the line format the AI answers in: `TEXT:` for a passage, `M:` and `W:` (男, 女) for the
/// lines of a dialogue, `LIST:` for a shared list, then per item `SAY:` for what is heard, `Q:` for what is printed,
/// `A)` to `D)` for its options, `KEY:` and `WHY:`.
const KINDS: &[Kind] = &[
    Kind {
        key: "l_form",
        mode: Mode::Text,
        body: Body::Dialogue,
        heard: true,
        hint: "Nghe hội thoại rồi điền vào chỗ trống, không quá hai từ hoặc một con số.",
        brief: "A conversation between a woman (W) and a man (M), one turn per line, then notes to complete: each Q is a note \
                with ____ and its KEY is one or two words or a number said in the conversation; give other ways to write the \
                same answer after a slash.",
        en: (
            "Exam: IELTS Listening. Task: a phone call about booking or asking for information, with notes to complete\nLevel: B1\nTopic: Du lịch\nWords: reservation, deposit\nQuestions: 3",
            "W: Good morning, Seaview Hotel. How can I help you?\n\
             M: Hi, I'd like to make a reservation for two nights, from the fourteenth of May.\n\
             W: Certainly. Can I have your surname, please?\n\
             M: It's Carter, C-A-R-T-E-R.\n\
             W: Thank you. We ask for a deposit of forty pounds to hold the room.\n\
             M: That's fine, I'll pay by card.\n\
             Q: Number of nights: ____\nKEY: two / 2\nWHY: Người nam nói \"two nights\" (hai đêm).\n\
             Q: Surname: ____\nKEY: Carter\nWHY: Người nam đánh vần họ là C-A-R-T-E-R.\n\
             Q: Deposit: ____ pounds\nKEY: forty / 40\nWHY: Nhân viên nói tiền đặt cọc là \"forty pounds\".",
        ),
        tokens: (220, 45),
        ..K
    },
    Kind {
        key: "l_dialogue",
        body: Body::Dialogue,
        heard: true,
        hint: "Nghe hội thoại rồi chọn đáp án đúng.",
        brief: "A conversation between a man (M) and a woman (W), one turn per line, then questions on it with their options.",
        en: (
            "Exam: IELTS Listening. Task: two students discussing a study project\nLevel: B2\nTopic: Sức khỏe\nWords: nutrition, schedule\nQuestions: 2",
            "M: Have you finished the survey on student eating habits?\n\
             W: Almost. Most students said they skip breakfast because their classes start so early.\n\
             M: Did anyone mention nutrition classes?\n\
             W: Yes, about a third would like a short course on cooking cheap, healthy meals.\n\
             Q: Why do most students skip breakfast?\nA) They are not hungry\nB) Their classes start early\nC) Breakfast is too expensive\n\
             KEY: B\nWHY: Người nữ nói sinh viên bỏ bữa sáng vì lớp học bắt đầu sớm.\n\
             Q: What do about a third of the students want?\nA) A cooking course\nB) A free canteen\nC) More sports\n\
             KEY: A\nWHY: \"a short course on cooking cheap, healthy meals\" là một khóa học nấu ăn.",
        ),
        zh: (
            "Exam: HSK Listening. Task: a longer dialogue with questions\nLevel: HSK 4\nTopic: Mua sắm\nWords: 打折, 质量\nQuestions: 2",
            "男：这件衣服现在打八折，你要不要买？\n\
             女：颜色不错，可是我觉得质量一般。\n\
             男：那我们再去别的商店看看吧。\n\
             女：好，不过我五点以前得回家做饭。\n\
             Q: 女的觉得这件衣服怎么样？\nA) 太贵了\nB) 质量一般\nC) 颜色不好\nD) 很合适\n\
             KEY: B\nWHY: Người nữ nói chất lượng bình thường (质量一般).\n\
             Q: 女的五点以前要做什么？\nA) 去商店\nB) 上班\nC) 回家做饭\nD) 见朋友\n\
             KEY: C\nWHY: Người nữ nói trước 5 giờ phải về nhà nấu cơm (回家做饭).",
        ),
        tokens: (220, 70),
        ..K
    },
    Kind {
        key: "l_talk",
        body: Body::Text,
        heard: true,
        hint: "Nghe bài nói rồi chọn đáp án đúng.",
        brief: "TEXT with one speaker's talk of about 120 words, then questions on it with their options.",
        en: (
            "Exam: IELTS Listening. Task: a short university lecture on an academic subject\nLevel: B2\nTopic: Môi trường\nWords: recycle, impact\nQuestions: 2",
            "TEXT: Today I want to look at how cities recycle plastic. In many places, less than ten percent of plastic waste is \
             actually recycled. The main problem is that different plastics are mixed together, and sorting them by hand is slow \
             and expensive. Some cities now use machines with cameras that recognise each type of plastic. These machines have \
             doubled the amount that can be reused, which reduces the impact on the environment.\n\
             Q: What is the main problem with recycling plastic?\nA) People throw away too much\nB) Different plastics are mixed together\nC) Recycling plants are too far away\n\
             KEY: B\nWHY: Người giảng nói vấn đề chính là các loại nhựa bị trộn lẫn (mixed together).\n\
             Q: What have the new machines done?\nA) Doubled the plastic that can be reused\nB) Replaced all the workers\nC) Made plastic cheaper\n\
             KEY: A\nWHY: Máy mới \"have doubled the amount that can be reused\".",
        ),
        zh: (
            "Exam: HSK Listening. Task: a short passage heard with questions\nLevel: HSK 5\nTopic: Sức khỏe\nWords: 坚持, 习惯\nQuestions: 2",
            "TEXT: 很多人知道运动对身体好，可是很难坚持。专家建议，刚开始的时候不要给自己定太高的目标，比如每天散步二十分钟就可以。\
             等运动成为习惯以后，再慢慢增加时间。另外，和朋友一起运动也更容易坚持下去。\n\
             Q: 专家建议刚开始的时候怎么做？\nA) 每天跑一个小时\nB) 定一个不太高的目标\nC) 去健身房\nD) 只在周末运动\n\
             KEY: B\nWHY: Chuyên gia khuyên lúc đầu đừng đặt mục tiêu quá cao (不要给自己定太高的目标).\n\
             Q: 怎样更容易坚持运动？\nA) 和朋友一起运动\nB) 晚上运动\nC) 换运动方式\nD) 少吃饭\n\
             KEY: A\nWHY: Bài nói tập cùng bạn bè thì dễ duy trì hơn (和朋友一起运动).",
        ),
        tokens: (220, 70),
        ..K
    },
    Kind {
        key: "l_short",
        heard: true,
        hint: "Nghe từng đoạn ngắn rồi chọn đáp án đúng.",
        brief: "For each question, SAY with what is heard: a short announcement, or a short dialogue whose turns start with \
                M: and W: (男： and 女： in Chinese); then the question and its options.",
        en: (
            "Exam: VSTEP Listening. Task: Part 1: short announcements, messages and instructions, one question each\nLevel: B1\nTopic: Du lịch\nWords: platform, delay\nQuestions: 2",
            "SAY: Attention, please. The train to Hanoi will now leave from platform four, not platform two, because of a short delay.\n\
             Q: Where will the train leave from?\nA) Platform two\nB) Platform four\nC) Platform six\nD) The main entrance\n\
             KEY: B\nWHY: Thông báo nói tàu sẽ khởi hành từ sân ga số bốn (platform four).\n\
             SAY: Before you start the machine, please put on your safety glasses and check that the door is closed.\n\
             Q: What should people do first?\nA) Turn on the machine\nB) Open the door\nC) Put on safety glasses\nD) Call the manager\n\
             KEY: C\nWHY: Hướng dẫn yêu cầu đeo kính bảo hộ trước khi bật máy.",
        ),
        zh: (
            "Exam: HSK Listening. Task: a short dialogue heard and a question with three options\nLevel: HSK 2\nTopic: Du lịch\nWords: 机场, 飞机\nQuestions: 2",
            "SAY: 男：你明天几点去机场？ 女：我早上八点出发，飞机十点起飞。\n\
             Q: 飞机几点起飞？\nA) 八点\nB) 九点\nC) 十点\n\
             KEY: C\nWHY: Người nữ nói máy bay cất cánh lúc mười giờ (十点起飞).\n\
             SAY: 女：你怎么不吃了？ 男：我已经吃饱了，这个菜太好吃了。\n\
             Q: 男的为什么不吃了？\nA) 菜不好吃\nB) 吃饱了\nC) 没有时间\n\
             KEY: B\nWHY: Người nam nói đã no rồi (吃饱了).",
        ),
        tokens: (0, 110),
        ..K
    },
    Kind {
        key: "l_photo",
        heard_options: true,
        hint: "Đọc mô tả bức tranh, nghe bốn câu A, B, C, D rồi chọn câu tả đúng.",
        brief: "Each Q describes a photo in one Vietnamese sentence; the four options are short English statements that will be \
                read aloud, only one of them true of the photo.",
        en: (
            "Exam: TOEIC Listening. Task: Part 1 Photographs\nLevel: B1\nTopic: Công việc văn phòng\nWords: document, printer\nQuestions: 2",
            "Q: Một người phụ nữ đang lấy giấy ra khỏi máy in trong văn phòng.\nA) She's repairing a computer.\nB) She's taking papers from a printer.\nC) She's opening a window.\nD) She's talking on the phone.\n\
             KEY: B\nWHY: Tranh tả người phụ nữ lấy giấy từ máy in (taking papers from a printer).\n\
             Q: Hai người đàn ông đang xem một tài liệu trên bàn họp.\nA) The men are looking at a document.\nB) The men are leaving the room.\nC) The table is being cleaned.\nD) One man is writing on a board.\n\
             KEY: A\nWHY: Hai người đàn ông đang cùng xem một tài liệu (looking at a document).",
        ),
        tokens: (0, 90),
        ..K
    },
    Kind {
        key: "l_response",
        heard_options: true,
        hint: "Nghe câu hỏi và ba câu đáp A, B, C rồi chọn câu đáp phù hợp.",
        brief: "For each question, SAY with a spoken question or statement, then three short spoken responses as options, \
                only one of them a fitting reply.",
        en: (
            "Exam: TOEIC Listening. Task: Part 2 Question-Response, at work\nLevel: B1\nTopic: Cuộc họp\nWords: client, report\nQuestions: 2",
            "SAY: When does the meeting with the clients start?\nA) In the main conference room.\nB) At ten thirty.\nC) Yes, they enjoyed it.\n\
             KEY: B\nWHY: Câu hỏi \"When\" hỏi thời gian, nên \"At ten thirty\" là câu đáp đúng.\n\
             SAY: Could you send me the sales report?\nA) Sure, I'll email it now.\nB) The sales were good.\nC) No, I didn't buy it.\n\
             KEY: A\nWHY: Đây là lời nhờ gửi báo cáo, \"Sure, I'll email it now\" đáp đúng ý.",
        ),
        tokens: (0, 70),
        ..K
    },
    Kind {
        key: "l_judge",
        heard: true,
        fixed: &["对", "错"],
        hint: "Nghe câu rồi cho biết câu in ra đúng (对) hay sai (错).",
        brief: "For each question, SAY with a sentence that is heard, Q with a statement about it, and KEY 对 when the statement \
                is true or 错 when it is false; about half of them false.",
        zh: (
            "Exam: HSK Listening. Task: a sentence heard and a statement to judge true or false\nLevel: HSK 2\nTopic: Ăn uống\nWords: 米饭, 喜欢\nQuestions: 2",
            "SAY: 我很喜欢吃米饭，每天中午都吃。\nQ: 他喜欢吃米饭。\nKEY: 对\nWHY: Câu nghe nói \"我很喜欢吃米饭\" (tôi rất thích ăn cơm).\n\
             SAY: 今天太冷了，我们不去公园了。\nQ: 他们今天去公园。\nKEY: 错\nWHY: Câu nghe nói hôm nay lạnh quá nên không đi công viên nữa.",
        ),
        tokens: (0, 60),
        ..K
    },
    Kind {
        key: "r_choice",
        body: Body::Text,
        hint: "Đọc bài rồi chọn đáp án đúng.",
        brief: "TEXT with a passage, then questions on it with their options.",
        en: (
            "Exam: TOEIC Reading. Task: Part 7 Reading Comprehension: an email, advertisement, article or notice\nLevel: B1\nTopic: Công việc văn phòng\nWords: schedule, flexible\nQuestions: 2",
            "TEXT: To all staff: From 1 June, the office will try a flexible schedule. You may start work at any time between 7 and \
             10 a.m., as long as you work eight hours a day. Team meetings will stay on Monday mornings at 10:30. Please tell your \
             manager your usual start time by 20 May.\n\
             Q: What will change on 1 June?\nA) The office will move\nB) Staff can choose when to start work\nC) Meetings will be on Fridays\nD) Working hours will be shorter\n\
             KEY: B\nWHY: Thông báo nói nhân viên được bắt đầu làm việc từ 7 đến 10 giờ sáng (flexible schedule).\n\
             Q: What must staff do by 20 May?\nA) Tell their manager their start time\nB) Attend a meeting\nC) Work eight extra hours\nD) Sign a new contract\n\
             KEY: A\nWHY: \"Please tell your manager your usual start time by 20 May\".",
        ),
        zh: (
            "Exam: HSK Reading. Task: a short text with questions\nLevel: HSK 3\nTopic: Sức khỏe\nWords: 跑步, 精神\nQuestions: 2",
            "TEXT: 小王每天早上六点起床，先跑步半个小时，然后吃早饭。他说运动以后一天都很有精神，工作也更认真了。\n\
             Q: 小王起床以后先做什么？\nA) 吃早饭\nB) 跑步\nC) 上班\n\
             KEY: B\nWHY: Bài nói Tiểu Vương dậy rồi chạy bộ trước (先跑步), sau đó mới ăn sáng.\n\
             Q: 运动以后小王觉得怎么样？\nA) 很累\nB) 很饿\nC) 很有精神\n\
             KEY: C\nWHY: \"运动以后一天都很有精神\" nghĩa là sau khi tập thấy rất khỏe khoắn.",
        ),
        tokens: (300, 80),
        ..K
    },
    Kind {
        key: "r_tfng",
        body: Body::Text,
        fixed: &["TRUE", "FALSE", "NOT GIVEN"],
        hint: "Đọc bài rồi cho biết mỗi câu là TRUE (đúng), FALSE (sai) hay NOT GIVEN (bài không nói).",
        brief: "TEXT with a passage, then statements as Q, each with KEY TRUE, FALSE or NOT GIVEN; use all three.",
        en: (
            "Exam: IELTS Reading. Task: an academic passage of about 250 words with statements to judge\nLevel: B2\nTopic: Du lịch\nWords: trade, heritage\nQuestions: 3",
            "TEXT: The town of Hoi An was an important trading port from the fifteenth to the nineteenth century. Merchants from \
             Japan, China and Europe came to buy silk and ceramics. When the river became too shallow for large ships, trade moved \
             to Da Nang. Today the old town is a World Heritage Site and attracts millions of visitors every year.\n\
             Q: Hoi An was a trading port for about four hundred years.\nKEY: TRUE\nWHY: Từ thế kỷ 15 đến thế kỷ 19 là khoảng bốn trăm năm.\n\
             Q: Trade moved to Da Nang because of a war.\nKEY: FALSE\nWHY: Bài nói thương mại chuyển đi vì sông quá cạn, không phải vì chiến tranh.\n\
             Q: Most visitors to Hoi An come from Japan.\nKEY: NOT GIVEN\nWHY: Bài không nói du khách đến từ đâu.",
        ),
        tokens: (320, 50),
        ..K
    },
    Kind {
        key: "r_headings",
        mode: Mode::Match,
        body: Body::Text,
        hint: "Chọn tiêu đề phù hợp cho mỗi đoạn văn.",
        brief: "TEXT with a passage of lettered paragraphs (A. B. C.), each on a line of its own; LIST with more headings than \
                paragraphs, numbered i. ii. iii. and separated by |; one Q per paragraph whose KEY is the number of its heading.",
        en: (
            "Exam: IELTS Reading. Task: an academic passage of four or five lettered paragraphs with a list of headings\nLevel: B2\nTopic: Công việc văn phòng\nWords: productive, colleague\nQuestions: 3",
            "TEXT: A. Many people now work from home at least one day a week. Most of them say they are more productive without \
             the noise of the office.\n\
             B. However, managers worry that teams lose contact. New staff in particular find it hard to learn from colleagues \
             they rarely meet.\n\
             C. Some companies have found a middle way: everyone comes in on the same two days, and meetings are only held then.\n\
             LIST: i. A shared timetable | ii. Getting more done at home | iii. The cost of offices | iv. A risk for teamwork\n\
             Q: Paragraph A\nKEY: ii\nWHY: Đoạn A nói làm ở nhà làm được nhiều việc hơn.\n\
             Q: Paragraph B\nKEY: iv\nWHY: Đoạn B nói các nhóm mất liên lạc, nhân viên mới khó học hỏi.\n\
             Q: Paragraph C\nKEY: i\nWHY: Đoạn C nói mọi người cùng đến văn phòng vào hai ngày cố định.",
        ),
        tokens: (380, 40),
        ..K
    },
    Kind {
        key: "r_complete",
        mode: Mode::Text,
        body: Body::Text,
        hint: "Hoàn thành câu bằng từ lấy từ bài đọc, không quá hai từ.",
        brief: "TEXT with a passage, then sentences to complete as Q with ____; each KEY is one or two words copied exactly from \
                the passage.",
        en: (
            "Exam: IELTS Reading. Task: an academic passage of about 250 words with sentences to complete in no more than two words\nLevel: B2\nTopic: Môi trường\nWords: pollinate, decline\nQuestions: 2",
            "TEXT: Bees pollinate about a third of the crops we eat. In recent years their numbers have declined sharply, mainly \
             because of chemicals used on farms and the loss of wild flowers. Some cities now plant flower strips along roads to \
             give bees food and shelter.\n\
             Q: Bees help to produce about ____ of our crops.\nKEY: a third\nWHY: Bài nói ong thụ phấn cho khoảng một phần ba (a third) cây trồng.\n\
             Q: Cities plant ____ along roads to help bees.\nKEY: flower strips\nWHY: \"plant flower strips along roads\".",
        ),
        tokens: (300, 50),
        ..K
    },
    Kind {
        key: "r_sentence",
        hint: "Chọn từ hoặc cụm từ đúng để hoàn thành câu.",
        brief: "Each Q is one sentence with ____ for a missing word, then four options of which one is right.",
        en: (
            "Exam: TOEIC Reading. Task: Part 5 Incomplete Sentences: grammar and business vocabulary\nLevel: B1\nTopic: Công việc văn phòng\nWords: invoice, deliver\nQuestions: 2",
            "Q: The new printer works much ____ than the old one.\nA) fast\nB) faster\nC) fastest\nD) more fast\n\
             KEY: B\nWHY: Có \"than\" nên dùng so sánh hơn \"faster\".\n\
             Q: Please send the invoice ____ the end of the month.\nA) by\nB) on\nC) at\nD) since\n\
             KEY: A\nWHY: \"by the end of the month\" nghĩa là chậm nhất là cuối tháng.",
        ),
        tokens: (0, 70),
        ..K
    },
    Kind {
        key: "r_text",
        body: Body::Text,
        hint: "Chọn đáp án đúng cho mỗi chỗ trống được đánh số trong bài.",
        brief: "TEXT with a short text that has numbered blanks (1) ____, (2) ____ and so on, then one Q per blank, written as its \
                number, with four options.",
        en: (
            "Exam: TOEIC Reading. Task: Part 6 Text Completion: an email, notice or memo with four blanks\nLevel: B1\nTopic: Mua sắm\nWords: order, stock\nQuestions: 2",
            "TEXT: Dear Ms. Lee, thank you for your order of twenty office chairs. Unfortunately, the model you chose is (1) ____ \
             out of stock. We expect new chairs to (2) ____ next Monday, and we will send yours the same day.\n\
             Q: (1)\nA) currently\nB) current\nC) currency\nD) currents\n\
             KEY: A\nWHY: Cần trạng từ đứng trước \"out of stock\": \"currently\" (hiện đang).\n\
             Q: (2)\nA) arrival\nB) arrive\nC) arrived\nD) arriving\n\
             KEY: B\nWHY: Sau \"expect ... to\" dùng động từ nguyên mẫu \"arrive\".",
        ),
        zh: (
            "Exam: HSK Reading. Task: a passage with numbered blanks and four options each\nLevel: HSK 5\nTopic: Ăn uống\nWords: 咖啡, 建议\nQuestions: 2",
            "TEXT: 很多人觉得早上喝一杯咖啡能让自己更有精神。（1）____，喝太多咖啡对身体并不好。医生建议每天最好（2）____两杯。\n\
             Q: (1)\nA) 因此\nB) 不过\nC) 而且\nD) 于是\n\
             KEY: B\nWHY: Hai vế trái ngược nhau nên dùng \"不过\" (tuy nhiên).\n\
             Q: (2)\nA) 不超过\nB) 不得不\nC) 来不及\nD) 舍不得\n\
             KEY: A\nWHY: \"不超过两杯\" nghĩa là không quá hai cốc.",
        ),
        tokens: (220, 60),
        ..K
    },
    Kind {
        key: "r_fill",
        mode: Mode::Match,
        hint: "Chọn từ trong khung để điền vào chỗ trống.",
        brief: "LIST with one more word than there are questions, each word after a letter A B C D E F and separated by |; each Q \
                is a sentence with （ ） for a missing word, and its KEY is the letter of that word.",
        zh: (
            "Exam: HSK Reading. Task: sentences with a blank and a bank of words\nLevel: HSK 3\nTopic: Trường học\nWords: 安静, 打算\nQuestions: 2",
            "LIST: A 安静 | B 打算 | C 而且\n\
             Q: 图书馆里很（ ），大家都在看书。\nKEY: A\nWHY: \"安静\" (yên tĩnh) hợp với thư viện, mọi người đang đọc sách.\n\
             Q: 你（ ）什么时候去中国？\nKEY: B\nWHY: \"打算\" (dự định) đi với \"什么时候去\".",
        ),
        tokens: (40, 50),
        ..K
    },
    Kind {
        key: "r_order",
        mode: Mode::Text,
        hint: "Sắp xếp ba câu A, B, C theo đúng thứ tự, ví dụ BAC.",
        brief: "Each Q is the words 排列顺序, then three sentences as options A) B) C) given out of order, and KEY is their right \
                order as three letters, such as BAC.",
        zh: (
            "Exam: HSK Reading. Task: three sentences to put in order\nLevel: HSK 3\nTopic: Sức khỏe\nWords: 感冒, 上班\nQuestions: 1",
            "Q: 排列顺序\nA) 所以我今天没去上班\nB) 昨天晚上我感冒了\nC) 现在已经好多了\n\
             KEY: BAC\nWHY: Trước tiên bị cảm (B), nên hôm nay không đi làm (A), bây giờ đã đỡ nhiều (C).",
        ),
        tokens: (0, 80),
        ..K
    },
    Kind {
        key: "r_error",
        hint: "Chọn câu có lỗi ngữ pháp.",
        brief: "Each Q is the words 选出有语病的一项 with four sentences as options, exactly one of them wrong, and KEY is that one.",
        zh: (
            "Exam: HSK Reading. Task: four sentences, one of them with a grammar error\nLevel: HSK 6\nTopic: Công việc\nWords: 通过, 认识\nQuestions: 1",
            "Q: 选出有语病的一项\nA) 他把作业写完了。\nB) 我们对这个问题进行了讨论。\nC) 通过这次活动，使我认识了很多朋友。\nD) 这本书我已经看了三遍。\n\
             KEY: C\nWHY: \"通过……使……\" làm câu mất chủ ngữ; bỏ \"使\" hoặc bỏ \"通过\" thì câu đúng.",
        ),
        tokens: (0, 110),
        ..K
    },
    Kind {
        key: "w_order",
        mode: Mode::Text,
        hint: "Sắp xếp các từ thành câu hoàn chỉnh rồi gõ lại câu.",
        brief: "Each Q gives the words of one sentence out of order, separated by /, and KEY is the whole sentence in the right \
                order with its punctuation.",
        zh: (
            "Exam: HSK Writing. Task: words to put in order as a sentence\nLevel: HSK 3\nTopic: Mua sắm\nWords: 买, 书\nQuestions: 2",
            "Q: 一本 / 我 / 买了 / 昨天 / 书\nKEY: 我昨天买了一本书。\nWHY: Thời gian 昨天 đứng sau chủ ngữ, rồi đến động từ 买了 và tân ngữ 一本书.\n\
             Q: 踢 / 弟弟 / 足球 / 喜欢\nKEY: 弟弟喜欢踢足球。\nWHY: Chủ ngữ 弟弟, động từ 喜欢, rồi cụm 踢足球.",
        ),
        tokens: (0, 60),
        ..K
    },
    Kind {
        key: "w_hanzi",
        mode: Mode::Text,
        hint: "Viết chữ Hán đúng với phiên âm trong ngoặc.",
        brief: "Each Q is a sentence where one character is replaced by its pinyin in brackets, and KEY is that character.",
        zh: (
            "Exam: HSK Writing. Task: a sentence with one character given in pinyin, to write as a character\nLevel: HSK 3\nTopic: Ăn uống\nWords: 茶, 水\nQuestions: 2",
            "Q: 我想喝一杯（chá）。\nKEY: 茶\nWHY: \"chá\" là chữ 茶 (trà).\n\
             Q: 天气太热了，我要多喝（shuǐ）。\nKEY: 水\nWHY: \"shuǐ\" là chữ 水 (nước).",
        ),
        tokens: (0, 40),
        ..K
    },
    Kind {
        key: "w_task",
        mode: Mode::Write,
        hint: "Viết bài theo yêu cầu, chú ý số từ tối thiểu.",
        brief: "One writing task after TASK:, worded as the official exam words it, on the topic given. When the task needs \
                figures, give them as a small table in text, one row per line. Write one TASK for each question asked.",
        en: (
            "Exam: IELTS Writing. Task: Academic Writing Task 1: a small table of figures given in text, to summarise and compare in at least 150 words\nLevel: B2\nTopic: Du lịch\nWords: increase, compared\nQuestions: 1",
            "TASK: The table below shows the number of international visitors to three Vietnamese cities in 2015 and 2023, in millions.\n\
             City | 2015 | 2023\nHanoi | 3.5 | 6.2\nDa Nang | 1.2 | 3.4\nHoi An | 1.0 | 1.8\n\
             Summarise the information by selecting and reporting the main features, and make comparisons where relevant.",
        ),
        zh: (
            "Exam: HSK Writing. Task: write a short text of about 80 characters using five given words\nLevel: HSK 5\nTopic: Du lịch\nWords: 旅游, 难忘\nQuestions: 1",
            "TASK: 请结合下列词语（要全部使用，顺序不分先后），写一篇80字左右的短文：旅游、计划、照片、难忘、朋友",
        ),
        tokens: (60, 160),
        ..K
    },
    Kind {
        key: "s_task",
        mode: Mode::Speak,
        hint: "Đọc câu hỏi, bấm Ghi âm rồi trả lời bằng giọng nói.",
        brief: "The questions an examiner asks, each after Q: on a line of its own.",
        en: (
            "Exam: IELTS Speaking. Task: Part 1: short questions about the candidate's life, work, study and interests\nLevel: B1\nTopic: Giải trí\nWords: weekend, relax\nQuestions: 3",
            "Q: Do you work or are you a student?\nQ: What do you usually do at the weekend?\nQ: How do you like to relax after a busy day?",
        ),
        zh: (
            "Exam: HSKK. Task: questions about daily life, to answer in at least five sentences\nLevel: HSK 2\nTopic: Gia đình\nWords: 家人, 周末\nQuestions: 2",
            "Q: 请介绍一下你的家人。\nQ: 你周末一般做什么？",
        ),
        tokens: (0, 30),
        ..K
    },
    Kind {
        key: "s_cue",
        mode: Mode::Speak,
        hint: "Đọc đề, dùng thời gian chuẩn bị để ghi ý, rồi nói liền mạch đến hết giờ.",
        brief: "One task card after Q:, its points on the lines below it, as the official exam words it.",
        en: (
            "Exam: IELTS Speaking. Task: Part 2: a cue card with a topic and four points to cover\nLevel: B2\nTopic: Du lịch\nWords: memorable, journey\nQuestions: 1",
            "Q: Describe a trip you enjoyed.\nYou should say:\n- where you went\n- who you went with\n- what you did there\nand explain why you enjoyed it.",
        ),
        tokens: (0, 80),
        ..K
    },
    Kind {
        key: "s_read",
        mode: Mode::Speak,
        hint: "Đọc to đoạn văn sau thời gian chuẩn bị; máy chấm phát âm theo từng từ.",
        brief: "One passage to read aloud after Q:, on a single line.",
        en: (
            "Exam: TOEIC Speaking. Task: Questions 1-2 Read a text aloud: an announcement or advertisement of about 50 words\nLevel: B1\nTopic: Ngân hàng\nWords: balance, assistant\nQuestions: 1",
            "Q: Thank you for calling City Bank. Our offices are open from nine to five, Monday to Friday. To check your balance, press one. To speak to an assistant, please stay on the line.",
        ),
        zh: (
            "Exam: HSKK. Task: a passage of about 120 characters to read aloud\nLevel: HSK 5\nTopic: Thành phố\nWords: 地铁, 方便\nQuestions: 1",
            "Q: 这几年，我们城市的变化非常大。以前去市中心要坐一个多小时的公共汽车，现在有了地铁，二十分钟就到了。城市里还多了很多公园，周末很多人带着孩子去散步。大家都说，生活越来越方便了。",
        ),
        tokens: (0, 120),
        ..K
    },
    Kind {
        key: "s_repeat",
        mode: Mode::Speak,
        heard: true,
        hint: "Nghe câu rồi nói lại đúng câu đó; máy chấm theo từng âm tiết và thanh điệu.",
        brief: "Each SAY is one short sentence to be heard and repeated.",
        zh: (
            "Exam: HSKK. Task: short sentences to repeat after hearing them\nLevel: HSK 2\nTopic: Đi lại\nWords: 地铁, 上班\nQuestions: 2",
            "SAY: 我每天坐地铁去上班。\nSAY: 这个周末你有时间吗？",
        ),
        tokens: (0, 25),
        ..K
    },
    Kind {
        key: "s_answer",
        mode: Mode::Speak,
        heard: true,
        hint: "Nghe câu hỏi rồi trả lời bằng một câu.",
        brief: "Each SAY is one short question to be heard and answered.",
        zh: (
            "Exam: HSKK. Task: short questions heard, to answer in one sentence\nLevel: HSK 2\nTopic: Ăn uống\nWords: 茶, 咖啡\nQuestions: 2",
            "SAY: 你喜欢喝茶还是喝咖啡？\nSAY: 你今天早上吃了什么？",
        ),
        tokens: (0, 25),
        ..K
    },
    Kind {
        key: "s_describe",
        mode: Mode::Speak,
        hint: "Đọc mô tả bức tranh rồi nói về bức tranh bằng tiếng Trung.",
        brief: "Each Q describes a picture in one or two Vietnamese sentences for the learner to talk about in Chinese.",
        zh: (
            "Exam: HSKK. Task: a picture, described in Vietnamese, to talk about\nLevel: HSK 4\nTopic: Gia đình\nWords: 餐厅, 庆祝\nQuestions: 1",
            "Q: Tranh: một gia đình đang ăn tối ở nhà hàng, giữa bàn có bánh sinh nhật, đứa bé đang cười rất vui.",
        ),
        tokens: (0, 60),
        ..K
    },
    Kind {
        key: "s_retell",
        mode: Mode::Speak,
        body: Body::Text,
        heard: true,
        hint: "Nghe câu chuyện rồi kể lại bằng lời của bạn.",
        brief: "TEXT with a short story of about 100 characters to be heard, then Q: 请复述你听到的内容 once for each retelling asked.",
        zh: (
            "Exam: HSKK. Task: a short story heard, to retell\nLevel: HSK 5\nTopic: Công việc\nWords: 迟到, 堵车\nQuestions: 1",
            "TEXT: 小李第一天上班就迟到了。那天早上路上堵车，他很着急，干脆下车跑到了公司。经理不但没有批评他，还说他很有责任心。\n\
             Q: 请复述你听到的内容。",
        ),
        tokens: (160, 20),
        ..K
    },
];

fn kind(key: &str) -> &'static Kind {
    KINDS.iter().find(|k| k.key == key).unwrap_or(&KINDS[0])
}

/// The skill's name in Vietnamese.
pub fn skill_vi(skill: &str) -> &'static str {
    match skill {
        "listening" => "Nghe",
        "reading" => "Đọc",
        "writing" => "Viết",
        _ => "Nói",
    }
}

// The goal of a roadmap.

/// Numbers written in `text`, with a decimal point or comma.
fn numbers(text: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars().chain([' ']) {
        if c.is_ascii_digit() || (matches!(c, '.' | ',') && !current.is_empty() && !current.contains('.')) {
            current.push(if c == ',' { '.' } else { c });
        } else if !current.is_empty() {
            if let Ok(n) = current.trim_end_matches('.').parse() {
                out.push(n);
            }
            current.clear();
        }
    }
    out
}

/// The first CEFR level written in `text` as a word of its own, as an index of `CEFR`.
fn cefr_in(text: &str) -> Option<usize> {
    let up: Vec<char> = text.to_uppercase().chars().collect();
    (0..up.len().saturating_sub(1)).find_map(|i| {
        let alone = |j: Option<&char>| j.is_none_or(|c| !c.is_alphanumeric());
        let level = CEFR.iter().position(|l| l.chars().eq([up[i], up[i + 1]]))?;
        (alone(i.checked_sub(1).and_then(|j| up.get(j))) && alone(up.get(i + 2))).then_some(level)
    })
}

/// The HSK level written after "HSK" in `text`.
fn hsk_in(text: &str) -> Option<usize> {
    let up = text.to_uppercase();
    let at = up.find("HSK")?;
    numbers(&up[at + 3..]).first().map(|n| *n as usize).filter(|n| (1..=9).contains(n))
}

/// A roadmap is hard when its goal names an exam, or a level of B2 or HSK 4 and above: its stages then end with tests.
pub fn is_hard(purpose: &str) -> bool {
    let up = ai::fold_label(purpose);
    ["IELTS", "TOEIC", "VSTEP", "HSK"].iter().any(|e| up.contains(e)) || cefr_in(purpose).is_some_and(|l| l >= 3)
}

fn round_half(x: f64) -> f64 {
    (x * 2.0 + 1e-9).round() / 2.0
}

/// What a test is for: its format, whether it is a full mock, the level its content is written at, and the pass
/// mark on its scale.
pub struct Plan {
    pub format: &'static Format,
    pub full: bool,
    /// "B2" or "HSK 4".
    pub level: String,
    pub pass: f64,
    pub skills: Vec<&'static str>,
    pub topics: Vec<String>,
    pub title: String,
}

/// The CEFR level content is written at for an IELTS band.
fn level_of_band(band: f64) -> &'static str {
    match band {
        b if b >= 8.5 => "C2",
        b if b >= 7.0 => "C1",
        b if b >= 5.5 => "B2",
        b if b >= 4.0 => "B1",
        _ => "A2",
    }
}

/// The CEFR level of a TOEIC listening and reading score, from ETS's mapping.
fn level_of_toeic(score: f64) -> &'static str {
    match score {
        s if s >= 945.0 => "C1",
        s if s >= 785.0 => "B2",
        s if s >= 550.0 => "B1",
        s if s >= 225.0 => "A2",
        _ => "A1",
    }
}

/// The VSTEP level of a score out of 10, empty below B1.
fn level_of_vstep(score: f64) -> &'static str {
    match score {
        s if s >= 8.5 => "C1",
        s if s >= 6.0 => "B2",
        s if s >= 4.0 => "B1",
        _ => "",
    }
}

/// The HSK format of a level.
fn hsk_format(level: usize) -> &'static Format {
    match level {
        0..=2 => &HSK12,
        3 => &HSK3,
        4 => &HSK4,
        5 => &HSK5,
        _ => &HSK6,
    }
}

/// The skills a stage names: "Nghe, Đọc" or "Listening, Writing".
fn skills_of(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for token in text.split([',', ';', '/', '，', '、', '+']).map(ai::fold_label) {
        let has = |starts: &[&str]| starts.iter().any(|s| token.starts_with(s));
        let skill = if has(&["NGHE", "LISTEN"]) {
            "listening"
        } else if has(&["DOC", "READ", "TUVUNG", "VOCAB"]) {
            "reading"
        } else if has(&["VIET", "WRIT"]) {
            "writing"
        } else if has(&["NOI", "SPEAK", "PHATAM", "PRONUN", "GIAOTIEP"]) {
            "speaking"
        } else {
            continue;
        };
        if !out.contains(&skill) {
            out.push(skill);
        }
    }
    out
}

/// The test of a stage of a roadmap (the last one of a hard roadmap being a full mock), or with no stage, the
/// optional full mock at the goal. The pass mark climbs from where the learner starts to the goal stage by stage.
pub fn plan(r: &roadmap::Roadmap, stage: Option<usize>, zh: bool) -> Plan {
    let n = r.steps.len().max(1);
    let stage = stage.filter(|&i| i < r.steps.len());
    let full = stage.is_none_or(|i| i + 1 == n);
    let t = stage.map_or(1.0, |i| (i + 1) as f64 / n as f64);
    let up = ai::fold_label(&r.purpose);
    let start = cefr_in(&r.level);
    let climb = |from: f64, to: f64| from + (to - from) * t;
    let (format, level, pass): (&'static Format, String, f64) = if zh {
        let from = hsk_in(&r.level).map_or(1, |l| (l + 1).min(9));
        let goal = hsk_in(&r.purpose).unwrap_or(from.clamp(1, 6)).max(from);
        let at = (climb(from as f64, goal as f64).round() as usize).clamp(1, 9);
        (hsk_format(at), format!("HSK {at}"), 60.0)
    } else if up.contains("IELTS") {
        let goal = numbers(&r.purpose).into_iter().find(|n| (3.0..=9.0).contains(n)).map_or(6.5, round_half);
        let from = start.map_or(goal - 1.5, |l| [2.5, 3.5, 4.5, 5.5, 7.0, 8.0][l]).clamp(2.0, goal);
        let pass = round_half(climb(from, goal));
        (&IELTS, level_of_band(pass).into(), pass)
    } else if up.contains("TOEIC") {
        let goal = numbers(&r.purpose).into_iter().find(|n| (10.0..=990.0).contains(n)).unwrap_or(600.0);
        let from = start.map_or(goal * 0.7, |l| [120.0, 225.0, 550.0, 785.0, 945.0, 945.0][l]).min(goal);
        let pass = (climb(from, goal) / 5.0).round() * 5.0;
        (&TOEIC, level_of_toeic(pass).into(), pass)
    } else if up.contains("VSTEP") {
        let low = r.purpose.to_lowercase();
        let step = low.find("bậc").or_else(|| low.find("bac")).and_then(|at| numbers(&low[at..]).first().map(|n| *n as usize));
        let named = cefr_in(&r.purpose).or(step.map(|b| b.clamp(3, 5) - 1));
        let goal = match named.unwrap_or(2) {
            0..=2 => 4.0,
            3 => 6.0,
            _ => 8.5,
        };
        let pass = round_half(climb((goal - 2.0f64).max(2.0), goal));
        (&VSTEP, level_of_vstep(pass).max("B1").into(), pass)
    } else {
        let goal = cefr_in(&r.purpose).unwrap_or_else(|| start.map_or(2, |l| (l + 1).min(5)));
        let from = start.unwrap_or(goal.saturating_sub(1)).min(goal);
        let at = climb(from as f64, goal as f64).round() as usize;
        (&CEFR_TEST, CEFR[at.min(5)].into(), 60.0)
    };
    let mut skills: Vec<&'static str> = if full {
        format.sections.iter().map(|s| s.skill).collect()
    } else {
        let named = stage.map(|i| skills_of(&r.steps[i].skills)).unwrap_or_default();
        format.sections.iter().map(|s| s.skill).filter(|s| named.contains(s)).collect()
    };
    if skills.is_empty() {
        skills = vec!["listening", "reading"];
    }
    let mut topics: Vec<String> = Vec::new();
    let steps: Vec<&roadmap::Step> = match stage {
        Some(i) if !full => vec![&r.steps[i]],
        _ => r.steps.iter().collect(),
    };
    for t in steps.iter().flat_map(|s| &s.topics) {
        if !topics.iter().any(|x| x.to_lowercase() == t.to_lowercase()) {
            topics.push(t.clone());
        }
    }
    let name = exam_name(format, &level);
    let title = match stage {
        Some(i) if !full => format!("Kiểm tra chặng {}: {}", i + 1, r.steps[i].title),
        _ => format!("Thi thử {name}"),
    };
    Plan { format, full, level, pass, skills, topics, title }
}

/// "IELTS", "CEFR B1", "HSK 4 và HSKK trung cấp".
fn exam_name(f: &Format, level: &str) -> String {
    match f.id {
        "cefr" => format!("CEFR {level}"),
        id if id.starts_with("hsk") => {
            let band = match f.sections.last().map(|s| s.name) {
                Some("HSKK 初级") => "sơ cấp",
                Some("HSKK 中级") => "trung cấp",
                _ => "cao cấp",
            };
            format!("{level} và HSKK {band}")
        }
        _ => f.name.into(),
    }
}

/// The pass mark as the learner reads it.
pub fn pass_text(format_id: &str, pass: f64, level: &str) -> String {
    match format_id {
        "ielts" => format!("band {pass:.1}"),
        "toeic" => format!("{pass:.0}/990"),
        "vstep" => format!("{pass:.1}/10 ({})", level_of_vstep(pass).max("B1")),
        "cefr" => format!("60% (đạt {level})"),
        _ => "60% số điểm".into(),
    }
}

/// The parts a plan takes from each of its sections, with their sets and items.
fn included(p: &Plan) -> Vec<(&'static Section, Vec<(&'static Part, usize, usize)>)> {
    p.format
        .sections
        .iter()
        .filter(|s| p.skills.contains(&s.skill))
        .map(|s| {
            let parts = s.parts.iter().map(|part| (part, if p.full { part.full } else { part.mini })).filter(|(_, (sets, per))| sets * per > 0);
            (s, parts.map(|(part, (sets, per))| (part, sets, per)).collect())
        })
        .collect()
}

/// The fewest words of a writing task: the part's, or for the CEFR test the level's, as the Cambridge exams of
/// that level ask.
fn min_words(part: &Part, format_id: &str, level: &str) -> usize {
    if format_id != "cefr" || part.kind != "w_task" {
        return part.words;
    }
    [25, 35, 100, 140, 220, 240][CEFR.iter().position(|l| *l == level).unwrap_or(2)]
}

/// What the page shows before a test: its sections with their time and items, and the pass mark.
pub fn plan_view(p: &Plan) -> Value {
    let sections: Vec<Value> = included(p)
        .iter()
        .map(|(s, parts)| {
            let items: usize = parts.iter().map(|(_, sets, per)| sets * per).sum();
            let secs: u32 = parts.iter().map(|(part, sets, per)| (sets * per) as u32 * (part.secs + part.prep)).sum();
            json!({ "skill": s.skill, "name": s.name, "vi": skill_vi(s.skill), "items": items, "minutes": secs.div_ceil(60),
                    "parts": parts.iter().map(|(part, ..)| part.label).collect::<Vec<_>>() })
        })
        .collect();
    json!({ "format": p.format.id, "name": exam_name(p.format, &p.level), "title": p.title, "full": p.full, "level": p.level,
            "pass": p.pass, "pass_text": pass_text(p.format.id, p.pass, &p.level), "plays": p.format.plays, "sections": sections,
            "topics": p.topics })
}

// The paper.

/// A line of a dialogue: who says it ("M" or "W") and what.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Line {
    pub who: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Item {
    /// What is heard for this item.
    pub say: String,
    /// What is printed: the question, the statement, the task.
    pub q: String,
    pub options: Vec<String>,
    /// The letter of the right option, the label of the right entry of the list, or the words to type, other ways
    /// of writing them after a slash.
    pub key: String,
    /// Why, in Vietnamese.
    pub why: String,
}

/// A passage, talk or conversation with its items, as one call of the AI writes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Set {
    pub text: String,
    pub lines: Vec<Line>,
    /// The entries a match item picks from: headings, a word bank.
    pub list: Vec<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaperPart {
    pub kind: String,
    pub label: String,
    pub hint: String,
    pub mode: String,
    pub heard: bool,
    pub heard_options: bool,
    pub words: usize,
    pub weight: u32,
    pub prep: u32,
    pub secs: u32,
    pub sets: Vec<Set>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaperSection {
    pub skill: String,
    pub name: String,
    pub vi: String,
    pub seconds: u32,
    pub parts: Vec<PaperPart>,
}

/// A whole paper as it is kept: everything a page and the scoring need, so that later changes to the formats leave
/// it as it was written.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Paper {
    pub format: String,
    pub name: String,
    pub title: String,
    pub full: bool,
    pub level: String,
    pub plays: u32,
    pub topics: Vec<String>,
    /// Who wrote it.
    pub by: String,
    pub sections: Vec<PaperSection>,
}

/// A speaker's label, made "M" or "W".
fn speaker(line: &str) -> Option<(String, &str)> {
    let (who, text) = line.split_once([':', '：'])?;
    let who = match who.trim().trim_matches('*').trim().to_uppercase().as_str() {
        "M" | "MAN" | "男" | "B" => "M",
        "W" | "WOMAN" | "女" | "A" => "W",
        _ => return None,
    };
    let text = text.trim().trim_start_matches('*').trim();
    (!text.is_empty()).then(|| (who.to_owned(), text))
}

/// An option line, `A) text`, `B. text`, `(C) text`, `D：text`: its index and its text.
fn option(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start_matches(['(', '（']);
    let mut chars = line.char_indices();
    let (_, letter) = chars.next()?;
    let (at, mark) = chars.next()?;
    let index = "ABCDE".find(letter.to_ascii_uppercase())?;
    if !letter.is_ascii_alphabetic() || !matches!(mark, ')' | '.' | ':' | '）' | '．' | '：' | '、') {
        return None;
    }
    let text = line[at + mark.len_utf8()..].trim();
    (!text.is_empty()).then_some((index, text))
}

/// The label a line starts with and what follows it.
fn label(line: &str) -> Option<(&'static str, &str)> {
    let (head, rest) = line.split_once([':', '：'])?;
    let head = head.trim().trim_matches('*').trim();
    if head.chars().count() > 14 {
        return None;
    }
    let name = match ai::fold_label(head).as_str() {
        _ if head == "答案" => "KEY",
        "TEXT" | "PASSAGE" | "SCRIPT" | "TALK" | "STORY" | "BAIDOC" | "DOANVAN" => "TEXT",
        "Q" | "QUESTION" | "CAU" | "CAUHOI" | "TASK" | "DEBAI" | "STATEMENT" => "Q",
        "KEY" | "ANSWER" | "DAPAN" | "CORRECT" => "KEY",
        "WHY" | "GIAITHICH" | "EXPLANATION" | "REASON" | "LYDO" => "WHY",
        "LIST" | "HEADINGS" | "WORDBANK" | "BANK" => "LIST",
        "SAY" | "AUDIO" | "HEARD" | "NGHE" => "SAY",
        _ => return None,
    };
    Some((name, rest.trim().trim_matches('*').trim()))
}

/// A numbered line such as `3. What is ...`, as a question.
fn numbered(line: &str) -> Option<&str> {
    let (n, rest) = line.split_once(['.', ')'])?;
    (!n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()) && !rest.trim().is_empty()).then(|| rest.trim())
}

/// A set from the AI's lines. Unlabelled lines go on the field before them; markdown stars and numbering are
/// dropped. Nothing is checked here: `check` does that.
pub fn parse(text: &str) -> Set {
    #[derive(PartialEq)]
    enum Field {
        Text,
        Line,
        List,
        Say,
        Q,
        Why,
        None,
    }
    let mut set = Set::default();
    let mut item: Option<Item> = None;
    let mut field = Field::Text;
    let add = |to: &mut String, line: &str, sep: &str| {
        if !to.is_empty() {
            to.push_str(sep);
        }
        to.push_str(line);
    };
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches(['#', '>']).trim();
        if line.is_empty() || line.chars().all(|c| matches!(c, '-' | '*' | '=' | '_')) {
            continue;
        }
        if let Some(it) = item.as_mut().filter(|it| it.key.is_empty() && field != Field::Why) {
            if let Some((_, text)) = option(line) {
                it.options.push(text.to_owned());
                field = Field::None;
                continue;
            }
        }
        if let Some((name, value)) = label(line) {
            match name {
                "TEXT" => {
                    add(&mut set.text, value, "\n");
                    field = Field::Text;
                }
                "LIST" => {
                    set.list.extend(value.split(['|', '｜']).map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned));
                    field = Field::List;
                }
                "SAY" => {
                    let open = item.as_ref().is_some_and(|it| it.key.is_empty() && it.say.is_empty() && !it.q.is_empty());
                    if !open {
                        set.items.extend(item.take());
                        item = Some(Item::default());
                    }
                    if let Some(it) = item.as_mut() {
                        it.say = value.to_owned();
                    }
                    field = Field::Say;
                }
                "Q" => {
                    let open = item.as_ref().is_some_and(|it| it.key.is_empty() && it.q.is_empty() && !it.say.is_empty());
                    if !open {
                        set.items.extend(item.take());
                        item = Some(Item::default());
                    }
                    if let Some(it) = item.as_mut() {
                        it.q = value.to_owned();
                    }
                    field = Field::Q;
                }
                "KEY" => {
                    if let Some(it) = item.as_mut().filter(|it| it.key.is_empty()) {
                        it.key = value.to_owned();
                    }
                    field = Field::None;
                }
                _ => {
                    if let Some(it) = item.as_mut() {
                        it.why = value.to_owned();
                    }
                    field = Field::Why;
                }
            }
            continue;
        }
        if item.is_none() && set.items.is_empty() {
            if let Some((who, text)) = speaker(line) {
                set.lines.push(Line { who, text: text.to_owned() });
                field = Field::Line;
                continue;
            }
        }
        if let (Some(q), true) = (numbered(line), item.is_some() || !set.items.is_empty() || !set.lines.is_empty()) {
            set.items.extend(item.take());
            item = Some(Item { q: q.to_owned(), ..Item::default() });
            field = Field::Q;
            continue;
        }
        match (&field, item.as_mut()) {
            // A model's own heading before the first label, such as "Here is the test:".
            (Field::Text, _) if set.text.is_empty() && line.ends_with(':') => {}
            (Field::Text, _) => add(&mut set.text, line, "\n"),
            (Field::Line, _) => {
                if let Some(last) = set.lines.last_mut() {
                    add(&mut last.text, line, " ");
                }
            }
            (Field::List, _) => set.list.push(line.to_owned()),
            (Field::Say, Some(it)) => add(&mut it.say, line, " "),
            (Field::Q, Some(it)) => add(&mut it.q, line, "\n"),
            (Field::Why, Some(it)) => add(&mut it.why, line, " "),
            _ => {}
        }
    }
    set.items.extend(item);
    set
}

/// The label of a list entry, `iv` of `iv. A risk` or `C` of `C 而且`: lower case for numerals, upper for letters.
fn entry_label(entry: &str) -> String {
    let head: String = entry.trim().trim_start_matches('(').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if head.chars().all(|c| "ivxlIVXL".contains(c)) && head.len() > 1 || head == "i" {
        head.to_lowercase()
    } else {
        head.to_uppercase()
    }
}

/// The index of the right option from a key as a model writes it: a letter, the option itself, or for the fixed
/// options a word that means it.
fn choice_key(key: &str, options: &[String], fixed: &[&str]) -> Option<usize> {
    let key = key.trim().trim_start_matches(['(', '（']).trim();
    if !fixed.is_empty() {
        let folded = ai::fold_label(key);
        let index = if fixed[0] == "对" {
            match () {
                _ if key.contains(['对', '√', '✓']) || ["TRUE", "T", "DUNG", "YES"].contains(&folded.as_str()) => 0,
                _ if key.contains(['错', '×', '✗']) || ["FALSE", "F", "SAI", "NO"].contains(&folded.as_str()) => 1,
                _ => return None,
            }
        } else {
            match folded.as_str() {
                "TRUE" | "T" | "DUNG" | "YES" => 0,
                "FALSE" | "F" | "SAI" | "NO" => 1,
                "NOTGIVEN" | "NG" | "KHONGCO" | "KHONGDECAP" => 2,
                _ => return None,
            }
        };
        return Some(index);
    }
    let mut chars = key.chars();
    if let (Some(first), next) = (chars.next(), chars.next()) {
        if let Some(i) = "ABCDE".find(first.to_ascii_uppercase()).filter(|_| first.is_ascii_alphabetic()) {
            if next.is_none_or(|c| !c.is_ascii_alphabetic()) && i < options.len() {
                return Some(i);
            }
        }
    }
    let plain = |s: &str| normal(s);
    options.iter().position(|o| plain(o) == plain(key))
}

/// Text as answers are compared: lower case, only letters, digits and Han characters.
fn normal(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// The items of a set that can be used, at most `want`: broken ones are dropped, keys made the letter or label the
/// scoring compares, fixed options put in. A set without the passage or conversation its items need keeps none.
pub fn check(kind_key: &str, mut set: Set, want: usize, options: usize) -> Set {
    let k = kind(kind_key);
    // A small model says the same paragraph or question twice.
    let mut seen = HashSet::new();
    set.text = set.text.lines().filter(|l| l.chars().count() < 20 || seen.insert(normal(l))).collect::<Vec<_>>().join("\n");
    let mut seen = HashSet::new();
    set.items.retain(|it| seen.insert(normal(&format!("{} {}", it.say, it.q))));
    let body_ok = match k.body {
        Body::None => true,
        Body::Text => set.text.chars().count() >= 30,
        Body::Dialogue => set.lines.len() >= 2 || set.text.chars().count() >= 30,
    };
    if !body_ok {
        set.items.clear();
    }
    let labels: Vec<String> = set.list.iter().map(|e| entry_label(e)).collect();
    let list = set.list.clone();
    set.items.retain_mut(|it| {
        it.q = it.q.trim().to_owned();
        it.say = it.say.trim().to_owned();
        if it.q.is_empty() && it.say.is_empty() {
            return false;
        }
        match k.mode {
            Mode::Choice => {
                if !k.fixed.is_empty() {
                    it.options = k.fixed.iter().map(|o| (*o).to_owned()).collect();
                } else if it.options.len() > options.max(2) {
                    it.options.truncate(options.max(2));
                }
                if it.q.is_empty() && !k.heard_options {
                    return false;
                }
                let Some(i) = choice_key(&it.key, &it.options, k.fixed).filter(|_| it.options.len() >= 2) else { return false };
                it.key = ["A", "B", "C", "D", "E"][i].into();
                true
            }
            Mode::Match => {
                let key = entry_label(&it.key);
                let found = labels.iter().position(|l| !key.is_empty() && *l == key).or_else(|| list.iter().position(|e| normal(e).ends_with(&normal(&it.key)) && !it.key.trim().is_empty()));
                match found {
                    Some(i) => {
                        it.key = labels[i].clone();
                        !it.q.is_empty()
                    }
                    None => false,
                }
            }
            Mode::Text => {
                let key = it.key.trim().to_owned();
                if key.is_empty() || it.q.is_empty() || key.chars().count() > 80 {
                    return false;
                }
                match kind_key {
                    "l_form" | "r_complete" => key.split('/').all(|alt| alt.split_whitespace().count() <= 3),
                    "w_order" => {
                        let pieces: Vec<&str> = it.q.split(['/', '／']).map(str::trim).filter(|p| !p.is_empty()).collect();
                        let sorted = |s: &str| -> Vec<char> {
                            let mut v: Vec<char> = normal(s).chars().collect();
                            v.sort_unstable();
                            v
                        };
                        pieces.len() >= 3 && sorted(&pieces.concat()) == sorted(&key)
                    }
                    "r_order" => {
                        let letters: String = key.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect();
                        let mut sorted: Vec<char> = letters.chars().collect();
                        sorted.sort_unstable();
                        let want: Vec<char> = "ABCDE".chars().take(it.options.len()).collect();
                        it.key = letters;
                        it.options.len() >= 3 && sorted == want
                    }
                    "w_hanzi" => (1..=2).contains(&key.chars().count()) && key.chars().all(pron::is_han),
                    _ => true,
                }
            }
            Mode::Write | Mode::Speak => {
                it.key.clear();
                true
            }
        }
    });
    set.items.truncate(want);
    set
}

/// Whether `given` answers an objective item.
pub fn right(mode: &str, item: &Item, given: &str) -> bool {
    let given = given.trim();
    if given.is_empty() || item.key.is_empty() {
        return false;
    }
    match mode {
        "choice" => given.eq_ignore_ascii_case(&item.key),
        "match" => entry_label(given) == item.key,
        _ => item.key.split('/').map(normal).any(|alt| !alt.is_empty() && alt == normal(given)),
    }
}

// Scoring on each exam's scale.

/// The IELTS Academic band of a listening or reading score out of 40, from the published conversion tables.
pub fn ielts_band(skill: &str, raw: usize) -> f64 {
    const LISTENING: [(usize, f64); 16] = [(39, 9.0), (37, 8.5), (35, 8.0), (32, 7.5), (30, 7.0), (26, 6.5), (23, 6.0), (18, 5.5),
                                            (16, 5.0), (13, 4.5), (10, 4.0), (8, 3.5), (6, 3.0), (4, 2.5), (2, 2.0), (1, 1.0)];
    const READING: [(usize, f64); 16] = [(39, 9.0), (37, 8.5), (35, 8.0), (33, 7.5), (30, 7.0), (27, 6.5), (23, 6.0), (19, 5.5),
                                          (15, 5.0), (13, 4.5), (10, 4.0), (8, 3.5), (6, 3.0), (4, 2.5), (2, 2.0), (1, 1.0)];
    let table = if skill == "listening" { &LISTENING } else { &READING };
    table.iter().find(|(from, _)| raw >= *from).map_or(0.0, |(_, band)| *band)
}

/// The IELTS overall band: the mean of the skills to the nearest half band, a quarter going up (6.25 is 6.5, 6.75
/// is 7.0, 6.125 is 6.0).
pub fn ielts_overall(bands: &[f64]) -> f64 {
    round_half(bands.iter().sum::<f64>() / bands.len().max(1) as f64)
}

/// A TOEIC listening or reading score, 5 to 495 in steps of 5. ETS converts with a table for each form; a straight
/// line over the share of right answers is the usual estimate.
pub fn toeic_scaled(fraction: f64) -> f64 {
    5.0 + (fraction.clamp(0.0, 1.0) * 98.0).round() * 5.0
}

/// A skill's score on the exam's scale, from the share of its marks.
fn skill_score(format_id: &str, skill: &str, fraction: f64, raw40: usize) -> f64 {
    match format_id {
        "ielts" if matches!(skill, "listening" | "reading") => ielts_band(skill, raw40),
        "ielts" => round_half(fraction * 9.0),
        "toeic" if matches!(skill, "listening" | "reading") => toeic_scaled(fraction),
        "toeic" => (fraction * 20.0).round() * 10.0,
        "vstep" => round_half(fraction * 10.0),
        _ => (fraction * 100.0).round(),
    }
}

fn skill_text(format_id: &str, skill: &str, score: f64) -> String {
    match format_id {
        "ielts" => format!("{score:.1}"),
        "toeic" if matches!(skill, "listening" | "reading") => format!("{score:.0}/495"),
        "toeic" => format!("{score:.0}/200"),
        "vstep" => format!("{score:.1}/10"),
        "cefr" => format!("{score:.0}%"),
        _ => format!("{score:.0}/100"),
    }
}

/// The overall result of the skills taken, `(skill, score, fraction)`: the score on the exam's scale, how it reads,
/// and whether it reaches `pass`.
pub fn overall(format_id: &str, skills: &[(&str, f64, f64)], pass: f64) -> (f64, String, bool) {
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    match format_id {
        "ielts" => {
            let band = ielts_overall(&skills.iter().map(|s| s.1).collect::<Vec<_>>());
            (band, format!("band {band:.1}"), band + 1e-9 >= pass)
        }
        "toeic" => {
            let lr: Vec<f64> = skills.iter().filter(|s| matches!(s.0, "listening" | "reading")).map(|s| s.1).collect();
            let total = match lr.len() {
                2 => lr[0] + lr[1],
                1 => lr[0] * 2.0,
                _ => (mean(&skills.iter().map(|s| s.2).collect::<Vec<_>>()) * 990.0 / 5.0).round() * 5.0,
            };
            let text = if lr.len() == 2 { format!("{total:.0}/990") } else { format!("khoảng {total:.0}/990") };
            (total, text, total + 1e-9 >= pass)
        }
        "vstep" => {
            let score = round_half(mean(&skills.iter().map(|s| s.1).collect::<Vec<_>>()));
            let level = level_of_vstep(score);
            let text = if level.is_empty() { format!("{score:.1}/10, chưa đạt bậc 3") } else { format!("{score:.1}/10, {level}") };
            (score, text, score + 1e-9 >= pass)
        }
        "cefr" => {
            let score = mean(&skills.iter().map(|s| s.1).collect::<Vec<_>>()).round();
            (score, format!("{score:.0}%"), score + 1e-9 >= pass)
        }
        _ => {
            // HSK: the written sections add up against 60% of their points, HSKK on its own against 60.
            let written: Vec<f64> = skills.iter().filter(|s| s.0 != "speaking").map(|s| s.1).collect();
            let spoken: Vec<f64> = skills.iter().filter(|s| s.0 == "speaking").map(|s| s.1).collect();
            let total: f64 = written.iter().sum();
            let max = 100.0 * written.len() as f64;
            let mut parts = Vec::new();
            if !written.is_empty() {
                parts.push(format!("HSK {total:.0}/{max:.0} (đạt từ {:.0})", max * 0.6));
            }
            if let Some(s) = spoken.first() {
                parts.push(format!("HSKK {s:.0}/100 (đạt từ 60)"));
            }
            let passed = (written.is_empty() || total + 1e-9 >= max * 0.6) && spoken.iter().all(|s| *s >= 60.0);
            let score = mean(&skills.iter().map(|s| s.1).collect::<Vec<_>>()).round();
            (score, parts.join(", "), passed)
        }
    }
}

// Writing it.

/// One generation at a time, which the learner can stop.
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Stops a recording being read aloud, between two of its lines.
pub static STOP: AtomicBool = AtomicBool::new(false);

pub const SMALL_MODEL: &str = "Qwen 0.5B trên máy chưa đủ sức viết đề thi. Vào Cài đặt, chọn Qwen 1.5B hoặc một nguồn AI trên mạng (ChatGPT, Gemini, DeepSeek) rồi thử lại.";
pub const LOCAL_WARNING: &str = "Đề do Qwen 1.5B trên máy viết có thể còn lỗi và mất vài phút. Nguồn AI trên mạng viết đề tốt hơn.";

/// Whether the AI the learner chose can write a paper: never Qwen 0.5B, Qwen 1.5B with a warning.
pub fn writer(settings: &ai::Settings, dir: &Path) -> Result<Option<&'static str>, &'static str> {
    if ai::PROVIDERS[1..].contains(&settings.provider.as_str()) {
        return Ok(None);
    }
    if ai::local_group(dir, &settings.local_model) == "llm15" { Ok(Some(LOCAL_WARNING)) } else { Err(SMALL_MODEL) }
}

/// What the AI is told and asked for one set.
fn prompt(f: &Format, s: &Section, part: &Part, level: &str, topic: &str, words: &[String], n: usize, zh: bool) -> (String, String) {
    let k = kind(part.kind);
    let language = if zh { "Chinese" } else { "English" };
    let mut system = format!(
        "You write material for a practice {} test for a Vietnamese learner of {language}. {} Write it at {level} level, \
         on the topic given, using some of the words given where they fit naturally.",
        f.name, k.brief
    );
    if zh {
        system.push_str(" Write the Chinese in simplified characters, with no pinyin unless the task asks for it.");
    }
    if matches!(k.mode, Mode::Write | Mode::Speak) {
        system.push_str(" Write exactly as many as asked, in the language of the exam.");
    } else {
        system.push_str(&format!(
            " Write exactly the number of questions asked, each with one correct answer{}, a KEY line and a WHY line \
             explaining the answer in Vietnamese.",
            if k.mode == Mode::Choice && k.fixed.is_empty() { format!(" among {} options", part.options) } else { String::new() }
        ));
    }
    system.push_str(" Reply in exactly the format of the example, one field per line, with no Markdown and nothing else.");
    let question = format!(
        "Exam: {} {}. Task: {}\nLevel: {level}\nTopic: {topic}\nWords: {}\nQuestions: {n}",
        f.name,
        if zh { s.name.split(' ').next().unwrap_or(s.name) } else { s.name },
        part.task,
        if words.is_empty() { "-".to_owned() } else { words.join(", ") }
    );
    (system, question)
}

/// The learner's words on the plan's topics, learned ones first; else the last words learned in the course.
fn plan_words(db: &Connection, lang: &str, topics: &[String]) -> Vec<String> {
    let words: Vec<db::Word> = db::list(db, "").unwrap_or_default().into_iter().filter(|w| w.lang == lang && !w.skipped).collect();
    let wanted: HashSet<String> = topics.iter().map(|t| t.to_lowercase()).collect();
    let mut on: Vec<&db::Word> = words.iter().filter(|w| wanted.contains(&w.topic.to_lowercase())).collect();
    if on.len() < 6 {
        on.extend(words.iter().filter(|w| w.learned_at > 0 && !wanted.contains(&w.topic.to_lowercase())).take(30));
    }
    on.sort_by_key(|w| w.learned_at == 0);
    on.into_iter().map(|w| w.word.clone()).filter(|w| w.chars().count() <= 30).take(40).collect()
}

/// A set without what a small model copied word for word from the worked example: its passage's lines and its items.
fn uncopied(mut set: Set, example: &str) -> Set {
    let copied = |text: &str| text.chars().count() >= 20 && example.contains(text.trim());
    set.text = set.text.lines().filter(|l| !copied(l)).collect::<Vec<_>>().join("\n");
    set.lines.retain(|l| !copied(&l.text));
    set.items.retain(|it| !copied(&it.q) && !copied(&it.say));
    set
}

/// Writes the paper of a plan section by section, each set a call of its own, telling `progress` how far it is. A set
/// with too few good items is asked for once more; a section left with fewer than half its items fails it all.
pub fn write_paper(ai: &ai::Ai, p: &Plan, words: &[String], zh: bool, progress: &dyn Fn(usize, usize, &str)) -> Result<Paper, String> {
    let plan = included(p);
    let total: usize = plan.iter().flat_map(|(_, parts)| parts.iter().map(|(_, sets, _)| sets)).sum();
    let mut done = 0;
    let mut by = String::new();
    let mut sections = Vec::new();
    for (s, parts) in &plan {
        let mut out = PaperSection { skill: s.skill.into(), name: s.name.into(), vi: skill_vi(s.skill).into(), ..PaperSection::default() };
        let (mut got, mut want) = (0, 0);
        for &(part, sets, per) in parts {
            let k = kind(part.kind);
            let mut written = Vec::new();
            for _ in 0..sets {
                if CANCEL.load(Ordering::SeqCst) {
                    return Err("Đã dừng tạo đề".into());
                }
                progress(done, total, &format!("{} {}: {}", p.format.name, s.name, part.label));
                let topic = p.topics.get(done % p.topics.len().max(1)).cloned().unwrap_or_else(|| p.title.clone());
                let pick: Vec<String> = words.iter().cycle().skip(done * 6).take(6.min(words.len())).cloned().collect();
                let (system, question) = prompt(p.format, s, part, &p.level, &topic, &pick, per, zh);
                let example = if zh { k.zh } else { k.en };
                let scale = if zh { 1.4 } else { 1.0 };
                let max_tokens = (((k.tokens.0 + k.tokens.1 * per + 60) as f64 * scale) as usize).min(1800);
                let prompt = ai::Prompt { system: &system, example, question: &question, max_tokens, cjk: zh };
                let mut best = Set::default();
                for _ in 0..2 {
                    let (text, answered) = ai.chat(&prompt)?;
                    let set = check(part.kind, uncopied(parse(&text), example.1), per, part.options);
                    if set.items.len() > best.items.len() {
                        best = set;
                        by = answered.name();
                    }
                    if best.items.len() >= per {
                        break;
                    }
                }
                got += best.items.len();
                want += per;
                if !best.items.is_empty() {
                    written.push(best);
                }
                done += 1;
            }
            if written.is_empty() {
                continue;
            }
            out.seconds += (written.iter().map(|w| w.items.len()).sum::<usize>() as u32) * (part.secs + part.prep);
            out.parts.push(PaperPart {
                kind: part.kind.into(),
                label: part.label.into(),
                hint: k.hint.into(),
                mode: json!(k.mode).as_str().unwrap_or_default().into(),
                heard: k.heard,
                heard_options: k.heard_options,
                words: min_words(part, p.format.id, &p.level),
                weight: part.weight,
                prep: part.prep,
                secs: part.secs,
                sets: written,
            });
        }
        if got * 2 < want || out.parts.is_empty() {
            return Err(format!(
                "AI chưa viết đủ câu hỏi dùng được cho phần {} (được {got}/{want} câu). Hãy thử lại, hoặc chọn nguồn AI trên mạng trong Cài đặt.",
                skill_vi(s.skill)
            ));
        }
        sections.push(out);
    }
    progress(total, total, "");
    Ok(Paper {
        format: p.format.id.into(),
        name: exam_name(p.format, &p.level),
        title: p.title.clone(),
        full: p.full,
        level: p.level.clone(),
        plays: p.format.plays,
        topics: p.topics.clone(),
        by,
        sections,
    })
}

// Grading writing and speaking.

/// An examiner's grade: a score per criterion, the corrections, a better version and advice.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Grade {
    pub scores: Vec<f64>,
    pub fixes: Vec<String>,
    pub better: String,
    pub note: String,
}

/// The lines of a grade. A model that leaves the SCORES line out but writes a criterion on each line still counts;
/// missing scores take the mean of the others.
pub fn parse_grade(text: &str, n: usize, rate: f64) -> Option<Grade> {
    let mut g = Grade::default();
    let mut loose: Vec<f64> = Vec::new();
    let mut open = 0;
    for line in text.lines().map(|l| l.trim().trim_start_matches(['-', '*', '#']).trim()).filter(|l| !l.is_empty()) {
        let (head, value) = line.split_once([':', '：']).unwrap_or(("", line));
        let head = ai::fold_label(head);
        let value = value.trim().trim_matches('*').trim();
        match head.as_str() {
            "SCORES" | "SCORE" | "DIEM" | "BAND" | "BANDS" if g.scores.is_empty() => {
                g.scores = numbers(value).into_iter().filter(|x| *x <= rate + 1e-9).collect();
                open = 0;
            }
            "FIX" | "SUA" | "LOI" | "CORRECTION" => {
                if !value.is_empty() {
                    g.fixes.push(value.to_owned());
                }
                open = 0;
            }
            "BETTER" | "VIETLAI" | "IMPROVED" | "MAU" => {
                g.better = value.to_owned();
                open = 1;
            }
            "NOTE" | "NHANXET" | "GOIY" | "ADVICE" => {
                g.note = value.to_owned();
                open = 2;
            }
            _ if open == 1 => g.better = format!("{}\n{line}", g.better).trim().to_owned(),
            _ if open == 2 => g.note = format!("{} {line}", g.note).trim().to_owned(),
            _ if !head.is_empty() => loose.extend(numbers(value).into_iter().take(1).filter(|x| *x <= rate + 1e-9)),
            _ => {}
        }
    }
    if g.scores.is_empty() {
        g.scores = loose;
    }
    if g.scores.is_empty() {
        return None;
    }
    g.scores.truncate(n);
    let mean = g.scores.iter().sum::<f64>() / g.scores.len() as f64;
    g.scores.resize(n, mean);
    let step = if rate == 9.0 { 2.0 } else { 1.0 };
    for s in &mut g.scores {
        *s = ((s.clamp(0.0, rate) * step).round()) / step;
    }
    g.fixes.truncate(6);
    Some(g)
}

/// A worked example of a grade on a scale of `rate`, for the course.
fn grade_example(rate: f64, n: usize, zh: bool, speaking: bool) -> (String, String) {
    let shares = [0.65, 0.6, 0.6, 0.55];
    let step = if rate == 9.0 { 2.0 } else { 1.0 };
    let scores: Vec<String> = shares.iter().take(n).map(|s| format!("{}", ((s * rate * step).round()) / step)).collect();
    let (question, rest) = match (zh, speaking) {
        (false, false) => (
            "Task: Some people think children should learn a foreign language at primary school. Do you agree?\nMinimum: 250 words\nLength: 41 words\nAnswer: I am agree that children should learn foreign language early. Because they learn more fast than adult. In my country many student start English at grade three and they speaks good.",
            "FIX: I am agree -> I agree: \"agree\" là động từ, không dùng với \"am\".\n\
             FIX: learn more fast -> learn faster: so sánh hơn của \"fast\" là \"faster\".\n\
             FIX: they speaks good -> they speak well: chủ ngữ số nhiều, và bổ nghĩa cho động từ dùng trạng từ \"well\".\n\
             BETTER: I agree that children should learn a foreign language early, because young learners pick up new sounds faster than adults. In my country, many pupils start English in grade three and soon speak it with confidence.\n\
             NOTE: Bài quá ngắn so với 250 từ nên bị trừ điểm hoàn thành yêu cầu. Hãy viết mở bài, hai đoạn thân bài có ví dụ và kết bài.",
        ),
        (false, true) => (
            "Q: What do you usually do at the weekend?\nAnswer: at weekend i usually go to the park with my friend and we play badminton it is very fun and relax",
            "FIX: at weekend -> at the weekend: cần mạo từ \"the\".\n\
             FIX: it is very fun and relax -> it is great fun and very relaxing: dùng tính từ \"relaxing\".\n\
             BETTER: At the weekend I usually go to the park with my friends and we play badminton. It's great fun and really relaxing after a busy week.\n\
             NOTE: Câu trả lời đúng ý nhưng còn ngắn. Hãy thêm lý do và ví dụ, dùng từ nối như because, so, after that.",
        ),
        (true, false) => (
            "Task: 请结合下列词语写一篇80字左右的短文：旅游、计划、照片、难忘、朋友\nMinimum: 80 characters\nLength: 38 characters\nAnswer: 去年我和朋友去旅游。我们没有计划，所以很难忘。我拍了很多照片，我很高兴。",
            "FIX: 我们没有计划，所以很难忘 -> 虽然我们没有计划，但是这次旅游很难忘: quan hệ \"không có kế hoạch\" và \"khó quên\" là tương phản, không phải nhân quả.\n\
             BETTER: 去年暑假，我和几个好朋友去云南旅游。虽然我们出发前没有做详细的计划，但是一路上遇到了很多热情的人。我拍了很多照片，每次看到它们，都会想起那次难忘的旅行。\n\
             NOTE: Bài còn ngắn so với 80 chữ và câu khá đơn giản. Hãy thêm chi tiết về thời gian, địa điểm và cảm xúc, dùng các cặp từ nối như 虽然……但是…….",
        ),
        (true, true) => (
            "Q: 你周末一般做什么？\nAnswer: 我周末一般在家看书 有时候和朋友去公园 我觉得很放松",
            "FIX: 我觉得很放松 -> 这让我觉得很放松: nói rõ điều gì làm mình thư giãn.\n\
             BETTER: 我周末一般在家看书，有时候也和朋友去公园散步。这样的周末让我觉得很放松。\n\
             NOTE: Trả lời đúng ý nhưng ngắn. Hãy nói thêm lý do và một ví dụ cụ thể.",
        ),
    };
    (question.to_owned(), format!("SCORES: {}\n{rest}", scores.join(" | ")))
}

/// How long an answer is: its words, or in Chinese its characters.
pub fn length(text: &str, zh: bool) -> usize {
    if zh { text.chars().filter(|c| pron::is_han(*c)).count() } else { text.split_whitespace().count() }
}

/// An answer to a writing task graded on the format's criteria.
fn grade_writing(ai: &ai::Ai, f: &Format, task: &str, min: usize, answer: &str, zh: bool) -> Result<Grade, String> {
    let names: Vec<&str> = f.writing.iter().map(|c| c.0).collect();
    let unit = if zh { "characters" } else { "words" };
    let system = format!(
        "You are an official {} examiner. Grade the learner's answer to the writing task on these criteria, each from 0 to \
         {}{}: {}. An answer shorter than the minimum is marked down for the task. Reply in exactly the format of the \
         example: SCORES with one number per criterion in that order, separated by |; up to five FIX lines (the learner's \
         words -> the correction: why, in Vietnamese); BETTER with an improved version of the whole answer in {}; NOTE with \
         two or three sentences of advice in Vietnamese. Nothing else.",
        f.name,
        f.rate,
        if f.rate == 9.0 { " in half bands" } else { "" },
        names.join(", "),
        if zh { "Chinese" } else { "English" }
    );
    let answer: String = answer.chars().take(4000).collect();
    let question = format!("Task: {}\nMinimum: {min} {unit}\nLength: {} {unit}\nAnswer: {answer}", task.trim(), length(&answer, zh));
    let (example_q, example_a) = grade_example(f.rate, 4, zh, false);
    let prompt = ai::Prompt { system: &system, example: (example_q.as_str(), example_a.as_str()), question: &question, max_tokens: 900, cjk: zh };
    for _ in 0..2 {
        let (text, _) = ai.chat(&prompt)?;
        if let Some(g) = parse_grade(&text, 4, f.rate) {
            return Ok(g);
        }
    }
    Err("AI chưa chấm được bài viết, hãy bấm Nộp lại.".into())
}

/// Spoken answers graded together, from their transcripts, on the criteria other than pronunciation.
fn grade_speaking(ai: &ai::Ai, f: &Format, answers: &[(String, String)], zh: bool) -> Result<Grade, String> {
    let names: Vec<&str> = f.speaking[..3].iter().map(|c| c.0).collect();
    let system = format!(
        "You are an official {} speaking examiner. Grade the learner's spoken answers from their transcripts on these \
         criteria, each from 0 to {}{}: {}. The transcripts come from speech recognition: ignore punctuation and capitals. \
         An empty or very short answer scores low. Reply in exactly the format of the example: SCORES with one number per \
         criterion in that order, separated by |; up to five FIX lines (the learner's words -> the correction: why, in \
         Vietnamese); BETTER with a better version of one answer in {}; NOTE with two or three sentences of advice in \
         Vietnamese. Nothing else.",
        f.name,
        f.rate,
        if f.rate == 9.0 { " in half bands" } else { "" },
        names.join(", "),
        if zh { "Chinese" } else { "English" }
    );
    let mut question = String::new();
    for (q, a) in answers {
        let a: String = a.chars().take(700).collect();
        question.push_str(&format!("Q: {}\nAnswer: {}\n", q.replace('\n', " "), if a.trim().is_empty() { "(no answer)" } else { a.trim() }));
    }
    let (example_q, example_a) = grade_example(f.rate, 3, zh, true);
    let prompt = ai::Prompt { system: &system, example: (example_q.as_str(), example_a.as_str()), question: question.trim(), max_tokens: 700, cjk: zh };
    for _ in 0..2 {
        let (text, _) = ai.chat(&prompt)?;
        if let Some(g) = parse_grade(&text, 3, f.rate) {
            return Ok(g);
        }
    }
    Err("AI chưa chấm được phần Nói, hãy bấm Nộp lại.".into())
}

/// The sounds of an answer that repeats or reads aloud `target`, 0 to 100, by the app's pronunciation scoring.
fn pronunciation(target: &str, heard: &str, confidence: f32) -> u8 {
    if pron::is_chinese(target) { speech::score_chinese(target, heard, confidence).score } else { speech::score(target, heard, confidence).score }
}

/// What a spoken answer holds: its transcript and how sure Whisper was of it.
fn spoken(answer: &Value) -> (String, f32) {
    let text = answer["text"].as_str().or(answer.as_str()).unwrap_or_default().trim().to_owned();
    (text, answer["confidence"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32)
}

/// A submitted section scored: objective items from their keys, writing and speaking by the AI and the app. Every
/// item earns a share of its weight; the section's share is the weighted mean.
pub fn score_section(ai: &ai::Ai, f: &Format, s: &PaperSection, answers: &[Value], zh: bool) -> Result<Value, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut free: Vec<(usize, String, String, f32)> = Vec::new();
    let mut at = 0;
    for p in &s.parts {
        for set in &p.sets {
            for item in &set.items {
                let given = answers.get(at).cloned().unwrap_or(Value::Null);
                at += 1;
                let base = json!({ "part": p.label, "kind": p.kind, "weight": p.weight, "q": item.q, "say": item.say, "options": item.options,
                                   "key": item.key, "why": item.why });
                let mut v = base;
                match p.mode.as_str() {
                    "write" => {
                        let text = given.as_str().unwrap_or_default().to_owned();
                        let task = if set.text.is_empty() { item.q.clone() } else { format!("{}\n{}", set.text, item.q) };
                        let g = if length(&text, zh) == 0 {
                            Grade { scores: vec![0.0; 4], note: "Bạn chưa viết bài này.".into(), ..Grade::default() }
                        } else {
                            grade_writing(ai, f, &task, p.words, &text, zh)?
                        };
                        v["fraction"] = json!(g.scores.iter().sum::<f64>() / (4.0 * f.rate));
                        v["given"] = text.into();
                        v["grade"] = json!(g);
                    }
                    "speak" => {
                        let (text, confidence) = spoken(&given);
                        v["given"] = text.clone().into();
                        if matches!(p.kind.as_str(), "s_repeat" | "s_read") {
                            let target = if item.say.is_empty() { &item.q } else { &item.say };
                            let score = if text.is_empty() { 0 } else { pronunciation(target, &text, confidence) };
                            v["pron"] = score.into();
                            v["fraction"] = json!(f64::from(score) / 100.0);
                        } else {
                            let q = if item.q.is_empty() { item.say.clone() } else { item.q.clone() };
                            free.push((out.len(), q, text, confidence));
                        }
                    }
                    mode => {
                        let given = given.as_str().unwrap_or_default().to_owned();
                        let ok = right(mode, item, &given);
                        v["given"] = given.into();
                        v["right"] = ok.into();
                        v["fraction"] = json!(if ok { 1.0 } else { 0.0 });
                    }
                }
                out.push(v);
            }
        }
    }
    let mut feedback = Value::Null;
    if !free.is_empty() {
        let said: Vec<&(usize, String, String, f32)> = free.iter().filter(|f| !f.2.is_empty()).collect();
        // Pronunciation is how sure Whisper was of what it heard, the only measure there is of free speech.
        let clarity = said.iter().map(|f| f64::from(f.3)).sum::<f64>() / said.len().max(1) as f64;
        let mut g = if said.is_empty() {
            Grade { scores: vec![0.0; 3], note: "Chưa nghe được câu trả lời nào.".into(), ..Grade::default() }
        } else {
            grade_speaking(ai, f, &free.iter().map(|f| (f.1.clone(), f.2.clone())).collect::<Vec<_>>(), zh)?
        };
        let step = if f.rate == 9.0 { 2.0 } else { 1.0 };
        g.scores.push(((clarity * f.rate * step).round()) / step);
        let share = g.scores.iter().sum::<f64>() / (4.0 * f.rate);
        for (i, _, text, confidence) in &free {
            out[*i]["fraction"] = json!(if text.is_empty() { 0.0 } else { share });
            out[*i]["clarity"] = json!((f64::from(*confidence) * 100.0).round());
        }
        feedback = json!(g);
    }
    let weight: f64 = out.iter().map(|v| v["weight"].as_f64().unwrap_or(1.0)).sum();
    let earned: f64 = out.iter().map(|v| v["weight"].as_f64().unwrap_or(1.0) * v["fraction"].as_f64().unwrap_or(0.0)).sum();
    let objective: Vec<&Value> = out.iter().filter(|v| v.get("right").is_some()).collect();
    Ok(json!({
        "fraction": if weight > 0.0 { earned / weight } else { 0.0 },
        "right": objective.iter().filter(|v| v["right"] == true).count(),
        "total": objective.len(),
        "items": out,
        "feedback": feedback,
        "criteria": if s.skill == "speaking" { f.speaking.iter().map(|c| c.1).collect::<Vec<_>>() } else { f.writing.iter().map(|c| c.1).collect() },
        "skipped": false,
    }))
}

/// A section's score on the exam's scale and how it reads, from its result.
fn section_score(format_id: &str, skill: &str, r: &Value) -> (f64, String) {
    let fraction = r["fraction"].as_f64().unwrap_or(0.0);
    let (right, total) = (r["right"].as_u64().unwrap_or(0) as usize, r["total"].as_u64().unwrap_or(0) as usize);
    let raw40 = if total > 0 { (right * 40 + total / 2) / total } else { (fraction * 40.0).round() as usize };
    let score = skill_score(format_id, skill, fraction, raw40);
    (score, skill_text(format_id, skill, score))
}

/// The result of a finished exam: the score of each skill and overall on the exam's scale, whether it passes, the
/// strengths, the weaknesses and what to study next, and the wrong answers.
pub fn outcome(paper: &Paper, results: &Map<String, Value>, pass: f64) -> Value {
    let id = paper.format.as_str();
    let mut skills = Vec::new();
    let mut scored: Vec<(&str, f64, f64)> = Vec::new();
    let mut wrong = Vec::new();
    let mut weak_parts = Vec::new();
    for (i, s) in paper.sections.iter().enumerate() {
        let r = &results.get(&i.to_string()).map_or(Value::Null, |v| v["result"].clone());
        if r["skipped"] == true || r.is_null() {
            skills.push(json!({ "skill": s.skill, "name": s.name, "vi": s.vi, "skipped": true }));
            continue;
        }
        let fraction = r["fraction"].as_f64().unwrap_or(0.0);
        let (right, total) = (r["right"].as_u64().unwrap_or(0), r["total"].as_u64().unwrap_or(0));
        let (score, shown) = section_score(id, &s.skill, r);
        scored.push((s.skill.as_str(), score, fraction));
        skills.push(json!({ "skill": s.skill, "name": s.name, "vi": s.vi, "score": score, "text": shown,
                            "fraction": fraction, "right": right, "total": total, "skipped": false }));
        let mut by_part: Vec<(String, usize, usize)> = Vec::new();
        for item in r["items"].as_array().into_iter().flatten().filter(|v| v.get("right").is_some()) {
            let label = item["part"].as_str().unwrap_or_default().to_owned();
            let ok = item["right"] == true;
            match by_part.iter_mut().find(|p| p.0 == label) {
                Some(p) => (p.1, p.2) = (p.1 + usize::from(ok), p.2 + 1),
                None => by_part.push((label, usize::from(ok), 1)),
            }
            if !ok && wrong.len() < 60 {
                wrong.push(json!({ "vi": s.vi, "part": item["part"], "q": item["q"], "say": item["say"], "options": item["options"],
                                   "given": item["given"], "key": item["key"], "why": item["why"] }));
            }
        }
        weak_parts.extend(by_part.into_iter().filter(|p| p.2 >= 2 && p.1 * 10 < p.2 * 6).map(|p| format!("Dạng {}: đúng {}/{} câu.", p.0, p.1, p.2)));
    }
    let (score, text, passed) = overall(id, &scored, pass);
    let mut strengths = Vec::new();
    let mut weaknesses = Vec::new();
    let mut next = Vec::new();
    for s in skills.iter().filter(|s| s["skipped"] == false) {
        let f = s["fraction"].as_f64().unwrap_or(0.0);
        let line = format!("{}: {}{}", s["vi"].as_str().unwrap_or_default(), s["text"].as_str().unwrap_or_default(),
                           if s["total"].as_u64().unwrap_or(0) > 0 { format!(", đúng {}/{} câu", s["right"], s["total"]) } else { String::new() });
        if f >= 0.7 {
            strengths.push(line);
        } else if f < 0.55 {
            weaknesses.push(line);
            next.push(match s["skill"].as_str().unwrap_or_default() {
                "listening" => "Nghe: luyện Nghe chép và Nói đuổi với câu ví dụ mỗi ngày, rồi nghe lại bản ghi của bài này và đọc theo.",
                "reading" => "Đọc: mở Đọc hiểu với các bài về chủ đề của chặng, tìm từ khóa trong câu hỏi rồi dò ý đồng nghĩa trong bài.",
                "writing" => "Viết: luyện Đặt câu với từ mới, chép lại bản viết tốt hơn mà AI gợi ý và học thuộc các lỗi đã sửa.",
                _ => "Nói: luyện Nói đuổi và Hội thoại nhập vai, trả lời đủ ý bằng hai ba câu có từ nối.",
            }.to_owned());
        }
    }
    weaknesses.extend(weak_parts);
    if !paper.topics.is_empty() && !weaknesses.is_empty() {
        next.push(format!("Ôn lại từ của các chủ đề {} rồi làm lại bài kiểm tra.", paper.topics.join(", ")));
    }
    if next.is_empty() {
        next.push(if passed { "Giữ nhịp học đều, sang chặng tiếp theo và làm đề thi thử đầy đủ trước ngày thi." } else { "Ôn lại những câu sai bên dưới rồi làm một đề mới." }.to_owned());
    }
    json!({ "skills": skills, "score": score, "text": text, "passed": passed, "pass_text": pass_text(id, pass, &paper.level),
            "strengths": strengths, "weaknesses": weaknesses, "next": next, "wrong": wrong })
}

// The exams a course keeps.

/// An exam as the `exams` table keeps it. `roadmap` and `stage` are uids; an optional mock has no stage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exam {
    pub uid: String,
    pub roadmap: String,
    pub stage: String,
    pub kind: String,
    pub format: String,
    pub level: String,
    pub pass: f64,
    pub paper: Paper,
    /// Per submitted section, by its index: the answers given and the section's result.
    pub answers: Map<String, Value>,
    pub result: Value,
    pub score: f64,
    pub passed: bool,
    pub created: i64,
    pub finished: i64,
}

const COLUMNS: &str = "uid, roadmap, stage, kind, format, level, pass, paper, answers, result, score, passed, created, finished";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Exam> {
    let json = |i: usize| -> rusqlite::Result<Value> { Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null)) };
    Ok(Exam {
        uid: r.get(0)?,
        roadmap: r.get(1)?,
        stage: r.get(2)?,
        kind: r.get(3)?,
        format: r.get(4)?,
        level: r.get(5)?,
        pass: r.get(6)?,
        paper: serde_json::from_value(json(7)?).unwrap_or_default(),
        answers: json(8)?.as_object().cloned().unwrap_or_default(),
        result: json(9)?,
        score: r.get(10)?,
        passed: r.get(11)?,
        created: r.get(12)?,
        finished: r.get(13)?,
    })
}

pub fn load(db: &Connection, lang: &str, uid: &str) -> rusqlite::Result<Option<Exam>> {
    db.query_row(&format!("SELECT {COLUMNS} FROM exams WHERE uid = ?1 AND lang = ?2 AND deleted = 0"), [uid, lang], row).optional()
}

/// The exams of a roadmap, the newest first; of all the course's with no roadmap given.
fn of_roadmap(db: &Connection, lang: &str, roadmap: &str) -> rusqlite::Result<Vec<Exam>> {
    let mut stmt = db.prepare(&format!(
        "SELECT {COLUMNS} FROM exams WHERE lang = ?1 AND (?2 = '' OR roadmap = ?2) AND deleted = 0 ORDER BY created DESC, id DESC"
    ))?;
    stmt.query_map([lang, roadmap], row)?.collect()
}

/// Keeps a new exam and answers its uid.
pub fn insert(db: &Connection, lang: &str, e: &Exam, now: i64) -> rusqlite::Result<String> {
    db.query_row(
        "INSERT INTO exams (lang, roadmap, stage, kind, format, level, pass, paper, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) RETURNING uid",
        params![lang, e.roadmap, e.stage, e.kind, e.format, e.level, e.pass, json!(e.paper).to_string(), now],
        |r| r.get(0),
    )
}

/// Keeps what changed as the learner answers: the answers, and once finished the result.
fn save(db: &Connection, e: &Exam, now: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE exams SET answers = ?2, result = ?3, score = ?4, passed = ?5, finished = ?6, updated = ?7 WHERE uid = ?1",
        params![e.uid, Value::Object(e.answers.clone()).to_string(), if e.result.is_null() { String::new() } else { e.result.to_string() }, e.score, e.passed, e.finished, now],
    )
    .map(drop)
}

pub fn delete(db: &Connection, lang: &str, uid: &str, now: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE exams SET deleted = 1, updated = ?3 WHERE uid = ?1 AND lang = ?2", params![uid, lang, now]).map(drop)
}

/// An exam as the history lists it.
fn summary(e: &Exam) -> Value {
    json!({ "uid": e.uid, "title": e.paper.title, "name": e.paper.name, "kind": e.kind, "stage": e.stage, "created": e.created,
            "finished": e.finished, "passed": e.passed, "text": e.result["text"], "pass_text": pass_text(&e.format, e.pass, &e.level),
            "submitted": e.answers.len(), "sections": e.paper.sections.len() })
}

/// The test of each stage of a hard roadmap, None for an easy one: whether it was passed, and what its card shows.
pub struct StageTest {
    pub passed: bool,
    pub info: Value,
}

pub fn stage_tests(db: &Connection, lang: &str, r: &roadmap::Roadmap) -> rusqlite::Result<Option<Vec<StageTest>>> {
    if !is_hard(&r.purpose) {
        return Ok(None);
    }
    // Only what the cards show, so that a view of the roadmap never reads the papers.
    let mut stmt = db.prepare("SELECT stage, passed, finished, result FROM exams WHERE lang = ?1 AND roadmap = ?2 AND deleted = 0 ORDER BY created DESC, id DESC")?;
    let exams: Vec<(String, bool, i64, Value)> = stmt
        .query_map([lang, &r.uid], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or(Value::Null))))?
        .collect::<rusqlite::Result<_>>()?;
    let zh = lang == "zh";
    Ok(Some(
        r.steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mine: Vec<&(String, bool, i64, Value)> = exams.iter().filter(|e| !s.uid.is_empty() && e.0 == s.uid).collect();
                let passed = mine.iter().any(|e| e.1);
                let last = mine.iter().find(|e| e.2 > 0);
                let open = mine.iter().find(|e| e.2 == 0);
                let p = plan(r, Some(i), zh);
                let state = if passed { "passed" } else if open.is_some() { "open" } else if last.is_some() { "failed" } else { "none" };
                let info = json!({ "full": p.full, "pass_text": pass_text(p.format.id, p.pass, &p.level), "name": exam_name(p.format, &p.level),
                                   "state": state, "score": last.map(|e| e.3["text"].clone()), "best": mine.iter().find(|e| e.1).map(|e| e.3["text"].clone()) });
                StageTest { passed, info }
            })
            .collect(),
    ))
}

/// What the page holds of an exam: the paper without the keys, the explanations and what is heard of the sections
/// not submitted yet, each submitted section with its result, and the outcome once finished.
pub fn view(e: &Exam) -> Value {
    let mut sections = Vec::new();
    for (i, s) in e.paper.sections.iter().enumerate() {
        let done = e.answers.get(&i.to_string());
        let mut s = s.clone();
        if done.is_none() {
            for p in &mut s.parts {
                for set in &mut p.sets {
                    if p.heard {
                        set.text.clear();
                        set.lines.clear();
                    }
                    for it in &mut set.items {
                        it.key.clear();
                        it.why.clear();
                        if p.heard || p.heard_options {
                            it.say.clear();
                        }
                        if p.heard_options {
                            it.options.iter_mut().for_each(String::clear);
                        }
                    }
                }
            }
        }
        let mut v = json!(s);
        v["submitted"] = done.is_some().into();
        v["result"] = done.map_or(Value::Null, |d| d["result"].clone());
        v["answers"] = done.map_or(Value::Null, |d| d["answers"].clone());
        sections.push(v);
    }
    let next = (0..e.paper.sections.len()).find(|i| !e.answers.contains_key(&i.to_string()));
    let mut paper = json!(e.paper);
    paper["sections"] = sections.into();
    json!({ "uid": e.uid, "roadmap": e.roadmap, "stage": e.stage, "kind": e.kind, "format": e.format, "level": e.level, "pass": e.pass,
            "pass_text": pass_text(&e.format, e.pass, &e.level), "paper": paper, "next": next, "result": e.result,
            "created": e.created, "finished": e.finished })
}

/// The roadmap an exam is for and its stage: the active one of the course unless named, the stage by its index (none
/// for the optional mock). A stage without a uid, from a roadmap made before tests, gets one.
fn target(db: &Connection, lang: &str, uid: &str, stage: Option<usize>, now: i64) -> Result<(roadmap::Roadmap, Option<usize>), String> {
    let e = |e: rusqlite::Error| e.to_string();
    let list = roadmap::roadmaps(db, lang).map_err(e)?;
    let r = list.iter().find(|r| (uid.is_empty() && r.active) || r.uid == uid).cloned().ok_or("Hãy lập lộ trình trước khi làm bài thi thử")?;
    let stage = stage.filter(|&i| i < r.steps.len());
    if stage.is_some_and(|i| r.steps[i].uid.is_empty()) {
        roadmap::save_roadmap(db, lang, &r, now)?;
        let r = roadmap::roadmaps(db, lang).map_err(e)?.into_iter().find(|x| x.uid == r.uid).ok_or("Không tìm thấy lộ trình này")?;
        return Ok((r, stage));
    }
    Ok((r, stage))
}

fn stage_arg(args: &Value) -> Option<usize> {
    args["stage"].as_i64().filter(|i| *i >= 0).map(|i| i as usize)
}

/// The quick requests of the page about exams.
pub fn handle(db: &Connection, dir: &Path, lang: &str, settings: &ai::Settings, cmd: &str, args: &Value, now: i64) -> Result<Value, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let uid = args["uid"].as_str().unwrap_or_default();
    match cmd {
        "exam_plan" => {
            let (r, stage) = target(db, lang, args["roadmap"].as_str().unwrap_or_default(), stage_arg(args), now)?;
            let p = plan(&r, stage, lang == "zh");
            let step = stage.map(|i| r.steps[i].uid.clone()).unwrap_or_default();
            let mine: Vec<Exam> = of_roadmap(db, lang, &r.uid).map_err(e)?.into_iter().filter(|x| x.stage == step).collect();
            let mut v = plan_view(&p);
            v["roadmap"] = r.uid.clone().into();
            v["stage"] = json!(stage);
            v["hard"] = is_hard(&r.purpose).into();
            v["open"] = mine.iter().find(|x| x.finished == 0).map_or(Value::Null, summary);
            v["last"] = mine.iter().find(|x| x.finished > 0).map_or(Value::Null, summary);
            v["ai"] = match writer(settings, dir) {
                Ok(warning) => json!({ "error": "", "warning": warning.unwrap_or_default() }),
                Err(error) => json!({ "error": error, "warning": "" }),
            };
            v["whisper"] = speech::chosen(dir, lang, "").is_some().into();
            Ok(v)
        }
        "exam_get" => load(db, lang, uid).map_err(e)?.map(|x| view(&x)).ok_or_else(|| "Không tìm thấy bài thi này".into()),
        "exam_list" => Ok(json!(of_roadmap(db, lang, args["roadmap"].as_str().unwrap_or_default()).map_err(e)?.iter().map(summary).collect::<Vec<_>>())),
        "exam_delete" => {
            delete(db, lang, uid, now).map_err(e)?;
            Ok(json!(of_roadmap(db, lang, args["roadmap"].as_str().unwrap_or_default()).map_err(e)?.iter().map(summary).collect::<Vec<_>>()))
        }
        "exam_cancel" => {
            CANCEL.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        }
        "exam_stop" => {
            STOP.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        }
        _ => Err(format!("Không có lệnh {cmd}")),
    }
}

fn open(dir: &Path) -> Result<Connection, String> {
    db::open(&dir.join("words.db")).map_err(|e| e.to_string())
}

/// The exam of a stage, or the optional mock: the one under way when there is one, so that reopening never writes it
/// again; else a new paper the AI writes now.
pub fn make(dir: &Path, settings: &ai::Settings, lang: &str, args: &Value, day: &str, progress: impl Fn(Value)) -> Result<Value, String> {
    let db = open(dir)?;
    let now = crate::now();
    let (r, stage) = target(&db, lang, args["roadmap"].as_str().unwrap_or_default(), stage_arg(args), now)?;
    let step = stage.map(|i| r.steps[i].uid.clone()).unwrap_or_default();
    let mine = of_roadmap(&db, lang, &r.uid).map_err(|e| e.to_string())?;
    if let Some(e) = mine.iter().find(|x| x.stage == step && x.finished == 0) {
        return Ok(view(e));
    }
    writer(settings, dir)?;
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Đang tạo một đề khác, hãy chờ xong hoặc bấm Dừng".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let p = plan(&r, stage, lang == "zh");
    let ai = ai::Ai::new(settings, dir);
    let words = plan_words(&db, lang, &p.topics);
    let paper = write_paper(&ai, &p, &words, lang == "zh", &|done, total, label| progress(json!({ "done": done, "total": total, "label": label })));
    RUNNING.store(false, Ordering::SeqCst);
    crate::record_usage(dir, day, &ai.attempts.borrow());
    let paper = paper?;
    let mut e = Exam {
        roadmap: r.uid.clone(),
        stage: step,
        kind: if p.full { "full" } else { "stage" }.into(),
        format: p.format.id.into(),
        level: p.level.clone(),
        pass: p.pass,
        paper,
        created: now,
        ..Exam::default()
    };
    e.uid = insert(&db, lang, &e, now).map_err(|e| e.to_string())?;
    Ok(view(&e))
}

/// A section's answers handed in: scored, kept, and once every section is in, the exam is finished with its outcome.
/// Only speaking can be skipped, for a learner without the course's Whisper model.
pub fn submit(dir: &Path, settings: &ai::Settings, lang: &str, args: &Value, day: &str) -> Result<Value, String> {
    let db = open(dir)?;
    let uid = args["uid"].as_str().unwrap_or_default();
    let mut e = load(&db, lang, uid).map_err(|e| e.to_string())?.ok_or("Không tìm thấy bài thi này")?;
    let index = args["section"].as_u64().unwrap_or(0) as usize;
    let s = e.paper.sections.get(index).ok_or("Không có phần thi này")?.clone();
    if e.answers.contains_key(&index.to_string()) {
        return Ok(view(&e));
    }
    let f = format(&e.format).ok_or("Không rõ dạng đề của bài thi này")?;
    let answers: Vec<Value> = args["answers"].as_array().cloned().unwrap_or_default();
    let result = if args["skip"].as_bool().unwrap_or(false) {
        if s.skill != "speaking" {
            return Err("Chỉ bỏ qua được phần Nói".into());
        }
        json!({ "skipped": true, "fraction": 0.0, "items": [] })
    } else {
        // Grading writing and speaking asks as much of the model as writing the paper.
        if matches!(s.skill.as_str(), "writing" | "speaking") {
            writer(settings, dir)?;
        }
        let ai = ai::Ai::new(settings, dir);
        let scored = score_section(&ai, f, &s, &answers, lang == "zh");
        crate::record_usage(dir, day, &ai.attempts.borrow());
        let mut result = scored?;
        result["text"] = section_score(&e.format, &s.skill, &result).1.into();
        result
    };
    e.answers.insert(index.to_string(), json!({ "answers": answers, "result": result }));
    if e.answers.len() >= e.paper.sections.len() {
        e.result = outcome(&e.paper, &e.answers, e.pass);
        e.score = e.result["score"].as_f64().unwrap_or(0.0);
        e.passed = e.result["passed"] == true;
        e.finished = crate::now();
    }
    save(&db, &e, crate::now()).map_err(|e| e.to_string())?;
    Ok(view(&e))
}

/// The lines of a set or of one item as they are read aloud, each with its speaker ("M", "W" or "" for the narrator):
/// the passage or dialogue, then for each item its number, what is heard of it and its heard options.
pub fn script(db: &Connection, lang: &str, args: &Value) -> Result<Vec<(String, String)>, String> {
    let e = load(db, lang, args["uid"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?.ok_or("Không tìm thấy bài thi này")?;
    let at = |key: &str| args[key].as_u64().unwrap_or(0) as usize;
    let part = e.paper.sections.get(at("section")).and_then(|s| s.parts.get(at("part"))).ok_or("Không có phần nghe này")?;
    let set = part.sets.get(at("set")).ok_or("Không có phần nghe này")?;
    let zh = lang == "zh";
    // A short dialogue in one SAY, "男：... 女：...", is read by its two speakers.
    let turns = split_speakers;
    if let Some(i) = args["item"].as_u64() {
        let it = set.items.get(i as usize).ok_or("Không có câu này")?;
        return Ok(turns(if it.say.is_empty() { &it.q } else { &it.say }));
    }
    let mut lines: Vec<(String, String)> = set.lines.iter().map(|l| (l.who.clone(), l.text.clone())).collect();
    if lines.is_empty() && part.heard && !set.text.is_empty() {
        lines.push((String::new(), set.text.clone()));
    }
    let first = e.paper.sections[at("section")].parts[..at("part")].iter().flat_map(|p| &p.sets).map(|s| s.items.len()).sum::<usize>()
        + part.sets[..at("set")].iter().map(|s| s.items.len()).sum::<usize>();
    for (i, it) in set.items.iter().enumerate() {
        if it.say.is_empty() && !part.heard_options {
            continue;
        }
        let n = first + i + 1;
        lines.push((String::new(), if zh { format!("第{n}题。") } else { format!("Number {n}.") }));
        if !it.say.is_empty() {
            lines.extend(turns(&it.say));
        }
        if part.heard_options {
            for (j, o) in it.options.iter().enumerate() {
                lines.push((String::new(), format!("{}. {o}", ["A", "B", "C", "D", "E"][j.min(4)])));
            }
        }
    }
    Ok(lines)
}

/// Text cut where a speaker's label starts ("男：", "M:"), each piece with its speaker, "" before the first label.
fn split_speakers(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut rest = text;
    let mut who = String::new();
    loop {
        let next = ["男：", "女：", "男:", "女:", "M:", "W:"].iter().filter_map(|l| rest.find(l).map(|at| (at, *l))).min_by_key(|(at, _)| *at);
        let (at, label) = match next {
            Some((at, label)) if at == 0 || !rest[..at].ends_with(|c: char| c.is_alphanumeric()) => (at, label),
            _ => {
                if !rest.trim().is_empty() {
                    out.push((who, rest.trim().to_owned()));
                }
                return out;
            }
        };
        if !rest[..at].trim().is_empty() {
            out.push((who.clone(), rest[..at].trim().to_owned()));
        }
        who = if label.starts_with(['男', 'M']) { "M" } else { "W" }.into();
        rest = &rest[at + label.len()..];
    }
}

/// The Kokoro voice for a speaker: the chosen one when its gender fits, else the first of the course's voices of the
/// speaker's gender; the narrator reads with the chosen one.
pub fn voice_for(lang: &str, chosen: i32, who: &str) -> i32 {
    let male = |name: &str| name.to_lowercase().contains("nam");
    let voices: Vec<&(&str, i32, &str)> = crate::voice::VOICES.iter().filter(|v| v.0 == lang).collect();
    let want = match who {
        "M" => Some(true),
        "W" => Some(false),
        _ => None,
    };
    let fits = |sid: i32| voices.iter().find(|v| v.1 == sid).is_some_and(|v| want.is_none_or(|m| male(v.2) == m));
    if fits(chosen) {
        return chosen;
    }
    voices.iter().find(|v| want.is_none_or(|m| male(v.2) == m)).map_or(chosen, |v| v.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn memory() -> Connection {
        db::open(Path::new(":memory:")).unwrap()
    }

    fn roadmap(purpose: &str, level: &str, skills: &[&str]) -> roadmap::Roadmap {
        roadmap::Roadmap {
            uid: "r1".into(),
            purpose: purpose.into(),
            level: level.into(),
            deadline: "2027-06-01".into(),
            steps: skills
                .iter()
                .enumerate()
                .map(|(i, s)| roadmap::Step { uid: format!("s{i}"), title: format!("Chặng {}", i + 1), topics: vec![format!("Chủ đề {i}")], skills: (*s).into(), target: 2, days: 10, ..roadmap::Step::default() })
                .collect(),
            ..roadmap::Roadmap::default()
        }
    }

    #[test]
    fn the_formats_add_up() {
        for f in FORMATS {
            assert!(!f.sections.is_empty(), "{}", f.id);
            assert!(f.plays >= 1 && f.rate >= 5.0);
            for s in f.sections {
                assert!(["listening", "reading", "writing", "speaking"].contains(&s.skill), "{} {}", f.id, s.skill);
                assert!(s.parts.iter().any(|p| p.mini.0 * p.mini.1 > 0), "{} {} has nothing for a stage test", f.id, s.name);
                for p in s.parts {
                    let k = kind(p.kind);
                    assert_eq!(k.key, p.kind, "{} {} names a known kind", f.id, p.label);
                    let example = if f.id.starts_with("hsk") { k.zh } else { k.en };
                    assert!(!example.0.is_empty(), "{} has an example for {}", p.kind, f.id);
                    assert!(p.full.0 * p.full.1 >= p.mini.0 * p.mini.1, "{} {}: a full mock has at least the stage test's items", f.id, p.label);
                    assert!(p.secs > 0 && (2..=4).contains(&p.options) && p.weight >= 1);
                    let objective = !matches!(k.mode, Mode::Write | Mode::Speak);
                    assert_eq!(objective, matches!(s.skill, "listening" | "reading") || p.kind.starts_with("w_order") || p.kind == "w_hanzi", "{}", p.kind);
                }
            }
            // The official order: listening before reading, which comes before writing.
            let order: Vec<&str> = f.sections.iter().map(|s| s.skill).collect();
            assert_eq!(&order[..2], ["listening", "reading"], "{}", f.id);
        }
        // Each example parses into as many good items as it shows.
        for k in KINDS {
            for (q, a) in [k.en, k.zh].into_iter().filter(|e| !e.0.is_empty()) {
                let want = numbers(q.lines().last().unwrap())[0] as usize;
                let set = check(k.key, parse(a), 9, 4);
                assert_eq!(set.items.len(), want, "{} example: {set:?}", k.key);
            }
        }
        // The full IELTS mock has every section and part; the HSK 1-2 test has no writing.
        let ielts = plan(&roadmap("IELTS 6.5", "B1", &["Nghe, Đọc", "Viết"]), None, false);
        assert_eq!(included(&ielts).iter().map(|(s, parts)| (s.skill, parts.len())).collect::<Vec<_>>(),
                   [("listening", 4), ("reading", 3), ("writing", 2), ("speaking", 3)]);
        let view = plan_view(&ielts);
        assert_eq!(view["sections"][0]["items"], 20);
        assert_eq!(view["sections"][2]["minutes"], 60, "Task 1 and Task 2 take the official hour");
        assert!(!HSK12.sections.iter().any(|s| s.skill == "writing"));
        assert_eq!(min_words(&CEFR_TEST.sections[2].parts[0], "cefr", "B2"), 140);
    }

    #[test]
    fn goals_make_a_roadmap_hard_and_set_the_pass_marks() {
        assert!(is_hard("IELTS 6.5") && is_hard("thi toeic 750") && is_hard("VSTEP bậc 4") && is_hard("HSK 3") && is_hard("Đạt B2"));
        assert!(!is_hard("Du lịch") && !is_hard("Giao tiếp B1") && !is_hard("B2B sales"));
        // IELTS: the pass mark climbs from the start level to the goal, the last stage being the full mock.
        let r = roadmap("IELTS 6.5", "B1", &["Nghe, Đọc", "Viết", "Nói", "Nghe"]);
        let marks: Vec<(f64, bool)> = (0..4).map(|i| plan(&r, Some(i), false)).map(|p| (p.pass, p.full)).collect();
        assert_eq!(marks, [(5.0, false), (5.5, false), (6.0, false), (6.5, true)]);
        let first = plan(&r, Some(0), false);
        assert_eq!((first.format.id, first.skills.clone(), first.level.as_str()), ("ielts", vec!["listening", "reading"], "B1"));
        assert_eq!(plan(&r, Some(1), false).skills, ["writing"]);
        assert_eq!(plan(&r, Some(3), false).skills.len(), 4, "the full mock takes all four skills");
        assert_eq!(first.title, "Kiểm tra chặng 1: Chặng 1");
        // TOEIC, VSTEP, a CEFR level, HSK.
        let toeic = plan(&roadmap("TOEIC 750", "", &["", ""]), Some(0), false);
        assert_eq!((toeic.format.id, toeic.pass), ("toeic", 640.0), "from 70% of the goal, halfway, to a step of 5");
        assert_eq!(toeic.skills, ["listening", "reading"], "a stage naming no skill tests listening and reading");
        let vstep = plan(&roadmap("VSTEP B2", "", &["Nói"]), Some(0), false);
        assert_eq!((vstep.format.id, vstep.pass, vstep.full), ("vstep", 6.0, true));
        assert_eq!(pass_text("vstep", 6.0, &vstep.level), "6.0/10 (B2)");
        let cefr = plan(&roadmap("Giao tiếp công việc", "A2", &["Nghe", "Nói"]), None, false);
        assert_eq!((cefr.format.id, cefr.level.as_str(), cefr.pass), ("cefr", "B1", 60.0));
        let hsk = roadmap("HSK 4", "HSK1", &["Nghe", "Đọc, Viết", "Nói"]);
        let levels: Vec<String> = (0..3).map(|i| plan(&hsk, Some(i), true).level).collect();
        assert_eq!(levels, ["HSK 3", "HSK 3", "HSK 4"]);
        assert_eq!(plan(&hsk, Some(0), true).format.id, "hsk3");
        assert_eq!(plan(&hsk, Some(1), true).skills, ["reading", "writing"]);
        assert_eq!(exam_name(plan(&hsk, None, true).format, "HSK 4"), "HSK 4 và HSKK trung cấp");
        assert_eq!(plan(&roadmap("HSK 2", "", &["Viết", "Nghe"]), Some(0), true).skills, ["listening", "reading"], "HSK 2 has no writing");
    }

    #[test]
    fn answers_in_the_line_format_parse_and_broken_items_are_dropped() {
        let text = "Here is the test:\n\
                    **M:** Hello, I'd like to book a table for four.\n\
                    W: Certainly, for what time?\n\
                    M: Seven o'clock, please,\n\
                    and near the window.\n\
                    1. How many people?\n\
                    A) Two\nB) Four\nC) Six\n\
                    KEY: B\nWHY: Người nam nói \"a table for four\".\n\
                    **Q:** What time?\n(A) Six\n(B) Seven\n(C) Eight\nAnswer: (B) Seven\n\
                    Q: Where do they sit?\nA) By the door\nB) At the bar\nKEY: E\n\
                    Q: No options here\nKEY: A\n";
        let set = parse(text);
        assert_eq!(set.lines.len(), 3);
        assert_eq!(set.lines[2].text, "Seven o'clock, please, and near the window.");
        assert_eq!(set.items.len(), 4);
        let set = check("l_dialogue", set, 5, 3);
        assert_eq!(set.items.iter().map(|i| (i.q.as_str(), i.key.as_str())).collect::<Vec<_>>(), [("How many people?", "B"), ("What time?", "B")],
                   "a key past the options and an item without options go");
        // A dialogue that never came leaves the set empty.
        assert!(check("l_dialogue", parse("Q: What?\nA) a\nB) b\nKEY: A"), 3, 3).items.is_empty());
        // True / False / Not Given in words or letters, headings by number, completion within two words.
        let tfng = check("r_tfng", parse("TEXT: A passage long enough to read about trade in the old port town.\nQ: One\nKEY: True\nQ: Two\nKEY: NG\nQ: Three\nKEY: maybe"), 5, 3);
        assert_eq!(tfng.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["A", "C"]);
        assert_eq!(tfng.items[0].options, ["TRUE", "FALSE", "NOT GIVEN"]);
        let headings = check("r_headings", parse("TEXT: A. First paragraph about work.\nB. Second paragraph about teams and staff.\nLIST: i. One | ii. Two\niii. Three\nQ: Paragraph A\nKEY: ii\nQ: Paragraph B\nKEY: (iii)\nQ: Paragraph C\nKEY: vii"), 5, 4);
        assert_eq!(headings.text.lines().count(), 2);
        assert_eq!(headings.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["ii", "iii"]);
        let complete = check("r_complete", parse("TEXT: Bees pollinate about a third of the crops we eat, mostly fruit.\nQ: Bees help ____.\nKEY: a third\nQ: Long\nKEY: far too many words here"), 5, 4);
        assert_eq!(complete.items.len(), 1);
        // Chinese: a judged statement, words to put in order that must be the key's, characters from pinyin.
        let judge = check("l_judge", parse("SAY: 今天很冷。\nQ: 今天很热。\nKEY: ×\nSAY: 我喝茶。\nQ: 他喝茶。\n答案：对"), 5, 2);
        assert_eq!(judge.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["B", "A"]);
        let order = check("w_order", parse("Q: 我 / 一本 / 买了 / 书\nKEY: 我买了一本书。\nQ: 我 / 去 / 学校\nKEY: 他去学校。"), 5, 4);
        assert_eq!(order.items.len(), 1, "a key that is not the words given goes");
        let hanzi = check("w_hanzi", parse("Q: 我想喝（chá）。\nKEY: 茶\nQ: 我想喝（shuǐ）。\nKEY: shui"), 5, 4);
        assert_eq!(hanzi.items.len(), 1);
        let order3 = check("r_order", parse("Q: 排列顺序\nA) 一\nB) 二\nC) 三\nKEY: B, A, C\nQ: 排列顺序\nA) 一\nB) 二\nC) 三\nKEY: BAA"), 5, 3);
        assert_eq!(order3.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["BAC"]);
        // Speaking prompts keep their lines; at most what was asked for.
        let cue = check("s_cue", parse("Q: Describe a trip.\nYou should say:\n- where you went\nQ: Another one"), 1, 4);
        assert_eq!(cue.items.len(), 1);
        assert_eq!(cue.items[0].q.lines().count(), 3);
        // What a small model says twice, or copies from the worked example, goes.
        let twice = check("r_tfng", parse("TEXT: The same paragraph about trade, said twice over.\nThe same paragraph about trade, said twice over.\nQ: One\nKEY: T\nQ: One\nKEY: T"), 5, 3);
        assert_eq!((twice.text.lines().count(), twice.items.len()), (1, 1));
        let copied = uncopied(parse("TEXT: Merchants from Japan, China and Europe came to buy silk and ceramics.\nA new line about hotels and their guests.\nQ: Hoi An was a trading port for about four hundred years.\nKEY: TRUE"), kind("r_tfng").en.1);
        assert_eq!((copied.text.as_str(), copied.items.len()), ("A new line about hotels and their guests.", 0));
        // A per-item dialogue is read with its speakers.
        assert_eq!(split_speakers("男：你好。 女：你好！"), [("M".to_owned(), "你好。".to_owned()), ("W".to_owned(), "你好！".to_owned())]);
        assert_eq!(split_speakers("Attention, please."), [(String::new(), "Attention, please.".to_owned())]);
    }

    #[test]
    fn objective_items_score_from_their_keys() {
        let item = |key: &str| Item { key: key.into(), ..Item::default() };
        assert!(right("choice", &item("B"), "b") && !right("choice", &item("B"), "C") && !right("choice", &item("B"), ""));
        assert!(right("match", &item("iii"), "iii") && right("match", &item("C"), "C 而且") && !right("match", &item("ii"), "iii"));
        assert!(right("text", &item("forty / 40"), " 40 ") && right("text", &item("Carter"), "carter.") && !right("text", &item("Carter"), "Karter"));
        assert!(right("text", &item("我昨天买了一本书。"), "我昨天买了一本书") && right("text", &item("BAC"), "bac"));
        let s = PaperSection {
            skill: "reading".into(),
            parts: vec![PaperPart {
                label: "P".into(),
                mode: "choice".into(),
                weight: 1,
                sets: vec![Set { items: vec![item("A"), item("B"), item("C")], ..Set::default() }],
                ..PaperPart::default()
            }],
            ..PaperSection::default()
        };
        let settings = ai::Settings::default();
        let ai = ai::Ai::new(&settings, Path::new("."));
        let r = score_section(&ai, &IELTS, &s, &[json!("A"), json!("C"), json!("C")], false).unwrap();
        assert_eq!((r["right"].as_u64(), r["total"].as_u64()), (Some(2), Some(3)));
        assert!((r["fraction"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
        assert!(ai.attempts.borrow().is_empty(), "objective items never ask the AI");
    }

    #[test]
    fn scores_map_onto_each_exams_scale() {
        assert_eq!([6.125, 6.25, 6.5, 6.75, 6.875].map(|m| ielts_overall(&[m])), [6.0, 6.5, 6.5, 7.0, 7.0]);
        assert_eq!(ielts_overall(&[6.5, 6.5, 5.0, 7.0]), 6.5, "6.25 goes up to 6.5");
        assert_eq!(ielts_overall(&[4.0, 3.5, 4.0, 4.0]), 4.0, "3.875 goes to 4.0");
        assert_eq!([40, 35, 30, 26, 23, 18, 0].map(|r| ielts_band("listening", r)), [9.0, 8.0, 7.0, 6.5, 6.0, 5.5, 0.0]);
        assert_eq!([33, 30, 27, 23].map(|r| ielts_band("reading", r)), [7.5, 7.0, 6.5, 6.0]);
        assert_eq!([0.0, 0.5, 1.0].map(toeic_scaled), [5.0, 250.0, 495.0]);
        let (total, text, passed) = overall("toeic", &[("listening", 400.0, 0.8), ("reading", 350.0, 0.7)], 750.0);
        assert_eq!((total, text.as_str(), passed), (750.0, "750/990", true));
        assert_eq!(overall("toeic", &[("listening", 300.0, 0.6)], 750.0).0, 600.0, "one half is doubled for an estimate");
        let (band, _, passed) = overall("ielts", &[("listening", 6.0, 0.0), ("reading", 6.5, 0.0), ("writing", 5.5, 0.0), ("speaking", 6.0, 0.0)], 6.0);
        assert_eq!((band, passed), (6.0, true));
        assert_eq!(overall("vstep", &[("listening", 6.0, 0.6), ("reading", 5.5, 0.55)], 6.0).1, "6.0/10, B2");
        assert_eq!(overall("vstep", &[("listening", 3.0, 0.3)], 4.0).1, "3.0/10, chưa đạt bậc 3");
        // HSK: the written sections against 60% of their points, HSKK on its own against 60.
        let (_, text, passed) = overall("hsk4", &[("listening", 70.0, 0.7), ("reading", 60.0, 0.6), ("writing", 50.0, 0.5)], 60.0);
        assert_eq!((text.as_str(), passed), ("HSK 180/300 (đạt từ 180)", true));
        assert!(!overall("hsk4", &[("listening", 70.0, 0.7), ("speaking", 55.0, 0.55)], 60.0).2, "HSKK under 60 fails");
        assert!(!overall("hsk12", &[("listening", 59.0, 0.59), ("reading", 60.0, 0.6)], 60.0).2, "119 of 200 fails");
        // A whole outcome: a skipped section stays out, the wrong answers and weak skills are named.
        let paper = Paper {
            format: "ielts".into(),
            level: "B2".into(),
            topics: vec!["Du lịch".into()],
            sections: ["listening", "speaking"].iter().map(|s| PaperSection { skill: (*s).into(), vi: skill_vi(s).into(), ..PaperSection::default() }).collect(),
            ..Paper::default()
        };
        let mut results = Map::new();
        let items: Vec<Value> = (0..10).map(|i| json!({ "part": "Part 1", "right": i < 4, "q": format!("Q{i}"), "key": "A", "given": "B" })).collect();
        results.insert("0".into(), json!({ "result": { "fraction": 0.4, "right": 4, "total": 10, "items": items } }));
        results.insert("1".into(), json!({ "result": { "skipped": true } }));
        let o = outcome(&paper, &results, 5.5);
        assert_eq!((o["skills"][0]["score"].as_f64(), o["skills"][1]["skipped"].as_bool()), (Some(5.0), Some(true)), "16 of 40 is band 5.0");
        assert_eq!((o["text"].as_str(), o["passed"].as_bool()), (Some("band 5.0"), Some(false)));
        assert_eq!(o["wrong"].as_array().unwrap().len(), 6);
        assert!(o["weaknesses"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Part 1: đúng 4/10")));
        assert!(o["next"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Du lịch")));
    }

    #[test]
    fn grades_parse_from_tidy_and_messy_answers() {
        let g = parse_grade("SCORES: 6 | 5.5 | 6.5 | 7.2\nFIX: I am agree -> I agree: sai.\nBETTER: I agree that...\nwith more.\nNOTE: Viết dài hơn.\nThêm ví dụ.", 4, 9.0).unwrap();
        assert_eq!(g.scores, [6.0, 5.5, 6.5, 7.0], "rounded to half bands");
        assert_eq!((g.fixes.len(), g.better.as_str(), g.note.as_str()), (1, "I agree that...\nwith more.", "Viết dài hơn. Thêm ví dụ."));
        let loose = parse_grade("**Task Response**: 6\nCoherence and Cohesion: 5\nNOTE: ok", 4, 9.0).unwrap();
        assert_eq!(loose.scores, [6.0, 5.0, 5.5, 5.5], "missing criteria take the mean");
        assert_eq!(parse_grade("SCORES: 12 | 4\nNOTE: x", 3, 5.0).unwrap().scores, [4.0, 4.0, 4.0], "a number past the scale is dropped");
        assert!(parse_grade("Bài viết tốt.", 4, 9.0).is_none());
        assert_eq!(length("I agree with this.", false), 4);
        assert_eq!(length("我同意，因为……", true), 5);
    }

    #[test]
    fn a_stage_of_a_hard_roadmap_is_done_once_its_test_is_passed() {
        let db = memory();
        let now = 100 * db::DAY;
        for i in 0..3 {
            let id = db::save(&db, &db::Word { word: format!("w{i}"), topic: "Du lịch".into(), ..db::Word::default() }, 0).unwrap();
            db.execute("UPDATE words SET learned_at = 5 WHERE id = ?1", [id]).unwrap();
        }
        let mut r = roadmap::Roadmap {
            purpose: "IELTS 6.0".into(),
            deadline: "1970-06-01".into(),
            steps: vec![
                roadmap::Step { title: "Du lịch".into(), topics: vec!["Du lịch".into()], target: 3, days: 10, ..roadmap::Step::default() },
                roadmap::Step { title: "Công việc".into(), topics: vec!["Công việc".into()], target: 5, days: 10, ..roadmap::Step::default() },
            ],
            ..roadmap::Roadmap::default()
        };
        r.uid = roadmap::save_roadmap(&db, "en", &r, now).unwrap();
        let v = roadmap::view(&db, "en", "1970-04-11", now).unwrap();
        let stage = &v["active"]["steps"][0];
        assert_eq!((stage["state"].as_str(), stage["test"]["state"].as_str(), stage["test"]["pending"].as_bool()), (Some("current"), Some("none"), Some(true)),
                   "the words are learned but the test is pending");
        assert_eq!(v["pace"]["remaining"], 5, "the words of the second stage remain");
        assert_eq!(roadmap::focus_topics(&db, "en").unwrap(), ["Du lịch"]);
        let step = roadmap::roadmaps(&db, "en").unwrap()[0].steps[0].uid.clone();
        assert_eq!(step.len(), 16, "stages get a uid as the roadmap is saved");
        // A failed test leaves it pending; a passed one finishes it.
        let mut e = Exam { roadmap: r.uid.clone(), stage: step, kind: "stage".into(), format: "ielts".into(), pass: 5.0, ..Exam::default() };
        e.uid = insert(&db, "en", &e, now).unwrap();
        (e.finished, e.result) = (now, json!({ "text": "band 4.5" }));
        save(&db, &e, now).unwrap();
        let v = roadmap::view(&db, "en", "1970-04-11", now).unwrap();
        assert_eq!((v["active"]["steps"][0]["state"].as_str(), v["active"]["steps"][0]["test"]["state"].as_str()), (Some("current"), Some("failed")));
        assert_eq!(v["active"]["steps"][0]["test"]["score"], "band 4.5");
        e.passed = true;
        save(&db, &e, now).unwrap();
        let v = roadmap::view(&db, "en", "1970-04-11", now).unwrap();
        assert_eq!((v["active"]["steps"][0]["state"].as_str(), v["active"]["current"].as_i64()), (Some("done"), Some(1)));
        assert_eq!(v["active"]["steps"][1]["test"]["full"], true, "the last stage takes the full mock");
        // Marked done by hand, a stage is done whatever its test; an easy roadmap has no tests at all.
        let mut saved = roadmap::roadmaps(&db, "en").unwrap()[0].clone();
        saved.steps[1].done = true;
        roadmap::save_roadmap(&db, "en", &saved, now).unwrap();
        assert_eq!(roadmap::view(&db, "en", "1970-04-11", now).unwrap()["pace"]["verdict"], "done");
        saved.purpose = "Du lịch".into();
        saved.steps[1].done = false;
        roadmap::save_roadmap(&db, "en", &saved, now).unwrap();
        let v = roadmap::view(&db, "en", "1970-04-11", now).unwrap();
        assert!(v["active"]["steps"][0]["test"].is_null());
        // The history lists the exam, the page never sees a key before its section is in, and a deleted one goes.
        assert_eq!(handle(&db, Path::new("."), "en", &ai::Settings::default(), "exam_list", &json!({ "roadmap": r.uid }), now).unwrap().as_array().unwrap().len(), 1);
        delete(&db, "en", &e.uid, now).unwrap();
        assert!(of_roadmap(&db, "en", &r.uid).unwrap().is_empty());
    }

    #[test]
    fn the_page_never_sees_keys_or_scripts_before_a_section_is_in() {
        let set = Set { lines: vec![Line { who: "M".into(), text: "Hello".into() }], items: vec![Item { q: "Q".into(), say: "s".into(), key: "A".into(), why: "w".into(), options: vec!["x".into(), "y".into()] }], ..Set::default() };
        let part = PaperPart { mode: "choice".into(), heard: true, heard_options: true, sets: vec![set], ..PaperPart::default() };
        let paper = Paper { sections: vec![PaperSection { parts: vec![part.clone()], ..PaperSection::default() }, PaperSection { parts: vec![part], ..PaperSection::default() }], ..Paper::default() };
        let mut e = Exam { paper, ..Exam::default() };
        e.answers.insert("0".into(), json!({ "answers": ["A"], "result": { "fraction": 1.0 } }));
        let v = view(&e);
        let shown = &v["paper"]["sections"][0]["parts"][0]["sets"][0];
        let hidden = &v["paper"]["sections"][1]["parts"][0]["sets"][0];
        assert_eq!((shown["items"][0]["key"].as_str(), shown["lines"][0]["text"].as_str()), (Some("A"), Some("Hello")));
        assert_eq!((hidden["items"][0]["key"].as_str(), hidden["items"][0]["why"].as_str(), hidden["items"][0]["say"].as_str()), (Some(""), Some(""), Some("")));
        assert_eq!((hidden["lines"].as_array().unwrap().len(), hidden["items"][0]["options"][1].as_str()), (0, Some("")));
        assert_eq!(v["next"], 1);
    }

    #[test]
    fn speakers_get_a_voice_of_their_gender() {
        assert_eq!(voice_for("en", 3, "M"), 16, "Heart is a woman's voice, Michael the first man's");
        assert_eq!(voice_for("en", 26, "M"), 26);
        assert_eq!(voice_for("en", 26, "W"), 3);
        assert_eq!(voice_for("en", 26, ""), 26);
        assert_eq!(voice_for("zh", 3, "M"), 58);
    }

    #[test]
    fn only_a_model_strong_enough_writes_papers() {
        let dir = std::env::temp_dir().join(format!("card-english-exam-{}", std::process::id()));
        let local = ai::Settings { provider: "local".into(), ..ai::Settings::default() };
        assert_eq!(writer(&local, &dir), Err(SMALL_MODEL));
        assert_eq!(writer(&ai::Settings { local_model: "llm15".into(), ..local }, &dir), Ok(Some(LOCAL_WARNING)));
        assert_eq!(writer(&ai::Settings { provider: "gemini".into(), ..ai::Settings::default() }, &dir), Ok(None));
    }

    /// Writes one small section with Qwen 1.5B and prints it: `cargo test qwen_writes_a_section -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the downloaded Qwen 1.5B: cargo test -- --ignored"]
    fn qwen_writes_a_section() {
        let dir = crate::models_only();
        let settings = ai::Settings { provider: "local".into(), local_model: "llm15".into(), ..ai::Settings::default() };
        let ai = ai::Ai::new(&settings, &dir);
        for (purpose, zh, skill) in [("IELTS 6.5", false, "Đọc"), ("HSK 3", true, "Nghe")] {
            let r = roadmap(purpose, "", &[skill, "Viết"]);
            let mut p = plan(&r, Some(0), zh);
            p.topics = vec!["Du lịch".into()];
            let start = std::time::Instant::now();
            let paper = write_paper(&ai, &p, &["hotel".into(), "luggage".into()], zh, &|d, t, l| println!("{d}/{t} {l}")).unwrap();
            println!("{purpose} in {:?}: {}", start.elapsed(), serde_json::to_string_pretty(&paper).unwrap());
            assert!(paper.sections[0].parts.iter().map(|p| p.sets.iter().map(|s| s.items.len()).sum::<usize>()).sum::<usize>() >= 2);
        }
    }
}
