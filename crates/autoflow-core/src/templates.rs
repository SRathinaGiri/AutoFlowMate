//! Reusable flow templates: ready-made flows to start from. Built-in ones are in `templates\`
//! next to the programs; your own are saved in `%APPDATA%\AutoFlowMate\templates\`.

use anyhow::{anyhow, bail, Context as _, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TemplateInfo {
    pub path: PathBuf,
    /// The file name without .json, used by `autoflow new --template NAME`.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Saved by the user (rather than shipped with AutoFlowMate).
    pub own: bool,
}

/// The folder for your own templates.
pub fn user_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("AutoFlowMate").join("templates"))
}

/// Folders searched: next to the program, the source tree's folder (when run from target\release),
/// then your own.
pub fn dirs() -> Vec<(PathBuf, bool)> {
    let mut v = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push((d.join("templates"), false));
            v.push((d.join("..").join("..").join("templates"), false));
        }
    }
    if let Some(u) = user_dir() {
        v.push((u, true));
    }
    v
}

/// Every template, built-in ones first, sorted by name.
pub fn list() -> Vec<TemplateInfo> {
    let mut out: Vec<TemplateInfo> = vec![];
    for (dir, own) in dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut found = vec![];
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let id = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if out.iter().any(|t| t.id == id && t.own == own) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let name = if s("name").is_empty() { id.clone() } else { s("name") };
            found.push(TemplateInfo { path: p, id, name, description: s("description"), own });
        }
        found.sort_by_key(|t| t.name.to_lowercase());
        out.extend(found);
    }
    out
}

/// Find a template by id (file name) or name.
pub fn find(name: &str) -> Option<TemplateInfo> {
    let n = name.trim().trim_end_matches(".json");
    let all = list();
    all.iter().find(|t| t.id.eq_ignore_ascii_case(n)).or_else(|| all.iter().find(|t| t.name.eq_ignore_ascii_case(n))).cloned()
}

/// Save a flow (its JSON text) as one of your own templates. Returns where it was saved.
pub fn save_own(flow_json: &str, name: &str) -> Result<PathBuf> {
    let dir = user_dir().ok_or_else(|| anyhow!("APPDATA is not set"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let file: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if file.is_empty() {
        bail!("give the template a name");
    }
    let path = dir.join(format!("{file}.json"));
    std::fs::write(&path, flow_json).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// The text of a template, ready to save as a new flow.
pub fn read(t: &TemplateInfo) -> Result<String> {
    std::fs::read_to_string(&t.path).with_context(|| format!("cannot read {}", t.path.display()))
}

/// Is `p` inside one of the template folders? (Studio saves a copy instead of overwriting.)
pub fn is_template(p: &Path) -> bool {
    let Ok(c) = std::fs::canonicalize(p) else { return false };
    dirs().iter().filter_map(|(d, _)| std::fs::canonicalize(d).ok()).any(|d| c.starts_with(d))
}
