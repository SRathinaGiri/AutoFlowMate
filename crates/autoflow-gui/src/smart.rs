//! Field editors that know about the PC and the user's settings: provider pickers fed by the
//! presets (File > Settings), AI model lists, ODBC data sources, and example hints.

use crate::i18n::tr;
use autoflow_core::presets::Presets;
use eframe::egui::{self, RichText};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Lists the editors offer, loaded once per session (presets reload after Settings > Save).
#[derive(Clone, Default)]
pub struct Lists {
    pub presets: Presets,
    /// ODBC data sources: (name, driver).
    pub dsns: Vec<(String, String)>,
    pub drivers: Vec<String>,
}

impl Lists {
    pub fn load() -> Lists {
        Lists { presets: Presets::load(), dsns: autoflow_core::db::odbc_data_sources(), drivers: autoflow_core::db::odbc_drivers() }
    }
}

/// Results of buttons that ask something (loaded models, connection tests), by field path.
#[derive(Default)]
pub struct Cache {
    pub models: HashMap<String, Result<Vec<String>, String>>,
    pub tests: HashMap<String, Result<String, String>>,
}

/// Actions whose `provider` names a mail preset.
pub fn is_mail_action(action: &str) -> bool {
    matches!(action, "send_email" | "read_email" | "trigger_email")
}

/// A text box plus a ▾ list of suggestions. Typing stays possible (for `${variables}`).
pub fn text_with_list(ui: &mut egui::Ui, path: &str, v: &mut Value, items: &[(String, String)], hint: &str, width: f32) -> bool {
    let mut s = v.as_str().unwrap_or("").to_string();
    let mut changed = ui.add_sized([width, 22.0], egui::TextEdit::singleline(&mut s).hint_text(hint)).changed();
    egui::ComboBox::from_id_salt(format!("{path}#list")).selected_text("▾").width(24.0).show_ui(ui, |ui| {
        if items.is_empty() {
            ui.weak(tr("(nothing to choose)"));
        }
        for (value, label) in items {
            if ui.selectable_label(s == *value, label).clicked() {
                s = value.clone();
                changed = true;
            }
        }
    });
    if changed {
        *v = Value::String(s);
    }
    changed
}

/// The "e.g. …" part of a field's help text, used as the empty field's hint.
pub fn example_hint(help: &str) -> Option<String> {
    let i = help.find("e.g. ")?;
    let rest = &help[i + 5..];
    // Up to the end of the sentence / clause.
    let end = rest.find(['\n']).unwrap_or(rest.len());
    let mut s = rest[..end].trim().to_string();
    for stop in [" (", "; ", ". "] {
        if let Some(j) = s.find(stop) {
            s.truncate(j);
        }
    }
    let s = s.trim_end_matches(['.', ',', ')']).trim_matches('"').to_string();
    (!s.is_empty()).then(|| format!("e.g. {s}"))
}

/// The AI preset a step's `provider` points at (if it is not a ${variable}).
pub fn ai_preset<'a>(lists: &'a Lists, ctx: &Map<String, Value>) -> Option<&'a autoflow_core::presets::AiPreset> {
    let p = ctx.get("provider").and_then(Value::as_str).unwrap_or("");
    if p.contains("${") {
        return None;
    }
    lists.presets.find_ai(p)
}

/// Provider names for a field, by action.
pub fn provider_items(lists: &Lists, action: &str) -> Vec<(String, String)> {
    if action == "ai_prompt" {
        lists
            .presets
            .ai
            .iter()
            .map(|p| {
                let where_ = if p.base_url.contains("localhost") || p.base_url.contains("127.0.0.1") {
                    "on this PC"
                } else if p.base_url.is_empty() {
                    "set Server URL"
                } else {
                    "cloud"
                };
                (p.name.clone(), format!("{}   ({}, {where_})", p.name, p.api.label()))
            })
            .collect()
    } else if is_mail_action(action) {
        let mut v: Vec<(String, String)> = lists
            .presets
            .mail
            .iter()
            .map(|p| (p.name.clone(), format!("{}   ({})", p.name, if p.smtp_host.is_empty() { &p.imap_host } else { &p.smtp_host })))
            .collect();
        v.push(("custom".into(), "custom   (set the server fields)".into()));
        v
    } else {
        vec![]
    }
}

