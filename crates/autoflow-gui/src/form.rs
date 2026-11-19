//! Property editor: renders a step's JSON fields as purpose-built widgets.
//!
//! * file / folder fields get Browse buttons,
//! * numeric fields that accept variables get a Value | Variable switch with a
//!   picker of already-defined variables,
//! * text fields get an "insert variable" menu,
//! * output-variable fields suggest existing names,
//! * optional fields get a "use default / current position" checkbox.

use autoflow_core::flow::VarType;
use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// Fields with a fixed set of values.
fn choices(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "on_error" => &["stop", "continue"],
        "level" => &["info", "warn", "error"],
        "button" => &["left", "right", "middle"],
        "state" => &["normal", "maximized", "minimized"],
        "mode" => &["keys", "unicode", "paste"],
        "buttons" => &["ok", "ok_cancel", "yes_no", "yes_no_cancel", "retry_cancel"],
        "icon" => &["info", "warning", "error", "question", "none"],
        "tween" => &[
            "linear",
            "ease_in_quad",
            "ease_out_quad",
            "ease_in_out_quad",
            "ease_in_cubic",
            "ease_out_cubic",
            "ease_in_out_cubic",
            "ease_in_out_sine",
            "ease_out_back",
            "ease_out_bounce",
        ],
        "op" => &[
            "equals",
            "not_equals",
            "contains",
            "not_contains",
            "starts_with",
            "ends_with",
            "greater_than",
            "less_than",
            "greater_or_equal",
            "less_or_equal",
            "is_empty",
            "not_empty",
            "is_true",
            "is_false",
            "path_exists",
            "matches",
        ],
        "browser" => &["edge", "chrome"],
        "provider" => &["gmail", "outlook365", "outlook.com", "yahoo", "zoho", "custom"],
        "security" => &["starttls", "tls", "none"],
        _ => return None,
    })
}

const CONTROL_TYPES: &[&str] = &[
    "Button",
    "Edit",
    "Document",
    "Text",
    "CheckBox",
    "RadioButton",
    "ComboBox",
    "List",
    "ListItem",
    "Menu",
    "MenuItem",
    "MenuBar",
    "Tab",
    "TabItem",
    "Tree",
    "TreeItem",
    "DataGrid",
    "DataItem",
    "Hyperlink",
    "Pane",
    "Window",
    "ToolBar",
    "Group",
    "Custom",
];

const COMMON_KEYS: &[&str] = &[
    "enter",
    "tab",
    "esc",
    "space",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "f2",
    "f5",
    "f12",
    "ctrl+a",
    "ctrl+c",
    "ctrl+v",
    "ctrl+x",
    "ctrl+z",
    "ctrl+s",
    "ctrl+shift+s",
    "ctrl+n",
    "ctrl+o",
    "ctrl+p",
    "ctrl+w",
    "ctrl+f",
    "ctrl+home",
    "alt+f4",
    "alt+tab",
    "win+r",
    "win+d",
    "shift+tab",
];

/// Built-in variables offered in "insert variable" menus.
const BUILTIN_VARS: &[(&str, &str)] = &[
    ("now", "date and time, 2026-01-31 14:05:09"),
    ("date", "2026-01-31"),
    ("time", "14:05:09"),
    ("timestamp", "20260131_140509"),
    ("now:yyyyMMdd_HHmmss", "custom format (edit the pattern)"),
    ("now:dd-MM-yyyy", "custom format (edit the pattern)"),
    ("unix", "seconds since 1970"),
    ("flow_dir", "folder of this flow file"),
    ("env.USERNAME", "Windows user name"),
    ("env.USERPROFILE", "user folder"),
    ("newline", "line break"),
    ("tab", "tab character"),
    ("last_error", "message of the last step that failed with on_error=continue"),
];

/// Numeric fields that also accept a `${variable}`.
const INT_EXPR: &[&str] = &["x", "y", "ms", "times", "amount", "from_x", "from_y", "to_x", "to_y", "by"];
const MULTILINE: &[&str] = &["content", "text", "message", "comment", "script", "prompt", "system", "body", "sql", "input"];
const SKIP: &[&str] = &["action", "then", "else", "steps", "catch", "finally"];

