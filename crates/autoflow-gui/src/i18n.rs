//! Translations. Every text Studio shows goes through the `tr` function (with the English text); a language file maps
//! the English text to the translation, and anything missing stays in English.
//!
//! Language files are JSON objects in `lang\` next to autoflow-studio.exe, or in
//! `%APPDATA%\AutoFlowMate\lang\` for your own:
//! `{ "_language": "தமிழ் (Tamil)", "File": "கோப்பு", "Run (F5)": "இயக்கு (F5)" }`.
//! `autoflow-studio --language-template out.json` writes every text with empty values to start from.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

static TABLE: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// The translation of an English text (the text itself when there is none).
pub fn tr(english: &str) -> String {
    if let Ok(g) = TABLE.read() {
        if let Some(map) = g.as_ref() {
            if let Some(tr) = map.get(english).filter(|s| !s.trim().is_empty()) {
                return tr.clone();
            }
        }
    }
    english.to_string()
}

/// Folders searched for language files.
pub fn folders() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.join("lang"));
            // Running from target\release in the source tree: use the repository's lang folder.
            v.push(d.join("..").join("..").join("lang"));
        }
    }
    if let Ok(a) = std::env::var("APPDATA") {
        v.push(PathBuf::from(a).join("AutoFlowMate").join("lang"));
    }
    v
}

/// Languages available: (code = file name without .json, display name).
pub fn available() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![("en".into(), "English".into())];
    for dir in folders() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let code = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if code == "template" || out.iter().any(|(c, _)| *c == code) {
                continue;
            }
            let name = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
                .and_then(|m| m.get("_language").cloned())
                .unwrap_or_else(|| code.clone());
            out.push((code, name));
        }
    }
    out
}

/// Switch language ("en" = built-in English). Returns an error text if the file can't be read.
pub fn set_language(code: &str) -> Result<(), String> {
    if code.is_empty() || code == "en" {
        *TABLE.write().unwrap() = None;
        return Ok(());
    }
    for dir in folders() {
        let p = dir.join(format!("{code}.json"));
        if let Ok(text) = std::fs::read_to_string(&p) {
            let map: HashMap<String, String> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            *TABLE.write().unwrap() = Some(map);
            return Ok(());
        }
    }
    Err(format!("language file {code}.json not found"))
}

/// Every text Studio can show, for translators: the texts passed to `tr` in the source (collected at
/// build time) plus action descriptions, field names, help and option descriptions.
pub fn all_texts(schema: &crate::schema_info::SchemaInfo) -> Vec<String> {
    let mut v: Vec<String> = include!(concat!(env!("OUT_DIR"), "/ui_strings.rs")).iter().map(|s| s.to_string()).collect();
    v.extend(crate::look::SCALES.iter().map(|(_, l)| l.to_string()));
    for a in autoflow_core::catalog() {
        v.push(a.category.to_string());
        v.push(a.description.to_string());
        if let Some(fields) = schema.fields(a.name) {
            for (k, f) in fields {
                v.push(crate::form::pretty_english(k));
                if !f.help.is_empty() {
                    v.push(f.help.clone());
                }
                for (_, d) in &f.options {
                    if !d.is_empty() {
                        v.push(d.replace('`', ""));
                    }
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v.retain(|s| !s.trim().is_empty() && s.chars().any(char::is_alphabetic));
    v
}

/// Write a template: every text with an empty translation.
pub fn write_template(path: &std::path::Path, schema: &crate::schema_info::SchemaInfo) -> std::io::Result<usize> {
    let texts = all_texts(schema);
    let mut m = serde_json::Map::new();
    m.insert("_language".into(), serde_json::Value::String("Language name, e.g. தமிழ் (Tamil)".into()));
    for s in &texts {
        m.insert(s.clone(), serde_json::Value::String(String::new()));
    }
    std::fs::write(path, serde_json::to_string_pretty(&serde_json::Value::Object(m))?)?;
    Ok(texts.len())
}
