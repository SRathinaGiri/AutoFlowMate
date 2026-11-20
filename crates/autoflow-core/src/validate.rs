//! Static checks on flow JSON, designed to give AI agents and humans precise,
//! fixable messages before anything runs.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

use crate::actions::{catalog, Action};
use crate::flow::{Flow, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    /// Location such as `steps[2].then[0]`.
    pub location: String,
    pub message: String,
}

const COMMON_KEYS: &[&str] = &["action", "id", "enabled", "comment", "on_error", "retries", "retry_delay_ms", "delay_before_ms", "delay_after_ms"];

/// Validate flow JSON text. Returns the parsed flow (if it parses) and all issues found.
pub fn validate_json(text: &str) -> (Option<Flow>, Vec<Issue>) {
    let mut issues = Vec::new();
    let raw: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            issues.push(err("$", format!("invalid JSON: {e}")));
            return (None, issues);
        }
    };
    if let Some(steps) = raw.get("steps").and_then(Value::as_array) {
        check_raw_steps(steps, "steps", &mut issues);
    } else {
        issues.push(err("$", "missing \"steps\" array".into()));
    }
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return (None, issues);
    }
    let flow: Flow = match serde_json::from_value(raw) {
        Ok(f) => f,
        Err(e) => {
            issues.push(err("$", format!("flow does not match the format: {e}")));
            return (None, issues);
        }
    };
    check_flow(&flow, &mut issues);
    (Some(flow), issues)
}

fn err(loc: &str, message: String) -> Issue {
    Issue { severity: Severity::Error, location: loc.into(), message }
}
fn warn(loc: &str, message: String) -> Issue {
    Issue { severity: Severity::Warning, location: loc.into(), message }
}

/// Checks that need the raw JSON: unknown actions, unknown/typo'd keys, wrong types.
fn check_raw_steps(steps: &[Value], base: &str, issues: &mut Vec<Issue>) {
    let cat = catalog();
    for (i, s) in steps.iter().enumerate() {
        let loc = format!("{base}[{i}]");
        let Some(obj) = s.as_object() else {
            issues.push(err(&loc, "step must be an object".into()));
            continue;
        };
        let Some(name) = obj.get("action").and_then(Value::as_str) else {
            issues.push(err(&loc, "step has no \"action\" string".into()));
            continue;
        };
        let Some(info) = cat.iter().find(|a| a.name == name) else {
            let close: Vec<&str> = cat.iter().map(|a| a.name).filter(|n| similar(n, name)).collect();
            let hint = if close.is_empty() { String::new() } else { format!(" (did you mean {}?)", close.join(", ")) };
            issues.push(err(&loc, format!("unknown action \"{name}\"{hint}. Run `autoflow actions` for the list")));
            continue;
        };
        let known: HashSet<&str> = info.template.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
        for key in obj.keys() {
            if !known.contains(key.as_str()) && !COMMON_KEYS.contains(&key.as_str()) {
                let mut valid: Vec<&str> = known.iter().copied().filter(|k| *k != "action").collect();
                valid.sort();
                issues.push(warn(&loc, format!("unknown field \"{key}\" for {name} (ignored). Valid fields: {}", valid.join(", "))));
            }
        }
        // Nested steps first, so a mistake inside `then` / `catch` is reported at that step.
        let before = issues.iter().filter(|i| i.severity == Severity::Error).count();
        for key in ["then", "else", "steps", "catch", "finally"] {
            if let Some(children) = obj.get(key).and_then(Value::as_array) {
                check_raw_steps(children, &format!("{loc}.{key}"), issues);
            }
        }
        let child_errors = issues.iter().filter(|i| i.severity == Severity::Error).count() > before;
        // Type errors for this step alone (unless they come from a child already reported).
        if !child_errors {
            if let Err(e) = serde_json::from_value::<Step>(s.clone()) {
                issues.push(err(&loc, format!("{name}: {e}")));
            }
        }
    }
}

fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase().replace(['.', '-', ' '], "_"));
    a == b || a.contains(&b) || b.contains(&a) || (a.len() > 3 && b.len() > 3 && a[..4] == b[..4.min(b.len())])
}