#[derive(Clone, Copy)]
enum PathKind {
    Open(&'static str, &'static [&'static str]),
    Save(&'static str, &'static [&'static str]),
    Folder,
    FileOrFolder,
}

fn path_kind(action: &str, key: &str) -> Option<PathKind> {
    const ANY: &[&str] = &["*"];
    Some(match (action, key) {
        (_, "working_dir") => PathKind::Folder,
        ("file_copy" | "file_move", "source") => PathKind::FileOrFolder,
        ("file_copy" | "file_move", "destination") => PathKind::FileOrFolder,
        ("file_delete" | "file_read" | "file_exists", "path") => PathKind::Open("Files", ANY),
        ("file_info", "path") => PathKind::FileOrFolder,
        ("file_write", "path") => PathKind::Save("Text", &["txt", "csv", "log", "json", "*"]),
        ("dir_create" | "dir_delete" | "dir_list", "path") => PathKind::Folder,
        ("run_program", "program") => PathKind::Open("Programs", &["exe", "bat", "cmd", "ps1"]),
        ("run_flow", "path") => PathKind::Open("Flows", &["json"]),
        ("open", "target") => PathKind::FileOrFolder,
        ("screenshot" | "browser_screenshot", "path") => PathKind::Save("PNG image", &["png"]),
        (a, "image") if a.starts_with("image_") || a.starts_with("ocr_") => PathKind::Open("Images", &["png", "jpg", "jpeg", "bmp"]),
        ("browser_open" | "browser_navigate", "url") => PathKind::Open("Web pages", &["html", "htm"]),
        ("read_excel", "path") => PathKind::Open("Workbooks", &["xlsx", "xlsm", "xls", "xlsb", "ods"]),
        ("read_csv", "path") => PathKind::Open("CSV / text tables", &["csv", "tsv", "txt"]),
        ("read_pdf", "path") => PathKind::Open("PDF", &["pdf"]),
        (_, "save_attachments_to") => PathKind::Folder,
        ("trigger_file", "folder" | "move_to" | "move_failed_to") => PathKind::Folder,
        ("read_pdf_table", "path") => PathKind::Open("PDF", &["pdf"]),
        ("word_read", "path") | ("word_fill", "template") => PathKind::Open("Word", &["docx"]),
        ("word_fill" | "word_write", "path") => PathKind::Save("Word", &["docx"]),
        ("zip_create", "path") => PathKind::Save("Zip", &["zip"]),
        ("zip_extract", "path") => PathKind::Open("Zip", &["zip"]),
        ("zip_extract", "destination") => PathKind::Folder,
        ("write_csv", "path") => PathKind::Save("CSV", &["csv", "tsv", "txt"]),
        ("write_excel", "path") => PathKind::Save("Excel workbook", &["xlsx"]),
        ("http_request", "save_to") => PathKind::Save("Files", ANY),
        ("input_box", "image") => PathKind::Open("Images", &["png", "jpg", "jpeg", "bmp"]),
        _ => return None,
    })
}

/// "timeout_ms" -> "Timeout (ms)", "output_var" -> "Output variable".
fn pretty(key: &str) -> String {
    let special = match key {
        "id" => Some("Step id"),
        "x_var" => Some("X → variable"),
        "y_var" => Some("Y → variable"),
        "found_var" => Some("Found → variable"),
        "output_var" => Some("Output → variable"),
        "exit_code_var" => Some("Exit code → variable"),
        "width_var" => Some("Width → variable"),
        "height_var" => Some("Height → variable"),
        "count_var" => Some("Count → variable"),
        "status_var" => Some("Status → variable"),
        "api_key_secret" => Some("API key (secret name)"),
        "bearer_secret" => Some("Bearer token (secret name)"),
        "password_secret" => Some("Password (secret name)"),
        "base_url" => Some("Server URL"),
        "excel_bom" => Some("Excel UTF-8 marker"),
        "page_count_var" => Some("Page count → variable"),
        "item_var" => Some("Item variable"),
        "index_var" => Some("Index variable"),
        "element_id" => Some("Element id"),
        "automation_id" => Some("Automation id"),
        "url" => Some("URL / file"),
        _ => None,
    };
    if let Some(s) = special {
        return s.into();
    }
    let mut words: Vec<String> = key.split('_').map(str::to_string).collect();
    let unit = match words.last().map(String::as_str) {
        Some("ms") => Some("(ms)"),
        Some("minutes") => Some("(min)"),
        _ => None,
    };
    if unit.is_some() {
        words.pop();
    }
    let mut s = words.join(" ");
    if let Some(f) = s.get(0..1) {
        s = f.to_uppercase() + &s[1..];
    }
    if let Some(u) = unit {
        s = format!("{s} {u}");
    }
    s
}

/// A variable the user can pick: name and (if declared) its type.
pub type VarInfo = (String, Option<VarType>);

pub struct Form<'a> {
    /// Field descriptions from the JSON schema (tooltips).
    pub help: &'a HashMap<String, String>,
    /// Text buffers for fields edited as raw JSON / lines, keyed by field path.
    pub json_buffers: &'a mut HashMap<String, String>,
    pub action: String,
    /// Variables that exist in the flow (declared + created by steps).
    pub vars: &'a [VarInfo],
    /// Folder of the flow file (paths inside it are stored relative).
    pub flow_dir: Option<&'a Path>,
    /// Set when the user asks for a new variable: (suggested name, type).
    pub new_var: Option<(String, VarType)>,
    /// Sheets and column headers of the file a read_excel/read_csv step points at.
    pub table: Option<&'a TableInfo>,
    /// Set when the user clicks "Set password…" for a secret name.
    pub secret_request: Option<String>,
}

/// What the editor knows about a data file (for the sheet / column dropdowns).
#[derive(Clone, Default)]
pub struct TableInfo {
    pub sheets: Vec<String>,
    pub headers: Vec<String>,
    pub error: Option<String>,
}

impl Form<'_> {
    /// Edit all fields of an object. Returns true if anything changed.
    pub fn object(&mut self, ui: &mut egui::Ui, prefix: &str, obj: &mut Map<String, Value>) -> bool {
        let mut changed = false;
        let keys: Vec<String> = obj.keys().filter(|k| !SKIP.contains(&k.as_str())).cloned().collect();
        egui::Grid::new(format!("grid_{prefix}")).num_columns(2).spacing([10.0, 7.0]).striped(true).min_col_width(90.0).show(ui, |ui| {
            for key in keys {
                let path = format!("{prefix}.{key}");
                let label = ui.label(pretty(&key));
                if let Some(h) = self.help.get(&key) {
                    label.on_hover_text(h);
                }
                let v = obj.get_mut(&key).unwrap();
                ui.horizontal(|ui| {
                    changed |= self.field(ui, &path, &key, v);
                });
                ui.end_row();
            }
        });
        changed
    }

    fn field(&mut self, ui: &mut egui::Ui, path: &str, key: &str, v: &mut Value) -> bool {
        if let Some(opts) = choices(key) {
            return combo(ui, path, v, opts);
        }
        // Optional numbers: null = "current position" / "flow default".
        if self.optional_number(key) {
            return self.optional(ui, path, key, v);
        }
        if key == "region" {
            return region(ui, path, v);
        }
        if INT_EXPR.contains(&key) {
            return self.int_or_var(ui, path, key, v);
        }
        if is_var_name_field(&self.action, key) {
            return self.var_name(ui, path, v);
        }
        match v {
            Value::Bool(b) => ui.checkbox(b, "").changed(),
            Value::Number(n) => {
                if n.is_f64() && !n.is_i64() && !n.is_u64() {
                    let mut f = n.as_f64().unwrap_or(0.0);
                    let c = ui.add(egui::DragValue::new(&mut f).speed(0.01).max_decimals(3)).changed();
                    if c {
                        *v = Value::from(f);
                    }
                    c
                } else {
                    let mut i = n.as_i64().unwrap_or(0);
                    let c = ui.add(egui::DragValue::new(&mut i).speed(1.0).range(0..=i64::MAX)).changed();
                    if c {
                        *v = Value::from(i);
                    }
                    c
                }
            }
            Value::String(_) if !key.ends_with("_secret") => self.text(ui, path, key, v),
            Value::Object(o) if key == "condition" => {
                let mut changed = false;
                ui.vertical(|ui| changed = self.object(ui, path, o));
                changed
            }
            Value::Array(a) if a.iter().all(Value::is_string) && key == "attachments" => {
                // One file per line, plus a Browse button that adds files.
                let mut changed = false;
                ui.vertical(|ui| {
                    let buf = self.json_buffers.entry(path.to_string()).or_insert_with(|| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"));
                    let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(2).desired_width(260.0).hint_text("one file per line, ${variables} allowed"));
                    if resp.changed() {
                        changed = true;
                    }
                    let mut add: Vec<String> = vec![];
                    ui.horizontal(|ui| {
                        if ui.button("📄 Add file…").clicked() {
                            let mut d = rfd::FileDialog::new();
                            if let Some(dir) = self.flow_dir {
                                d = d.set_directory(dir);
                            }
                            if let Some(files) = d.pick_files() {
                                add = files.iter().map(|p| self.relative(p)).collect();
                            }
                        }
                    });
                    let buf = self.json_buffers.get_mut(path).unwrap();
                    for f in add {
                        if !buf.is_empty() && !buf.ends_with('\n') {
                            buf.push('\n');
                        }
                        buf.push_str(&f);
                        changed = true;
                    }
                    if changed {
                        *a = buf.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(l.to_string())).collect();
                    }
                });
                changed
            }
            Value::String(_) if key.ends_with("_secret") => {
                let mut s = v.as_str().unwrap_or("").to_string();
                let mut changed = ui
                    .add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut s).hint_text(if key == "password_secret" { "e.g. gmail" } else { "e.g. openai" }))
                    .changed();
                let stored = !s.trim().is_empty() && !s.contains("${") && autoflow_core::mail::secrets::exists(&s);
                if stored {
                    ui.colored_label(Color32::from_rgb(70, 170, 90), "✔ stored");
                } else {
                    ui.colored_label(Color32::from_rgb(220, 140, 40), "not stored");
                }
                if ui
                    .button(if key == "password_secret" { "🔑 Set password…" } else { "🔑 Set key…" })
                    .on_hover_text("Store the password / API key in Windows Credential Manager under this name. It is never saved in the flow.")
                    .clicked()
                    && !s.trim().is_empty()
                {
                    self.secret_request = Some(s.trim().to_string());
                }
                if changed {
                    *v = Value::String(s);
                } else {
                    changed = false;
                }
                changed
            }
            Value::Array(a) if a.iter().all(Value::is_string) && matches!(key, "args" | "labels" | "columns" | "params" | "sources") => {
                let buf = self.json_buffers.entry(path.to_string()).or_insert_with(|| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"));
                let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(2).desired_width(300.0).hint_text(match key {
                    "labels" => "one label per line, e.g. Fav",
                    "columns" => "one column per line (empty = all)",
                    "params" => "one value per ? placeholder, in order",
                    "sources" => "one file or folder per line (wildcards allowed)",
                    _ => "one argument per line",
                }));
                if resp.changed() {
                    *a = buf.lines().map(|l| Value::String(l.to_string())).collect();
                    return true;
                }
                false
            }
            other if key == "items" => self.items(ui, path, other),
            other => self.raw_json(ui, path, other),
        }
    }

    fn optional_number(&self, key: &str) -> bool {
        let a = self.action.as_str();
        matches!(
            (a, key),
            ("mouse_click" | "mouse_down" | "mouse_up" | "mouse_scroll", "x" | "y")
                | ("mouse_drag", "from_x" | "from_y")
                | (_, "duration_ms")
                | ("type_text", "delay_ms")
                | ("key_press", "interval_ms")
        )
    }

    fn optional(&mut self, ui: &mut egui::Ui, path: &str, key: &str, v: &mut Value) -> bool {
        let is_pos = matches!(key, "x" | "y" | "from_x" | "from_y");
        let label = if is_pos { "current position" } else { "flow default" };
        let mut use_default = v.is_null();
        let mut changed = false;
        if ui
            .checkbox(&mut use_default, label)
            .on_hover_text(if is_pos { "Use wherever the mouse is when the step runs" } else { "Use the value from the flow's run settings" })
            .changed()
        {
            *v = if use_default { Value::Null } else { Value::from(0) };
            changed = true;
        }
        if !v.is_null() {
            changed |= self.int_or_var(ui, path, key, v);
        }
        changed
    }

    /// A number, or a ${variable} picked from a list.
    fn int_or_var(&mut self, ui: &mut egui::Ui, path: &str, key: &str, v: &mut Value) -> bool {
        let mut is_var = v.is_string();
        let mut changed = false;
        let before = is_var;
        egui::ComboBox::from_id_salt(format!("{path}#mode")).width(68.0).selected_text(if is_var { "Variable" } else { "Value" }).show_ui(ui, |ui| {
            ui.selectable_value(&mut is_var, false, "Value");
            ui.selectable_value(&mut is_var, true, "Variable");
        });
        if is_var != before {
            changed = true;
            if is_var {
                let first = self.number_vars().first().cloned();
                *v = Value::String(first.map(|n| format!("${{{n}}}")).unwrap_or_default());
            } else {
                *v = Value::from(0);
            }
        }
        if is_var {
            let current = v.as_str().unwrap_or("").to_string();
            let mut picked: Option<String> = None;
            let names = self.number_vars();
            egui::ComboBox::from_id_salt(format!("{path}#var"))
                .width(150.0)
                .selected_text(if current.is_empty() { "choose…".to_string() } else { current.clone() })
                .show_ui(ui, |ui| {
                    if names.is_empty() {
                        ui.weak("no number variables yet");
                    }
                    for n in &names {
                        let t = format!("${{{n}}}");
                        if ui.selectable_label(current == t, &t).clicked() {
                            picked = Some(t);
                        }
                    }
                    ui.separator();
                    if ui.button("➕ New variable…").clicked() {
                        let name = self.suggest_name(key);
                        picked = Some(format!("${{{name}}}"));
                        self.new_var = Some((name, VarType::Integer));
                    }
                });
            if let Some(p) = picked {
                *v = Value::String(p);
                changed = true;
            }
        } else {
            let mut i = v.as_i64().unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut i).speed(1.0)).changed() {
                *v = Value::from(i);
                changed = true;
            }
        }
        changed
    }

    fn number_vars(&self) -> Vec<String> {
        self.vars.iter().filter(|(_, t)| matches!(t, None | Some(VarType::Integer) | Some(VarType::Number))).map(|(n, _)| n.clone()).collect()
    }

    fn suggest_name(&self, key: &str) -> String {
        let base = format!("{}_{}", self.action.split('_').next_back().unwrap_or("value"), key.trim_end_matches("_ms"));
        let base: String = base.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let mut name = base.clone();
        let mut i = 2;
        while self.vars.iter().any(|(n, _)| *n == name) {
            name = format!("{base}{i}");
            i += 1;
        }
        name
    }

    /// Output-variable name: type a new name or pick an existing one.
    fn var_name(&mut self, ui: &mut egui::Ui, path: &str, v: &mut Value) -> bool {
        let mut s = v.as_str().unwrap_or("").to_string();
        let mut changed = ui.add_sized([150.0, 22.0], egui::TextEdit::singleline(&mut s).hint_text("variable name")).changed();
        let names: Vec<String> = self.vars.iter().map(|(n, _)| n.clone()).collect();
        egui::ComboBox::from_id_salt(format!("{path}#names")).width(24.0).selected_text("▾").show_ui(ui, |ui| {
            if names.is_empty() {
                ui.weak("no variables yet - type a new name");
            }
            for n in &names {
                if ui.selectable_label(s == *n, n).clicked() {
                    s = n.clone();
                    changed = true;
                }
            }
        });
        if changed {
            *v = Value::String(s);
        }
        changed
    }

    /// Text with an "insert variable" menu, plus Browse buttons for paths and helpers for keys.
    fn text(&mut self, ui: &mut egui::Ui, path: &str, key: &str, v: &mut Value) -> bool {
        let mut s = v.as_str().unwrap_or("").to_string();
        let mut changed = false;
        let pk = path_kind(&self.action, key);
        let table_list: Option<(Vec<String>, &str)> = match (self.table, key) {
            (Some(t), "sheet") if !t.sheets.is_empty() => Some((t.sheets.clone(), "first sheet")),
            (Some(t), "column") if !t.headers.is_empty() => Some((t.headers.clone(), "all columns")),
            _ => None,
        };
        if let Some((items, empty_label)) = table_list {
            // Sheet / column picked from the actual file.
            egui::ComboBox::from_id_salt(format!("{path}#tbl"))
                .width(200.0)
                .selected_text(if s.is_empty() { format!("({empty_label})") } else { s.clone() })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(s.is_empty(), format!("({empty_label})")).clicked() {
                        s.clear();
                        changed = true;
                    }
                    for it in &items {
                        let label = if it.is_empty() { "(blank)".to_string() } else { it.clone() };
                        if ui.selectable_label(s == *it, label).clicked() {
                            s = it.clone();
                            changed = true;
                        }
                    }
                });
            changed |= self.insert_var_menu(ui, path, &mut s, key);
        } else if key == "control_type" {
            changed |= ui.add(egui::TextEdit::singleline(&mut s).desired_width(140.0)).changed();
            egui::ComboBox::from_id_salt(path).selected_text("▾").width(24.0).show_ui(ui, |ui| {
                for c in CONTROL_TYPES {
                    if ui.selectable_label(s == *c, *c).clicked() {
                        s = c.to_string();
                        changed = true;
                    }
                }
            });
        } else if MULTILINE.contains(&key) {
            ui.vertical(|ui| {
                changed |=
                    ui.add(egui::TextEdit::multiline(&mut s).desired_rows(3).desired_width(300.0).code_editor_if(matches!(key, "script" | "sql"))).changed();
                changed |= self.insert_var_menu(ui, path, &mut s, key);
            });
        } else {
            let reserve = if pk.is_some() { 90.0 } else { 34.0 } + if key == "keys" { 30.0 } else { 0.0 };
            // Capped: a box sized to "all available width" makes the side panel grow every frame.
            let w = (ui.available_width() - reserve).clamp(80.0, 230.0);
            changed |= ui.add_sized([w, 22.0], egui::TextEdit::singleline(&mut s)).changed();
            if key == "keys" {
                egui::ComboBox::from_id_salt(format!("{path}#keys")).selected_text("▾").width(24.0).show_ui(ui, |ui| {
                    for k in COMMON_KEYS {
                        if ui.selectable_label(s == *k, *k).clicked() {
                            s = k.to_string();
                            changed = true;
                        }
                    }
                });
            }
            if let Some(kind) = pk {
                if let Some(p) = self.browse(ui, kind) {
                    s = p;
                    changed = true;
                }
            }
            if key != "keys" {
                changed |= self.insert_var_menu(ui, path, &mut s, key);
            }
        }
        if changed {
            *v = Value::String(s);
        }
        changed
    }

    /// "𝑥" menu that appends ${variable} to the text.
    fn insert_var_menu(&mut self, ui: &mut egui::Ui, path: &str, s: &mut String, key: &str) -> bool {
        let mut picked: Option<String> = None;
        let names: Vec<String> = self.vars.iter().map(|(n, _)| n.clone()).collect();
        let resp = ui.menu_button(RichText::new("𝑥").strong(), |ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new("Insert variable").strong());
            if names.is_empty() {
                ui.weak("no flow variables yet");
            }
            for n in &names {
                if ui.button(format!("${{{n}}}")).clicked() {
                    picked = Some(n.clone());
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button("Built-in", |ui| {
                for (n, d) in BUILTIN_VARS {
                    if ui.button(format!("${{{n}}}")).on_hover_text(*d).clicked() {
                        picked = Some(n.to_string());
                        ui.close();
                    }
                }
            });
            if ui.button("➕ New variable…").clicked() {
                let name = self.suggest_name(key);
                self.new_var = Some((name.clone(), VarType::Text));
                picked = Some(name);
                ui.close();
            }
        });
        resp.response.on_hover_text(format!("Insert a ${{variable}} into {}", pretty(key).to_lowercase()));
        match picked {
            Some(n) => {
                s.push_str(&format!("${{{n}}}"));
                let _ = path;
                true
            }
            None => false,
        }
    }

    /// Browse buttons; returns the chosen path (relative when inside the flow folder).
    fn browse(&self, ui: &mut egui::Ui, kind: PathKind) -> Option<String> {
        let mut out = None;
        let dir = self.flow_dir.map(Path::to_path_buf);
        let dialog = || {
            let d = rfd::FileDialog::new();
            match &dir {
                Some(p) => d.set_directory(p),
                None => d,
            }
        };
        match kind {
            PathKind::Open(name, ext) => {
                if ui.button("📄").on_hover_text("Choose a file…").clicked() {
                    let mut d = dialog();
                    if ext != ["*"] {
                        d = d.add_filter(name, ext);
                    }
                    out = d.add_filter("All files", &["*"]).pick_file();
                }
            }
            PathKind::Save(name, ext) => {
                if ui.button("💾").on_hover_text("Choose where to save…").clicked() {
                    out = dialog().add_filter(name, ext).save_file();
                }
            }
            PathKind::Folder => {
                if ui.button("📁").on_hover_text("Choose a folder…").clicked() {
                    out = dialog().pick_folder();
                }
            }
            PathKind::FileOrFolder => {
                if ui.button("📄").on_hover_text("Choose a file…").clicked() {
                    out = dialog().pick_file();
                }
                if ui.button("📁").on_hover_text("Choose a folder…").clicked() {
                    out = dialog().pick_folder().map(|p| {
                        // A trailing \ means "into this folder" for copy/move.
                        let mut s = p.into_os_string();
                        s.push("\\");
                        s.into()
                    });
                }
            }
        }
        out.map(|p| self.relative(&p))
    }

    fn relative(&self, p: &Path) -> String {
        if let Some(base) = self.flow_dir {
            if let Ok(rel) = p.strip_prefix(base) {
                let r = rel.display().to_string();
                if !r.is_empty() {
                    return r;
                }
            }
        }
        p.display().to_string()
    }

    /// for_each items: a ${list variable}, text lines, or a JSON array.
    fn items(&mut self, ui: &mut egui::Ui, path: &str, v: &mut Value) -> bool {
        let mut changed = false;
        let mut is_list = v.is_array();
        let before = is_list;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut is_list, false, "Variable / text");
                ui.radio_value(&mut is_list, true, "Fixed list");
            });
            if is_list != before {
                *v = if is_list { Value::Array(vec![]) } else { Value::String(String::new()) };
                self.json_buffers.remove(path);
                changed = true;
            }
            if is_list {
                changed |= self.raw_json(ui, path, v);
            } else {
                let mut s = v.as_str().unwrap_or("").to_string();
                let lists: Vec<String> = self.vars.iter().filter(|(_, t)| matches!(t, None | Some(VarType::List))).map(|(n, _)| n.clone()).collect();
                ui.horizontal(|ui| {
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(170.0).hint_text("${files}")).changed() {
                        changed = true;
                    }
                    egui::ComboBox::from_id_salt(format!("{path}#lists")).selected_text("▾").width(24.0).show_ui(ui, |ui| {
                        for n in &lists {
                            let t = format!("${{{n}}}");
                            if ui.selectable_label(s == t, &t).clicked() {
                                s = t;
                                changed = true;
                            }
                        }
                    });
                });
                if changed {
                    *v = Value::String(s);
                }
            }
        });
        changed
    }

    /// Arrays/objects edited as JSON text; applied when the text parses.
    fn raw_json(&mut self, ui: &mut egui::Ui, path: &str, v: &mut Value) -> bool {
        let buf = self.json_buffers.entry(path.to_string()).or_insert_with(|| serde_json::to_string_pretty(v).unwrap_or_default());
        let mut changed = false;
        ui.vertical(|ui| {
            let resp = ui.add(egui::TextEdit::multiline(buf).code_editor().desired_rows(3).desired_width(300.0));
            match serde_json::from_str::<Value>(buf) {
                Ok(parsed) => {
                    if resp.changed() && parsed != *v {
                        *v = parsed;
                        changed = true;
                    }
                }
                Err(e) => {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), format!("JSON: {e}"));
                }
            }
        });
        changed
    }
}

