//! read_imap against a tiny fake IMAP server on localhost (plain TCP is only allowed for localhost).

use autoflow_core::mailread::{read_imap, ImapSettings, Query};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const MSG: &str = "From: Tally Bot <bot@example.com>\r\nTo: me@example.com\r\nSubject: Balance Sheet\r\n\
Date: Thu, 19 Nov 2026 10:00:00 +0530\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nAttached.\r\n\
--b\r\nContent-Type: application/xml\r\nContent-Disposition: attachment; filename=\"BS.xml\"\r\n\r\n<ENVELOPE/>\r\n--b--\r\n";

fn fake_imap() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let log2 = log.clone();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut w = stream.try_clone().unwrap();
        let mut r = BufReader::new(stream);
        w.write_all(b"* OK fake IMAP ready\r\n").unwrap();
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let line = line.trim_end().to_string();
            log2.lock().unwrap().push(line.clone());
            let (tag, rest) = line.split_once(' ').unwrap();
            let cmd = rest.to_ascii_uppercase();
            let reply = if cmd.starts_with("CAPABILITY") {
                format!("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n")
            } else if cmd.starts_with("LOGIN") {
                if rest.contains("\"secret\"") || rest.ends_with(" secret") {
                    format!("{tag} OK logged in\r\n")
                } else {
                    format!("{tag} NO bad password\r\n")
                }
            } else if cmd.starts_with("SELECT") {
                format!("* 2 EXISTS\r\n* FLAGS (\\Seen)\r\n{tag} OK [READ-WRITE] done\r\n")
            } else if cmd.starts_with("UID SEARCH") {
                format!("* SEARCH 5 7\r\n{tag} OK done\r\n")
            } else if cmd.starts_with("UID FETCH") {
                let uid: u32 = rest.split_whitespace().nth(2).unwrap().parse().unwrap();
                let body = MSG.replace("Balance Sheet", &format!("Balance Sheet {uid}"));
                format!("* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n{body})\r\n{tag} OK done\r\n", body.len())
            } else if cmd.starts_with("LOGOUT") {
                let r = format!("* BYE\r\n{tag} OK bye\r\n");
                w.write_all(r.as_bytes()).unwrap();
                break;
            } else {
                format!("{tag} OK done\r\n")
            };
            w.write_all(reply.as_bytes()).unwrap();
        }
    });
    (port, log)
}

#[test]
fn reads_marks_and_moves() {
    let (port, log) = fake_imap();
    let dir = std::env::temp_dir().join(format!("afm_imap_{}", std::process::id()));
    let s = ImapSettings { host: "127.0.0.1".into(), port, username: "me@example.com".into(), password: "secret".into() };
    let q = Query {
        folder: "INBOX".into(),
        unread_only: true,
        subject_contains: "Balance".into(),
        max: 10,
        newest_first: true,
        mark_read: true,
        move_to: "Processed".into(),
        save_dir: Some(dir.clone()),
        ..Query::default()
    };
    let mails = read_imap(&s, &q).unwrap();
    assert_eq!(mails.len(), 2);
    assert_eq!(mails[0]["subject"], "Balance Sheet 7", "newest (highest UID) first");
    assert_eq!(mails[0]["uid"], 7);
    assert_eq!(mails[0]["from"], "bot@example.com");
    assert_eq!(mails[1]["attachment_names"][0], "BS.xml");
    assert!(dir.join("BS.xml").exists() && dir.join("BS (2).xml").exists());
    let log = log.lock().unwrap().join("\n");
    assert!(log.contains("UID SEARCH UNSEEN SUBJECT \"Balance\""), "{log}");
    assert!(log.contains("BODY.PEEK[]"), "reading does not mark messages read by itself");
    assert!(log.contains("UID STORE 7,5 +FLAGS (\\Seen)"), "{log}");
    assert!(log.contains("UID MOVE 7,5 Processed") || log.contains("UID MOVE 7,5 \"Processed\""), "{log}");
    std::fs::remove_dir_all(dir).ok();
}

/// A dry run reads the mail but saves no attachments and changes nothing in the mailbox.
#[test]
fn dry_run_reads_but_changes_nothing() {
    use autoflow_core::{run_flow, Flow, RunOptions};
    let (port, log) = fake_imap();
    let secret = format!("afm-test-imap-{}", std::process::id());
    autoflow_core::mail::secrets::set(&secret, "secret").unwrap();
    let dir = std::env::temp_dir().join(format!("afm_imap_dry_{}", std::process::id()));
    let flow: Flow = serde_json::from_value(serde_json::json!({"name": "dry", "steps": [
        {"id": "r", "action": "read_email", "provider": "custom", "imap_host": "127.0.0.1", "imap_port": port,
         "username": "me@example.com", "password_secret": secret, "mark_read": true, "move_to": "Processed",
         "save_attachments_to": dir.display().to_string(), "output_var": "mails", "count_var": "n"}
    ]}))
    .unwrap();
    let opts = RunOptions { dry_run: true, failsafe: Some(false), ..Default::default() };
    let r = run_flow(&flow, Default::default(), &opts, &mut |_| {});
    autoflow_core::mail::secrets::delete(&secret).ok();
    assert!(r.success, "{}", r.message);
    assert_eq!(r.variables["n"], 2, "messages are still read");
    assert!(!dir.exists(), "no attachments saved in a dry run");
    let log = log.lock().unwrap().join("\n");
    assert!(!log.contains("UID STORE") && !log.contains("UID MOVE"), "{log}");
}

#[test]
fn wrong_password_is_reported() {
    let (port, _) = fake_imap();
    let s = ImapSettings { host: "127.0.0.1".into(), port, username: "me".into(), password: "nope".into() };
    let e = read_imap(&s, &Query { max: 1, ..Query::default() }).unwrap_err().to_string();
    assert!(e.contains("login") && e.contains("App Password"), "{e}");
}