/// Semantic checks on the parsed flow.
fn check_flow(flow: &Flow, issues: &mut Vec<Issue>) {
    for m in crate::watch::check(&flow.triggers) {
        let (loc, msg) = m.split_once(": ").unwrap_or(("triggers", &m));
        issues.push(err(loc, msg.to_string()));
    }
    let s = &flow.settings;
    if !s.abort_hotkey.trim().is_empty() {
        if let Err(e) = crate::failsafe::parse_hotkey(&s.abort_hotkey) {
            issues.push(err("settings.abort_hotkey", e.to_string()));
        }
    }
    if !s.failsafe && s.abort_hotkey.trim().is_empty() {
        issues.push(warn("settings", "fail-safe and abort hotkey are both off: a runaway flow can only be stopped from Studio/Task Manager".into()));
    }
    for (name, ty) in &flow.variable_types {
        match flow.variables.get(name) {
            None => issues.push(warn(&format!("variable_types.{name}"), format!("type given for `{name}`, which is not in variables"))),
            Some(v) => {
                if let Some(msg) = ty.check(v) {
                    issues.push(warn(&format!("variables.{name}"), msg));
                }
            }
        }
    }
    for name in flow.variables.keys() {
        let ok = name.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !ok {
            issues.push(warn(&format!("variables.{name}"), format!("`{name}`: use letters, digits and _ only (no spaces or dots)")));
        }
    }
    if s.key_delay_ms > 2000 {
        issues.push(warn("settings.key_delay_ms", format!("{} ms per keystroke is very slow", s.key_delay_ms)));
    }
    let mut ids = HashSet::new();
    let mut defined: HashSet<String> = flow.variables.keys().cloned().collect();
    defined.insert("last_error".into());
    for t in &flow.triggers {
        defined.insert(match t {
            crate::watch::Trigger::File(f) => f.variable.clone(),
            crate::watch::Trigger::Email(m) => m.variable.clone(),
        });
    }
    collect_defined(&flow.steps, &mut defined);
    walk(&flow.steps, "steps", 0, &mut ids, &defined, issues);
}

/// Every variable name a flow can use: declared variables first, then the ones
/// steps create (set_variable/increment names and *_var outputs), in flow order.
/// Built-ins are not included (see `vars::BUILTINS`).
pub fn defined_variables(flow: &Flow) -> Vec<String> {
    let mut out: Vec<String> = flow.variables.keys().cloned().collect();
    // Variables a trigger sets when the watcher starts the flow.
    for t in &flow.triggers {
        let v = match t {
            crate::watch::Trigger::File(f) => &f.variable,
            crate::watch::Trigger::Email(m) => &m.variable,
        };
        if !v.trim().is_empty() && !out.iter().any(|x| x == v.trim()) {
            out.push(v.trim().to_string());
        }
    }
    fn walk(steps: &[Step], out: &mut Vec<String>) {
        for s in steps {
            let mut add = |n: &str| {
                let n = n.trim();
                if !n.is_empty() && !n.contains("${") && !out.iter().any(|x| x == n) {
                    out.push(n.to_string());
                }
            };
            match &s.action {
                Action::SetVariable(a) => add(&a.name),
                Action::Increment(a) => add(&a.name),
                Action::ListAppend(a) => add(&a.name),
                _ => {}
            }
            if let Ok(Value::Object(o)) = serde_json::to_value(&s.action) {
                for (k, v) in o {
                    if k.ends_with("_var") {
                        if let Some(n) = v.as_str() {
                            add(n);
                        }
                    }
                }
            }
            for c in s.action.children() {
                walk(c, out);
            }
        }
    }
    walk(&flow.steps, &mut out);
    out
}

fn collect_defined(steps: &[Step], out: &mut HashSet<String>) {
    for s in steps {
        match &s.action {
            Action::SetVariable(a) => {
                out.insert(a.name.clone());
            }
            Action::Increment(a) => {
                out.insert(a.name.clone());
            }
            Action::ListAppend(a) => {
                out.insert(a.name.clone());
            }
            _ => {}
        }
        if let Action::RunFlow(a) = &s.action {
            if a.share_variables {
                out.insert("*".into()); // sub-flow may define anything
            }
        }
        if let Ok(Value::Object(o)) = serde_json::to_value(&s.action) {
            for (k, v) in o {
                if k.ends_with("_var") {
                    if let Some(name) = v.as_str() {
                        out.insert(name.to_string());
                    }
                }
            }
        }
        for c in s.action.children() {
            collect_defined(c, out);
        }
    }
}

