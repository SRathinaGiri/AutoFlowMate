//! Engine tests that need no desktop: variables, control flow, files, sub-flows, errors.

use autoflow_core::{run_flow, Event, Flow, RunOptions};
use serde_json::{json, Map};
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("autoflow_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(flow: serde_json::Value, dir: &PathBuf) -> (autoflow_core::RunResult, Vec<Event>) {
    let flow: Flow = serde_json::from_value(flow).unwrap();
    let opts = RunOptions { flow_dir: dir.clone(), failsafe: Some(false), ..Default::default() };
    let mut events = vec![];
    let r = run_flow(&flow, Map::new(), &opts, &mut |e| events.push(e));
    (r, events)
}

#[test]
fn loops_conditions_and_variables() {
    let dir = tmpdir("loops");
    let (r, _) = run(
        json!({"name":"t","variables":{"total":0,"base":"${flow_dir}"},"steps":[
            {"action":"repeat","times":5,"steps":[
                {"action":"if","condition":{"left":"${index}","op":"greater_or_equal","right":"3"},
                 "then":[{"action":"break"}],
                 "else":[{"action":"set_variable","name":"total","value":"${index}"}]}
            ]},
            {"action":"set_variable","name":"list","value":["a","b","c"]},
            {"action":"set_variable","name":"joined","value":""},
            {"action":"for_each","items":"${list}","steps":[
                {"action":"set_variable","name":"joined","value":"${joined}${item}${index}"}
            ]},
            {"action":"set_variable","name":"n","value":0},
            {"action":"while","condition":{"left":"${n}","op":"less_than","right":"3"},"steps":[
                {"action":"increment","name":"n"}
            ]},
            {"action":"increment","name":"half","by":-2}
        ]}),
        &dir,
    );
    assert!(r.success, "{}", r.message);
    assert_eq!(r.variables["total"], json!(2)); // "${index}" alone copies the raw number
    assert_eq!(r.variables["joined"], json!("a0b1c2"));
    assert_eq!(r.variables["n"], json!(3));
    assert_eq!(r.variables["half"], json!(-2));
    assert_eq!(r.variables["base"], json!(dir.display().to_string()));
}

#[test]
fn file_actions() {
    let dir = tmpdir("files");
    let (r, _) = run(
        json!({"name":"t","steps":[
            {"action":"file_write","path":"in/a.txt","content":"hello"},
            {"action":"file_write","path":"in/a.txt","content":" world","append":true},
            {"action":"file_write","path":"in/b.log","content":"x"},
            {"action":"file_copy","source":"in/*.txt","destination":"out/"},
            {"action":"file_move","source":"in/b.log","destination":"out/renamed.log"},
            {"action":"file_read","path":"out/a.txt","output_var":"content"},
            {"action":"dir_list","path":"out","pattern":"*","output_var":"files"},
            {"action":"file_exists","path":"in/b.log","output_var":"b_left"},
            {"action":"file_info","path":"out/a.txt","output_var":"info"},
            {"action":"file_delete","path":"out/*.log"}
        ]}),
        &dir,
    );
    assert!(r.success, "{}", r.message);
    assert_eq!(r.variables["content"], json!("hello world"));
    assert_eq!(r.variables["files"].as_array().unwrap().len(), 2);
    assert_eq!(r.variables["b_left"], json!("false"));
    assert_eq!(r.variables["info"]["size"], json!(11));
    assert!(!dir.join("out/renamed.log").exists());
    assert!(dir.join("out/a.txt").exists());
}

#[test]
fn errors_retries_and_continue() {
    let dir = tmpdir("errors");
    let (r, events) = run(
        json!({"name":"t","steps":[
            {"id":"bad","action":"file_read","path":"missing.txt","output_var":"x","retries":2,"retry_delay_ms":10,"on_error":"continue"},
            {"id":"after","action":"set_variable","name":"reached","value":true},
            {"id":"fatal","action":"file_read","path":"missing.txt","output_var":"x"},
            {"id":"never","action":"set_variable","name":"never","value":true}
        ]}),
        &dir,
    );
    assert!(!r.success);
    assert!(r.message.contains("fatal"), "{}", r.message);
    assert_eq!(r.variables["reached"], json!(true));
    assert!(r.variables.get("never").is_none());
    let retries = events.iter().filter(|e| matches!(e, Event::Log { message, .. } if message.contains("retrying"))).count();
    assert_eq!(retries, 2);
}

