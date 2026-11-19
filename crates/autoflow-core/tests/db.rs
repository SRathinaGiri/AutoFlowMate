//! db_execute / db_query through a flow, on a SQLite file.

use autoflow_core::{run_flow, validate_json, Flow, RunOptions};
use serde_json::json;

#[test]
fn sqlite_flow() {
    let dir = std::env::temp_dir().join(format!("afm_db_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let flow_json = json!({
        "name": "db",
        "variables": {"items": [{"Name": "Pen", "Qty": 10, "Price": 5.5}, {"Name": "O'Neil notebook", "Qty": 2, "Price": 40}], "min": 3},
        "steps": [
            {"id": "create", "action": "db_execute", "connection": "sqlite:data/shop.db",
             "sql": "DROP TABLE IF EXISTS stock; CREATE TABLE stock (name TEXT, qty INTEGER, price REAL);"},
            {"id": "each", "action": "for_each", "items": "${items}", "item_var": "it", "steps": [
                {"id": "ins", "action": "db_execute", "connection": "sqlite:data/shop.db",
                 "sql": "INSERT INTO stock (name, qty, price) VALUES (?, ?, ?)", "params": ["${it.Name}", "${it.Qty}", "${it.Price}"], "affected_var": "n"}
            ]},
            {"id": "q", "action": "db_query", "connection": "sqlite:data/shop.db",
             "sql": "SELECT name, qty * price AS value FROM stock WHERE qty >= ? ORDER BY name", "params": ["${min}"], "output_var": "rows", "count_var": "count"},
            {"id": "out", "action": "log", "message": "${count}: ${rows.0.name} = ${rows.0.value}"},
            {"id": "csv", "action": "write_csv", "path": "out.csv", "data": "${rows}"}
        ]
    });
    let (_, issues) = validate_json(&flow_json.to_string());
    assert!(issues.is_empty(), "{issues:?}");
    let flow: Flow = serde_json::from_value(flow_json).unwrap();
    let opts = RunOptions { flow_dir: dir.clone(), failsafe: Some(false), ..Default::default() };
    let mut logs = vec![];
    let r = run_flow(&flow, Default::default(), &opts, &mut |e| {
        if let autoflow_core::Event::Log { message, .. } = e {
            logs.push(message)
        }
    });
    assert!(r.success, "{} {logs:?}", r.message);
    assert!(logs.contains(&"1: Pen = 55".to_string()), "{logs:?}");
    assert!(dir.join("data/shop.db").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn dry_run_never_creates_or_changes_a_database() {
    let dir = std::env::temp_dir().join(format!("afm_db_dry_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |steps: serde_json::Value, dry: bool| {
        let flow: Flow = serde_json::from_value(json!({"name": "d", "steps": steps})).unwrap();
        let opts = RunOptions { flow_dir: dir.clone(), failsafe: Some(false), dry_run: dry, ..Default::default() };
        let mut logs = vec![];
        let r = run_flow(&flow, Default::default(), &opts, &mut |e| {
            if let autoflow_core::Event::Log { message, .. } = e {
                logs.push(message)
            }
        });
        (r, logs)
    };
    // Missing database: a dry run logs the query and creates nothing.
    let (r, logs) = run(json!([{"id": "q", "action": "db_query", "connection": "sqlite:new/none.db", "sql": "SELECT 1 AS x", "output_var": "rows"}]), true);
    assert!(r.success, "{}", r.message);
    assert!(logs.iter().any(|l| l.contains("not created yet")), "{logs:?}");
    assert!(!dir.join("new").exists(), "no folder or file created");
    // Existing database: a dry run reads it, read-only.
    let (r, _) = run(
        json!([{"id": "c", "action": "db_execute", "connection": "sqlite:real.db", "sql": "CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (7);"}]),
        false,
    );
    assert!(r.success, "{}", r.message);
    let (r, _) = run(json!([{"id": "q", "action": "db_query", "connection": "sqlite:real.db", "sql": "SELECT a FROM t", "output_var": "rows"}]), true);
    assert!(r.success, "{}", r.message);
    assert_eq!(r.variables["rows"], json!([{"a": 7}]));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn sql_checks() {
    let text = json!({"name": "v", "steps": [
        {"id": "a", "action": "db_query", "connection": "x.db", "sql": "SELECT * FROM t WHERE a = ? AND b = '?'", "params": []},
        {"id": "b", "action": "db_query", "connection": "x.db", "sql": "SELECT * FROM t WHERE a = '${v}'"},
        {"id": "c", "action": "db_execute", "connection": "Driver={SQL Server};Server=s;UID=sa;PWD=hunter2", "sql": "DELETE FROM t"}
    ]})
    .to_string();
    let (_, issues) = validate_json(&text);
    let all = issues.iter().map(|i| format!("{} {}", i.location, i.message)).collect::<Vec<_>>().join("\n");
    assert!(all.contains("db_query: the SQL has 1 ? placeholder(s) but 0 param(s)"), "{all}");
    assert!(all.contains("db_query: a ${variable} inside the SQL"), "{all}");
    assert!(all.contains("db_execute: the connection contains a password"), "{all}");
}
