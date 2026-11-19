//! The flow document model. A flow is plain JSON, so it can be written by the
//! GUI, by hand, or by an AI agent, and is validated against the JSON Schema
//! produced by `autoflow schema`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::actions::Action;

pub const FORMAT_VERSION: u32 = 1;

/// A complete automation flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Flow {
    /// Optional path/URL of the JSON schema (ignored by the engine).
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Human-readable name of the flow.
    #[serde(default)]
    pub name: String,
    /// What the flow does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Flow format version. Currently 1.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Run-wide timing and safety settings.
    #[serde(default)]
    pub settings: Settings,
    /// Initial variables. Can be overridden at run time (`--var name=value`).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<String, Value>,
    /// Optional type of each variable (used by the editor and for validation).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub variable_types: std::collections::BTreeMap<String, VarType>,
    /// Steps executed in order.
    #[serde(default)]
    pub steps: Vec<Step>,
}

fn default_version() -> u32 {
    FORMAT_VERSION
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            schema: None,
            name: "New flow".into(),
            description: String::new(),
            version: FORMAT_VERSION,
            settings: Settings::default(),
            variables: Map::new(),
            variable_types: Default::default(),
            steps: Vec::new(),
        }
    }
}

impl Flow {
    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(text)?)
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
        Self::from_json(&text).map_err(|e| anyhow::anyhow!("invalid flow {}: {e}", path.display()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("flow serializes")
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        std::fs::write(path, self.to_json())?;
        Ok(())
    }

    /// Give every step without an id a unique one (`s1`, `s2`, ...).
    pub fn assign_ids(&mut self) {
        let mut used = std::collections::HashSet::new();
        visit(&self.steps, &mut |s| {
            if !s.id.is_empty() {
                used.insert(s.id.clone());
            }
        });
        let mut n = 1;
        visit_mut(&mut self.steps, &mut |s| {
            if s.id.is_empty() {
                while used.contains(&format!("s{n}")) {
                    n += 1;
                }
                s.id = format!("s{n}");
                used.insert(s.id.clone());
            }
        });
    }
}

/// Run-wide timing and safety settings (the PyAutoGUI `PAUSE` / `FAILSAFE`
/// equivalents). Command-line flags can override them for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    /// Delay between keystrokes in ms for type_text and repeated key_press.
    /// Raise it (150-250) for slow applications such as Tally.
    pub key_delay_ms: u64,
    /// Pause after every mouse/keyboard action in ms (like PyAutoGUI's PAUSE).
    pub input_pause_ms: u64,
    /// Pause after every step in ms.
    pub step_delay_ms: u64,
    /// Default duration of mouse movements in ms (0 = jump instantly).
    pub mouse_move_ms: u64,
    /// Abort the run when the mouse is pushed into any screen corner (like PyAutoGUI's FAILSAFE).
    pub failsafe: bool,
    /// Key combination that aborts the run at any time, e.g. `ctrl+alt+q`. Empty = none.
    pub abort_hotkey: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            key_delay_ms: 100,
            input_pause_ms: 100,
            step_delay_ms: 0,
            mouse_move_ms: 0,
            failsafe: true,
            abort_hotkey: "ctrl+alt+q".into(),
        }
    }
}

/// Type of a flow variable. Values are stored as plain JSON in `variables`;
/// the type tells the editor which input to show and lets `validate` check the value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    /// Any text (may contain ${other} variables).
    #[default]
    Text,
    /// Whole number.
    Integer,
    /// Decimal number.
    Number,
    /// true / false.
    Boolean,
    /// Date as YYYY-MM-DD.
    Date,
    /// Time as HH:MM or HH:MM:SS (24 h).
    Time,
    /// Date and time as YYYY-MM-DD HH:MM[:SS].
    Datetime,
    /// Web address.
    Url,
    /// Path of a file.
    File,
    /// Path of a folder.
    Folder,
    /// JSON array, e.g. ["a", "b"].
    List,
}