fn walk(steps: &[Step], base: &str, loop_depth: usize, ids: &mut HashSet<String>, defined: &HashSet<String>, issues: &mut Vec<Issue>) {
    for (i, s) in steps.iter().enumerate() {
        let loc = if s.id.is_empty() { format!("{base}[{i}]") } else { format!("{base}[{i}] ({})", s.id) };
        if !s.id.is_empty() && !ids.insert(s.id.clone()) {
            issues.push(err(&loc, format!("duplicate step id \"{}\"", s.id)));
        }
        check_action(&s.action, &loc, loop_depth, issues);
        // Variable references.
        if !defined.contains("*") {
            if let Ok(v) = serde_json::to_value(&s.action) {
                let mut refs = Vec::new();
                collect_refs(&v, &mut refs, true);
                for r in refs {
                    let head = r.split('.').next().unwrap_or("");
                    let builtin = crate::vars::BUILTINS.contains(&r.as_str()) || r.starts_with("env.") || r.starts_with("now:");
                    if !builtin && !defined.contains(&r) && !defined.contains(head) {
                        issues.push(warn(&loc, format!("variable `${{{r}}}` is never set in this flow (pass it with --var {head}=...)")));
                    }
                }
            }
        }
        let is_loop = matches!(s.action, Action::Repeat(_) | Action::While(_) | Action::ForEach(_));
        for (label, children) in s.action.child_labels().iter().zip(s.action.children()) {
            walk(children, &format!("{loc}.{label}"), loop_depth + is_loop as usize, ids, defined, issues);
        }
    }
}

/// Collect `${name}` references in all string values except nested step lists.
fn collect_refs(v: &Value, out: &mut Vec<String>, top: bool) {
    match v {
        Value::String(s) => {
            let mut rest = s.as_str();
            while let Some(p) = rest.find("${") {
                if p > 0 && rest.as_bytes()[p - 1] == b'$' {
                    rest = &rest[p + 2..];
                    continue;
                }
                let body = &rest[p + 2..];
                match body.find('}') {
                    Some(e) => {
                        out.push(body[..e].trim().to_string());
                        rest = &body[e + 1..];
                    }
                    None => break,
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_refs(x, out, false)),
        Value::Object(o) => {
            for (k, x) in o {
                if top && matches!(k.as_str(), "then" | "else" | "steps" | "catch" | "finally") {
                    continue;
                }
                collect_refs(x, out, false);
            }
        }
        _ => {}
    }
}

/// Number of `?` placeholders outside quoted strings.
fn placeholders(sql: &str) -> usize {
    let (mut n, mut quote) = (0, None::<char>);
    for c in sql.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '?') => n += 1,
            _ => {}
        }
    }
    n
}

fn check_sql(issues: &mut Vec<Issue>, loc: &str, name: &str, db: &crate::actions::DbConnection, sql: &str, params: &[Value]) {
    let lower = db.connection.to_ascii_lowercase().replace(' ', "");
    if (lower.contains("pwd=") || lower.contains("password=")) && !db.connection.contains("{password}") {
        issues.push(warn(loc, format!("{name}: the connection contains a password; write PWD={{password}} and set password_secret instead")));
    }
    if db.connection.contains("{password}") && db.password_secret.trim().is_empty() {
        issues.push(err(loc, format!("{name}: the connection uses {{password}}: set password_secret")));
    }
    if sql.contains("${") {
        issues.push(warn(
            loc,
            format!("{name}: a ${{variable}} inside the SQL is pasted in as-is (a quote in the value breaks the statement); use ? and params for values"),
        ));
    }
    let q = placeholders(sql);
    if q != params.len() && !(params.is_empty() && name == "db_execute" && q == 0) {
        issues.push(err(loc, format!("{name}: the SQL has {q} ? placeholder(s) but {} param(s) are given", params.len())));
    }
}

