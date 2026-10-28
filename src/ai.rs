//! The language model behind explaining a word in Vietnamese and suggesting new words: Qwen2.5 0.5B on this
//! machine, the user's ChatGPT plan, or Gemini and DeepSeek with an API key. The chosen one is asked first; when
//! it fails, the others that are ready take over, and every call is kept for measuring usage.

use crate::{chatgpt, cloud, codex, db, models, pron, roadmap};
use candle_core::{Device, Tensor};
use candle_transformers::{
    generation::LogitsProcessor, models::quantized_qwen2::ModelWeights, utils::apply_repeat_penalty,
};
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, path::Path, sync::Mutex};
use tokenizers::Tokenizer;

pub const PROVIDERS: [&str; 5] = ["local", "chatgpt", "codex", "gemini", "deepseek"];

const EXPLAIN_SYSTEM: &str = "You are an English-Vietnamese dictionary for Vietnamese learners. \
Reply with exactly these ten lines and nothing else:\n\
TYPE: <part of speech in English>\n\
NGHIA: <short Vietnamese meaning>\n\
GIAITHICH: <one short Vietnamese sentence on when to use it>\n\
VIDU: <one simple English sentence using the word>\n\
DICH: <Vietnamese translation of that sentence>\n\
DONGNGHIA: <up to 3 English synonyms, comma separated, or ->\n\
TRAINGHIA: <up to 3 English antonyms, comma separated, or ->\n\
CUMTU: <up to 3 common English collocations with the word, comma separated>\n\
CAPDO: <the CEFR level of the word: A1, A2, B1, B2, C1 or C2>\n\
CHUDE: <one short Vietnamese topic such as Du lịch, Công việc, Ẩm thực>";
/// One worked answer, which a model this small needs in order to keep to the format.
const EXPLAIN_SHOT: (&str, &str) = (
    "apple",
    "TYPE: noun\n\
     NGHIA: quả táo\n\
     GIAITHICH: Dùng để chỉ loại quả tròn, vỏ đỏ hoặc xanh, ăn được.\n\
     VIDU: She eats an apple every morning.\n\
     DICH: Cô ấy ăn một quả táo mỗi sáng.\n\
     DONGNGHIA: -\n\
     TRAINGHIA: -\n\
     CUMTU: apple juice, apple pie, green apple\n\
     CAPDO: A1\n\
     CHUDE: Ẩm thực",
);

const CHECK_SYSTEM: &str = "You are a friendly English teacher for Vietnamese learners. The learner wrote one sentence \
with a given word. Reply with exactly these four lines and nothing else:\n\
DUNG: <YES if the sentence is correct and natural, otherwise NO>\n\
SUA: <the corrected sentence, or the same sentence if it is already correct>\n\
GIAITHICH: <one or two short Vietnamese sentences on the mistakes, or a short praise if it is correct>\n\
GOIY: <one more natural English sentence using the word>";
const CHECK_SHOT: (&str, &str) = (
    "Word: deadline\nSentence: I must finish report before deadline.",
    "DUNG: NO\n\
     SUA: I must finish the report before the deadline.\n\
     GIAITHICH: Cần thêm mạo từ \"the\" trước \"report\" và \"deadline\" vì người nghe biết đó là báo cáo và hạn chót nào.\n\
     GOIY: The deadline for this project is next Friday.",
);

const STORY_SYSTEM: &str = "You write very short, simple English stories for Vietnamese learners. Use every word of the \
list at least once. Reply with exactly these three lines and nothing else:\n\
TITLE: <a short English title>\n\
STORY: <the story, 60 to 110 words, in one paragraph>\n\
DICH: <the Vietnamese translation of the story, in one paragraph>";
const STORY_SHOT: (&str, &str) = (
    "Words: luggage, delay, souvenir",
    "TITLE: A Long Day at the Airport\n\
     STORY: Mai packed her luggage the night before her trip. At the airport there was a two-hour delay, so she \
     walked around the shops. She bought a small souvenir for her brother, a key ring with a plane on it. When the \
     plane finally took off, Mai smiled. The delay was boring, but now she had a gift and a good story to tell.\n\
     DICH: Mai đóng hành lý vào tối hôm trước chuyến đi. Ở sân bay chuyến bay bị trễ hai tiếng, nên cô đi dạo \
     quanh các cửa hàng. Cô mua một món quà lưu niệm nhỏ cho em trai, một chiếc móc khóa hình máy bay. Khi máy bay \
     cuối cùng cũng cất cánh, Mai mỉm cười. Việc chờ đợi thật chán, nhưng giờ cô có một món quà và một câu chuyện \
     hay để kể.",
);

const EXTRACT_SYSTEM: &str = "You help Vietnamese learners pick vocabulary from an English text. Choose up to 10 words or \
short phrases of the text that are most worth learning for an intermediate learner; skip names and very common words \
such as the, is, have, good. Reply with lines like: english word | nghĩa tiếng Việt theo ngữ cảnh. Use the base form \
of each word. No numbering, no explanations.";
const EXTRACT_SHOT: (&str, &str) = (
    "Text: The meeting was postponed because the manager had to negotiate a new contract with an important client.",
    "postpone | hoãn lại\nnegotiate | đàm phán\ncontract | hợp đồng\nclient | khách hàng",
);
/// The longest text read for its words, about a thousand tokens.
const EXTRACT_CHARS: usize = 4000;

// The same five questions for the Chinese course: the answers keep the Vietnamese labels the parsers read.
const EXPLAIN_ZH_SYSTEM: &str = "You are a Chinese-Vietnamese dictionary for Vietnamese learners. \
Reply with exactly these eleven lines and nothing else:\n\
TYPE: <part of speech in English>\n\
NGHIA: <short Vietnamese meaning>\n\
GIAITHICH: <one short Vietnamese sentence on when to use it>\n\
VIDU: <one simple Chinese sentence in simplified characters using the word>\n\
PINYIN: <the pinyin of that sentence with tone marks>\n\
DICH: <Vietnamese translation of that sentence>\n\
DONGNGHIA: <up to 3 Chinese synonyms in simplified characters, comma separated, or ->\n\
TRAINGHIA: <up to 3 Chinese antonyms in simplified characters, comma separated, or ->\n\
CUMTU: <up to 3 common Chinese collocations with the word, comma separated>\n\
CAPDO: <the HSK level of the word, such as HSK 2>\n\
CHUDE: <one short Vietnamese topic such as Du lịch, Công việc, Ẩm thực>";
const EXPLAIN_ZH_SHOT: (&str, &str) = (
    "苹果",
    "TYPE: noun\n\
     NGHIA: quả táo\n\
     GIAITHICH: Dùng để chỉ loại quả tròn, vỏ đỏ hoặc xanh, ăn được.\n\
     VIDU: 我每天早上吃一个苹果。\n\
     PINYIN: Wǒ měitiān zǎoshang chī yí ge píngguǒ.\n\
     DICH: Mỗi sáng tôi ăn một quả táo.\n\
     DONGNGHIA: -\n\
     TRAINGHIA: -\n\
     CUMTU: 苹果汁, 红苹果, 吃苹果\n\
     CAPDO: HSK 1\n\
     CHUDE: Ẩm thực",
);

const SUGGEST_ZH_SYSTEM: &str = "You suggest useful Chinese vocabulary to Vietnamese learners. \
Reply with exactly 8 lines and nothing else. Each line is: Chinese word in simplified characters | nghĩa tiếng Việt. \
Use common, practical words. No pinyin, no numbering, no explanations.";
const SUGGEST_ZH_SHOT: (&str, &str) = (
    "Topic: weather",
    "晴天 | trời nắng\n下雨 | mưa\n阴天 | trời âm u\n刮风 | có gió\n温度 | nhiệt độ\n天气预报 | dự báo thời tiết\n潮湿 | ẩm ướt\n寒冷 | lạnh giá",
);

