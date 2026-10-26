//! The language model behind explaining a word in Vietnamese and suggesting new words: Qwen2.5 0.5B on this
//! machine, the user's ChatGPT plan, or Gemini and DeepSeek with an API key. The chosen one is asked first; when
//! it fails, the others that are ready take over, and every call is kept for measuring usage.

use crate::{chatgpt, cloud, db, models, pron};
use candle_core::{Device, Tensor};
use candle_transformers::{
    generation::LogitsProcessor, models::quantized_qwen2::ModelWeights, utils::apply_repeat_penalty,
};
use serde::Serialize;
use serde_json::Value;
use std::{cell::RefCell, path::Path, sync::Mutex};
use tokenizers::Tokenizer;

pub const PROVIDERS: [&str; 4] = ["local", "chatgpt", "gemini", "deepseek"];
const LOCAL_MODEL: &str = "qwen2.5-0.5b-instruct";

const EXPLAIN_SYSTEM: &str = "You are an English-Vietnamese dictionary for Vietnamese learners. \
Reply with exactly these five lines and nothing else:\n\
TYPE: <part of speech in English>\n\
NGHIA: <short Vietnamese meaning>\n\
GIAITHICH: <one short Vietnamese sentence on when to use it>\n\
VIDU: <one simple English sentence using the word>\n\
DICH: <Vietnamese translation of that sentence>";
/// One worked answer, which a model this small needs in order to keep to the format.
const EXPLAIN_SHOT: (&str, &str) = (
    "apple",
    "TYPE: noun\n\
     NGHIA: quả táo\n\
     GIAITHICH: Dùng để chỉ loại quả tròn, vỏ đỏ hoặc xanh, ăn được.\n\
     VIDU: She eats an apple every morning.\n\
     DICH: Cô ấy ăn một quả táo mỗi sáng.",
);

const SUGGEST_SYSTEM: &str = "You suggest useful English vocabulary to Vietnamese learners. \
Reply with exactly 8 lines and nothing else. Each line is: english word | nghĩa tiếng Việt. \
Use common, practical words. No numbering, no explanations.";
const SUGGEST_SHOT: (&str, &str) = (
    "Topic: weather",
    "sunny | có nắng\nrain | mưa\ncloudy | nhiều mây\nstorm | bão\ntemperature | nhiệt độ\nforecast | dự báo\nhumid | ẩm ướt\nfreezing | lạnh cóng",
);
pub const SUGGEST_COUNT: usize = 8;

/// What every provider is asked: a system prompt, one worked example and the question.
pub struct Prompt<'a> {
    pub system: &'a str,
    pub example: (&'a str, &'a str),
    pub question: &'a str,
    /// For the model on this machine; the providers keep their answers short by themselves.
    pub max_tokens: usize,
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
    pub chatgpt_model: String,
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

pub fn label(provider: &str) -> &'static str {
    match provider {
        "chatgpt" => "ChatGPT",
        "gemini" => "Gemini",
        "deepseek" => "DeepSeek",
        _ => "Model trên máy (Qwen 0.5B)",
    }
}

fn usage_url(provider: &str) -> &'static str {
    match provider {
        "chatgpt" => chatgpt::USAGE_URL,
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

    pub fn ready(&self, provider: &str) -> bool {
        match provider {
            "local" => models::ready(self.dir, "llm"),
            "chatgpt" => {
                self.database().ok().and_then(|db| chatgpt::session(&db)).is_some_and(|s| s.signed_in() && s.plan())
            }
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
            "gemini" => cloud::gemini_models(&self.settings.gemini_key),
            "deepseek" => cloud::deepseek_models(&self.settings.deepseek_key),
            _ => Ok(vec![ModelInfo { id: LOCAL_MODEL.into(), name: "Qwen2.5 0.5B Instruct".into() }]),
        }
    }

    fn chosen(&self, provider: &str) -> &str {
        match provider {
            "chatgpt" => &self.settings.chatgpt_model,
            "gemini" => &self.settings.gemini_model,
            "deepseek" => &self.settings.deepseek_model,
            _ => LOCAL_MODEL,
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
            "gemini" => cloud::gemini_chat(&self.settings.gemini_key, &model, prompt),
            "deepseek" => cloud::deepseek_chat(&self.settings.deepseek_key, &model, prompt),
            _ => local(prompt, self.dir),
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
                    failures.push(format!("{}: {}.", label(provider), e.message.trim_end_matches('.')));
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
                format!("{} Đã dùng {} thay thế.", failures.join(" "), label(provider))
            };
            let by = By { source: provider.into(), source_label: label(provider).into(), model, note, manage_url: manage_url.into() };
            return Ok((answer.text, by));
        }
        Err(failures.join(" "))
    }

    pub fn explain(&self, word: &str) -> Result<Explain, String> {
        let word = word.trim();
        if word.is_empty() || word.chars().count() > 80 {
            return Err("Từ không hợp lệ".into());
        }
        let prompt = Prompt { system: EXPLAIN_SYSTEM, example: EXPLAIN_SHOT, question: word, max_tokens: 220 };
        let (text, by) = self.chat(&prompt)?;
        let mut out = parse_explain(&text);
        if out.meaning.is_empty() {
            return Err(format!("AI trả lời không đúng mẫu: {}", text.trim()));
        }
        out.by = by;
        Ok(out)
    }

    /// New words on a topic or, without one, near the words already learned; none that are in `known`.
    pub fn suggest(&self, topic: &str, learned: &[String], known: &[String]) -> Result<Suggestions, String> {
        let topic: String = topic.trim().chars().take(80).collect();
        let question = if !topic.is_empty() {
            format!("Topic: {topic}")
        } else if !learned.is_empty() {
            let sample: Vec<&str> = learned.iter().take(15).map(String::as_str).collect();
            format!("I already know: {}. Suggest new words related to these, not these.", sample.join(", "))
        } else {
            "Topic: everyday life".to_owned()
        };
        let prompt = Prompt { system: SUGGEST_SYSTEM, example: SUGGEST_SHOT, question: &question, max_tokens: 200 };
        let (text, by) = self.chat(&prompt)?;
        let items = parse_suggestions(&text, known);
        if items.is_empty() {
            return Err("AI chưa gợi ý được từ mới, hãy thử chủ đề khác".into());
        }
        Ok(Suggestions { items, by })
    }
}

