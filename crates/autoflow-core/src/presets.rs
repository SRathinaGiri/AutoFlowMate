//! Provider presets: the AI services and mail servers that `provider` fields refer to.
//!
//! Built-in presets cover the common services. The user can change, add or delete them in
//! Studio (File > Settings) or by editing `%APPDATA%\AutoFlowMate\presets.json`; once that
//! file exists its lists replace the built-in ones. Flows refer to presets by name, so a flow
//! that uses a self-made preset needs the same preset on the PC that runs it.

use anyhow::{anyhow, Context as _, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The API an AI preset speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiApi {
    /// OpenAI's chat-completions format at any address: LM Studio, Ollama, vLLM, Groq, OpenRouter...
    #[default]
    OpenaiCompatible,
    /// api.openai.com (uses max_completion_tokens).
    Openai,
    /// Google Gemini (generateContent).
    Gemini,
    /// Anthropic Claude (Messages API).
    Anthropic,
}

impl AiApi {
    pub const ALL: [AiApi; 4] = [AiApi::OpenaiCompatible, AiApi::Openai, AiApi::Gemini, AiApi::Anthropic];

    pub fn label(self) -> &'static str {
        match self {
            AiApi::OpenaiCompatible => "OpenAI-compatible",
            AiApi::Openai => "OpenAI",
            AiApi::Gemini => "Gemini",
            AiApi::Anthropic => "Anthropic (Claude)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AiPreset {
    /// The name flows use in `provider`, e.g. lmstudio or openai.
    pub name: String,
    pub api: AiApi,
    /// Server address, e.g. http://localhost:1234/v1.
    pub base_url: String,
    /// Model used when a step leaves `model` empty (empty = the model loaded in a local server).
    pub default_model: String,
    /// Models offered in Studio's model list.
    pub models: Vec<String>,
    /// The service needs an API key.
    pub needs_key: bool,
    /// Credential Manager name of the key, used when a step leaves api_key_secret empty.
    pub key_secret: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MailPreset {
    /// The name flows use in `provider`, e.g. gmail.
    pub name: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// starttls, tls or none.
    pub smtp_security: String,
    pub imap_host: String,
    pub imap_port: u16,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Presets {
    pub ai: Vec<AiPreset>,
    pub mail: Vec<MailPreset>,
}

fn ai(name: &str, api: AiApi, base: &str, model: &str, models: &[&str], key: &str) -> AiPreset {
    AiPreset {
        name: name.into(),
        api,
        base_url: base.into(),
        default_model: model.into(),
        models: models.iter().map(|m| m.to_string()).collect(),
        needs_key: !key.is_empty(),
        key_secret: key.into(),
    }
}

fn mail(name: &str, smtp: &str, smtp_port: u16, sec: &str, imap: &str) -> MailPreset {
    MailPreset { name: name.into(), smtp_host: smtp.into(), smtp_port, smtp_security: sec.into(), imap_host: imap.into(), imap_port: 993 }
}

impl Presets {
    /// The presets AutoFlowMate ships with.
    pub fn builtin() -> Presets {
        use AiApi::*;
        Presets {
            ai: vec![
                ai("lmstudio", OpenaiCompatible, "http://localhost:1234/v1", "", &[], ""),
                ai("ollama", OpenaiCompatible, "http://localhost:11434/v1", "", &["llama3.2", "qwen2.5", "mistral"], ""),
                ai("openai", Openai, "https://api.openai.com/v1", "gpt-4.1-mini", &["gpt-4.1-mini", "gpt-4.1", "gpt-4o-mini"], "openai"),
                ai("gemini", Gemini, "https://generativelanguage.googleapis.com/v1beta", "gemini-2.5-flash", &["gemini-2.5-flash", "gemini-2.5-pro"], "gemini"),
                ai(
                    "anthropic",
                    Anthropic,
                    "https://api.anthropic.com",
                    "claude-opus-5-5",
                    &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
                    "anthropic",
                ),
                ai("openai_compatible", OpenaiCompatible, "", "", &[], ""),
            ],
            mail: vec![
                mail("gmail", "smtp.gmail.com", 587, "starttls", "imap.gmail.com"),
                mail("outlook365", "smtp.office365.com", 587, "starttls", "outlook.office365.com"),
                mail("outlook.com", "smtp-mail.outlook.com", 587, "starttls", "outlook.office365.com"),
                mail("yahoo", "smtp.mail.yahoo.com", 465, "tls", "imap.mail.yahoo.com"),
                mail("zoho", "smtp.zoho.com", 587, "starttls", "imap.zoho.com"),
                mail("zoho.in", "smtp.zoho.in", 587, "starttls", "imap.zoho.in"),
            ],
        }
    }

    /// Where the user's presets live (AUTOFLOW_PRESETS overrides it, e.g. for tests).
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("AUTOFLOW_PRESETS") {
            return PathBuf::from(p);
        }
        let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        base.join("AutoFlowMate").join("presets.json")
    }

    /// The user's presets, or the built-in ones when the file does not exist (or is unreadable).
    pub fn load() -> Presets {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(Self::builtin)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        for (kind, names) in [("AI", self.ai.iter().map(|a| &a.name).collect::<Vec<_>>()), ("mail", self.mail.iter().map(|m| &m.name).collect())] {
            let mut seen = std::collections::HashSet::new();
            for n in names {
                if n.trim().is_empty() {
                    return Err(anyhow!("every {kind} preset needs a name"));
                }
                if !seen.insert(n.trim().to_lowercase()) {
                    return Err(anyhow!("two {kind} presets are called `{n}`"));
                }
            }
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?).with_context(|| format!("cannot write {}", p.display()))?;
        Ok(p)
    }

    pub fn find_ai(&self, name: &str) -> Option<&AiPreset> {
        let n = norm(name);
        let n = match n.as_str() {
            "" | "lm studio" | "lm_studio" => "lmstudio".to_string(),
            "claude" => "anthropic".into(),
            "chatgpt" => "openai".into(),
            "google" => "gemini".into(),
            "compatible" | "custom" => "openai_compatible".into(),
            _ => n,
        };
        self.ai.iter().find(|p| norm(&p.name) == n)
    }

    pub fn find_mail(&self, name: &str) -> Option<&MailPreset> {
        let n = norm(name);
        let n = match n.as_str() {
            "google" => "gmail".to_string(),
            "office365" | "microsoft365" | "m365" => "outlook365".into(),
            "hotmail" => "outlook.com".into(),
            "zoho_in" => "zoho.in".into(),
            _ => n,
        };
        self.mail.iter().find(|p| norm(&p.name) == n)
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_aliases() {
        let p = Presets::builtin();
        assert_eq!(p.find_ai("Claude").unwrap().api, AiApi::Anthropic);
        assert_eq!(p.find_ai("LM Studio").unwrap().base_url, "http://localhost:1234/v1");
        assert!(p.find_ai("nope").is_none());
        assert_eq!(p.find_mail("Office365").unwrap().smtp_host, "smtp.office365.com");
        assert_eq!(p.find_mail("zoho.in").unwrap().imap_host, "imap.zoho.in");
    }

    #[test]
    fn save_checks_names() {
        let mut p = Presets::builtin();
        p.ai.push(AiPreset { name: "OpenAI".into(), ..AiPreset::default() });
        assert!(p.save().unwrap_err().to_string().contains("two AI presets"));
    }
}
