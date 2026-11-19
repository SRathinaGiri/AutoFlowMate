//! `ai_prompt`: one prompt, one reply, from a local or cloud language model.
//!
//! Providers:
//! - `lmstudio` / `ollama` / `openai_compatible`: the OpenAI chat-completions format
//!   (LM Studio's local server, Ollama, vLLM, LocalAI, Groq, OpenRouter...).
//! - `openai`: api.openai.com.
//! - `gemini`: Google's Gemini API (generateContent).
//! - `anthropic`: Anthropic's Messages API (Claude).
//!
//! API keys are read from Windows Credential Manager (`autoflow secret set NAME`), never from the flow.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

use crate::http;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    LmStudio,
    Ollama,
    OpenAiCompatible,
    OpenAi,
    Gemini,
    Anthropic,
}

impl Provider {
    pub fn parse(s: &str) -> Result<Provider> {
        Ok(match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "" | "lmstudio" | "lm_studio" => Provider::LmStudio,
            "ollama" => Provider::Ollama,
            "openai_compatible" | "compatible" | "custom" => Provider::OpenAiCompatible,
            "openai" | "chatgpt" => Provider::OpenAi,
            "gemini" | "google" => Provider::Gemini,
            "anthropic" | "claude" => Provider::Anthropic,
            other => bail!("unknown AI provider `{other}` (use lmstudio, ollama, openai_compatible, openai, gemini or anthropic)"),
        })
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::LmStudio => "http://localhost:1234/v1",
            Provider::Ollama => "http://localhost:11434/v1",
            Provider::OpenAiCompatible => "",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Provider::Anthropic => "https://api.anthropic.com",
        }
    }

    /// Model used when the step leaves `model` empty. Local servers use whatever model is loaded.
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::OpenAi => "gpt-4.1-mini",
            Provider::Gemini => "gemini-2.5-flash",
            Provider::Anthropic => "claude-opus-5-5",
            _ => "",
        }
    }

    /// Cloud providers need an API key.
    pub fn needs_key(self) -> bool {
        matches!(self, Provider::OpenAi | Provider::Gemini | Provider::Anthropic)
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub system: String,
    pub prompt: String,
    pub temperature: Option<f64>,
    pub max_tokens: u32,
    /// Ask for a JSON reply (where the provider supports a JSON mode).
    pub json: bool,
    pub timeout: Duration,
}

/// The model's text reply. Returns (reply, model actually used).
pub fn complete(r: &Request) -> Result<(String, String)> {
    let base = if r.base_url.trim().is_empty() { r.provider.default_base_url().to_string() } else { r.base_url.trim().to_string() };
    if base.is_empty() {
        bail!("set base_url for provider openai_compatible (e.g. http://localhost:8000/v1)");
    }
    let base = base.trim_end_matches('/').to_string();
    if r.provider.needs_key() && r.api_key.is_empty() {
        bail!("this provider needs an API key: store it with `autoflow secret set NAME` and put NAME in api_key_secret");
    }
    let model = if r.model.trim().is_empty() { r.provider.default_model().to_string() } else { r.model.trim().to_string() };
    let reply = match r.provider {
        Provider::Gemini => gemini(r, &base, &model)?,
        Provider::Anthropic => anthropic(r, &base, &model)?,
        _ => {
            let model = if model.is_empty() { first_local_model(r, &base)? } else { model };
            let reply = chat_completions(r, &base, &model)?;
            return Ok((strip_thinking(&reply), model));
        }
    };
    Ok((strip_thinking(&reply), model))
}

fn bearer(r: &Request) -> Vec<(String, String)> {
    if r.api_key.is_empty() {
        vec![]
    } else {
        vec![("authorization".into(), format!("Bearer {}", r.api_key))]
    }
}

/// LM Studio / Ollama: the first model the server lists (the loaded one).
fn first_local_model(r: &Request, base: &str) -> Result<String> {
    let v = http::get_json(&format!("{base}/models"), &bearer(r), Duration::from_secs(10)).map_err(|e| {
        anyhow!("cannot reach the local AI server at {base} ({e}). Start LM Studio's local server (Developer tab) or Ollama, and load a model")
    })?;
    v.get("data")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("the AI server at {base} has no model loaded; load one or set `model`"))
}