impl VarType {
    pub const ALL: [VarType; 11] = [
        VarType::Text, VarType::Integer, VarType::Number, VarType::Boolean, VarType::Date, VarType::Time,
        VarType::Datetime, VarType::Url, VarType::File, VarType::Folder, VarType::List,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VarType::Text => "Text",
            VarType::Integer => "Integer",
            VarType::Number => "Number",
            VarType::Boolean => "Yes/No",
            VarType::Date => "Date",
            VarType::Time => "Time",
            VarType::Datetime => "Date + time",
            VarType::Url => "URL",
            VarType::File => "File",
            VarType::Folder => "Folder",
            VarType::List => "List",
        }
    }

    /// A sensible starting value.
    pub fn default_value(self) -> Value {
        let now = chrono::Local::now();
        match self {
            VarType::Integer => Value::from(0),
            VarType::Number => Value::from(0.0),
            VarType::Boolean => Value::Bool(false),
            VarType::Date => Value::from(now.format("%Y-%m-%d").to_string()),
            VarType::Time => Value::from(now.format("%H:%M").to_string()),
            VarType::Datetime => Value::from(now.format("%Y-%m-%d %H:%M").to_string()),
            VarType::Url => Value::from("https://"),
            VarType::List => Value::Array(vec![]),
            _ => Value::from(""),
        }
    }

    /// Guess a type from an existing value (for flows written without types).
    pub fn guess(v: &Value) -> VarType {
        match v {
            Value::Bool(_) => VarType::Boolean,
            Value::Number(n) if n.is_i64() || n.is_u64() => VarType::Integer,
            Value::Number(_) => VarType::Number,
            Value::Array(_) => VarType::List,
            Value::String(s) if s.starts_with("http://") || s.starts_with("https://") => VarType::Url,
            Value::String(s) if chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() => VarType::Date,
            _ => VarType::Text,
        }
    }

    /// Why `v` is not a valid value of this type (None = fine). Values containing
    /// `${…}` are not checked, since they are filled in at run time.
    pub fn check(self, v: &Value) -> Option<String> {
        if let Value::String(s) = v {
            if s.contains("${") {
                return None;
            }
        }
        let text = crate::vars::to_text(v);
        let t = text.trim();
        let ok = match self {
            VarType::Integer => v.is_i64() || v.is_u64() || t.parse::<i64>().is_ok(),
            VarType::Number => v.is_number() || t.parse::<f64>().is_ok(),
            VarType::Boolean => v.is_boolean() || matches!(t, "true" | "false"),
            VarType::Date => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").is_ok(),
            VarType::Time => {
                chrono::NaiveTime::parse_from_str(t, "%H:%M").is_ok() || chrono::NaiveTime::parse_from_str(t, "%H:%M:%S").is_ok()
            }
            VarType::Datetime => {
                chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M").is_ok()
                    || chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").is_ok()
            }
            VarType::Url => t.is_empty() || t.contains("://") || t.starts_with("about:"),
            VarType::List => v.is_array(),
            VarType::Text | VarType::File | VarType::Folder => true,
        };
        (!ok).then(|| format!("`{text}` is not a valid {}", self.label().to_lowercase()))
    }
}

/// Error handling policy for a step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// Abort the whole flow (default).
    #[default]
    Stop,
    /// Log the error and continue with the next step.
    Continue,
}

/// One step of a flow: common settings plus the action (`"action": "..."`)
/// and the action's own fields, all at the same JSON level.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    /// Unique id within the flow, used in logs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Disabled steps are skipped.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Free-text note shown in the editor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// What to do when the step fails after all retries.
    #[serde(default, skip_serializing_if = "is_default")]
    pub on_error: OnError,
    /// Extra attempts after a failure.
    #[serde(default, skip_serializing_if = "is_default")]
    pub retries: u32,
    /// Delay between retries in milliseconds.
    #[serde(default = "default_retry_delay", skip_serializing_if = "is_default_retry_delay")]
    pub retry_delay_ms: u64,
    /// Wait this many ms before the step runs.
    #[serde(default, skip_serializing_if = "is_default")]
    pub delay_before_ms: u64,
    /// Wait this many ms after the step succeeds (in addition to settings.step_delay_ms).
    #[serde(default, skip_serializing_if = "is_default")]
    pub delay_after_ms: u64,
    #[serde(flatten)]
    pub action: Action,
}

impl Step {
    pub fn new(action: Action) -> Self {
        Self {
            id: String::new(),
            enabled: true,
            comment: String::new(),
            on_error: OnError::Stop,
            retries: 0,
            retry_delay_ms: default_retry_delay(),
            delay_before_ms: 0,
            delay_after_ms: 0,
            action,
        }
    }
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn default_retry_delay() -> u64 {
    1000
}
fn is_default_retry_delay(v: &u64) -> bool {
    *v == default_retry_delay()
}

/// Visit every step, including nested ones, depth first.
pub fn visit(steps: &[Step], f: &mut dyn FnMut(&Step)) {
    for s in steps {
        f(s);
        for child in s.action.children() {
            visit(child, f);
        }
    }
}

pub fn visit_mut(steps: &mut [Step], f: &mut dyn FnMut(&mut Step)) {
    for s in steps {
        f(s);
        for child in s.action.children_mut() {
            visit_mut(child, f);
        }
    }
}

/// Field descriptions for one action, taken from the JSON schema
/// (doc comments on the action's struct, including flattened parts).
pub fn action_field_help(action: &str) -> Map<String, Value> {
    let schema = json_schema();
    let defs = schema.get("$defs").cloned().unwrap_or_default();
    let mut out = Map::new();
    let variants = defs.pointer("/Step/oneOf").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(variant) = variants
        .iter()
        .find(|v| v.pointer("/properties/action/const").and_then(Value::as_str) == Some(action))
    else {
        return out;
    };
    fn collect(v: &Value, defs: &Value, out: &mut Map<String, Value>, depth: usize) {
        if depth > 6 {
            return;
        }
        if let Some(r) = v.get("$ref").and_then(Value::as_str) {
            if let Some(name) = r.strip_prefix("#/$defs/") {
                if let Some(d) = defs.get(name) {
                    collect(d, defs, out, depth + 1);
                }
            }
        }
        if let Some(Value::Object(props)) = v.get("properties") {
            for (k, p) in props {
                if let Some(d) = p.get("description").and_then(Value::as_str) {
                    out.entry(k.clone()).or_insert_with(|| Value::String(d.replace('\n', " ")));
                }
            }
        }
        for key in ["allOf", "anyOf"] {
            if let Some(Value::Array(a)) = v.get(key) {
                a.iter().for_each(|x| collect(x, defs, out, depth + 1));
            }
        }
    }
    collect(variant, &defs, &mut out, 0);
    out
}

/// JSON Schema for flow files.
pub fn json_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Flow)).expect("schema serializes")
}