const CHECK_ZH_SYSTEM: &str = "You are a friendly Chinese teacher for Vietnamese learners. The learner wrote one sentence \
with a given word. Reply with exactly these four lines and nothing else:\n\
DUNG: <YES if the sentence is correct and natural, otherwise NO>\n\
SUA: <the corrected sentence in simplified characters, or the same sentence if it is already correct>\n\
GIAITHICH: <one or two short Vietnamese sentences on the mistakes, or a short praise if it is correct>\n\
GOIY: <one more natural Chinese sentence in simplified characters using the word>";
const CHECK_ZH_SHOT: (&str, &str) = (
    "Word: 喜欢\nSentence: 我喜欢很吃中国菜。",
    "DUNG: NO\n\
     SUA: 我很喜欢吃中国菜。\n\
     GIAITHICH: Phó từ \"很\" phải đứng trước động từ \"喜欢\", không đặt giữa \"喜欢\" và \"吃\".\n\
     GOIY: 她喜欢周末在家看电影。",
);

const STORY_ZH_SYSTEM: &str = "You write very short, simple Chinese stories in simplified characters for Vietnamese \
learners at HSK 2 to 3 level. Use every word of the list at least once. Reply with exactly these three lines and \
nothing else:\n\
TITLE: <a short Chinese title>\n\
STORY: <the story, 60 to 120 Chinese characters, in one paragraph>\n\
DICH: <the Vietnamese translation of the story, in one paragraph>";
const STORY_ZH_SHOT: (&str, &str) = (
    "Words: 行李, 晚点, 纪念品",
    "TITLE: 机场的一天\n\
     STORY: 小美前一天晚上就收拾好了行李。到了机场，她的航班晚点了两个小时，所以她在商店里走了走。她给弟弟买了一个小纪念品。\
     飞机终于起飞的时候，小美笑了。等飞机很无聊，但是现在她有了礼物，也有了一个好故事。\n\
     DICH: Tiểu Mỹ đã thu dọn hành lý từ tối hôm trước. Đến sân bay, chuyến bay của cô bị trễ hai tiếng, nên cô đi dạo \
     một vòng trong các cửa hàng. Cô mua cho em trai một món quà lưu niệm nhỏ. Khi máy bay cuối cùng cũng cất cánh, \
     Tiểu Mỹ mỉm cười. Chờ máy bay thật chán, nhưng giờ cô đã có quà, và cũng có một câu chuyện hay.",
);

const EXTRACT_ZH_SYSTEM: &str = "You help Vietnamese learners pick vocabulary from a Chinese text. Choose up to 10 words of \
the text that are most worth learning for an HSK 3 to 4 learner; skip names and very common words such as 的, 是, 我, \
你, 好. Reply with lines like: 中文词 | nghĩa tiếng Việt theo ngữ cảnh. Use simplified characters. No pinyin, no \
numbering, no explanations.";
const EXTRACT_ZH_SHOT: (&str, &str) = (
    "Text: 因为经理要和一个重要的客户谈新合同，所以会议推迟了。",
    "经理 | giám đốc, quản lý\n重要 | quan trọng\n客户 | khách hàng\n合同 | hợp đồng\n推迟 | hoãn lại",
);

const SUGGEST_SYSTEM: &str = "You suggest useful English vocabulary to Vietnamese learners. \
Reply with exactly 8 lines and nothing else. Each line is: english word | nghĩa tiếng Việt. \
Use common, practical words. No numbering, no explanations.";
const SUGGEST_SHOT: (&str, &str) = (
    "Topic: weather",
    "sunny | có nắng\nrain | mưa\ncloudy | nhiều mây\nstorm | bão\ntemperature | nhiệt độ\nforecast | dự báo\nhumid | ẩm ướt\nfreezing | lạnh cóng",
);
pub const SUGGEST_COUNT: usize = 8;

// A roadmap: one line per stage, the Vietnamese the app shows; the exercises are the app's own.
const ROADMAP_SYSTEM: &str = "You plan English vocabulary study for a Vietnamese learner. Split the days given into \
stages, from easy to hard, as many as asked. Reply with one line per stage and nothing else. Each line is: stage title | \
topics, comma separated | number of new words | skills | exercises | days. Write titles, topics and skills in \
Vietnamese. Choose exercises from: Flashcard, Điền từ, Chọn đáp án, Nghe chép, Luyện nói, Đặt câu, Truyện ngắn. \
The days of all stages add up to the days given.";
const ROADMAP_SHOT: (&str, &str) = (
    "Goal: TOEIC 750\nLevel: A2\nDays: 90\nMinutes a day: 20\nStages: 6",
    "Nền tảng công sở | Công việc văn phòng, Giao tiếp hằng ngày | 60 | Nghe, Từ vựng | Flashcard, Chọn đáp án | 14\n\
     Email và điện thoại | Email, Điện thoại | 60 | Đọc, Viết | Điền từ, Đặt câu | 14\n\
     Họp và thuyết trình | Cuộc họp, Thuyết trình | 70 | Nghe, Nói | Nghe chép, Luyện nói | 15\n\
     Mua bán và hợp đồng | Kinh doanh, Hợp đồng | 70 | Đọc, Từ vựng | Chọn đáp án, Điền từ | 15\n\
     Công tác nước ngoài | Du lịch, Khách sạn | 60 | Nghe, Đọc | Nghe chép, Truyện ngắn | 16\n\
     Luyện đề TOEIC | Từ vựng TOEIC | 50 | Nghe, Đọc | Flashcard, Chọn đáp án | 16",
);
const ROADMAP_ZH_SYSTEM: &str = "You plan Chinese vocabulary study for a Vietnamese learner. Split the days given into \
stages, from easy to hard, as many as asked. Reply with one line per stage and nothing else. Each line is: stage title | \
topics, comma separated | number of new words | skills | exercises | days. Write titles, topics and skills in \
Vietnamese. Choose exercises from: Flashcard, Điền từ, Chọn đáp án, Nghe chép, Luyện nói, Đặt câu, Truyện ngắn. \
The days of all stages add up to the days given.";
const ROADMAP_ZH_SHOT: (&str, &str) = (
    "Goal: HSK 3\nLevel: HSK1\nDays: 120\nMinutes a day: 20\nStages: 5",
    "Chào hỏi và gia đình | Chào hỏi, Gia đình | 60 | Nghe, Phát âm | Flashcard, Luyện nói | 20\n\
     Ăn uống và mua sắm | Ăn uống, Mua sắm | 70 | Nghe, Nói | Chọn đáp án, Nghe chép | 25\n\
     Thời gian và trường học | Thời gian, Trường học | 70 | Đọc, Viết | Điền từ, Đặt câu | 25\n\
     Du lịch và đi lại | Du lịch, Giao thông | 70 | Nghe, Đọc | Nghe chép, Truyện ngắn | 25\n\
     Luyện đề HSK 3 | Từ vựng HSK 3 | 60 | Đọc, Nghe | Flashcard, Chọn đáp án | 25",
);

/// What every provider is asked: a system prompt, one worked example and the question.
pub struct Prompt<'a> {
    pub system: &'a str,
    pub example: (&'a str, &'a str),
    pub question: &'a str,
    /// For the model on this machine; the providers keep their answers short by themselves.
    pub max_tokens: usize,
    /// Whether the answer is in Chinese; otherwise the model on this machine keeps Chinese characters out of it.
    pub cjk: bool,
}

/// A failed call: what to tell the user, whether a usage limit was reached, and what the server said about it.
#[derive(Debug, Default)]
pub struct CallError {
    pub message: String,
    pub limit: bool,
    pub details: Value,
}

impl From<String> for CallError {
    fn from(message: String) -> Self {
        Self { message, ..Self::default() }
    }
}

impl From<&str> for CallError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

/// A model's answer and what it took.
#[derive(Debug, Default)]
pub struct Answer {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The quota left as the server's headers put it, when it says.
    pub limits: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// The choices from the settings.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub provider: String,
    /// The Qwen on this machine: "llm" for 0.5B, "llm15" for 1.5B.
    pub local_model: String,
    pub chatgpt_model: String,
    pub codex_model: String,
    pub gemini_key: String,
    pub gemini_model: String,
    pub deepseek_key: String,
    pub deepseek_model: String,
}