fn chat_completions(r: &Request, base: &str, model: &str) -> Result<String> {
    let mut messages = Vec::new();
    if !r.system.trim().is_empty() {
        messages.push(json!({"role": "system", "content": r.system}));
    }
    messages.push(json!({"role": "user", "content": r.prompt}));
    let mut body = json!({"model": model, "messages": messages, "stream": false});
    // OpenAI's newer models only accept max_completion_tokens; local servers expect max_tokens.
    let limit_key = if r.provider == Provider::OpenAi { "max_completion_tokens" } else { "max_tokens" };
    body[limit_key] = json!(r.max_tokens);
    if let Some(t) = r.temperature {
        body["temperature"] = json!(t);
    }
    if r.json && r.provider == Provider::OpenAi {
        body["response_format"] = json!({"type": "json_object"});
    }
    let v = http::post_json(&format!("{base}/chat/completions"), &bearer(r), &body, r.timeout).map_err(|e| key_hint(e, r))?;
    let choice = v.get("choices").and_then(|c| c.get(0)).ok_or_else(|| anyhow!("no reply in response: {}", short(&v)))?;
    let text = choice.pointer("/message/content").and_then(Value::as_str).unwrap_or("").to_string();
    if text.trim().is_empty() {
        let why = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("unknown");
        bail!("the model returned an empty reply (finish_reason: {why}); raise max_tokens if it is `length`");
    }
    Ok(text)
}

fn gemini(r: &Request, base: &str, model: &str) -> Result<String> {
    let mut body = json!({
        "contents": [{"role": "user", "parts": [{"text": r.prompt}]}],
        "generationConfig": {"maxOutputTokens": r.max_tokens},
    });
    if !r.system.trim().is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": r.system}]});
    }
    if let Some(t) = r.temperature {
        body["generationConfig"]["temperature"] = json!(t);
    }
    if r.json {
        body["generationConfig"]["responseMimeType"] = json!("application/json");
    }
    let headers = vec![("x-goog-api-key".to_string(), r.api_key.clone())];
    let url = format!("{base}/models/{model}:generateContent");
    let v = http::post_json(&url, &headers, &body, r.timeout).map_err(|e| key_hint(e, r))?;
    let Some(cand) = v.get("candidates").and_then(|c| c.get(0)) else {
        let why = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str).unwrap_or("no candidates");
        bail!("Gemini returned no reply ({why})");
    };
    let text: String = cand
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|p| !p.get("thought").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        let why = cand.get("finishReason").and_then(Value::as_str).unwrap_or("unknown");
        bail!("Gemini returned an empty reply (finishReason: {why}); raise max_tokens if it is MAX_TOKENS");
    }
    Ok(text)
}

/// Models that support Anthropic's server-side refusal fallback.
const FALLBACK_MODELS: &[&str] = &["claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5", "claude-fable-5-2", "claude-fable-5-1"];

fn anthropic(r: &Request, base: &str, model: &str) -> Result<String> {
    let mut body = json!({
        "model": model,
        "max_tokens": r.max_tokens,
        "messages": [{"role": "user", "content": r.prompt}],
    });
    if !r.system.trim().is_empty() {
        body["system"] = json!(r.system);
    }
    // Current Claude models reject sampling parameters, so `temperature` is not sent.
    let mut headers = vec![("x-api-key".to_string(), r.api_key.clone()), ("anthropic-version".to_string(), "2023-06-01".to_string())];
    if FALLBACK_MODELS.contains(&model) {
        // If the model declines a request, let the API retry it on a suitable model.
        headers.push(("anthropic-beta".into(), "server-side-fallback-2026-07-01".into()));
        body["fallbacks"] = json!("default");
    }
    let v = http::post_json(&format!("{base}/v1/messages"), &headers, &body, r.timeout).map_err(|e| key_hint(e, r))?;
    let stop = v.get("stop_reason").and_then(Value::as_str).unwrap_or("");
    if stop == "refusal" {
        let cat = v.pointer("/stop_details/category").and_then(Value::as_str).unwrap_or("unspecified");
        bail!("Claude declined to answer this prompt (category: {cat})");
    }
    let text: String = v
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        bail!("Claude returned an empty reply (stop_reason: {stop}); raise max_tokens if it is max_tokens");
    }
    Ok(text)
}

