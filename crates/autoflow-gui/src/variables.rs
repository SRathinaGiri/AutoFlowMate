//! The Variables tab: declare flow variables with a type and a type-specific editor.

use autoflow_core::flow::VarType;
use autoflow_core::Flow;
use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// Per-tab editing state (name/list buffers survive between frames).
#[derive(Default)]
pub struct VarsUi {
    names: HashMap<usize, String>,
    lists: HashMap<String, String>,
    /// Row to highlight (just created from the properties form).
    pub highlight: Option<String>,
}

fn valid_name(n: &str) -> bool {
    n.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) && n.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Add a variable (used by the properties form's "➕ New variable…").
pub fn add_variable(flow: &mut Flow, name: &str, ty: VarType) {
    if !flow.variables.contains_key(name) {
        flow.variables.insert(name.to_string(), ty.default_value());
        flow.variable_types.insert(name.to_string(), ty);
    }
}

/// Rename a variable, keeping its position, and update `${old}` references in all steps.
fn rename(flow: &mut Flow, old: &str, new: &str) -> bool {
    if old == new || flow.variables.contains_key(new) || !valid_name(new) {
        return false;
    }
    let vars: Map<String, Value> = std::mem::take(&mut flow.variables)
        .into_iter()
        .map(|(k, v)| if k == old { (new.to_string(), v) } else { (k, v) })
        .collect();
    flow.variables = vars;
    if let Some(t) = flow.variable_types.remove(old) {
        flow.variable_types.insert(new.to_string(), t);
    }
    // References in steps and in other variables' values.
    let fix = |s: String| s.replace(&format!("${{{old}}}"), &format!("${{{new}}}")).replace(&format!("${{{old}."), &format!("${{{new}."));
    if let Ok(text) = serde_json::to_string(&flow.steps) {
        if let Ok(steps) = serde_json::from_str(&fix(text)) {
            flow.steps = steps;
        }
    }
    for v in flow.variables.values_mut() {
        if let Value::String(s) = v {
            *s = fix(std::mem::take(s));
        }
    }
    true
}

/// Draw the tab. Returns true when the flow changed.
pub fn show(ui: &mut egui::Ui, flow: &mut Flow, state: &mut VarsUi, flow_dir: Option<&Path>) -> bool {
    let mut changed = false;
    ui.label(RichText::new("Flow variables").strong());
    ui.weak("Values every run starts with. Use them in any field as ${name}, or pick them in a field's Variable mode. Override at run time with --var name=value.");
    ui.add_space(6.0);

    let names: Vec<String> = flow.variables.keys().cloned().collect();
    let mut delete: Option<String> = None;
    let mut renames: Vec<(String, String)> = vec![];

    egui::Grid::new("vars_grid").num_columns(4).spacing([8.0, 6.0]).striped(true).show(ui, |ui| {
        ui.strong("Name");
        ui.strong("Type");
        ui.strong("Value");
        ui.label("");
        ui.end_row();
        for (i, name) in names.iter().enumerate() {
            // Name (applied when editing finishes, so references are renamed once).
            let buf = state.names.entry(i).or_insert_with(|| name.clone());
            if !ui.memory(|m| m.has_focus(egui::Id::NULL.with(("var_name", i)))) && *buf != *name {
                *buf = name.clone();
            }
            let ok = valid_name(buf);
            let mut edit = egui::TextEdit::singleline(buf).id(egui::Id::NULL.with(("var_name", i))).desired_width(120.0);
            if !ok {
                edit = edit.text_color(Color32::from_rgb(220, 80, 80));
            }
            if state.highlight.as_deref() == Some(name.as_str()) {
                edit = edit.background_color(Color32::from_rgba_unmultiplied(80, 150, 240, 50));
            }
            let r = ui.add_sized([130.0, 22.0], edit).on_hover_text("Letters, digits and _ only");
            if r.lost_focus() && *buf != *name {
                renames.push((name.clone(), buf.clone()));
            }

            // Type.
            let mut ty = flow.variable_types.get(name).copied().unwrap_or_else(|| VarType::guess(&flow.variables[name]));
            let before = ty;
            egui::ComboBox::from_id_salt(("var_type", i)).width(95.0).selected_text(ty.label()).show_ui(ui, |ui| {
                for t in VarType::ALL {
                    ui.selectable_value(&mut ty, t, t.label());
                }
            });
            if ty != before || !flow.variable_types.contains_key(name) {
                flow.variable_types.insert(name.clone(), ty);
                if ty != before {
                    // Convert the value when the old one does not fit the new type.
                    let v = &flow.variables[name];
                    if ty.check(v).is_some() {
                        flow.variables.insert(name.clone(), ty.default_value());
                    }
                    state.lists.remove(name);
                    changed = true;
                }
            }

            // Value.
            let others: Vec<String> = names.iter().filter(|n| *n != name).cloned().collect();
            if let Some(v) = flow.variables.get_mut(name) {
                ui.horizontal(|ui| {
                    changed |= value_editor(ui, name, ty, v, &others, state, flow_dir);
                });
            }
            if let Some(v) = flow.variables.get(name) {
                if let Some(msg) = ty.check(v) {
                    ui.colored_label(Color32::from_rgb(220, 140, 40), "⚠").on_hover_text(msg);
                } else if ui.small_button("🗑").on_hover_text("Delete variable").clicked() {
                    delete = Some(name.clone());
                }
            }
            ui.end_row();
        }
    });

    for (old, new) in renames {
        if rename(flow, &old, &new) {
            changed = true;
            state.names.clear();
        }
    }
    if let Some(d) = delete {
        flow.variables.shift_remove(&d);
        flow.variable_types.remove(&d);
        state.names.clear();
        changed = true;
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.menu_button("➕ Add variable", |ui| {
            for t in VarType::ALL {
                if ui.button(t.label()).clicked() {
                    let mut n = 1;
                    while flow.variables.contains_key(&format!("var{n}")) {
                        n += 1;
                    }
                    let name = format!("var{n}");
                    add_variable(flow, &name, t);
                    state.highlight = Some(name);
                    state.names.clear();
                    changed = true;
                    ui.close();
                }
            }
        });
    });

    // Read-only: variables created by steps, and built-ins.
    let flow_vars: Vec<String> = flow.variables.keys().cloned().collect();
    let produced: Vec<String> = autoflow_core::validate::defined_variables(flow).into_iter().filter(|n| !flow_vars.contains(n)).collect();
    ui.add_space(10.0);
    egui::CollapsingHeader::new(format!("Created by steps ({})", produced.len())).default_open(!produced.is_empty()).show(ui, |ui| {
        if produced.is_empty() {
            ui.weak("none - steps with an output variable (file_read, dir_list, ui_get_text…) add theirs here");
        }
        for n in produced {
            ui.monospace(format!("${{{n}}}"));
        }
    });
    egui::CollapsingHeader::new("Built-in").default_open(false).show(ui, |ui| {
        for (n, d) in [
            ("now", "2026-01-31 14:05:09"),
            ("date / time", "2026-01-31 / 14:05:09"),
            ("timestamp", "20260131_140509"),
            ("now:FORMAT", "e.g. ${now:dd-MM-yyyy}, ${now:yyyyMMdd_HHmmss}"),
            ("unix", "seconds since 1970"),
            ("flow_dir", "folder of this flow"),
            ("env.NAME", "any environment variable, e.g. ${env.USERNAME}"),
            ("newline / tab", "line break / tab"),
            ("last_error", "message of the last failed step (on_error = continue)"),
        ] {
            ui.horizontal(|ui| {
                ui.monospace(format!("${{{n}}}"));
                ui.weak(d);
            });
        }
    });
    changed
}

