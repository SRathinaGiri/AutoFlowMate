//! ai_prompt / http_request / write_csv against a fake local HTTP server that
//! speaks the OpenAI-compatible (LM Studio), Gemini and Anthropic formats.

use autoflow_core::ai::{complete, AiApi, Request};
use autoflow_core::{run_flow, Flow, RunOptions};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<(String, Value, Vec<String>)>>>;

/// Serves forever on a random port. Replies "UnFav" to prompts containing "bad", else "Fav".
fn fake_server() -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut len = 0;
            let mut headers = vec![];
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                headers.push(lower.trim().to_string());
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let text = req.to_string();
            let answer = if text.contains("bad") { "UnFav" } else { "<think>positive words</think>Fav" };
            let reply = if path.ends_with("/models") {
                json!({"data": [{"id": "fake-local-model"}]})
            } else if path.contains(":generateContent") {
                json!({"candidates": [{"content": {"parts": [{"text": "thinking...", "thought": true}, {"text": answer}]}, "finishReason": "STOP"}]})
            } else if path.ends_with("/v1/messages") {
                json!({"content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": answer}], "stop_reason": "end_turn"})
            } else if path.ends_with("/echo") {
                json!({"method": line.split_whitespace().next(), "got": req})
            } else {
                json!({"choices": [{"message": {"role": "assistant", "content": answer}, "finish_reason": "stop"}]})
            };
            log2.lock().unwrap().push((path, req, headers));
            let out = reply.to_string();
            let mut s = stream;
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).unwrap();
        }
    });
    (base, log)
}

fn req(api: AiApi, base: &str, prompt: &str, model: &str) -> Request {
    Request {
        api,
        model: model.into(),
        base_url: base.into(),
        needs_key: api != AiApi::OpenaiCompatible,
        api_key: "test-key".into(),
        system: "Classify.".into(),
        prompt: prompt.into(),
        temperature: Some(0.0),
        max_tokens: 64,
        json: false,
        timeout: Duration::from_secs(10),
    }
}

#[test]
fn providers_speak_their_formats() {
    let (base, log) = fake_server();

    let (reply, model) = complete(&req(AiApi::OpenaiCompatible, &format!("{base}/v1"), "great product", "")).unwrap();
    assert_eq!((reply.as_str(), model.as_str()), ("Fav", "fake-local-model"), "thinking stripped, loaded model used");

    let (reply, model) = complete(&req(AiApi::Gemini, &base, "bad service", "gemini-2.5-flash")).unwrap();
    assert_eq!((reply.as_str(), model.as_str()), ("UnFav", "gemini-2.5-flash"));

    let (reply, model) = complete(&req(AiApi::Anthropic, &base, "bad service", "claude-opus-5-5")).unwrap();
    assert_eq!((reply.as_str(), model.as_str()), ("UnFav", "claude-opus-5-5"));

    let log = log.lock().unwrap();
    let (_, chat, _) = log.iter().find(|(p, ..)| p == "/v1/chat/completions").unwrap();
    assert_eq!(chat["model"], "fake-local-model");
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(chat["max_tokens"], 64);

    let (gpath, gem, gh) = log.iter().find(|(p, ..)| p.contains("generateContent")).unwrap();
    assert_eq!(gpath, "/models/gemini-2.5-flash:generateContent");
    assert_eq!(gem["systemInstruction"]["parts"][0]["text"], "Classify.");
    assert!(gh.iter().any(|h| h == "x-goog-api-key: test-key"));

    let (_, cl, ch) = log.iter().find(|(p, ..)| p == "/v1/messages").unwrap();
    assert_eq!(cl["system"], "Classify.");
    assert!(cl.get("temperature").is_none(), "sampling parameters are not sent to Claude");
    assert_eq!(cl["fallbacks"], "default");
    assert!(ch.iter().any(|h| h == "x-api-key: test-key"));
    assert!(ch.iter().any(|h| h == "anthropic-version: 2023-06-01"));
}

#[test]
fn classify_rows_and_write_csv() {
    // Use the built-in presets, not this PC's own settings file.
    std::env::set_var("AUTOFLOW_PRESETS", std::env::temp_dir().join("afm-no-presets.json"));
    let (base, _log) = fake_server();
    let dir = std::env::temp_dir().join(format!("afm_ai_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let flow: Flow = serde_json::from_value(json!({
        "name": "classify",
        "variables": {"comments": [{"Id": 1, "Comment": "Great, fast delivery"}, {"Id": 2, "Comment": "bad, \"awful\" support"}], "results": []},
        "steps": [
            {"id": "each", "action": "for_each", "items": "${comments}", "item_var": "row", "steps": [
                {"id": "ask", "action": "ai_prompt", "base_url": format!("{base}/v1"), "prompt": "Comment: ${row.Comment}",
                 "labels": ["Fav", "UnFav", "Neutral"], "output_var": "label"},
                {"id": "add", "action": "list_append", "name": "results", "value": {"Id": "${row.Id}", "Comment": "${row.Comment}", "Label": "${label}"}}
            ]},
            {"id": "csv", "action": "write_csv", "path": "out.csv", "data": "${results}", "count_var": "n"},
            {"id": "xlsx", "action": "write_excel", "path": "out.xlsx", "sheet": "Results", "data": "${results}"},
            {"id": "api", "action": "http_request", "method": "POST", "url": format!("{base}/echo"), "body": {"rows": "${n}"}, "status_var": "code"},
            {"id": "check", "action": "if", "condition": {"left": "${response.got.rows}", "op": "equals", "right": "2"},
             "else": [{"id": "bad", "action": "stop", "success": false, "message": "echo: ${response}"}]}
        ]
    }))
    .unwrap();
    let opts = RunOptions { flow_dir: dir.clone(), failsafe: Some(false), ..Default::default() };
    let result = run_flow(&flow, Default::default(), &opts, &mut |e| eprintln!("{e:?}"));
    assert!(result.success, "{}", result.message);

    let csv = std::fs::read_to_string(dir.join("out.csv")).unwrap();
    let csv = csv.trim_start_matches('\u{feff}');
    assert_eq!(csv.lines().next().unwrap(), "Id,Comment,Label");
    assert!(csv.contains("1,\"Great, fast delivery\",Fav"), "{csv}");
    assert!(csv.contains("2,\"bad, \"\"awful\"\" support\",UnFav"), "{csv}");
    assert!(dir.join("out.xlsx").exists());
    std::fs::remove_dir_all(dir).ok();
}