/// One call to one provider.
#[derive(Debug)]
pub struct Attempt {
    pub provider: &'static str,
    pub model: String,
    pub ok: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub error: String,
    pub limit: bool,
    pub details: Value,
    pub limits: Value,
}

/// Who answered, and what happened to the providers asked before it.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct By {
    pub source: String,
    pub source_label: String,
    pub model: String,
    /// Why another provider answered, empty when the chosen one did.
    pub note: String,
    /// Where to manage the limit that was reached, if one was.
    pub manage_url: String,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Explain {
    pub pos: String,
    pub meaning: String,
    pub explain: String,
    pub example: String,
    pub example_vi: String,
    pub synonyms: String,
    pub antonyms: String,
    pub collocations: String,
    /// A1 to C2, HSK1 to HSK9, or empty when the answer had none.
    pub level: String,
    pub topic: String,
    /// Chinese: the Sino-Vietnamese reading of the word, and the pinyin of the example.
    pub hanviet: String,
    pub example_reading: String,
    #[serde(flatten)]
    pub by: By,
}

/// What a teacher says of a sentence the learner wrote.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Check {
    pub ok: bool,
    pub corrected: String,
    pub explain: String,
    pub better: String,
    #[serde(flatten)]
    pub by: By,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Story {
    pub title: String,
    pub story: String,
    pub translation: String,
    /// The words it was asked to use.
    pub words: Vec<String>,
    /// Chinese: the pinyin of the story.
    pub pinyin: String,
    #[serde(flatten)]
    pub by: By,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub word: String,
    pub meaning: String,
    pub ipa: String,
}

#[derive(Debug, Serialize)]
pub struct Suggestions {
    pub items: Vec<Suggestion>,
    #[serde(flatten)]
    pub by: By,
}

/// The stages a model proposes for a goal, made whole by `roadmap::fit`.
#[derive(Debug, Serialize)]
pub struct Plan {
    pub steps: Vec<roadmap::Step>,
    #[serde(flatten)]
    pub by: By,
}

/// The name of a provider; for the model on this machine, of the Qwen in `group`.
fn label(provider: &str, group: &str) -> &'static str {
    match provider {
        "chatgpt" => "ChatGPT",
        "codex" => "ChatGPT (Codex)",
        "gemini" => "Gemini",
        "deepseek" => "DeepSeek",
        _ if group == "llm15" => "Model trên máy (Qwen 1.5B)",
        _ => "Model trên máy (Qwen 0.5B)",
    }
}

/// The local model a call uses: the chosen one when it is downloaded, else the other one that is, else the chosen.
pub fn local_group(dir: &Path, wanted: &str) -> &'static str {
    let wanted = if wanted == "llm15" { "llm15" } else { "llm" };
    let other = if wanted == "llm15" { "llm" } else { "llm15" };
    if !models::ready(dir, wanted) && models::ready(dir, other) { other } else { wanted }
}

/// The name of a local model in the usage records and the answers.
fn local_model_id(group: &str) -> &'static str {
    if group == "llm15" { "qwen2.5-1.5b-instruct" } else { "qwen2.5-0.5b-instruct" }
}

fn usage_url(provider: &str) -> &'static str {
    match provider {
        "chatgpt" => chatgpt::USAGE_URL,
        "codex" => codex::USAGE_URL,
        "gemini" => cloud::GEMINI_USAGE_URL,
        "deepseek" => cloud::DEEPSEEK_USAGE_URL,
        _ => "",
    }
}

/// The model a provider uses when none is chosen. A newer stable flash model has Gemini's most generous free
/// quota; ChatGPT lists its models in the order it recommends them.
fn default_model(provider: &str, list: &[ModelInfo]) -> Option<String> {
    let first = list.first().map(|m| m.id.clone());
    let with = |word: &str| list.iter().find(|m| m.id.contains(word)).map(|m| m.id.clone());
    match provider {
        "gemini" => {
            const AVOID: [&str; 5] = ["lite", "exp", "preview", "thinking", "8b"];
            let version = |id: &str| {
                id.trim_start_matches("models/gemini-").split('-').next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0)
            };
            list.iter()
                .filter(|m| m.id.contains("flash") && !AVOID.iter().any(|w| m.id.contains(w)))
                .max_by(|a, b| version(&a.id).total_cmp(&version(&b.id)))
                .map(|m| m.id.clone())
                .or_else(|| with("flash"))
                .or(first)
        }
        "deepseek" => with("flash").or_else(|| with("chat")).or(first),
        _ => first,
    }
}

pub struct Ai<'a> {
    settings: &'a Settings,
    dir: &'a Path,
    pub attempts: RefCell<Vec<Attempt>>,
}

impl<'a> Ai<'a> {
    pub fn new(settings: &'a Settings, dir: &'a Path) -> Self {
        Self { settings, dir, attempts: RefCell::new(Vec::new()) }
    }

    fn database(&self) -> Result<rusqlite::Connection, String> {
        db::open(&self.dir.join("words.db")).map_err(|e| e.to_string())
    }