fn value_editor(ui: &mut egui::Ui, name: &str, ty: VarType, v: &mut Value, others: &[String], state: &mut VarsUi, flow_dir: Option<&Path>) -> bool {
    let mut changed = false;
    // Values that reference other variables are kept as text in any type.
    let is_expr = v.as_str().map(|s| s.contains("${")).unwrap_or(false);
    match ty {
        VarType::Integer if !is_expr => {
            let mut n = v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut n)).changed() || !v.is_i64() {
                *v = Value::from(n);
                changed = true;
            }
        }
        VarType::Number if !is_expr => {
            let mut n = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0.0);
            if ui.add(egui::DragValue::new(&mut n).speed(0.1).max_decimals(4)).changed() || !v.is_number() {
                *v = Value::from(n);
                changed = true;
            }
        }
        VarType::Boolean => {
            let mut b = v.as_bool().unwrap_or_else(|| v.as_str() == Some("true"));
            let label = if b { "yes" } else { "no" };
            if ui.checkbox(&mut b, label).changed() || !v.is_boolean() {
                *v = Value::Bool(b);
                changed = true;
            }
        }
        VarType::Date | VarType::Datetime if !is_expr => {
            let text = v.as_str().unwrap_or("").to_string();
            let (d, rest) = match text.split_once(' ') {
                Some((d, t)) => (d.to_string(), t.to_string()),
                None => (text.clone(), "09:00".to_string()),
            };
            let mut date: jiff::civil::Date = d.parse().unwrap_or_else(|_| jiff::Zoned::now().date());
            let before = date;
            ui.add(egui_extras::DatePickerButton::new(&mut date).id_salt(&format!("date_{name}")).calendar_week(false));
            let mut time = rest.clone();
            if ty == VarType::Datetime {
                ui.add_sized([60.0, 22.0], egui::TextEdit::singleline(&mut time).hint_text("HH:MM"));
            }
            if date != before || time != rest || !text.starts_with(&date.to_string()) {
                let s = if ty == VarType::Datetime { format!("{date} {time}") } else { date.to_string() };
                if s != text {
                    *v = Value::String(s);
                    changed = true;
                }
            }
        }
        VarType::List => {
            let buf = state.lists.entry(name.to_string()).or_insert_with(|| serde_json::to_string(v).unwrap_or_else(|_| "[]".into()));
            ui.vertical(|ui| {
                let r = ui.add_sized([220.0, 22.0], egui::TextEdit::singleline(buf).hint_text("[\"a\", \"b\"]"));
                match serde_json::from_str::<Value>(buf) {
                    Ok(p @ Value::Array(_)) => {
                        if r.changed() && p != *v {
                            *v = p;
                            changed = true;
                        }
                    }
                    _ => {
                        ui.colored_label(Color32::from_rgb(220, 80, 80), "a JSON list, e.g. [\"a\", \"b\"]");
                    }
                }
            });
        }
        _ => {
            // Text, Time, Url, File, Folder and expressions.
            let mut s = match &*v {
                Value::String(s) => s.clone(),
                other => autoflow_core::vars::to_text(other),
            };
            let hint = match ty {
                VarType::Time => "HH:MM",
                VarType::Url => "https://…",
                VarType::File => "C:\\path\\file.xlsx",
                VarType::Folder => "C:\\path\\folder",
                _ => "",
            };
            let w = if matches!(ty, VarType::File | VarType::Folder) { 170.0 } else { 200.0 };
            if ui.add_sized([w, 22.0], egui::TextEdit::singleline(&mut s).hint_text(hint)).changed() {
                changed = true;
            }
            if matches!(ty, VarType::File | VarType::Folder) {
                let mut d = rfd::FileDialog::new();
                if let Some(dir) = flow_dir {
                    d = d.set_directory(dir);
                }
                let picked = if ty == VarType::File {
                    ui.button("📄").on_hover_text("Choose a file…").clicked().then(|| d.pick_file()).flatten()
                } else {
                    ui.button("📁").on_hover_text("Choose a folder…").clicked().then(|| d.pick_folder()).flatten()
                };
                if let Some(p) = picked {
                    s = p.display().to_string();
                    changed = true;
                }
            }
            if !others.is_empty() {
                ui.menu_button("𝑥", |ui| {
                    ui.label(RichText::new("Insert variable").strong());
                    for o in others {
                        if ui.button(format!("${{{o}}}")).clicked() {
                            s.push_str(&format!("${{{o}}}"));
                            changed = true;
                            ui.close();
                        }
                    }
                    ui.separator();
                    for b in ["flow_dir", "date", "timestamp", "env.USERPROFILE"] {
                        if ui.button(format!("${{{b}}}")).clicked() {
                            s.push_str(&format!("${{{b}}}"));
                            changed = true;
                            ui.close();
                        }
                    }
                });
            }
            if changed {
                *v = Value::String(s);
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_updates_references() {
        let mut f: Flow = serde_json::from_value(serde_json::json!({
            "name": "t",
            "variables": {"a": 1, "out": "${a}\\x", "b": 2},
            "steps": [{"action": "log", "message": "${a} and ${a.0} and ${ab}"}]
        }))
        .unwrap();
        assert!(rename(&mut f, "a", "count"));
        assert_eq!(f.variables.keys().cloned().collect::<Vec<_>>(), ["count", "out", "b"]);
        assert_eq!(f.variables["out"], serde_json::json!("${count}\\x"));
        let msg = serde_json::to_value(&f.steps[0]).unwrap()["message"].clone();
        assert_eq!(msg, serde_json::json!("${count} and ${count.0} and ${ab}"));
        assert!(!rename(&mut f, "b", "count"), "duplicate name refused");
        assert!(!rename(&mut f, "b", "bad name"), "invalid name refused");
    }
}