fn is_var_name_field(action: &str, key: &str) -> bool {
    key.ends_with("_var") || (matches!(action, "set_variable" | "increment") && key == "name")
}

fn combo(ui: &mut egui::Ui, path: &str, v: &mut Value, opts: &[&str]) -> bool {
    let mut cur = v.as_str().unwrap_or("").to_string();
    let before = cur.clone();
    egui::ComboBox::from_id_salt(path).selected_text(cur.replace('_', " ")).width(160.0).show_ui(ui, |ui| {
        for o in opts {
            ui.selectable_value(&mut cur, o.to_string(), o.replace('_', " "));
        }
    });
    if cur != before {
        *v = Value::String(cur);
        return true;
    }
    false
}

/// Optional screen rectangle: "whole screen" or x/y/width/height.
fn region(ui: &mut egui::Ui, path: &str, v: &mut Value) -> bool {
    let mut whole = v.is_null();
    let mut changed = false;
    ui.vertical(|ui| {
        if ui.checkbox(&mut whole, "Whole screen").changed() {
            *v = if whole { Value::Null } else { serde_json::json!({"x": 0, "y": 0, "width": 400, "height": 300}) };
            changed = true;
        }
        if let Value::Object(o) = v {
            ui.horizontal(|ui| {
                for (k, min) in [("x", i64::MIN), ("y", i64::MIN), ("width", 1), ("height", 1)] {
                    ui.label(k);
                    let mut n = o.get(k).and_then(Value::as_i64).unwrap_or(0);
                    if ui.add(egui::DragValue::new(&mut n).range(min..=100_000)).changed() {
                        o.insert(k.into(), Value::from(n));
                        changed = true;
                    }
                }
            });
        }
    });
    let _ = path;
    changed
}

trait CodeEditorIf {
    fn code_editor_if(self, yes: bool) -> Self;
}
impl CodeEditorIf for egui::TextEdit<'_> {
    fn code_editor_if(self, yes: bool) -> Self {
        if yes {
            self.code_editor()
        } else {
            self
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(pretty("timeout_ms"), "Timeout (ms)");
        assert_eq!(pretty("output_var"), "Output → variable");
        assert_eq!(pretty("press_enter"), "Press enter");
        assert_eq!(pretty("time_limit_minutes"), "Time limit (min)");
    }

    #[test]
    fn path_kinds() {
        assert!(matches!(path_kind("dir_list", "path"), Some(PathKind::Folder)));
        assert!(matches!(path_kind("screenshot", "path"), Some(PathKind::Save(..))));
        assert!(matches!(path_kind("image_click", "image"), Some(PathKind::Open(..))));
        assert!(path_kind("log", "message").is_none());
    }
}