fn key_hint(e: anyhow::Error, r: &Request) -> anyhow::Error {
    let msg = format!("{e:#}");
    if msg.starts_with("HTTP 401") || msg.starts_with("HTTP 403") {
        anyhow!("{msg} - check the API key stored in Credential Manager (api_key_secret)")
    } else if msg.to_lowercase().contains("connect") && matches!(r.provider, Provider::LmStudio | Provider::Ollama) {
        anyhow!("{msg} - is the local AI server (LM Studio / Ollama) running?")
    } else {
        e
    }
}

fn short(v: &Value) -> String {
    v.to_string().chars().take(300).collect()
}

/// Remove `<think>...</think>` reasoning that some local models put in their reply.
pub fn strip_thinking(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(start) = out.find("<think>") {
        match out[start..].find("</think>") {
            Some(end) => out.replace_range(start..start + end + "</think>".len(), ""),
            None => {
                out.truncate(start);
                break;
            }
        }
    }
    out.trim().to_string()
}

/// Instruction appended to the system prompt when the step has `labels`.
pub fn label_instruction(labels: &[String]) -> String {
    format!("Reply with exactly one of these labels and nothing else: {}.", labels.join(", "))
}

/// Find which label a reply means: an exact match first, then the earliest
/// label that appears as a whole word ("UnFav" does not count as "Fav").
pub fn pick_label(reply: &str, labels: &[String]) -> Option<String> {
    let clean = reply.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    if let Some(l) = labels.iter().find(|l| l.trim().to_lowercase() == clean) {
        return Some(l.clone());
    }
    let hay = reply.to_lowercase();
    let mut best: Option<(usize, usize, &String)> = None; // (position, -len, label)
    for l in labels {
        let needle = l.trim().to_lowercase();
        if needle.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(i) = hay[from..].find(&needle) {
            let s = from + i;
            let e = s + needle.len();
            let before = hay[..s].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
            let after = hay[e..].chars().next().map_or(true, |c| !c.is_alphanumeric());
            if before && after {
                let better = match best {
                    None => true,
                    Some((p, n, _)) => s < p || (s == p && needle.len() > n),
                };
                if better {
                    best = Some((s, needle.len(), l));
                }
                break;
            }
            from = s + needle.len().max(1);
        }
    }
    best.map(|(_, _, l)| l.clone())
}

/// Parse a JSON reply, tolerating ```json fences and text around the object.
pub fn extract_json(reply: &str) -> Option<Value> {
    let t = reply.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let t = t.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(s), Some(e)) = (t.find(open), t.rfind(close)) {
            if e > s {
                if let Ok(v) = serde_json::from_str::<Value>(&t[s..=e]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn labels() {
        let labels = l(&["Fav", "UnFav", "Neutral"]);
        assert_eq!(pick_label("UnFav", &labels).as_deref(), Some("UnFav"));
        assert_eq!(pick_label(" fav.", &labels).as_deref(), Some("Fav"));
        assert_eq!(pick_label("**Unfav**", &labels).as_deref(), Some("UnFav"));
        assert_eq!(pick_label("The sentiment is UnFav because...", &labels).as_deref(), Some("UnFav"));
        assert_eq!(pick_label("Label: Neutral (not Fav)", &labels).as_deref(), Some("Neutral"));
        assert_eq!(pick_label("Positive", &labels), None);
    }

    #[test]
    fn thinking_and_json() {
        assert_eq!(strip_thinking("<think>hmm\nok</think>\nFav"), "Fav");
        assert_eq!(extract_json("```json\n{\"a\": 1}\n```").unwrap()["a"], 1);
        assert_eq!(extract_json("Sure! {\"label\": \"Fav\"} hope that helps").unwrap()["label"], "Fav");
        assert!(extract_json("no json here").is_none());
    }

    #[test]
    fn providers() {
        assert_eq!(Provider::parse("LM Studio").unwrap(), Provider::LmStudio);
        assert_eq!(Provider::parse("claude").unwrap(), Provider::Anthropic);
        assert!(Provider::parse("nope").is_err());
    }
}
