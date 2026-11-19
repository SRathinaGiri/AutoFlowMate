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
    /// Initial variables. Can be overridden at run time (`--var name=value`).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<String, Value>,
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
            variables: Map::new(),
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