#[test]
fn stop_and_subflow() {
    let dir = tmpdir("sub");
    std::fs::write(
        dir.join("child.json"),
        json!({"name":"child","variables":{"greeting":"hi"},"steps":[
            {"action":"set_variable","name":"result","value":"${greeting} ${who}"}
        ]})
        .to_string(),
    )
    .unwrap();
    let (r, _) = run(
        json!({"name":"t","steps":[
            {"action":"run_flow","path":"child.json","variables":{"who":"there"},"share_variables":true},
            {"action":"stop","message":"done early","success":true},
            {"action":"set_variable","name":"never","value":1}
        ]}),
        &dir,
    );
    assert!(r.success);
    assert_eq!(r.message, "done early");
    assert_eq!(r.variables["result"], json!("hi there"));
    assert!(r.variables.get("never").is_none());
}

#[test]
fn dry_run_touches_nothing() {
    let dir = tmpdir("dry");
    let flow: Flow = serde_json::from_value(json!({"name":"t","steps":[
        {"action":"file_write","path":"x.txt","content":"x"},
        {"action":"key_press","keys":"ctrl+s"},
        {"action":"mouse_click","x":10,"y":10}
    ]}))
    .unwrap();
    let opts = RunOptions { flow_dir: dir.clone(), dry_run: true, ..Default::default() };
    let r = run_flow(&flow, Map::new(), &opts, &mut |_| {});
    assert!(r.success, "{}", r.message);
    assert!(!dir.join("x.txt").exists());
}

#[test]
fn step_and_flow_delays() {
    let dir = tmpdir("delays");
    let started = std::time::Instant::now();
    let (r, _) = run(
        json!({"name":"t","settings":{"step_delay_ms":50,"failsafe":false,"abort_hotkey":""},"steps":[
            {"action":"set_variable","name":"a","value":1,"delay_before_ms":100},
            {"action":"set_variable","name":"b","value":2,"delay_after_ms":100}
        ]}),
        &dir,
    );
    assert!(r.success, "{}", r.message);
    // 100 before + 100 after + 2 x 50 step delay
    assert!(started.elapsed() >= std::time::Duration::from_millis(300), "{:?}", started.elapsed());
}

#[test]
fn stop_flag_cancels_long_wait() {
    let flow: Flow = serde_json::from_value(json!({"name":"t","settings":{"failsafe":false,"abort_hotkey":""},"steps":[
        {"action":"wait","ms":10000}
    ]}))
    .unwrap();
    let opts = RunOptions { flow_dir: std::env::temp_dir(), ..Default::default() };
    let stop = opts.stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let r = run_flow(&flow, Map::new(), &opts, &mut |_| {});
    assert!(!r.success);
    assert!(r.message.contains("cancelled"), "{}", r.message);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn tweens_hit_endpoints() {
    use autoflow_core::actions::Tween::*;
    for t in [Linear, EaseInQuad, EaseOutQuad, EaseInOutQuad, EaseInCubic, EaseOutCubic, EaseInOutCubic, EaseInOutSine, EaseOutBack, EaseOutBounce] {
        assert!((t.apply(0.0)).abs() < 1e-9, "{t:?}");
        assert!((t.apply(1.0) - 1.0).abs() < 1e-9, "{t:?}");
    }
}

#[test]
fn settings_default_and_roundtrip() {
    let f: Flow = serde_json::from_value(json!({"name":"t","steps":[]})).unwrap();
    assert_eq!(f.settings.key_delay_ms, 100);
    assert!(f.settings.failsafe);
    let back: Flow = serde_json::from_str(&f.to_json()).unwrap();
    assert_eq!(back.settings, f.settings);
}
