//! The providers that take an API key: Gemini (a free key from Google AI Studio; Google allows no other way for
//! an outside app) and DeepSeek.

use crate::{
    ai::{Answer, CallError, ModelInfo, Prompt},
    net,
};
use serde_json::{Value, json};

const GEMINI: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEEPSEEK: &str = "https://api.deepseek.com";
pub const GEMINI_KEY_URL: &str = "https://aistudio.google.com/apikey";
pub const GEMINI_USAGE_URL: &str = "https://aistudio.google.com/rate-limit";
pub const DEEPSEEK_KEY_URL: &str = "https://platform.deepseek.com/api_keys";
pub const DEEPSEEK_USAGE_URL: &str = "https://platform.deepseek.com/usage";

fn missing(key: &str, name: &str) -> Result<(), CallError> {
    if key.trim().is_empty() { Err(format!("Chưa có {name} API key").into()) } else { Ok(()) }
}

pub fn deepseek_chat(key: &str, model: &str, prompt: &Prompt) -> Result<Answer, CallError> {
    missing(key, "DeepSeek")?;
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": prompt.system },
            { "role": "user", "content": prompt.example.0 },
            { "role": "assistant", "content": prompt.example.1 },
            { "role": "user", "content": prompt.question },
        ],
        "temperature": 0.3,
        "stream": false,
    });
    let auth = format!("Bearer {}", key.trim());
    let headers = [("Content-Type", "application/json"), ("Authorization", auth.as_str())];
    let reply = net::fetch(&format!("{DEEPSEEK}/chat/completions"), &headers, Some(&body.to_string()), 90)?;
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    if reply.status != 200 {
        return Err(deepseek_error(reply.status, &json));
    }
    let text = json["choices"][0]["message"]["content"].as_str().ok_or("DeepSeek trả về rỗng")?;
    Ok(Answer {
        text: text.to_owned(),
        input_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        limits: Value::Null,
    })
}

fn deepseek_error(status: u16, json: &Value) -> CallError {
    let reason = json["error"]["message"].as_str().unwrap_or("không rõ lý do");
    match status {
        401 => "DeepSeek API key không đúng".into(),
        402 => CallError { message: "Tài khoản DeepSeek đã hết số dư".into(), limit: true, details: json!({ "status": 402 }) },
        429 => CallError { message: "DeepSeek đang giới hạn tốc độ, thử lại sau ít phút".into(), limit: true, details: json!({ "status": 429 }) },
        _ => format!("DeepSeek lỗi {status}: {reason}").into(),
    }
}

pub fn deepseek_models(key: &str) -> Result<Vec<ModelInfo>, CallError> {
    missing(key, "DeepSeek")?;
    let auth = format!("Bearer {}", key.trim());
    let reply = net::fetch(&format!("{DEEPSEEK}/models"), &[("Authorization", &auth)], None, 20)?;
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    if reply.status != 200 {
        return Err(deepseek_error(reply.status, &json));
    }
    Ok(json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .map(|id| ModelInfo { id: id.to_owned(), name: id.to_owned() })
        .collect())
}