    fn group(&self) -> &'static str {
        local_group(self.dir, &self.settings.local_model)
    }

    fn label(&self, provider: &str) -> &'static str {
        label(provider, self.group())
    }

    pub fn ready(&self, provider: &str) -> bool {
        match provider {
            "local" => models::ready(self.dir, self.group()),
            "chatgpt" => {
                self.database().ok().and_then(|db| chatgpt::session(&db)).is_some_and(|s| s.signed_in() && s.plan())
            }
            "codex" => self.database().ok().and_then(|db| codex::session(&db)).is_some_and(|s| s.signed_in()),
            "gemini" => !self.settings.gemini_key.trim().is_empty(),
            "deepseek" => !self.settings.deepseek_key.trim().is_empty(),
            _ => false,
        }
    }

    /// The chosen provider first, then the others that are ready.
    fn order(&self) -> Vec<&'static str> {
        let chosen = PROVIDERS.iter().copied().find(|p| *p == self.settings.provider).unwrap_or("local");
        let mut order = vec![chosen];
        order.extend(PROVIDERS.iter().copied().filter(|p| *p != chosen && self.ready(p)));
        order
    }

    pub fn models(&self, provider: &str) -> Result<Vec<ModelInfo>, CallError> {
        match provider {
            "chatgpt" => chatgpt::models(&self.database()?),
            "codex" => codex::models(&self.database()?),
            "gemini" => cloud::gemini_models(&self.settings.gemini_key),
            "deepseek" => cloud::deepseek_models(&self.settings.deepseek_key),
            _ => Ok(vec![
                ModelInfo { id: local_model_id("llm").into(), name: "Qwen2.5 0.5B Instruct".into() },
                ModelInfo { id: local_model_id("llm15").into(), name: "Qwen2.5 1.5B Instruct".into() },
            ]),
        }
    }

    fn chosen(&self, provider: &str) -> &str {
        match provider {
            "chatgpt" => &self.settings.chatgpt_model,
            "codex" => &self.settings.codex_model,
            "gemini" => &self.settings.gemini_model,
            "deepseek" => &self.settings.deepseek_model,
            _ => local_model_id(self.group()),
        }
    }

    /// The models of a provider and the one in use: the chosen one, or the default the next call would take.
    pub fn choices(&self, provider: &str) -> Result<(Vec<ModelInfo>, String), CallError> {
        let list = self.models(provider)?;
        let chosen = self.chosen(provider);
        let selected = if chosen.is_empty() { default_model(provider, &list).unwrap_or_default() } else { chosen.to_owned() };
        Ok((list, selected))
    }

    /// The model a provider uses: the chosen one, or its default from the provider's list, which is then kept.
    pub fn model(&self, provider: &str) -> Result<String, CallError> {
        let chosen = self.chosen(provider);
        if !chosen.is_empty() {
            return Ok(chosen.to_owned());
        }
        let pick = default_model(provider, &self.models(provider)?).ok_or("Nhà cung cấp không có model nào để dùng")?;
        if let Ok(db) = self.database() {
            let _ = db::set_setting(&db, &format!("{provider}_model"), &pick);
        }
        Ok(pick)
    }

    fn call(&self, provider: &'static str, prompt: &Prompt) -> (String, Result<Answer, CallError>) {
        let model = match self.model(provider) {
            Ok(model) => model,
            Err(e) => return (String::new(), Err(e)),
        };
        let result = match provider {
            "chatgpt" => self.database().map_err(CallError::from).and_then(|db| chatgpt::chat(&db, &model, prompt)),
            "codex" => self.database().map_err(CallError::from).and_then(|db| codex::chat(&db, &model, prompt)),
            "gemini" => cloud::gemini_chat(&self.settings.gemini_key, &model, prompt),
            "deepseek" => cloud::deepseek_chat(&self.settings.deepseek_key, &model, prompt),
            _ => local(prompt, self.dir, self.group()),
        };
        (model, result)
    }

    /// Asks the providers in order until one answers.
    fn chat(&self, prompt: &Prompt) -> Result<(String, By), String> {
        let mut failures = Vec::new();
        let mut manage_url = "";
        for provider in self.order() {
            let (model, result) = self.call(provider, prompt);
            let answer = match result {
                Ok(answer) => answer,
                Err(e) => {
                    if e.limit && manage_url.is_empty() {
                        manage_url = usage_url(provider);
                    }
                    failures.push(format!("{}: {}.", self.label(provider), e.message.trim_end_matches('.')));
                    self.attempts.borrow_mut().push(Attempt {
                        provider,
                        model,
                        ok: false,
                        input_tokens: 0,
                        output_tokens: 0,
                        error: e.message,
                        limit: e.limit,
                        details: e.details,
                        limits: Value::Null,
                    });
                    continue;
                }
            };
            self.attempts.borrow_mut().push(Attempt {
                provider,
                model: model.clone(),
                ok: true,
                input_tokens: answer.input_tokens,
                output_tokens: answer.output_tokens,
                error: String::new(),
                limit: false,
                details: Value::Null,
                limits: answer.limits,
            });
            let note = if failures.is_empty() {
                String::new()
            } else {
                format!("{} Đã dùng {} thay thế.", failures.join(" "), self.label(provider))
            };
            let by = By { source: provider.into(), source_label: self.label(provider).into(), model, note, manage_url: manage_url.into() };
            return Ok((answer.text, by));
        }
        Err(failures.join(" "))
    }

    /// An English or a Chinese word explained in Vietnamese; the script of the word tells which.
    pub fn explain(&self, word: &str) -> Result<Explain, String> {
        let word = word.trim();
        if word.is_empty() || word.chars().count() > 80 {
            return Err("Từ không hợp lệ".into());
        }
        let zh = pron::is_chinese(word);
        let (system, example) = if zh { (EXPLAIN_ZH_SYSTEM, EXPLAIN_ZH_SHOT) } else { (EXPLAIN_SYSTEM, EXPLAIN_SHOT) };
        let prompt = Prompt { system, example, question: word, max_tokens: 380, cjk: zh };
        let (text, by) = self.chat(&prompt)?;
        let mut out = parse_explain(&text);
        if out.meaning.is_empty() {
            return Err(format!("AI trả lời không đúng mẫu: {}", text.trim()));
        }
        if zh {
            out.hanviet = pron::hanviet(word);
            // The dictionary's pinyin when the model gave none, or gave characters instead.
            if out.example_reading.is_empty() || pron::is_chinese(&out.example_reading) {
                out.example_reading = pron::pinyin(&out.example);
            }
        } else {
            out.example_reading.clear();
        }
        out.by = by;
        Ok(out)
    }

    /// New words of a course on a topic or, without one, near the words already learned; none that are in `known`.
    pub fn suggest(&self, topic: &str, learned: &[String], known: &[String], zh: bool) -> Result<Suggestions, String> {
        let topic: String = topic.trim().chars().take(80).collect();
        let question = if !topic.is_empty() {
            format!("Topic: {topic}")
        } else if !learned.is_empty() {
            let sample: Vec<&str> = learned.iter().take(15).map(String::as_str).collect();
            format!("I already know: {}. Suggest new words related to these, not these.", sample.join(", "))
        } else {
            "Topic: everyday life".to_owned()
        };
        let (system, example) = if zh { (SUGGEST_ZH_SYSTEM, SUGGEST_ZH_SHOT) } else { (SUGGEST_SYSTEM, SUGGEST_SHOT) };
        let prompt = Prompt { system, example, question: &question, max_tokens: 200, cjk: zh };
        let (text, by) = self.chat(&prompt)?;
        let mut items = parse_suggestions(&text, known);
        items.retain(|s| pron::is_chinese(&s.word) == zh);
        if items.is_empty() {
            return Err("AI chưa gợi ý được từ mới, hãy thử chủ đề khác".into());
        }
        Ok(Suggestions { items, by })
    }

    /// Corrects a sentence the learner wrote with `word` and says why, in Vietnamese.
    pub fn check_sentence(&self, word: &str, sentence: &str) -> Result<Check, String> {
        let (word, sentence) = (word.trim(), sentence.trim());
        if word.is_empty() || word.chars().count() > 80 {
            return Err("Từ không hợp lệ".into());
        }
        if sentence.is_empty() || sentence.chars().count() > 300 {
            return Err("Hãy viết một câu từ 1 đến 300 ký tự".into());
        }
        let question = format!("Word: {word}\nSentence: {sentence}");
        let zh = pron::is_chinese(word);
        let (system, example) = if zh { (CHECK_ZH_SYSTEM, CHECK_ZH_SHOT) } else { (CHECK_SYSTEM, CHECK_SHOT) };
        let prompt = Prompt { system, example, question: &question, max_tokens: 200, cjk: zh };
        let (text, by) = self.chat(&prompt)?;
        let mut out = parse_check(&text, sentence);
        if out.explain.is_empty() && out.better.is_empty() {
            return Err(format!("AI trả lời không đúng mẫu: {}", text.trim()));
        }
        out.by = by;
        Ok(out)
    }

    /// A short story that uses every one of `words`, with its translation.
    pub fn story(&self, words: &[String]) -> Result<Story, String> {
        let words: Vec<String> = words.iter().map(|w| w.trim().to_owned()).filter(|w| !w.is_empty() && w.chars().count() <= 80).take(12).collect();
        if words.is_empty() {
            return Err("Chưa có từ nào để viết truyện, hãy học vài từ trước".into());
        }
        let question = format!("Words: {}", words.join(", "));
        let zh = words.iter().any(|w| pron::is_chinese(w));
        let (system, example) = if zh { (STORY_ZH_SYSTEM, STORY_ZH_SHOT) } else { (STORY_SYSTEM, STORY_SHOT) };
        let prompt = Prompt { system, example, question: &question, max_tokens: 560, cjk: zh };
        let (text, by) = self.chat(&prompt)?;
        let mut out = parse_story(&text);
        if out.story.is_empty() {
            return Err("AI chưa viết được truyện, hãy thử lại".into());
        }
        if zh {
            out.pinyin = pron::pinyin(&out.story);
        }
        out.words = words;
        out.by = by;
        Ok(out)
    }

    /// The words of an English or a Chinese text most worth learning, none of them in `known`.
    pub fn extract(&self, text: &str, known: &[String]) -> Result<Suggestions, String> {
        let text: String = text.trim().chars().take(EXTRACT_CHARS).collect();
        let zh = text.chars().filter(|&c| pron::is_han(c)).count() >= 3;
        if !zh && text.chars().filter(char::is_ascii_alphabetic).count() < 3 {
            return Err("Hãy dán một đoạn văn tiếng Anh hoặc tiếng Trung".into());
        }
        let question = format!("Text: {text}");
        let (system, example) = if zh { (EXTRACT_ZH_SYSTEM, EXTRACT_ZH_SHOT) } else { (EXTRACT_SYSTEM, EXTRACT_SHOT) };
        let prompt = Prompt { system, example, question: &question, max_tokens: 260, cjk: zh };
        let (answer, by) = self.chat(&prompt)?;
        let mut items = parse_suggestions(&answer, known);
        items.retain(|s| pron::is_chinese(&s.word) == zh);
        if items.is_empty() {
            return Err("AI không tìm thấy từ mới nào đáng học trong đoạn này".into());
        }
        Ok(Suggestions { items, by })
    }

    /// The stages towards `goal` in `days` days, about one for every two weeks; what the model leaves out or gets
    /// wrong is filled in, and only an answer with no stage at all fails.
    pub fn roadmap(&self, goal: &roadmap::Roadmap, days: i64, zh: bool) -> Result<Plan, String> {
        let stages = (days / 14).clamp(roadmap::MIN_STEPS as i64, roadmap::MAX_STEPS as i64);
        let level = if goal.level.is_empty() { "beginner" } else { &goal.level };
        let question = format!(
            "Goal: {}\nLevel: {level}\nDays: {days}\nMinutes a day: {}\nStages: {stages}",
            roadmap::cut(&goal.purpose, 80),
            goal.minutes
        );
        let (system, example) = if zh { (ROADMAP_ZH_SYSTEM, ROADMAP_ZH_SHOT) } else { (ROADMAP_SYSTEM, ROADMAP_SHOT) };
        // The answer is Vietnamese in both courses.
        let prompt = Prompt { system, example, question: &question, max_tokens: 640, cjk: false };
        let (text, by) = self.chat(&prompt)?;
        let steps = parse_roadmap(&text);
        if steps.is_empty() {
            return Err("AI chưa lập được lộ trình, hãy thử lại hoặc tự lập lộ trình".into());
        }
        Ok(Plan { steps: roadmap::fit(steps, days, goal.minutes, zh), by })
    }
}