/// ai_prompt `model`: the preset's models, plus the models a local server reports.
pub fn model_field(ui: &mut egui::Ui, path: &str, v: &mut Value, lists: &Lists, ctx: &Map<String, Value>, cache: &mut Cache) -> bool {
    let preset = ai_preset(lists, ctx);
    let mut items: Vec<(String, String)> = vec![];
    let hint = match preset {
        Some(p) if !p.default_model.is_empty() => format!("default: {}", p.default_model),
        Some(_) => "(the model loaded in the server)".to_string(),
        None => String::new(),
    };
    if let Some(p) = preset {
        items.extend(p.models.iter().map(|m| (m.clone(), m.clone())));
    }
    if let Some(Ok(found)) = cache.models.get(path) {
        for m in found {
            if !items.iter().any(|(x, _)| x == m) {
                items.push((m.clone(), format!("{m}   (on the server)")));
            }
        }
    }
    let mut changed = text_with_list(ui, path, v, &items, &hint, 170.0);
    if let Some(p) = preset.filter(|p| p.api == autoflow_core::presets::AiApi::OpenaiCompatible) {
        let base = ctx.get("base_url").and_then(Value::as_str).filter(|b| !b.trim().is_empty()).unwrap_or(&p.base_url).to_string();
        if ui.small_button("⟳").on_hover_text(format!("Ask {base} which models it has (LM Studio / Ollama must be running)")).clicked() {
            let r = autoflow_core::ai::list_models(&base, "").map_err(|e| format!("{e:#}"));
            if let Ok(list) = &r {
                if v.as_str().unwrap_or("").is_empty() {
                    if let Some(first) = list.first() {
                        *v = Value::String(first.clone());
                        changed = true;
                    }
                }
            }
            cache.models.insert(path.to_string(), r);
        }
        match cache.models.get(path) {
            Some(Err(e)) => {
                ui.colored_label(crate::look::warn(), tr("server not reachable")).on_hover_text(e);
            }
            Some(Ok(list)) if list.is_empty() => {
                ui.weak(tr("no models loaded"));
            }
            _ => {}
        }
    }
    changed
}

/// db_query / db_execute `connection`: SQLite file, ODBC data sources and driver templates.
pub fn connection_field(ui: &mut egui::Ui, path: &str, v: &mut Value, lists: &Lists, flow_dir: Option<&std::path::Path>, cache: &mut Cache) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        let mut s = v.as_str().unwrap_or("").to_string();
        ui.horizontal(|ui| {
            changed |= ui.add_sized([210.0, 22.0], egui::TextEdit::singleline(&mut s).hint_text(tr("sqlite:data/app.db  or  DSN=…"))).changed();
            ui.menu_button("▾", |ui| {
                ui.set_min_width(300.0);
                if ui.button(tr("📄 SQLite database file…")).clicked() {
                    let mut d = rfd::FileDialog::new().add_filter("SQLite", &["db", "sqlite", "sqlite3"]);
                    if let Some(dir) = flow_dir {
                        d = d.set_directory(dir);
                    }
                    if let Some(p) = d.save_file() {
                        let shown = flow_dir
                            .and_then(|b| p.strip_prefix(b).ok())
                            .map(|r| r.display().to_string().replace('\\', "/"))
                            .unwrap_or_else(|| p.display().to_string());
                        s = format!("sqlite:{shown}");
                        changed = true;
                    }
                    ui.close();
                }
                ui.separator();
                ui.label(RichText::new(tr("Data sources (ODBC DSNs) on this PC")).strong());
                if lists.dsns.is_empty() {
                    ui.weak(tr("none set up (Windows: ODBC Data Sources 64-bit)"));
                }
                for (name, driver) in &lists.dsns {
                    if ui.button(format!("{name}   ({driver})")).clicked() {
                        s = format!("DSN={name}");
                        changed = true;
                        ui.close();
                    }
                }
                ui.separator();
                ui.label(RichText::new(tr("Drivers: start a connection string")).strong());
                for d in &lists.drivers {
                    if ui.button(d).on_hover_text(autoflow_core::db::connection_template(d)).clicked() {
                        s = autoflow_core::db::connection_template(d);
                        changed = true;
                        ui.close();
                    }
                }
            });
            if ui.small_button(tr("🔌 Test")).on_hover_text(tr("Open the connection once (needs no password field filled for SQLite)")).clicked() {
                let dir = flow_dir.map(|d| d.to_path_buf()).unwrap_or_default();
                let r = if s.contains("${") || s.contains("{password}") {
                    Err("uses a variable or {password}: test it by running the flow".to_string())
                } else {
                    autoflow_core::db::test_connection(&s, &dir).map_err(|e| format!("{e:#}"))
                };
                cache.tests.insert(path.to_string(), r);
            }
        });
        if let Some(r) = cache.tests.get(path) {
            match r {
                Ok(m) => ui.colored_label(crate::look::ok(), format!("✔ {m}")),
                Err(e) => ui.colored_label(crate::look::err(), format!("✖ {e}")),
            };
        }
        if changed {
            *v = Value::String(s);
            cache.tests.remove(path);
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_from_help() {
        assert_eq!(
            example_hint("The prompt (interpolated), e.g. \"Classify this comment: ${row.Comment}\".").as_deref(),
            Some("e.g. Classify this comment: ${row.Comment}")
        );
        assert_eq!(example_hint("Sheet name.").as_deref(), None);
        assert_eq!(example_hint("Folder to watch, e.g. inbox (relative paths ...)").as_deref(), Some("e.g. inbox"));
    }
}