/// The five lines of an explanation; labels are matched loosely since a small model drifts.
fn parse_explain(text: &str) -> Explain {
    let mut out = Explain::default();
    for line in text.lines() {
        let Some((label, value)) = line.split_once(':') else { continue };
        let label: String = label
            .chars()
            .filter(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_uppercase();
        let value = value.trim().trim_matches('*').trim().to_owned();
        // By the start of the ASCII letters, so that `NGHĨA` or `VÍ DỤ` with accents still match.
        let slot = match label.as_str() {
            l if l.starts_with("TYPE") => &mut out.pos,
            l if l.starts_with("NGH") => &mut out.meaning,
            l if l.starts_with("GI") => &mut out.explain,
            l if l.starts_with('V') => &mut out.example,
            l if l.starts_with('D') => &mut out.example_vi,
            _ => continue,
        };
        if slot.is_empty() {
            *slot = value;
        }
    }
    out
}

/// `word | meaning` lines, tolerant of numbering and other separators; words already known are left out.
fn parse_suggestions(text: &str, known: &[String]) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | ' '));
        let Some((word, meaning)) = ["|", " - ", ":", "–"].iter().find_map(|sep| line.split_once(sep)) else {
            continue;
        };
        let word = word.trim().trim_matches('*').trim().to_lowercase();
        let meaning = meaning.trim().trim_matches('*').trim().to_owned();
        let plain = word.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, ' ' | '-' | '\''));
        if word.is_empty() || word.len() > 40 || !plain || meaning.is_empty() {
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

/// Loaded on first use and kept, since reading the weights takes a second or two.
static LLM: Mutex<Option<Llm>> = Mutex::new(None);

/// The width of the output layer of the pinned Qwen2.5 0.5B file, larger than its tokenizer's vocabulary.
const QWEN_VOCAB: usize = 151_936;

fn load(dir: &Path) -> Result<Llm, String> {
    let (weights, tokenizer) = (models::path(dir, models::QWEN), models::path(dir, models::QWEN_TOKENIZER));
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
    if !models::ready(dir, "llm") {
        return;
    }
    let prompt = Prompt { system: "You are helpful.", example: ("hi", "hello"), question: "hi", max_tokens: 1 };
    if let Err(e) = local(&prompt, dir) {
        eprintln!("warm up: {}", e.message);
    }
}

fn local(prompt: &Prompt, dir: &Path) -> Result<Answer, CallError> {
    if !models::ready(dir, "llm") {
        return Err("chưa tải model, vào Cài đặt để tải hoặc chọn AI khác".into());
    }
    let mut guard = LLM.lock().map_err(|_| "model đang lỗi")?;
    if guard.is_none() {
        *guard = Some(load(dir)?);
    }
    let llm = guard.as_mut().ok_or("model chưa nạp")?;
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
        let logits = logits.broadcast_add(&llm.no_cjk).map_err(err)?;
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
        text: decoded.chars().filter(|&c| !is_cjk(c)).collect(),
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
        let s = ai.suggest("travel", &[], &known).unwrap();
        println!("travel: {:?}", s.items);
        assert!(s.items.len() >= 4 && s.items.iter().all(|s| s.word != "passport"));
        assert!(ai.attempts.borrow().iter().all(|a| a.ok && a.output_tokens > 0));
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
        assert_eq!(parse_explain(EXPLAIN_SHOT.1).meaning, "quả táo");
    }

    #[test]
    fn suggestions_skip_known_words_numbering_and_junk() {
        let text = "1. Hotel | khách sạn\n2) passport - hộ chiếu\n**luggage**: hành lý\nhotel | trùng\n航班 | chuyến bay\nSure, here you go\nboarding pass | thẻ lên máy bay";
        let s = parse_suggestions(text, &["Passport".into()]);
        let words: Vec<&str> = s.iter().map(|s| s.word.as_str()).collect();
        assert_eq!(words, ["hotel", "luggage", "boarding pass"]);
        assert_eq!(s[0].meaning, "khách sạn");
        assert_eq!(s[0].ipa, "/hoʊˈtɛl/");
        assert_eq!(parse_suggestions(SUGGEST_SHOT.1, &[]).len(), 8);
    }
}
