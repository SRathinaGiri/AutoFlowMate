//! Breakpoints and step-by-step: the run pauses and waits for the editor to resume it.

use autoflow_core::{run_flow, Debugger, Event, Flow, RunOptions};
use serde_json::json;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

fn flow() -> Flow {
    serde_json::from_value(json!({"name": "d", "variables": {"n": 0}, "steps": [
        {"id": "a", "action": "increment", "name": "n"},
        {"id": "b", "action": "increment", "name": "n"},
        {"id": "c", "action": "log", "message": "n=${n}"}
    ]}))
    .unwrap()
}

#[test]
fn breakpoint_pauses_with_variables_then_continues() {
    let debug = Arc::new(Debugger::new(["b".to_string()], false));
    let opts = RunOptions { failsafe: Some(false), debug: Some(debug.clone()), ..Default::default() };
    let (tx, rx) = channel();
    let f = flow();
    let h = std::thread::spawn(move || run_flow(&f, Default::default(), &opts, &mut |e| tx.send(e).unwrap()));
    let mut paused = vec![];
    loop {
        match rx.recv_timeout(Duration::from_secs(10)).expect("run hung") {
            Event::Paused { id, variables, .. } => {
                assert_eq!(variables["n"], 1, "a ran, b not yet");
                paused.push(id);
                debug.resume(false);
            }
            Event::Finished { success, .. } => {
                assert!(success);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(paused, ["b"]);
    h.join().unwrap();
}

#[test]
fn stepping_pauses_before_every_step_and_stop_ends_it() {
    let debug = Arc::new(Debugger::new(Vec::<String>::new(), true));
    let opts = RunOptions { failsafe: Some(false), debug: Some(debug.clone()), ..Default::default() };
    let stop = opts.stop.clone();
    let (tx, rx) = channel();
    let f = flow();
    let h = std::thread::spawn(move || run_flow(&f, Default::default(), &opts, &mut |e| tx.send(e).unwrap()));
    let mut paused = vec![];
    let mut ok = None;
    while let Ok(e) = rx.recv_timeout(Duration::from_secs(10)) {
        match e {
            Event::Paused { id, .. } => {
                paused.push(id.clone());
                if id == "b" {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                } else {
                    debug.resume(true);
                }
            }
            Event::Finished { success, .. } => {
                ok = Some(success);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(paused, ["a", "b"]);
    assert_eq!(ok, Some(false), "stopping while paused cancels the run");
    h.join().unwrap();
}
