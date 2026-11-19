//! E-mail tests without a real mail account: a tiny local SMTP server receives
//! what a flow sends. Plus a Credential Manager round trip.

use autoflow_core::{run_flow, Flow, RunOptions};
use serde_json::{json, Map};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;

/// Accept one SMTP session and return everything the client sent.
fn fake_smtp() -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut w = stream.try_clone().unwrap();
        let mut r = BufReader::new(stream);
        let mut log = String::new();
        writeln!(w, "220 fake ESMTP\r").unwrap();
        let mut in_data = false;
        let mut line = String::new();
        loop {
            line.clear();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            log.push_str(&line);
            if in_data {
                if line == ".\r\n" {
                    in_data = false;
                    writeln!(w, "250 2.0.0 queued as TEST123\r").unwrap();
                }
                continue;
            }
            let cmd = line.to_ascii_uppercase();
            if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                write!(w, "250-fake\r\n250 8BITMIME\r\n").unwrap();
            } else if cmd.starts_with("DATA") {
                in_data = true;
                writeln!(w, "354 go ahead\r").unwrap();
            } else if cmd.starts_with("QUIT") {
                writeln!(w, "221 bye\r").unwrap();
                break;
            } else {
                writeln!(w, "250 ok\r").unwrap();
            }
        }
        let _ = tx.send(log);
    });
    (port, rx)
}

#[test]
fn flow_sends_mail_with_attachment_over_smtp() {
    let dir = std::env::temp_dir().join(format!("afm_mail_flow_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("BalanceSheet_test.xml"), "<ENVELOPE><BSNAME>Capital</BSNAME></ENVELOPE>").unwrap();
    let (port, rx) = fake_smtp();
    let flow: Flow = serde_json::from_value(json!({
        "name": "mail",
        "settings": { "failsafe": false, "abort_hotkey": "" },
        "variables": { "file": "BalanceSheet_test.xml" },
        "steps": [{
            "action": "send_email", "provider": "custom", "smtp_host": "127.0.0.1", "smtp_port": port,
            "security": "none", "username": "", "from": "AutoFlowMate <bot@example.com>",
            "to": "boss@example.com; audit@example.com", "subject": "Balance Sheet ${file}",
            "body": "Please find the Balance Sheet attached.", "attachments": ["${file}"]
        }]
    }))
    .unwrap();
    let opts = RunOptions { flow_dir: dir.clone(), ..Default::default() };
    let mut logs = vec![];
    let r = run_flow(&flow, Map::new(), &opts, &mut |e| logs.push(format!("{e:?}")));
    assert!(r.success, "{} {:?}", r.message, logs);
    let session = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert!(session.contains("MAIL FROM:<bot@example.com>"), "{session}");
    assert!(session.contains("RCPT TO:<boss@example.com>"));
    assert!(session.contains("RCPT TO:<audit@example.com>"));
    assert!(session.contains("Subject: Balance Sheet BalanceSheet_test.xml"));
    assert!(session.contains("filename=\"BalanceSheet_test.xml\""));
    assert!(session.contains("Please find the Balance Sheet attached."));
    assert!(logs.iter().any(|l| l.contains("TEST123")), "server reply logged");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_attachment_fails_before_sending() {
    let flow: Flow = serde_json::from_value(json!({
        "name": "mail", "settings": { "failsafe": false, "abort_hotkey": "" },
        "steps": [{ "action": "send_email", "provider": "custom", "smtp_host": "127.0.0.1", "smtp_port": 1,
                    "security": "none", "from": "a@b.c", "to": "x@y.z", "subject": "s", "attachments": ["nope.xml"] }]
    }))
    .unwrap();
    let r = run_flow(&flow, Map::new(), &RunOptions { flow_dir: std::env::temp_dir(), ..Default::default() }, &mut |_| {});
    assert!(!r.success);
    assert!(r.message.contains("attachment not found"), "{}", r.message);
}

#[test]
fn secrets_round_trip_in_credential_manager() {
    use autoflow_core::mail::secrets;
    let name = format!("autoflowmate-test-{}", std::process::id());
    secrets::set(&name, "s3cret-value").unwrap();
    assert!(secrets::exists(&name));
    assert_eq!(secrets::get(&name).unwrap(), "s3cret-value");
    secrets::delete(&name).unwrap();
    assert!(!secrets::exists(&name));
    assert!(secrets::get(&name).unwrap_err().to_string().contains("autoflow secret set"));
}