/// A Vietnamese letter without its marks, `ệ` as `e` and `đ` as `d`; any other character as it is.
fn base_letter(c: char) -> char {
    const GROUPS: [(&str, char); 14] = [
        ("àáạảãâầấậẩẫăằắặẳẵ", 'a'),
        ("ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴ", 'A'),
        ("èéẹẻẽêềếệểễ", 'e'),
        ("ÈÉẸẺẼÊỀẾỆỂỄ", 'E'),
        ("ìíịỉĩ", 'i'),
        ("ÌÍỊỈĨ", 'I'),
        ("òóọỏõôồốộổỗơờớợởỡ", 'o'),
        ("ÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠ", 'O'),
        ("ùúụủũưừứựửữ", 'u'),
        ("ÙÚỤỦŨƯỪỨỰỬỮ", 'U'),
        ("ỳýỵỷỹ", 'y'),
        ("ỲÝỴỶỸ", 'Y'),
        ("đ", 'd'),
        ("Đ", 'D'),
    ];
    GROUPS.iter().find(|(set, _)| set.contains(c)).map_or(c, |&(_, base)| base)
}

/// A label such as `**Nghĩa**` or `VÍ DỤ` as its unaccented capital letters: `NGHIA`, `VIDU`.
fn fold_label(label: &str) -> String {
    label.chars().map(base_letter).filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_uppercase()).collect()
}

/// A value without the stars a model wraps it in.
fn clean(value: &str) -> String {
    value.trim().trim_matches('*').trim().to_owned()
}

/// The values of `LABEL: value` lines, in the slot `slot` gives each folded label; the first value wins. A line
/// without a known label goes on the value before it when that slot is in `long`, and is dropped otherwise.
fn labeled<const N: usize>(text: &str, slot: impl Fn(&str) -> Option<usize>, long: &[usize]) -> [String; N] {
    let mut out: [String; N] = std::array::from_fn(|_| String::new());
    let mut open = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        // Chinese answers often put a full-width colon after the label.
        let found = line.split_once([':', '：']).and_then(|(label, value)| {
            let label = fold_label(label);
            if label.is_empty() || label.len() > 20 { None } else { slot(&label).filter(|&i| i < N).map(|i| (i, value)) }
        });
        match found {
            Some((i, value)) => {
                open = None;
                if out[i].is_empty() {
                    out[i] = clean(value);
                    open = long.contains(&i).then_some(i);
                }
            }
            None => {
                if let Some(i) = open {
                    out[i].push(' ');
                    out[i].push_str(&clean(line));
                }
            }
        }
    }
    out
}

/// Nothing, for the dash or the "none" a model writes when it has no synonym.
fn unless_none(value: String) -> String {
    let lower = value.to_lowercase();
    let none = value.chars().all(|c| matches!(c, '-' | '–' | '—' | '.' | ' '))
        || ["none", "n/a", "không", "không có", "khong co"].contains(&lower.trim_end_matches('.'));
    if none { String::new() } else { value }
}

/// The first CEFR level in `text`, such as `B2`.
fn cefr(text: &str) -> String {
    let up = text.to_ascii_uppercase();
    let b = up.as_bytes();
    (0..b.len().saturating_sub(1))
        .find(|&i| matches!(b[i], b'A' | b'B' | b'C') && matches!(b[i + 1], b'1' | b'2'))
        .map(|i| up[i..i + 2].to_owned())
        .unwrap_or_default()
}

/// A level as the app keeps it: `HSK3` from "HSK 3", else the first CEFR level, such as `B2`.
fn level_of(text: &str) -> String {
    let up = text.to_ascii_uppercase();
    let hsk = up.find("HSK").and_then(|at| up[at + 3..].trim_start().chars().next()).filter(|c| ('1'..='9').contains(c));
    hsk.map_or_else(|| cefr(text), |n| format!("HSK{n}"))
}

/// The lines of an explanation; labels are matched by how they start since a small model drifts.
fn parse_explain(text: &str) -> Explain {
    let slot = |l: &str| {
        Some(match l {
            _ if l.starts_with("TY") || l.starts_with("LOAI") => 0,
            _ if l.starts_with("NGH") || l == "MEANING" => 1,
            _ if l.starts_with("GI") => 2,
            _ if l.starts_with('V') || l.starts_with("EXAM") => 3,
            _ if l.starts_with("DI") || l.starts_with("TRANSL") => 4,
            _ if l.starts_with("DO") || l.starts_with("SYN") => 5,
            _ if l.starts_with("TR") || l.starts_with("ANT") => 6,
            _ if l.starts_with("CU") || l.starts_with("COL") => 7,
            _ if l.starts_with("CA") || l.starts_with("LEV") || l.starts_with("CEFR") => 8,
            _ if l.starts_with("CH") || l.starts_with("TOP") => 9,
            _ if l.starts_with("PIN") => 10,
            _ => return None,
        })
    };
    let [pos, meaning, explain, example, example_vi, synonyms, antonyms, collocations, level, topic, example_reading] =
        labeled(text, slot, &[]);
    Explain {
        pos,
        meaning,
        explain,
        example,
        example_vi,
        synonyms: unless_none(synonyms),
        antonyms: unless_none(antonyms),
        collocations: unless_none(collocations),
        level: level_of(&level),
        topic: unless_none(topic),
        hanviet: String::new(),
        example_reading,
        by: By::default(),
    }
}

