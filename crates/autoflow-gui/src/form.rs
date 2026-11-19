//! Generic property editor: renders a step's JSON fields as widgets, so every
//! action gets a form without per-action UI code.

use eframe::egui;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Fields with a fixed set of values.
fn choices(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "on_error" => &["stop", "continue"],
        "level" => &["info", "warn", "error"],
        "button" => &["left", "right", "middle"],
        "state" => &["normal", "maximized", "minimized"],
        "mode" => &["keys", "unicode", "paste"],
        "op" => &[
            "equals", "not_equals", "contains", "not_contains", "starts_with", "ends_with", "greater_than",
            "less_than", "greater_or_equal", "less_or_equal", "is_empty", "not_empty", "is_true", "is_false",
            "path_exists", "matches",
        ],
        _ => return None,
    })
}

const CONTROL_TYPES: &[&str] = &[
    "Button", "Edit", "Document", "Text", "CheckBox", "RadioButton", "ComboBox", "List", "ListItem", "Menu",
    "MenuItem", "MenuBar", "Tab", "TabItem", "Tree", "TreeItem", "DataGrid", "DataItem", "Hyperlink", "Pane",
    "Window", "ToolBar", "Group", "Custom",
];

/// Numeric fields that also accept a `${variable}` string.
const INT_EXPR: &[&str] = &["x", "y", "ms", "times", "amount", "from_x", "from_y", "to_x", "to_y"];
const MULTILINE:&[&str] = &["content", "text", "message", "comment"];
const SKIP: &[&str] = &["action", "then", "else", "steps"];

pub struct Form<'a> {
    /// Field descriptions from the JSON schema (tooltips).
    pub help: &'a HashMap<String, String>,
    /// Text buffers for fields edited as raw JSON, keyed by field path.
    pub json_buffers: &'a mut HashMap<String, String>,
    pub action: String,
}

impl Form<'_> {
    /// Edit all fields of an object. Returns true if anything changed.
    pub fn object(&mut self, ui: &mut egui::Ui, prefix: &str, obj: &mut Map<String, Value>) -> bool {
        let mut changed = false;
        let keys: Vec<String> = obj.keys().filter(|k| !SKIP.contains(&k.as_str())).cloned().collect();
        egui::Grid::new(format!("grid_{prefix}")).num_columns(2).spacing([8.0, 6.0]).striped(true).show(ui, |ui| {
            for key in keys {
                let path = format!("{prefix}.{key}");
                let label = ui.label(key.replace('_', " "));
                if let Some(h) = self.help.get(&key) {
                    label.on_hover_text(h);
                }
                let v = obj.get_mut(&key).unwrap();
                changed |= self.field(ui, &path, &key, v);
                ui.end_row();
            }
        });
        changed
    }

    fn field(&mut self, ui: &mut egui::Ui, path: &str, key: &str, v: &mut Value) -> bool {
        if let Some(opts) = choices(key) {
            let mut cur = v.as_str().unwrap_or("").to_string();
            let before = cur.clone();
            egui::ComboBox::from_id_salt(path).selected_text(&cur).show_ui(ui, |ui| {
                for o in opts {
                    ui.selectable_value(&mut cur, o.to_string(), *o);
                }
            });
            if cur != before {
                *v = Value::String(cur);
                return true;
            }
            return false;
        }
        match v {
            Value::Bool(b) => ui.checkbox(b, "").changed(),
            Value::Number(n) => {
                let mut i = n.as_i64().unwrap_or(0);
                let mut changed = false;
                ui.horizontal(|ui| {
                    changed = ui.add(egui::DragValue::new(&mut i).speed(1.0)).changed();
                    if INT_EXPR.contains(&key) && ui.small_button("$").on_hover_text("Use a ${variable} instead of a number").clicked() {
                        *v = Value::String(format!("{i}"));
                        changed = true;
                    } else if changed {
                        *v = Value::from(i);
                    }
                    if self.nullable(key) && ui.small_button("✕").on_hover_text("Unset (use current position)").clicked() {
                        *v = Value::Null;
                        changed = true;
                    }
                });
                changed
            }
            Value::String(s) => {
                if key == "control_type" {
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(s).desired_width(140.0)).changed();
                        egui::ComboBox::from_id_salt(path).selected_text("…").width(30.0).show_ui(ui, |ui| {
                            for c in CONTROL_TYPES {
                                if ui.selectable_label(s == c, *c).clicked() {
                                    *s = c.to_string();
                                    changed = true;
                                }
                            }
                        });
                    });
                    changed
                } else if MULTILINE.contains(&key) {
                    ui.add(egui::TextEdit::multiline(s).desired_rows(3).desired_width(f32::INFINITY)).changed()
                } else {
                    ui.add(egui::TextEdit::singleline(s).desired_width(f32::INFINITY)).changed()
                }
            }
            Value::Null => {
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.weak("(not set)");
                    if ui.small_button("set").clicked() {
                        *v = Value::from(0);
                        changed = true;
                    }
                });
                changed
            }
            Value::Object(o) if key == "condition" => {
                let mut changed = false;
                ui.vertical(|ui| {
                    changed = self.object(ui, path, o);
                });
                changed
            }
            Value::Array(a) if a.iter().all(Value::is_string) && key == "args" => {
                // One argument per line.
                let buf = self
                    .json_buffers
                    .entry(path.to_string())
                    .or_insert_with(|| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"));
                let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(2).hint_text("one argument per line"));
                if resp.changed() {
                    *a = buf.lines().map(|l| Value::String(l.to_string())).collect();
                    return true;
                }
                false
            }
            other => self.raw_json(ui, path, other),
        }
    }

    fn nullable(&self, key: &str) -> bool {
        self.action == "mouse_click" && (key == "x" || key == "y")
    }

    /// Arrays/objects edited as JSON text; applied when the text parses.
    fn raw_json(&mut self, ui: &mut egui::Ui, path: &str, v: &mut Value) -> bool {
        let buf = self
            .json_buffers
            .entry(path.to_string())
            .or_insert_with(|| serde_json::to_string_pretty(v).unwrap_or_default());
        let mut changed = false;
        ui.vertical(|ui| {
            let resp = ui.add(egui::TextEdit::multiline(buf).code_editor().desired_rows(3).desired_width(f32::INFINITY));
            match serde_json::from_str::<Value>(buf) {
                Ok(parsed) => {
                    if resp.changed() && parsed != *v {
                        *v = parsed;
                        changed = true;
                    }
                }
                Err(e) => {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("JSON: {e}"));
                }
            }
        });
        changed
    }
}

/// Build tooltip texts from the schema's property descriptions.
pub fn schema_help() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let schema = autoflow_core::flow::json_schema();
    fn walk(v: &Value, out: &mut HashMap<String, String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::Object(props)) = o.get("properties") {
                    for (k, p) in props {
                        if let Some(d) = p.get("description").and_then(Value::as_str) {
                            out.entry(k.clone()).or_insert_with(|| d.to_string());
                        }
                    }
                }
                o.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&schema, &mut out);
    out
}
