//! What the JSON schema says about each field, per action: its description, the values it
//! allows (with their own descriptions) and its default. Built once from the same schema that
//! `autoflow schema` writes, so it always matches the action code.

use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct FieldInfo {
    pub help: String,
    /// Allowed values and what each means (empty = free value).
    pub options: Vec<(String, String)>,
    pub default: Option<Value>,
}

#[derive(Default)]
pub struct SchemaInfo {
    /// By action name (date_calc) and by definition name (FileTrigger, Settings, Step).
    fields: HashMap<String, HashMap<String, FieldInfo>>,
    /// Help texts by field, per action (what the property form's `help` expects).
    help: HashMap<String, HashMap<String, String>>,
}

fn def<'a>(defs: &'a Map<String, Value>, r: &str) -> Option<&'a Value> {
    defs.get(r.trim_start_matches("#/$defs/"))
}

/// Allowed values of an enum definition (oneOf of consts with descriptions, or a plain enum).
fn options(defs: &Map<String, Value>, v: &Value) -> Vec<(String, String)> {
    let target = match v.get("$ref").and_then(Value::as_str) {
        Some(r) => def(defs, r).unwrap_or(v),
        None => v,
    };
    // Option<Enum> appears as anyOf [ {$ref}, {type: null} ].
    if let Some(any) = target.get("anyOf").and_then(Value::as_array) {
        for a in any {
            let o = options(defs, a);
            if !o.is_empty() {
                return o;
            }
        }
    }
    if let Some(one) = target.get("oneOf").and_then(Value::as_array) {
        let o: Vec<(String, String)> = one
            .iter()
            .filter_map(|x| {
                let c = x.get("const").and_then(Value::as_str)?;
                Some((c.to_string(), x.get("description").and_then(Value::as_str).unwrap_or("").to_string()))
            })
            .collect();
        if o.len() == one.len() {
            return o;
        }
    }
    if let Some(e) = target.get("enum").and_then(Value::as_array) {
        return e.iter().filter_map(Value::as_str).map(|s| (s.to_string(), String::new())).collect();
    }
    vec![]
}

/// Properties of a definition, following $ref and allOf (flattened structs).
fn collect(defs: &Map<String, Value>, v: &Value, out: &mut HashMap<String, FieldInfo>, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Some(r) = v.get("$ref").and_then(Value::as_str) {
        if let Some(d) = def(defs, r) {
            collect(defs, d, out, depth + 1);
        }
    }
    for key in ["allOf", "anyOf"] {
        if let Some(list) = v.get(key).and_then(Value::as_array) {
            for x in list {
                collect(defs, x, out, depth + 1);
            }
        }
    }
    if let Some(props) = v.get("properties").and_then(Value::as_object) {
        for (k, p) in props {
            if k == "action" {
                continue;
            }
            let info = out.entry(k.clone()).or_default();
            if info.help.is_empty() {
                info.help = p.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            }
            if info.options.is_empty() {
                info.options = options(defs, p);
            }
            if info.default.is_none() {
                info.default = p.get("default").cloned();
            }
        }
    }
}

impl SchemaInfo {
    pub fn load() -> SchemaInfo {
        let schema = autoflow_core::flow::json_schema();
        let empty = Map::new();
        let defs = schema.get("$defs").and_then(Value::as_object).unwrap_or(&empty);
        let mut info = SchemaInfo::default();
        for (name, d) in defs {
            let mut fields = HashMap::new();
            collect(defs, d, &mut fields, 0);
            info.fields.insert(name.clone(), fields);
        }
        // Each action of a step: {properties: {action: {const: name}}, $ref: Def}.
        if let Some(variants) = defs.get("Step").and_then(|s| s.get("oneOf")).and_then(Value::as_array) {
            for v in variants {
                let Some(name) = v.pointer("/properties/action/const").and_then(Value::as_str) else { continue };
                let mut fields = HashMap::new();
                collect(defs, v, &mut fields, 0);
                info.fields.insert(name.to_string(), fields);
            }
        }
        for (k, f) in &info.fields {
            info.help.insert(k.clone(), f.iter().filter(|(_, x)| !x.help.is_empty()).map(|(n, x)| (n.clone(), x.help.clone())).collect());
        }
        info
    }

    /// Fields of an action (date_calc) or definition (FileTrigger).
    pub fn fields(&self, name: &str) -> Option<&HashMap<String, FieldInfo>> {
        self.fields.get(name)
    }

    pub fn help(&self, name: &str) -> &HashMap<String, String> {
        static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
        self.help.get(name).unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_action_fields_and_options() {
        let s = SchemaInfo::load();
        let d = s.fields("date_calc").unwrap();
        let ops: Vec<&str> = d["operation"].options.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(ops, ["format", "add", "diff", "parts"]);
        assert!(d["operation"].options[1].1.contains("Add"), "options carry their descriptions");
        let t = s.fields("text_transform").unwrap();
        assert!(t["operation"].options.iter().any(|(v, _)| v == "title"), "same field name, its own values");
        // Flattened structs (ocr_click -> OcrFind -> OcrSource) are followed.
        let o = s.fields("ocr_click").unwrap();
        assert!(o.contains_key("text") && o.contains_key("window") && o.contains_key("offset_x"));
        assert_eq!(s.fields("mouse_click").unwrap()["button"].options.len(), 3);
        assert!(!s.help("date_calc")["unit"].is_empty());
    }
}
