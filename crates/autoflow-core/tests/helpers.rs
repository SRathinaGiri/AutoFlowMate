//! try / catch / finally, text, date and calculate actions.

use autoflow_core::{run_flow, validate_json, Flow, RunOptions};
use serde_json::{json, Value};

fn run(steps: Value, vars: Value) -> (bool, String, Vec<String>) {
    let flow: Flow = serde_json::from_value(json!({"name": "t", "variables": vars, "steps": steps})).unwrap();
    let opts = RunOptions { failsafe: Some(false), ..Default::default() };
    let mut logs = vec![];
    let r = run_flow(&flow, Default::default(), &opts, &mut |e| {
        if let autoflow_core::Event::Log { message, .. } = e {
            logs.push(message);
        }
    });
    (r.success, r.message, logs)
}

#[test]
fn try_catch_finally() {
    let (ok, msg, logs) = run(
        json!([
            {"id": "t", "action": "try",
             "steps": [
                {"id": "a", "action": "log", "message": "before"},
                {"id": "boom", "action": "file_read", "path": "Z:/definitely/missing.txt", "output_var": "x"},
                {"id": "never", "action": "log", "message": "not reached"}
             ],
             "catch": [{"id": "c", "action": "log", "message": "caught -> ${error}"}],
             "finally": [{"id": "f", "action": "log", "message": "cleanup"}]},
            {"id": "after", "action": "log", "message": "flow continues"}
        ]),
        json!({}),
    );
    assert!(ok, "{msg}");
    assert!(logs.iter().any(|l| l.starts_with("caught -> step boom (file_read) failed")), "{logs:?}");
    assert!(!logs.iter().any(|l| l == "not reached"));
    let fin = logs.iter().position(|l| l == "cleanup").unwrap();
    let after = logs.iter().position(|l| l == "flow continues").unwrap();
    assert!(fin < after);

    // finally also runs when a loop is left with break, and on success
    let (ok, _, logs) = run(
        json!([{"id": "r", "action": "repeat", "times": 3, "steps": [
            {"id": "t", "action": "try", "steps": [{"id": "b", "action": "break"}],
             "finally": [{"id": "f", "action": "log", "message": "fin"}]}
        ]}]),
        json!({}),
    );
    assert!(ok);
    assert_eq!(logs.iter().filter(|l| *l == "fin").count(), 1);

    // a failure inside catch fails the flow, after finally ran
    let (ok, _, logs) = run(
        json!([{"id": "t", "action": "try",
            "steps": [{"id": "s", "action": "stop", "success": true, "message": "ok"}, {"id": "z", "action": "log", "message": "no"}],
            "finally": [{"id": "f", "action": "log", "message": "fin2"}]}]),
        json!({}),
    );
    assert!(ok);
    assert!(logs.contains(&"fin2".to_string()));
}

#[test]
fn text_date_calc() {
    let (ok, msg, logs) = run(
        json!([
            {"id": "u", "action": "text_transform", "input": "  hello world ", "operation": "title", "output_var": "t"},
            {"id": "u2", "action": "text_transform", "input": "${t}", "operation": "trim", "output_var": "t"},
            {"id": "r", "action": "text_replace", "input": "Inv-001, inv-002", "find": "inv-(\\d+)", "replace_with": "#$1", "regex": true, "ignore_case": true, "output_var": "r"},
            {"id": "x", "action": "text_extract", "input": "GSTIN: 33AAACH7409R1Z8 and 09AAACH7409R1ZZ", "pattern": "\\d{2}[A-Z]{5}\\d{4}[A-Z]\\d[A-Z][A-Z\\d]", "all": true, "output_var": "gst"},
            {"id": "x2", "action": "text_extract", "input": "Total: Rs. 1,180.00", "pattern": "Total:\\s*Rs\\.\\s*([\\d,.]+)", "group": 1, "output_var": "total", "found_var": "found"},
            {"id": "x3", "action": "text_replace", "input": "${total}", "find": ",", "replace_with": "", "output_var": "total"},
            {"id": "s", "action": "text_split", "input": "a, b,,c", "output_var": "parts"},
            {"id": "j", "action": "text_join", "items": "${parts}", "separator": "|", "output_var": "joined"},
            {"id": "c", "action": "calculate", "expression": "${total} * 18 / 118", "decimals": 2, "output_var": "tax"},
            {"id": "d", "action": "date_calc", "operation": "add", "date": "31/01/2026", "amount": 1, "unit": "months", "format": "dd-MMM-yyyy", "output_var": "due"},
            {"id": "d2", "action": "date_calc", "operation": "diff", "date": "01-04-2025", "other_date": "2026-03-31", "unit": "days", "output_var": "days"},
            {"id": "d3", "action": "date_calc", "operation": "parts", "date": "2026-11-19", "output_var": "p"},
            {"id": "out", "action": "log", "message": "${t}|${r}|${gst.1}|${found}|${joined}|${tax}|${due}|${days}|${p.weekday_name}"}
        ]),
        json!({}),
    );
    assert!(ok, "{msg}");
    assert_eq!(logs.last().unwrap(), "Hello World|#001, #002|09AAACH7409R1ZZ|true|a|b|c|180|28-Feb-2026|364|Thursday");
}

#[test]
fn validation_catches_mistakes() {
    let text = json!({"name": "v", "steps": [
        {"id": "a", "action": "text_extract", "pattern": "(", "output_var": "m"},
        {"id": "b", "action": "date_calc", "operation": "add", "unit": "fortnights", "output_var": "d"},
        {"id": "c", "action": "try", "steps": [{"id": "c1", "action": "log", "message": "x"}], "catch": [{"id": "c2", "action": "nope"}]},
        {"id": "e", "action": "ai_prompt", "provider": "nope", "prompt": "hi"}
    ]})
    .to_string();
    let (_, issues) = validate_json(&text);
    let all = issues.iter().map(|i| format!("{} {}", i.location, i.message)).collect::<Vec<_>>().join("\n");
    assert!(all.contains("steps[2].catch[0]"), "nested catch steps are checked:\n{all}");
    let (_, issues) = validate_json(
        &json!({"name": "v", "steps": [
            {"id": "a", "action": "text_extract", "pattern": "(", "output_var": "m"},
            {"id": "b", "action": "date_calc", "operation": "add", "unit": "fortnights", "output_var": "d"},
            {"id": "e", "action": "ai_prompt", "provider": "nope", "prompt": "hi"}
        ]})
        .to_string(),
    );
    let all = issues.iter().map(|i| i.message.clone()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("bad regular expression"), "{all}");
    assert!(all.contains("unknown unit"), "{all}");
    assert!(all.contains("unknown AI provider"), "{all}");
}