/// The balance, which is DeepSeek's measure of what is left.
pub fn deepseek_balance(key: &str) -> Result<Value, CallError> {
    missing(key, "DeepSeek")?;
    let auth = format!("Bearer {}", key.trim());
    let reply = net::fetch(&format!("{DEEPSEEK}/user/balance"), &[("Authorization", &auth)], None, 20)?;
    let json: Value = serde_json::from_str(&reply.body).unwrap_or_default();
    if reply.status != 200 {
        return Err(deepseek_error(reply.status, &json));
    }
    let balance: Vec<Value> = json["balance_infos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| json!({ "currency": b["currency"], "total_balance": b["total_balance"] }))
        .collect();
    Ok(json!({ "available": json["is_available"], "balance": balance }))
}

/// A Gemini model name as the API writes it, `models/gemini-...`; anything else could reach another path.
fn gemini_model_ok(model: &str) -> bool {
    model.strip_prefix("models/").is_some_and(|name| {
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
    })
}

pub fn gemini_chat(key: &str, model: &str, prompt: &Prompt) -> Result<Answer, CallError> {
    missing(key, "Gemini")?;
    if !gemini_model_ok(model) {
        return Err(format!("Tên model Gemini không hợp lệ: {model}").into());
    }
    let body = json!({
        "systemInstruction": { "parts": [{ "text": prompt.system }] },
        "contents": [
            { "role": "user", "parts": [{ "text": prompt.example.0 }] },
            { "role": "model", "parts": [{ "text": prompt.example.1 }] },
            { "role": "user", "parts": [{ "text": prompt.question }] },
        ],
        "generationConfig": { "temperature": 0.3 },
    });
    let headers = [("Content-Type", "application/json"), ("x-goog-api-key", key.trim())];
    let reply = net::fetch(&format!("{GEMINI}/{model}:generateContent"), &headers, Some(&body.to_string()), 90)?;
    if reply.status != 200 {
        return Err(gemini_error(reply.status, &reply.body));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    gemini_answer(&json)
}

/// The text of the first candidate, without the model's thoughts.
fn gemini_answer(json: &Value) -> Result<Answer, CallError> {
    let candidate = &json["candidates"][0];
    let text: String = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["thought"] != true)
        .filter_map(|part| part["text"].as_str())
        .collect();
    if text.trim().is_empty() {
        let reason = candidate["finishReason"].as_str().or(json["promptFeedback"]["blockReason"].as_str()).unwrap_or("không rõ lý do");
        return Err(format!("Gemini không trả lời ({reason})").into());
    }
    let usage = &json["usageMetadata"];
    let output = usage["candidatesTokenCount"].as_u64().unwrap_or(0) + usage["thoughtsTokenCount"].as_u64().unwrap_or(0);
    Ok(Answer { text, input_tokens: usage["promptTokenCount"].as_u64().unwrap_or(0), output_tokens: output, limits: Value::Null })
}

/// Gemini's errors; a used-up quota says which limit it was, how high, and when to try again.
fn gemini_error(status: u16, body: &str) -> CallError {
    let json: Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    let message = error["message"].as_str().unwrap_or("không rõ lý do");
    if status == 429 || error["status"] == "RESOURCE_EXHAUSTED" {
        let details = error["details"].as_array().cloned().unwrap_or_default();
        let of_type = |name: &str| details.iter().find(|d| d["@type"].as_str().is_some_and(|t| t.ends_with(name))).cloned();
        let violation = of_type("QuotaFailure").map(|d| d["violations"][0].clone()).unwrap_or_default();
        let retry = of_type("RetryInfo").and_then(|d| d["retryDelay"].as_str().map(str::to_owned));
        let quota_id = violation["quotaId"].as_str().unwrap_or_default();
        let value = violation["quotaValue"].as_str().map(str::to_owned).or(violation["quotaValue"].as_u64().map(|v| v.to_string()));
        let what = if quota_id.contains("Token") { "token" } else { "lượt" };
        let per = if quota_id.contains("PerDay") {
            " mỗi ngày"
        } else if quota_id.contains("PerMinute") {
            " mỗi phút"
        } else {
            ""
        };
        let mut text = "Đã chạm hạn mức Gemini".to_owned();
        if let Some(value) = &value {
            text.push_str(&format!(": tối đa {value} {what}{per}"));
        }
        if let Some(retry) = &retry {
            text.push_str(&format!(", thử lại sau {retry}"));
        }
        let details = json!({
            "quota_id": quota_id,
            "quota_value": value,
            "retry_delay": retry,
            "model": violation["quotaDimensions"]["model"],
        });
        return CallError { message: text, limit: true, details };
    }
    if body.contains("API_KEY_INVALID") {
        return "Gemini API key không đúng".into();
    }
    match status {
        403 => format!("Gemini từ chối (403): {message}").into(),
        404 => "Model Gemini này không còn, hãy chọn model khác".into(),
        _ => format!("Gemini lỗi {status}: {message}").into(),
    }
}

/// The Gemini models that write text, as the key may use them.
pub fn gemini_models(key: &str) -> Result<Vec<ModelInfo>, CallError> {
    missing(key, "Gemini")?;
    let reply = net::fetch(&format!("{GEMINI}/models?pageSize=1000"), &[("x-goog-api-key", key.trim())], None, 20)?;
    if reply.status != 200 {
        return Err(gemini_error(reply.status, &reply.body));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    Ok(parse_gemini_models(&json))
}

fn parse_gemini_models(json: &Value) -> Vec<ModelInfo> {
    const NOT_TEXT: [&str; 7] = ["embedding", "tts", "image", "live", "audio", "robotics", "computer-use"];
    json["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|ms| ms.iter().any(|x| x == "generateContent")))
        .filter_map(|m| {
            let id = m["name"].as_str()?;
            let ok = id.starts_with("models/gemini") && gemini_model_ok(id) && !NOT_TEXT.iter().any(|w| id.contains(w));
            ok.then(|| ModelInfo { id: id.to_owned(), name: m["displayName"].as_str().unwrap_or(id).to_owned() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_used_up_gemini_quota_says_which_and_when() {
        let body = r#"{"error":{"code":429,"message":"You exceeded your current quota","status":"RESOURCE_EXHAUSTED","details":[
            {"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests",
             "quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier","quotaDimensions":{"location":"global","model":"gemini-2.5-flash"},"quotaValue":"250"}]},
            {"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"49s"}]}}"#;
        let e = gemini_error(429, body);
        assert!(e.limit);
        assert_eq!(e.message, "Đã chạm hạn mức Gemini: tối đa 250 lượt mỗi ngày, thử lại sau 49s");
        assert_eq!(e.details["model"], "gemini-2.5-flash");
        let bad_key = r#"{"error":{"code":400,"message":"API key not valid.","status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#;
        assert_eq!(gemini_error(400, bad_key).message, "Gemini API key không đúng");
        assert!(!gemini_error(500, "").limit);
    }

    #[test]
    fn gemini_answers_skip_thoughts_and_count_tokens() {
        let json = json!({ "candidates": [{ "content": { "parts": [{ "text": "thinking", "thought": true }, { "text": "TYPE: noun" }] } }],
                           "usageMetadata": { "promptTokenCount": 40, "candidatesTokenCount": 10, "thoughtsTokenCount": 5 } });
        let a = gemini_answer(&json).unwrap();
        assert_eq!((a.text.as_str(), a.input_tokens, a.output_tokens), ("TYPE: noun", 40, 15));
        assert!(gemini_answer(&json!({ "candidates": [{ "finishReason": "SAFETY" }] })).unwrap_err().message.contains("SAFETY"));
    }

    #[test]
    fn gemini_models_are_text_models_with_safe_names() {
        let json = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/gemini-x/../../bad", "supportedGenerationMethods": ["generateContent"] } ] });
        let ids: Vec<String> = parse_gemini_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["models/gemini-2.5-flash"]);
        assert!(gemini_model_ok("models/gemini-3-flash-preview") && !gemini_model_ok("gemini-pro") && !gemini_model_ok("models/a?b"));
    }

    #[test]
    #[ignore = "talks to the real servers: cargo test -- --ignored"]
    fn real_servers_reject_a_wrong_key_in_words() {
        assert_eq!(gemini_models("AIza-not-a-real-key").unwrap_err().message, "Gemini API key không đúng");
        let prompt = Prompt { system: "s", example: ("a", "b"), question: "q", max_tokens: 10 };
        assert_eq!(gemini_chat("AIza-not-a-real-key", "models/gemini-2.5-flash", &prompt).unwrap_err().message, "Gemini API key không đúng");
        assert_eq!(deepseek_models("sk-not-a-real-key").unwrap_err().message, "DeepSeek API key không đúng");
        assert_eq!(deepseek_balance("sk-not-a-real-key").unwrap_err().message, "DeepSeek API key không đúng");
    }

    #[test]
    fn deepseek_out_of_balance_counts_as_a_limit() {
        assert!(deepseek_error(402, &Value::Null).limit);
        assert!(!deepseek_error(401, &Value::Null).limit);
    }
}