fn check_action(a: &Action, loc: &str, loop_depth: usize, issues: &mut Vec<Issue>) {
    let need = |issues: &mut Vec<Issue>, field: &str, v: &str| {
        if v.trim().is_empty() {
            issues.push(err(loc, format!("{}: \"{field}\" is required", a.name())));
        }
    };
    match a {
        Action::SetVariable(x) => need(issues, "name", &x.name),
        Action::Increment(x) => need(issues, "name", &x.name),
        Action::FileCopy(x) | Action::FileMove(x) => {
            need(issues, "source", &x.source);
            need(issues, "destination", &x.destination);
        }
        Action::FileDelete(x) => need(issues, "path", &x.path),
        Action::FileWrite(x) => need(issues, "path", &x.path),
        Action::FileRead(x) | Action::FileExists(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::FileInfo(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::DirCreate(x) => need(issues, "path", &x.path),
        Action::DirDelete(x) => need(issues, "path", &x.path),
        Action::DirList(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::RunProgram(x) => need(issues, "program", &x.program),
        Action::KillProgram(x) => need(issues, "name", &x.name),
        Action::Open(x) => need(issues, "target", &x.target),
        Action::RunFlow(x) => need(issues, "path", &x.path),
        Action::WindowActivate(x) | Action::WindowWait(x) | Action::WindowClose(x) | Action::WindowWaitClose(x) => need(issues, "title", &x.title),
        Action::WindowSetState(x) => need(issues, "title", &x.window.title),
        Action::WindowExists(x) => {
            need(issues, "title", &x.window.title);
            need(issues, "output_var", &x.output_var);
        }
        Action::KeyPress(x) => {
            if !x.keys.contains("${") {
                if let Err(e) = crate::input::parse_combo(&x.keys) {
                    issues.push(err(loc, format!("key_press: {e}")));
                }
            }
        }
        Action::UiClick(x) => check_selector(&x.target, loc, issues),
        Action::UiSetText(x) => check_selector(&x.target, loc, issues),
        Action::UiGetText(x) | Action::UiExists(x) => {
            check_selector(&x.target, loc, issues);
            need(issues, "output_var", &x.output_var);
        }
        Action::UiWait(x) => check_selector(&x.target, loc, issues),
        Action::ClipboardGet(x) => need(issues, "output_var", &x.output_var),
        Action::KeyDown(x) | Action::KeyUp(x) => {
            need(issues, "keys", &x.keys);
            if !x.keys.contains("${") && !x.keys.trim().is_empty() {
                if let Err(e) = crate::input::parse_combo(&x.keys) {
                    issues.push(err(loc, format!("{}: {e}", a.name())));
                }
            }
        }
        Action::MouseDown(x) | Action::MouseUp(x) if x.x.is_some() != x.y.is_some() => {
            issues.push(err(loc, format!("{}: set both x and y, or neither", a.name())))
        }
        Action::PixelGet(x) => need(issues, "output_var", &x.output_var),
        Action::PixelMatches(x) => {
            need(issues, "output_var", &x.output_var);
            if !x.color.contains("${") {
                if let Err(e) = crate::vision::parse_hex(&x.color) {
                    issues.push(err(loc, format!("pixel_matches: {e}")));
                }
            }
        }
        Action::ImageFind(x) => check_image(&x.target, loc, issues),
        Action::ImageFindAll(x) => {
            check_image(&x.target, loc, issues);
            need(issues, "output_var", &x.output_var);
        }
        Action::ImageClick(x) => check_image(&x.target, loc, issues),
        Action::ImageWait(x) | Action::ImageWaitVanish(x) => check_image(&x.target, loc, issues),
        Action::InputBox(x) => need(issues, "output_var", &x.output_var),
        Action::BrowserNavigate(x) => need(issues, "url", &x.url),
        Action::BrowserType(x) => web_target(&x.target, loc, a.name(), issues),
        Action::BrowserClick(x) | Action::BrowserWait(x) => web_target(&x.target, loc, a.name(), issues),
        Action::BrowserGetText(x) => {
            web_target(&x.target, loc, a.name(), issues);
            need(issues, "output_var", &x.output_var);
        }
        Action::BrowserScreenshot(x) => need(issues, "path", &x.path),
        Action::BrowserWaitText(x) => need(issues, "text", &x.text),
        Action::ReadExcel(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::ReadCsv(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::ListAppend(x) => need(issues, "name", &x.name),
        Action::ReadPdfTable(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
            if !x.pages.contains("${") {
                if let Err(e) = crate::documents::parse_pages(&x.pages) {
                    issues.push(err(loc, format!("read_pdf_table: {e}")));
                }
            }
        }
        Action::WordRead(x) => {
            need(issues, "path", &x.path);
            if x.path.to_ascii_lowercase().ends_with(".doc") {
                issues.push(err(loc, "word_read: old .doc files are not supported; save the file as .docx".into()));
            }
        }
        Action::WordFill(x) => {
            need(issues, "template", &x.template);
            need(issues, "path", &x.path);
            if x.values.is_empty() {
                issues.push(warn(loc, "word_fill: no values given for the {{placeholders}}".into()));
            }
        }
        Action::WordWrite(x) => need(issues, "path", &x.path),
        Action::ZipCreate(x) => {
            need(issues, "path", &x.path);
            if x.sources.iter().all(|s| s.trim().is_empty()) {
                issues.push(err(loc, "zip_create: list the files or folders to zip in sources".into()));
            }
        }
        Action::ZipExtract(x) => need(issues, "path", &x.path),
        Action::DbQuery(x) => {
            need(issues, "connection", &x.db.connection);
            need(issues, "sql", &x.sql);
            check_sql(issues, loc, "db_query", &x.db, &x.sql, &x.params);
        }
        Action::DbExecute(x) => {
            need(issues, "connection", &x.db.connection);
            need(issues, "sql", &x.sql);
            check_sql(issues, loc, "db_execute", &x.db, &x.sql, &x.params);
        }
        Action::ReadEmail(x) => {
            need(issues, "username", &x.username);
            if x.password_secret.trim().is_empty() {
                issues.push(err(loc, "read_email: set password_secret (the name of the password stored with `autoflow secret set NAME`)".into()));
            }
            if crate::mailread::imap_defaults(&x.provider).is_none() && x.imap_host.trim().is_empty() && !x.provider.contains("${") {
                issues.push(err(loc, format!("read_email: unknown provider `{}`; set imap_host", x.provider)));
            }
            need(issues, "output_var", &x.filter.output_var);
        }
        Action::OutlookRead(x) => need(issues, "output_var", &x.filter.output_var),
        Action::OcrRead(x) => {
            need(issues, "output_var", &x.output_var);
            if x.source.region.is_some_and(|r| r.width == 0 || r.height == 0) {
                issues.push(err(loc, "ocr_read: region width and height must be > 0".into()));
            }
        }
        Action::OcrFind(x) => need(issues, "text", &x.text),
        Action::OcrClick(x) => {
            need(issues, "text", &x.find.text);
            if !x.find.source.image.trim().is_empty() {
                issues.push(err(loc, "ocr_click reads the screen; remove `image` (use ocr_find to search a picture)".into()));
            }
        }
        Action::Try(x) => {
            if x.steps.is_empty() {
                issues.push(warn(loc, "try: no steps to try".into()));
            }
        }
        Action::TextTransform(x) => need(issues, "output_var", &x.output_var),
        Action::TextReplace(x) => {
            need(issues, "find", &x.find);
            need(issues, "output_var", &x.output_var);
            if x.regex && !x.find.contains("${") {
                if let Err(e) = regex::Regex::new(&x.find) {
                    issues.push(err(loc, format!("text_replace: bad regular expression: {e}")));
                }
            }
        }
        Action::TextExtract(x) => {
            need(issues, "pattern", &x.pattern);
            need(issues, "output_var", &x.output_var);
            if !x.pattern.contains("${") {
                match regex::Regex::new(&x.pattern) {
                    Err(e) => issues.push(err(loc, format!("text_extract: bad regular expression: {e}"))),
                    Ok(re) if x.group as usize >= re.captures_len() => issues.push(err(loc, format!("text_extract: the pattern has no group {}", x.group))),
                    _ => {}
                }
            }
        }
        Action::TextSplit(x) => need(issues, "output_var", &x.output_var),
        Action::TextJoin(x) => need(issues, "output_var", &x.output_var),
        Action::DateCalc(x) => {
            need(issues, "output_var", &x.output_var);
            let unit = x.unit.trim().to_ascii_lowercase();
            let ok = unit.contains("${")
                || matches!(unit.trim_end_matches('s'), "" | "second" | "sec" | "minute" | "min" | "hour" | "day" | "week" | "month" | "year");
            if !ok && matches!(x.operation, crate::actions::DateOperation::Add | crate::actions::DateOperation::Diff) {
                issues.push(err(loc, format!("date_calc: unknown unit `{}` (seconds, minutes, hours, days, weeks, months, years)", x.unit)));
            }
        }
        Action::Calculate(x) => {
            need(issues, "expression", &x.expression);
            need(issues, "output_var", &x.output_var);
        }
        Action::WriteCsv(x) => {
            need(issues, "path", &x.path);
            if x.data.is_null() || x.data.as_str().is_some_and(|s| s.trim().is_empty()) {
                issues.push(err(loc, "write_csv: \"data\" is required, e.g. \"${results}\"".into()));
            }
            if !matches!(x.delimiter.as_str(), "" | "\\t" | "\t" | "tab") && x.delimiter.chars().count() != 1 {
                issues.push(err(loc, format!("write_csv: delimiter must be one character or \\t, got `{}`", x.delimiter)));
            }
        }
        Action::WriteExcel(x) => {
            need(issues, "path", &x.path);
            if x.data.is_null() || x.data.as_str().is_some_and(|s| s.trim().is_empty()) {
                issues.push(err(loc, "write_excel: \"data\" is required, e.g. \"${results}\"".into()));
            }
            let p = x.path.to_ascii_lowercase();
            if !p.contains("${") && !p.ends_with(".xlsx") {
                issues.push(warn(loc, "write_excel: path should end in .xlsx".into()));
            }
        }
        Action::HttpRequest(x) => {
            need(issues, "url", &x.url);
            let m = x.method.trim().to_ascii_uppercase();
            if !m.contains("${") && !matches!(m.as_str(), "" | "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS") {
                issues.push(err(loc, format!("http_request: unknown method `{}`", x.method)));
            }
            for (k, v) in &x.headers {
                if k.eq_ignore_ascii_case("authorization") && v.as_str().is_some_and(|s| !s.contains("${") && s.len() > 12) {
                    issues.push(warn(loc, "http_request: a literal Authorization header puts a secret in the flow; use bearer_secret instead".into()));
                }
            }
        }
        Action::AiPrompt(x) => {
            need(issues, "prompt", &x.prompt);
            need(issues, "output_var", &x.output_var);
            if !x.provider.contains("${") {
                match crate::ai::resolve(&x.provider) {
                    Err(e) => issues.push(err(loc, format!("ai_prompt: {e}"))),
                    Ok(p) if p.needs_key && x.api_key_secret.trim().is_empty() && p.key_secret.trim().is_empty() => issues.push(err(
                        loc,
                        format!("ai_prompt: provider {} needs api_key_secret (store the key with `autoflow secret set NAME`)", x.provider.trim()),
                    )),
                    Ok(p) if p.base_url.trim().is_empty() && x.base_url.trim().is_empty() => {
                        issues.push(err(loc, format!("ai_prompt: provider {} has no server address; set base_url", x.provider.trim())))
                    }
                    _ => {}
                }
            }
            if x.json && !x.labels.is_empty() {
                issues.push(warn(loc, "ai_prompt: json is ignored when labels are given".into()));
            }
        }
        Action::SendEmail(x) => {
            need(issues, "to", &format!("{}{}{}", x.to, x.cc, x.bcc));
            if x.subject.trim().is_empty() {
                issues.push(warn(loc, "send_email: subject is empty".into()));
            }
            if !x.username.trim().is_empty() && x.password_secret.trim().is_empty() {
                issues.push(err(loc, "send_email: set password_secret (the name of the password stored with `autoflow secret set NAME`)".into()));
            }
            if crate::mail::provider_defaults(&x.provider).is_none() && x.smtp_host.trim().is_empty() {
                issues.push(err(
                    loc,
                    format!("send_email: provider `{}` is not known; use gmail / outlook365 / outlook.com / yahoo / zoho, or set smtp_host", x.provider),
                ));
            }
            if x.username.trim().is_empty() && x.from.trim().is_empty() {
                issues.push(err(loc, "send_email: set username (your e-mail address) or from".into()));
            }
        }
        Action::OutlookSend(x) => need(issues, "to", &format!("{}{}{}", x.to, x.cc, x.bcc)),
        Action::ReadPdf(x) => {
            need(issues, "path", &x.path);
            need(issues, "output_var", &x.output_var);
        }
        Action::BrowserEval(x) => {
            need(issues, "script", &x.script);
        }
        Action::Screenshot(x) => need(issues, "path", &x.path),
        Action::Break(_) if loop_depth == 0 => issues.push(err(loc, "break used outside of a loop".into())),
        Action::MouseClick(x) if x.x.is_some() != x.y.is_some() => issues.push(err(loc, "mouse_click: set both x and y, or neither".into())),
        Action::ForEach(x) if x.items.as_str().map(|s| s.trim().is_empty()).unwrap_or(false) => issues.push(err(loc, "for_each: \"items\" is required".into())),
        _ => {}
    }
}

fn web_target(t: &crate::actions::WebTarget, loc: &str, action: &str, issues: &mut Vec<Issue>) {
    if t.element_id.trim().is_empty() && t.selector.trim().is_empty() {
        issues.push(err(loc, format!("{action}: give the \"element_id\" (or a CSS \"selector\")")));
    }
    if t.element_id.trim().starts_with('#') {
        issues.push(warn(loc, format!("{action}: \"element_id\" should be the bare id without '#', e.g. \"email\"")));
    }
}

fn check_image(t: &crate::actions::ImageTarget, loc: &str, issues: &mut Vec<Issue>) {
    if t.image.trim().is_empty() {
        issues.push(err(loc, "\"image\" is required (a PNG of what to find)".into()));
    }
    if !(0.0..=1.0).contains(&t.confidence) {
        issues.push(err(loc, "confidence must be between 0.0 and 1.0".into()));
    } else if t.confidence < 0.7 {
        issues.push(warn(loc, format!("confidence {} is low; expect false matches", t.confidence)));
    }
    if let Some(r) = t.region {
        if r.width == 0 || r.height == 0 {
            issues.push(err(loc, "region width and height must be > 0".into()));
        }
    }
}

fn check_selector(s: &crate::actions::Selector, loc: &str, issues: &mut Vec<Issue>) {
    if s.name.is_empty() && s.automation_id.is_empty() && s.class_name.is_empty() && s.control_type.is_empty() {
        issues.push(err(loc, "selector needs at least one of name, automation_id, class_name, control_type".into()));
    }
    #[cfg(windows)]
    if !s.control_type.is_empty() && !s.control_type.contains("${") {
        if let Err(e) = crate::uia::parse_control_type(&s.control_type) {
            issues.push(err(loc, e.to_string()));
        }
    }
    if s.window.is_empty() {
        issues.push(warn(loc, "selector has no \"window\"; searching the whole desktop is slow".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_common_mistakes() {
        let text = r#"{"name":"t","steps":[
            {"action":"file_copyy","source":"a"},
            {"action":"file_copy","source":"a","dest":"b"},
            {"action":"break"},
            {"action":"log","message":"${undefined_thing}"}
        ]}"#;
        let (_, issues) = validate_json(text);
        let msgs: Vec<String> = issues.iter().map(|i| i.message.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains("unknown action \"file_copyy\"")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("unknown field \"dest\"")), "{msgs:?}");
    }

    #[test]
    fn semantic_checks() {
        let text = r#"{"name":"t","steps":[
            {"action":"break"},
            {"action":"log","message":"${nope}"},
            {"action":"key_press","keys":"ctrl+nokey"}
        ]}"#;
        let (flow, issues) = validate_json(text);
        assert!(flow.is_some());
        let msgs: Vec<String> = issues.iter().map(|i| i.message.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains("break used outside")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("${nope}")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("unknown key")), "{msgs:?}");
    }

    #[test]
    fn every_template_roundtrips() {
        for info in catalog() {
            let step: Step = serde_json::from_value(info.template.clone()).unwrap_or_else(|e| panic!("{}: {e}", info.name));
            assert_eq!(step.action.name(), info.name);
            let minimal: Step = serde_json::from_value(serde_json::json!({"action": info.name})).unwrap();
            assert_eq!(minimal.action.name(), info.name);
        }
    }
}