fn letters(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A teacher's four lines; without a clear yes or no, a sentence left unchanged counts as right.
fn parse_check(text: &str, sentence: &str) -> Check {
    let slot = |l: &str| {
        Some(match l {
            _ if l.starts_with("DU") || l.starts_with("CORRECT") => 0,
            _ if l.starts_with("SU") || l.starts_with("FIX") => 1,
            _ if l.starts_with("GI") || l.starts_with("EXPL") => 2,
            _ if l.starts_with("GO") || l.starts_with("BETTER") => 3,
            _ => return None,
        })
    };
    let [verdict, corrected, explain, better] = labeled(text, slot, &[2]);
    let corrected = if corrected.is_empty() { sentence.trim().to_owned() } else { corrected };
    let verdict = fold_label(&verdict);
    let ok = if ["YES", "CO", "DUNG", "TRUE"].iter().any(|v| verdict.starts_with(v)) {
        true
    } else if ["NO", "KHONG", "SAI", "FALSE"].iter().any(|v| verdict.starts_with(v)) {
        false
    } else {
        letters(&corrected) == letters(sentence)
    };
    Check { ok, corrected, explain, better, by: By::default() }
}

/// A title, the story and its translation. The labels are matched whole, since the story is free text that may
/// hold a colon; lines without a label go on the paragraph before them.
fn parse_story(text: &str) -> Story {
    let slot = |l: &str| match l {
        "TITLE" | "TIEUDE" => Some(0),
        "STORY" | "TRUYEN" | "CAUCHUYEN" => Some(1),
        "DICH" | "BANDICH" | "TRANSLATION" => Some(2),
        _ => None,
    };
    let [title, story, translation] = labeled(text, slot, &[1, 2]);
    // A model that leaves out the labels still wrote a story.
    let story = if story.is_empty() && translation.is_empty() { text.trim().to_owned() } else { story };
    Story { title, story, translation, ..Story::default() }
}

/// `word | meaning` lines, tolerant of numbering and other separators; words already known are left out. A word
/// is English letters or Chinese characters, never both.
fn parse_suggestions(text: &str, known: &[String]) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | ' '));
        let Some((word, meaning)) = ["|", "｜", " - ", ":", "：", "–"].iter().find_map(|sep| line.split_once(sep)) else {
            continue;
        };
        let word = word.trim().trim_matches('*').trim().to_lowercase();
        let meaning = meaning.trim().trim_matches('*').trim().to_owned();
        let english = word.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, ' ' | '-' | '\''));
        let chinese = word.chars().all(pron::is_han);
        if word.is_empty() || word.len() > 40 || !(english || chinese) || meaning.is_empty() {
            continue;
        }
        let seen = known.iter().any(|k| k.eq_ignore_ascii_case(&word)) || out.iter().any(|s| s.word == word);
        if !seen {
            out.push(Suggestion { ipa: pron::ipa(&word), word, meaning });
        }
        if out.len() == SUGGEST_COUNT + 2 {
            break;
        }
    }
    out
}

/// The first whole number in `text`, 0 when there is none.
fn first_number(text: &str) -> i64 {
    text.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// An exercise of the app from the name a model gave it.
fn exercise_of(name: &str) -> Option<&'static str> {
    let n = fold_label(name);
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    Some(match () {
        _ if has(&["FLASH"]) => "flash",
        _ if has(&["DIEN", "FILL", "GAP"]) => "fill",
        _ if has(&["CHON", "CHOICE", "QUIZ"]) => "choice",
        _ if has(&["NGHE", "DICTA", "LISTEN"]) => "dict",
        _ if has(&["NOI", "SPEAK", "PHATAM"]) => "speak",
        _ if has(&["DATCAU", "VIET", "WRIT", "SENTENCE"]) => "write",
        _ if has(&["TRUYEN", "STORY", "DOC", "READ"]) => "story",
        _ => return None,
    })
}

/// `title | topics | words | skills | exercises | days` lines, tolerant of numbering, a "Stage 1:" before the
/// title, the pipes of a Markdown table and its header, and spans in weeks or months. What is missing stays empty
/// or zero for `roadmap::fit` to fill; a line without a title and a topic is no stage.
fn parse_roadmap(text: &str) -> Vec<roadmap::Step> {
    const HEADERS: [&str; 7] = ["TENCHANG", "CHANG", "TITLE", "STAGE", "STAGETITLE", "GIAIDOAN", "TEN"];
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_matches(['|', '｜']);
        let fields: Vec<&str> = line.split(['|', '｜']).map(str::trim).collect();
        if fields.len() < 2 {
            continue;
        }
        let title = fields[0].trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | '#' | ' '));
        let title = match title.split_once([':', '：']) {
            Some((head, rest)) if head.chars().count() <= 14 && head.chars().any(|c| c.is_ascii_digit()) => rest,
            _ => title,
        };
        let title = clean(title);
        let header = HEADERS.contains(&fold_label(&title).as_str()) && !title.chars().any(|c| c.is_ascii_digit());
        if title.is_empty() || header {
            continue;
        }
        let field = |i: usize| fields.get(i).copied().unwrap_or_default();
        let topics = field(1).split([',', '，', '、', ';']).map(clean).filter(|t| !t.is_empty()).collect();
        let span = fold_label(field(5));
        let unit = if span.contains("TUAN") || span.contains("WEEK") {
            7
        } else if span.contains("THANG") || span.contains("MONTH") {
            30
        } else {
            1
        };
        let mut exercises: Vec<String> = Vec::new();
        for kind in field(4).split([',', '，', '、', ';', '/']).filter_map(exercise_of) {
            if !exercises.iter().any(|e| e == kind) {
                exercises.push(kind.into());
            }
        }
        out.push(roadmap::Step {
            title,
            topics,
            target: first_number(field(2)),
            skills: clean(field(3)),
            exercises,
            days: first_number(field(5)).saturating_mul(unit),
            done: false,
        });
    }
    out
}

/// The GPU of a Mac when there is one, else the CPU; made once, since tensors must share the same device.
/// CARD_ENGLISH_CPU=1 forces the CPU.
pub fn device() -> Device {
    static DEVICE: std::sync::OnceLock<Device> = std::sync::OnceLock::new();
    DEVICE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            if std::env::var_os("CARD_ENGLISH_CPU").is_none() {
                if let Ok(gpu) = Device::new_metal(0) {
                    return gpu;
                }
            }
            Device::Cpu
        })
        .clone()
}

/// Chinese, Japanese and Korean, which Qwen slips into its Vietnamese now and then.
fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}')
}

struct Llm {
    model: ModelWeights,
    tokenizer: Tokenizer,
    stops: Vec<u32>,
    /// Added to the logits: minus infinity on the tokens that spell CJK characters.
    no_cjk: Tensor,
}

/// Loaded on first use and kept with the group it came from, since reading the weights takes a second or two.
static LLM: Mutex<Option<(&str, Llm)>> = Mutex::new(None);

