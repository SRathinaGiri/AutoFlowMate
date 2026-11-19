//! Variable storage and `${...}` interpolation.

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

/// Built-in names, resolved when no user variable has that name:
/// `now` (2026-01-31 14:05:09), `date` (2026-01-31), `time` (14:05:09),
/// `timestamp` (20260131_140509), `unix` (seconds), `env.NAME`,
/// `flow_dir`, `cwd`, `newline`, `tab`.
pub const BUILTINS: &[&str] = &[
    "now", "date", "time", "timestamp", "unix", "env.NAME", "flow_dir", "cwd", "newline", "tab",
];

#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub map: Map<String, Value>,
    pub flow_dir: String,
}

impl Vars {
    pub fn get(&self, name: &str) -> Option<Value> {
        // Exact variable first, then dotted path into objects/arrays (`row.0`, `info.size`).
        if let Some(v) = self.map.get(name) {
            return Some(v.clone());
        }
        let mut parts = name.split('.');
        let head = parts.next()?;
        if let Some(mut cur) = self.map.get(head).cloned() {
            for p in parts {
                cur = match cur {
                    Value::Object(mut o) => o.remove(p)?,
                    Value::Array(mut a) => {
                        let i: usize = p.parse().ok()?;
                        if i < a.len() {
                            a.swap_remove(i)
                        } else {
                            return None;
                        }
                    }
                    _ => return None,
                };
            }
            return Some(cur);
        }
        self.builtin(name)
    }

    fn builtin(&self, name: &str) -> Option<Value> {
        let now = chrono::Local::now();
        let s = match name {
            "now" => now.format("%Y-%m-%d %H:%M:%S").to_string(),
            "date" => now.format("%Y-%m-%d").to_string(),
            "time" => now.format("%H:%M:%S").to_string(),
            "timestamp" => now.format("%Y%m%d_%H%M%S").to_string(),
            "unix" => now.timestamp().to_string(),
            "flow_dir" => self.flow_dir.clone(),
            "cwd" => std::env::current_dir().ok()?.display().to_string(),
            "newline" => "\r\n".into(),
            "tab" => "\t".into(),
            _ => {
                let env = name.strip_prefix("env.")?;
                std::env::var(env).ok()?
            }
        };
        Some(Value::String(s))
    }

    pub fn set(&mut self, name: &str, value: Value) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("variable name is empty");
        }
        self.map.insert(name.to_string(), value);
        Ok(())
    }

    pub fn set_str(&mut self, name: &str, value: impl Into<String>) -> Result<()> {
        self.set(name, Value::String(value.into()))
    }

    /// Replace every `${name}` in `text`. `$${` produces a literal `${`.
    pub fn interpolate(&self, text: &str) -> Result<String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(pos) = rest.find('$') {
            out.push_str(&rest[..pos]);
            let after = &rest[pos..];
            if after.starts_with("$${") {
                out.push_str("${");
                rest = &after[3..];
            } else if let Some(body) = after.strip_prefix("${") {
                let end = body
                    .find('}')
                    .ok_or_else(|| anyhow!("unclosed `${{` in \"{text}\""))?;
                let name = body[..end].trim();
                let value = self
                    .get(name)
                    .ok_or_else(|| anyhow!("unknown variable `{name}`"))?;
                out.push_str(&to_text(&value));
                rest = &body[end + 1..];
            } else {
                out.push('$');
                rest = &after[1..];
            }
        }
        out.push_str(rest);
        Ok(out)
    }

    /// Interpolate a JSON value: strings are interpolated (a string that is exactly
    /// `${x}` returns x's raw value); arrays and objects recursively.
    pub fn resolve_value(&self, v: &Value) -> Result<Value> {
        Ok(match v {
            Value::String(s) => {
                let t = s.trim();
                if t.starts_with("${") && t.ends_with('}') && t[2..].find('}') == Some(t.len() - 3) {
                    let name = t[2..t.len() - 1].trim();
                    self.get(name).ok_or_else(|| anyhow!("unknown variable `{name}`"))?
                } else {
                    Value::String(self.interpolate(s)?)
                }
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| self.resolve_value(x)).collect::<Result<_>>()?),
            Value::Object(o) => {
                let mut m = Map::new();
                for (k, x) in o {
                    m.insert(k.clone(), self.resolve_value(x)?);
                }
                Value::Object(m)
            }
            other => other.clone(),
        })
    }
}

/// How a value is rendered inside text.
pub fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Parse `name=value` overrides; value is parsed as JSON if possible, else kept as text.
pub fn parse_override(s: &str) -> Result<(String, Value)> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("expected name=value, got `{s}`"))?;
    let value = match serde_json::from_str::<Value>(v) {
        Ok(j @ (Value::Array(_) | Value::Object(_) | Value::Number(_) | Value::Bool(_))) => j,
        _ => Value::String(v.to_string()),
    };
    Ok((k.trim().to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interpolation() {
        let mut v = Vars::default();
        v.set("name", json!("World")).unwrap();
        v.set("list", json!(["a", "b"])).unwrap();
        v.set("obj", json!({"k": 5})).unwrap();
        assert_eq!(v.interpolate("Hello ${name}!").unwrap(), "Hello World!");
        assert_eq!(v.interpolate("${list.1}/${obj.k}").unwrap(), "b/5");
        assert_eq!(v.interpolate("cost $5 $${x}").unwrap(), "cost $5 ${x}");
        assert!(v.interpolate("${missing}").is_err());
        assert_eq!(v.resolve_value(&json!("${list}")).unwrap(), json!(["a", "b"]));
        assert_eq!(v.resolve_value(&json!("${name}-${name}")).unwrap(), json!("World-World"));
    }
}