/// Lets go of the model in memory, before its file is deleted or another one is chosen.
pub fn unload() {
    *LLM.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// The width of the output layer of the pinned Qwen2.5 0.5B file, larger than its tokenizer's vocabulary.
const QWEN_VOCAB: usize = 151_936;

fn load(dir: &Path, group: &str) -> Result<Llm, String> {
    let weights = models::path(dir, if group == "llm15" { models::QWEN15 } else { models::QWEN });
    let tokenizer = models::path(dir, models::QWEN_TOKENIZER);
    let mut file = std::fs::File::open(&weights).map_err(|e| format!("{}: {e}", weights.display()))?;
    let content = candle_core::quantized::gguf_file::Content::read(&mut file).map_err(|e| e.to_string())?;
    let model = ModelWeights::from_gguf(content, &mut file, &device()).map_err(|e| e.to_string())?;
    let tokenizer = Tokenizer::from_file(&tokenizer).map_err(|e| e.to_string())?;
    let stops = ["<|im_end|>", "<|endoftext|>"]
        .iter()
        .filter_map(|t| tokenizer.token_to_id(t))
        .collect();
    let mask: Vec<f32> = (0..QWEN_VOCAB as u32)
        .map(|id| {
            let text = tokenizer.decode(&[id], false).unwrap_or_default();
            if text.chars().any(is_cjk) { f32::NEG_INFINITY } else { 0.0 }
        })
        .collect();
    let no_cjk = Tensor::new(mask.as_slice(), &device()).map_err(|e| e.to_string())?;
    Ok(Llm { model, tokenizer, stops, no_cjk })
}

/// Loads the model ahead of the first question and runs it once, so that the GPU kernels are compiled by then.
pub fn warm_up(dir: &Path) {
    let wanted = db::open(&dir.join("words.db")).ok().and_then(|db| db::setting(&db, "local_model").ok().flatten()).unwrap_or_default();
    let group = local_group(dir, &wanted);
    if !models::ready(dir, group) {
        return;
    }
    let prompt = Prompt { system: "You are helpful.", example: ("hi", "hello"), question: "hi", max_tokens: 1, cjk: false };
    if let Err(e) = local(&prompt, dir, group) {
        eprintln!("warm up: {}", e.message);
    }
}

fn local(prompt: &Prompt, dir: &Path, group: &'static str) -> Result<Answer, CallError> {
    if !models::ready(dir, group) {
        return Err("chưa tải model, vào Cài đặt để tải hoặc chọn AI khác".into());
    }
    let mut guard = LLM.lock().map_err(|_| "model đang lỗi")?;
    if guard.as_ref().is_none_or(|(loaded, _)| *loaded != group) {
        // The old one goes first, so that both are never in memory together.
        *guard = None;
        *guard = Some((group, load(dir, group)?));
    }
    let (_, llm) = guard.as_mut().ok_or("model chưa nạp")?;
    let mut text = String::new();
    for (role, content) in [("system", prompt.system), ("user", prompt.example.0), ("assistant", prompt.example.1), ("user", prompt.question)] {
        text.push_str(&format!("<|im_start|>{role}\n{content}<|im_end|>\n"));
    }
    text.push_str("<|im_start|>assistant\n");
    let err = |e: candle_core::Error| e.to_string();
    let device = device();
    let mut tokens = llm.tokenizer.encode(text, false).map_err(|e| e.to_string())?.get_ids().to_vec();
    let prompt_len = tokens.len();
    // Greedy, so that the same word gets the same answer.
    let mut sampler = LogitsProcessor::new(0, None, None);
    let mut output = Vec::new();
    for i in 0..prompt.max_tokens {
        let (input, pos) = if i == 0 { (&tokens[..], 0) } else { (&tokens[tokens.len() - 1..], tokens.len() - 1) };
        let input = Tensor::new(input, &device).and_then(|t| t.unsqueeze(0)).map_err(err)?;
        let logits = llm.model.forward(&input, pos).and_then(|l| l.squeeze(0)).map_err(err)?;
        let logits = if prompt.cjk { logits } else { logits.broadcast_add(&llm.no_cjk).map_err(err)? };
        let recent = &tokens[prompt_len.max(tokens.len().saturating_sub(64))..];
        let logits = apply_repeat_penalty(&logits, 1.1, recent).map_err(err)?;
        let next = sampler.sample(&logits).map_err(err)?;
        if llm.stops.contains(&next) {
            break;
        }
        tokens.push(next);
        output.push(next);
    }
    let decoded = llm.tokenizer.decode(&output, true).map_err(|e| e.to_string())?;
    Ok(Answer {
        text: if prompt.cjk { decoded } else { decoded.chars().filter(|&c| !is_cjk(c)).collect() },
        input_tokens: prompt_len as u64,
        output_tokens: output.len() as u64,
        limits: Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the downloaded models: cargo test -- --ignored"]
    fn qwen_explains_and_suggests() {
        let settings = Settings { provider: "local".into(), ..Settings::default() };
        let dir = crate::data_dir();
        let ai = Ai::new(&settings, &dir);
        for word in ["happy", "take off", "environment"] {
            let start = std::time::Instant::now();
            let e = ai.explain(word).unwrap();
            println!("{word}: {e:?} in {:?}", start.elapsed());
            assert_eq!(e.by.source, "local");
            assert!(!e.meaning.is_empty() && !e.example.is_empty(), "{e:?}");
        }
        let known = ["passport".to_owned()];
        let s = ai.suggest("travel", &[], &known, false).unwrap();
        println!("travel: {:?}", s.items);
        assert!(s.items.len() >= 4 && s.items.iter().all(|s| s.word != "passport"));
        assert!(ai.attempts.borrow().iter().all(|a| a.ok && a.output_tokens > 0));
    }

    #[test]
    fn the_local_model_is_the_chosen_one_until_only_the_other_is_there() {
        let dir = std::env::temp_dir().join(format!("card-english-local-{}", std::process::id()));
        assert_eq!((local_group(&dir, ""), local_group(&dir, "llm15"), local_group(&dir, "x")), ("llm", "llm15", "llm"));
        assert_eq!((label("local", "llm"), label("local", "llm15")), ("Model trên máy (Qwen 0.5B)", "Model trên máy (Qwen 1.5B)"));
        assert_eq!(local_model_id("llm15"), "qwen2.5-1.5b-instruct");
        unload();
    }

    #[test]
    fn the_chosen_provider_goes_first_then_the_ready_ones() {
        let dir = std::env::temp_dir().join(format!("card-english-ai-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings { provider: "chatgpt".into(), gemini_key: "key".into(), ..Settings::default() };
        let ai = Ai::new(&settings, &dir);
        // No model downloaded and nobody signed in: only Gemini, which has a key, backs ChatGPT up.
        assert_eq!(ai.order(), ["chatgpt", "gemini"]);
        let unknown = Settings { provider: "x".into(), ..Settings::default() };
        assert_eq!(Ai::new(&unknown, &dir).order(), ["local"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_falls_through_and_says_so() {
        let dir = std::env::temp_dir().join(format!("card-english-ai-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings { provider: "local".into(), ..Settings::default() };
        let ai = Ai::new(&settings, &dir);
        let e = ai.explain("cat").unwrap_err();
        assert!(e.starts_with("Model trên máy (Qwen 0.5B): chưa tải model"), "{e}");
        assert_eq!(ai.attempts.borrow().len(), 1);
        assert!(!ai.attempts.borrow()[0].ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_models_prefer_a_stable_flash() {
        let list = |ids: &[&str]| ids.iter().map(|id| ModelInfo { id: (*id).into(), name: (*id).into() }).collect::<Vec<_>>();
        let gemini = list(&["models/gemini-2.0-flash", "models/gemini-2.5-pro", "models/gemini-2.5-flash", "models/gemini-3-flash-preview", "models/gemini-2.5-flash-lite"]);
        assert_eq!(default_model("gemini", &gemini).unwrap(), "models/gemini-2.5-flash");
        assert_eq!(default_model("gemini", &list(&["models/gemini-3-flash-preview"])).unwrap(), "models/gemini-3-flash-preview");
        assert_eq!(default_model("deepseek", &list(&["deepseek-v4-pro", "deepseek-v4-flash"])).unwrap(), "deepseek-v4-flash");
        assert_eq!(default_model("chatgpt", &list(&["gpt-a", "gpt-b"])).unwrap(), "gpt-a");
        assert_eq!(default_model("chatgpt", &[]), None);
    }

    #[test]
    fn parse_reads_loose_labels() {
        let text = "Here you go\n**TYPE**: verb\nNghĩa: chạy\nGIAI THICH: Di chuyển nhanh bằng chân.\nVIDU: I run every day.\nDICH: Tôi chạy mỗi ngày.";
        let e = parse_explain(text);
        assert_eq!(e.pos, "verb");
        assert_eq!(e.meaning, "chạy");
        assert_eq!(e.example, "I run every day.");
        assert_eq!(e.example_vi, "Tôi chạy mỗi ngày.");
        assert_eq!(e.explain, "Di chuyển nhanh bằng chân.");
        assert_eq!(parse_explain("VÍ DỤ: Hi.\nDỊCH: Chào.").example_vi, "Chào.");
        let shot = parse_explain(EXPLAIN_SHOT.1);
        assert_eq!((shot.meaning.as_str(), shot.synonyms.as_str(), shot.level.as_str(), shot.topic.as_str()), ("quả táo", "", "A1", "Ẩm thực"));
        assert_eq!(shot.collocations, "apple juice, apple pie, green apple");
    }

    #[test]
    fn the_longer_explanation_takes_accented_labels() {
        let text = "TYPE: verb\nNGHĨA: đàm phán\nĐỒNG NGHĨA: bargain, discuss\nTRÁI NGHĨA: none\nCỤM TỪ: negotiate a deal\nCẤP ĐỘ: Level b2 (upper)\nCHỦ ĐỀ: Công việc\nHope this helps: yes";
        let e = parse_explain(text);
        assert_eq!(e.meaning, "đàm phán");
        assert_eq!(e.synonyms, "bargain, discuss");
        assert_eq!(e.antonyms, "");
        assert_eq!(e.collocations, "negotiate a deal");
        assert_eq!(e.level, "B2");
        assert_eq!(e.topic, "Công việc");
        assert_eq!(fold_label("**Giải thích**"), "GIAITHICH");
        assert_eq!(cefr("c1"), "C1");
        assert_eq!(cefr("D4"), "");
    }

    #[test]
    fn a_checked_sentence_says_right_or_wrong() {
        let c = parse_check(CHECK_SHOT.1, "I must finish report before deadline.");
        assert!(!c.ok);
        assert_eq!(c.corrected, "I must finish the report before the deadline.");
        assert!(c.explain.starts_with("Cần thêm mạo từ") && c.better.starts_with("The deadline"));
        let yes = parse_check("ĐÚNG: Có\nSỬA: She is reliable.\nGIẢI THÍCH: Câu đúng.\nmore praise", "She is reliable.");
        assert!(yes.ok);
        assert_eq!(yes.explain, "Câu đúng. more praise");
        // No verdict: an unchanged sentence is right, a changed one is not.
        assert!(parse_check("SUA: She is reliable!\nGIAITHICH: Tốt.", "she is reliable").ok);
        assert!(!parse_check("SUA: She is a reliable friend.", "She is reliable.").ok);
        assert_eq!(parse_check("GIAITHICH: x", "Same.").corrected, "Same.");
    }

    #[test]
    fn a_story_keeps_its_paragraphs_and_colons() {
        let s = parse_story(STORY_SHOT.1);
        assert_eq!(s.title, "A Long Day at the Airport");
        assert!(s.story.starts_with("Mai packed") && s.story.ends_with("to tell."));
        assert!(s.translation.starts_with("Mai đóng"));
        let loose = parse_story("Title: Rain\nStory: It rained.\nStill, she said: we go.\nThe end.\nDịch: Trời mưa.\nHết.");
        assert_eq!(loose.story, "It rained. Still, she said: we go. The end.");
        assert_eq!(loose.translation, "Trời mưa. Hết.");
        assert_eq!(parse_story("Once upon a time.").story, "Once upon a time.");
    }

    #[test]
    fn suggestions_skip_known_words_numbering_and_junk() {
        let text = "1. Hotel | khách sạn\n2) passport - hộ chiếu\n**luggage**: hành lý\nhotel | trùng\n航班 | chuyến bay\nSure, here you go\nboarding pass | thẻ lên máy bay";
        let s = parse_suggestions(text, &["Passport".into()]);
        let words: Vec<&str> = s.iter().map(|s| s.word.as_str()).collect();
        // A Chinese word is a word too; each course keeps its own (see `suggest`).
        assert_eq!(words, ["hotel", "luggage", "航班", "boarding pass"]);
        assert_eq!(s[0].meaning, "khách sạn");
        assert_eq!(s[0].ipa, "/hoʊˈtɛl/");
        assert_eq!(parse_suggestions(SUGGEST_SHOT.1, &[]).len(), 8);
    }

    #[test]
    fn a_roadmap_parses_from_tidy_and_from_messy_answers() {
        let steps = parse_roadmap(ROADMAP_SHOT.1);
        assert_eq!(steps.len(), 6);
        assert_eq!(steps.iter().map(|s| s.days).sum::<i64>(), 90);
        let first = &steps[0];
        assert_eq!((first.title.as_str(), first.topics.clone(), first.target), ("Nền tảng công sở", vec!["Công việc văn phòng".to_owned(), "Giao tiếp hằng ngày".to_owned()], 60));
        assert_eq!((first.skills.as_str(), first.exercises.clone()), ("Nghe, Từ vựng", vec!["flash".to_owned(), "choice".to_owned()]));
        assert_eq!(steps[2].exercises, ["dict", "speak"]);
        assert_eq!(steps[4].exercises, ["dict", "story"]);
        let zh = parse_roadmap(ROADMAP_ZH_SHOT.1);
        assert_eq!((zh.len(), zh.iter().map(|s| s.days).sum::<i64>()), (5, 120));
        assert_eq!(zh[2].exercises, ["fill", "write"]);
        // What a small model writes: chatter, a Markdown table with its header, numbering, weeks, gaps.
        let messy = "Sure! Here is the plan:\n\
            | Tên chặng | Chủ đề | Số từ | Kỹ năng | Bài tập | Số ngày |\n\
            |---|---|---|---|---|---|\n\
            | 1. Chặng 1: **Làm quen** | Chào hỏi、Gia đình | khoảng 40 từ | Nghe | flashcard / luyện nói | 2 tuần |\n\
            2) Stage 2 - Đi chợ | Mua sắm\n\
            Chặng 3 | Du lịch | 50 | | Quiz, Story, Dance | 1 tháng\n\
            Good luck!";
        let steps = parse_roadmap(messy);
        assert_eq!(steps.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Làm quen", "Stage 2 - Đi chợ", "Chặng 3"]);
        assert_eq!((steps[0].topics.clone(), steps[0].target, steps[0].days), (vec!["Chào hỏi".to_owned(), "Gia đình".to_owned()], 40, 14));
        assert_eq!(steps[0].exercises, ["flash", "speak"]);
        assert_eq!((steps[1].target, steps[1].days, steps[1].exercises.len()), (0, 0, 0));
        assert_eq!((steps[2].days, steps[2].exercises.clone()), (30, vec!["choice".to_owned(), "story".to_owned()]));
        // Filled up to a whole roadmap of the goal's days.
        let fitted = roadmap::fit(steps, 60, 20, false);
        assert_eq!((fitted.len(), fitted.iter().map(|s| s.days).sum::<i64>()), (4, 60));
        assert!(fitted.iter().all(|s| s.target > 0 && !s.topics.is_empty() && !s.exercises.is_empty()));
        assert!(parse_roadmap("Học chăm chỉ mỗi ngày.\nChúc bạn thành công!").is_empty());
        assert_eq!((first_number("khoảng 1200 từ"), first_number("-"), exercise_of("Nghe chép")), (1200, 0, Some("dict")));
    }

    #[test]
    fn chinese_answers_parse() {
        let e = parse_explain(EXPLAIN_ZH_SHOT.1);
        assert_eq!((e.meaning.as_str(), e.level.as_str(), e.collocations.as_str()), ("quả táo", "HSK1", "苹果汁, 红苹果, 吃苹果"));
        assert_eq!(e.example, "我每天早上吃一个苹果。");
        assert!(e.example_reading.starts_with("Wǒ měitiān"), "{}", e.example_reading);
        assert_eq!([level_of("hsk 4"), level_of("B2"), level_of("HSK"), level_of("")], ["HSK4", "B2", "", ""]);
        // Full-width bars and colons, as Chinese answers have them; a word is one script or the other.
        let s = parse_suggestions("1. 机场｜sân bay\n护照：hộ chiếu\nhotel | khách sạn\n飞机 plane | máy bay", &["护照".into()]);
        assert_eq!(s.iter().map(|s| s.word.as_str()).collect::<Vec<_>>(), ["机场", "hotel"]);
        assert_eq!(s[0].ipa, "jīchǎng");
        assert_eq!(parse_suggestions(SUGGEST_ZH_SHOT.1, &[]).len(), 8);
        assert_eq!(parse_suggestions(EXTRACT_ZH_SHOT.1, &[]).len(), 5);
        let story = parse_story("TITLE：机场\nSTORY：她说：我们走吧。\n很好。\nDICH：Cô ấy nói: đi thôi.");
        assert_eq!((story.title.as_str(), story.story.as_str()), ("机场", "她说：我们走吧。 很好。"));
        let shot = parse_story(STORY_ZH_SHOT.1);
        assert!(shot.story.starts_with("小美") && shot.story.contains("纪念品。飞机") && shot.translation.starts_with("Tiểu Mỹ"));
        let check = parse_check(CHECK_ZH_SHOT.1, "我喜欢很吃中国菜。");
        assert_eq!((check.ok, check.corrected.as_str()), (false, "我很喜欢吃中国菜。"));
    }
}
