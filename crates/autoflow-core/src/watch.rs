//! Triggers: run a flow when a file arrives in a folder or new e-mail comes in.
//!
//! A flow lists its `triggers`; `autoflow watch flow.json` keeps running and starts the flow
//! once per event, with the file path or the mail record in a variable. A scheduled task with
//! the "logon" trigger can start the watcher automatically (`autoflow schedule add --watch`).

use anyhow::{anyhow, bail, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Something that starts the flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// A new (or changed) file in a folder.
    File(FileTrigger),
    /// New unread e-mail (IMAP or Outlook).
    Email(EmailTrigger),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FileTrigger {
    /// Folder to watch (relative paths are relative to the flow's folder).
    pub folder: String,
    /// Which files, e.g. *.pdf or Invoice_*.xlsx.
    pub pattern: String,
    /// Also watch sub-folders.
    pub recursive: bool,
    /// Variable that receives the file's full path when the flow runs.
    pub variable: String,
    /// Also run for files that are already there when watching starts.
    pub include_existing: bool,
    /// Wait until the file has not changed for this long (it may still be copying).
    pub settle_ms: u64,
    /// After a successful run, move the file into this folder (e.g. "done"); empty = leave it.
    pub move_to: String,
    /// After a failed run, move the file into this folder (e.g. "failed"); empty = leave it.
    pub move_failed_to: String,
}
impl Default for FileTrigger {
    fn default() -> Self {
        Self {
            folder: String::new(),
            pattern: "*".into(),
            recursive: false,
            variable: "file".into(),
            include_existing: false,
            settle_ms: 2000,
            move_to: String::new(),
            move_failed_to: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct EmailTrigger {
    /// "imap" (Gmail, Zoho, any IMAP server) or "outlook" (the Outlook desktop app).
    pub source: String,
    /// IMAP: a provider from File > Settings > E-mail providers, or custom with imap_host.
    pub provider: String,
    /// IMAP server; empty = the provider's.
    pub imap_host: String,
    /// IMAP port (0 = 993).
    pub imap_port: u16,
    /// IMAP login, usually the e-mail address.
    pub username: String,
    /// Name of the password stored with `autoflow secret set NAME` (Gmail: an App Password).
    pub password_secret: String,
    /// Outlook: account / mailbox name (empty = default).
    pub account: String,
    /// Folder to watch (IMAP: INBOX; Outlook: Inbox or Inbox/Sub).
    pub folder: String,
    /// Only mail from this sender (address or part of it).
    pub from: String,
    /// Only mail whose subject contains this text.
    pub subject_contains: String,
    /// Save attachments here (each mail's saved paths are in mail.attachments).
    pub save_attachments_to: String,
    /// Only attachments matching this pattern, e.g. *.pdf.
    pub attachment_filter: String,
    /// Move handled mail to this folder afterwards (it is always marked read).
    pub move_to: String,
    /// Check every this many minutes.
    pub every_minutes: u32,
    /// Variable that receives the mail record {from, subject, date, body, attachments, ...}.
    pub variable: String,
}
impl Default for EmailTrigger {
    fn default() -> Self {
        Self {
            source: "imap".into(),
            provider: "gmail".into(),
            imap_host: String::new(),
            imap_port: 0,
            username: String::new(),
            password_secret: String::new(),
            account: String::new(),
            folder: String::new(),
            from: String::new(),
            subject_contains: String::new(),
            save_attachments_to: String::new(),
            attachment_filter: String::new(),
            move_to: String::new(),
            every_minutes: 5,
            variable: "mail".into(),
        }
    }
}

/// One thing that happened: run the flow with `variable` = `value`.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub trigger: usize,
    pub variable: String,
    pub value: Value,
    /// For file events: the file, so it can be moved afterwards.
    pub file: Option<PathBuf>,
}

pub struct WatchOptions {
    /// Check once (files already there count as new) and return; for scheduled "every N minutes" use.
    pub once: bool,
    pub stop: std::sync::Arc<AtomicBool>,
    /// How often folders are checked.
    pub poll: Duration,
}

fn abs(dir: &Path, p: &str) -> PathBuf {
    let p = PathBuf::from(p.trim());
    if p.is_absolute() {
        p
    } else {
        dir.join(p)
    }
}

fn file_key(p: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// True when another program still has the file open for writing.
fn in_use(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(p).is_err()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(p).is_err()
    }
}

fn list(t: &FileTrigger, dir: &Path) -> Result<Vec<PathBuf>> {
    let folder = abs(dir, &t.folder);
    if !folder.is_dir() {
        bail!("watched folder {} does not exist", folder.display());
    }
    let base = glob::Pattern::escape(&folder.display().to_string());
    let pat = if t.pattern.trim().is_empty() { "*" } else { t.pattern.trim() };
    let full = if t.recursive { format!("{base}/**/{pat}") } else { format!("{base}/{pat}") };
    let mut v: Vec<PathBuf> = glob::glob(&full).map_err(|e| anyhow!("bad pattern: {e}"))?.flatten().filter(|p| p.is_file()).collect();
    // Never pick up files in the move_to / move_failed_to folders when watching recursively.
    let skip: Vec<PathBuf> = [&t.move_to, &t.move_failed_to].iter().filter(|s| !s.trim().is_empty()).map(|s| abs(&folder, s)).collect();
    v.retain(|p| !skip.iter().any(|s| p.starts_with(s)));
    v.sort();
    Ok(v)
}

/// Move a handled file into `to` (relative to the watched folder), without overwriting.
pub fn move_handled(file: &Path, t: &FileTrigger, dir: &Path, success: bool) -> Result<Option<PathBuf>> {
    let to = if success { &t.move_to } else { &t.move_failed_to };
    if to.trim().is_empty() {
        return Ok(None);
    }
    let target_dir = abs(&abs(dir, &t.folder), to);
    std::fs::create_dir_all(&target_dir)?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = crate::mailread::unique_path(&target_dir, &name);
    std::fs::rename(file, &target).map_err(|e| anyhow!("cannot move {} to {}: {e}", file.display(), target.display()))?;
    Ok(Some(target))
}

/// Watch the triggers and call `run` for every event (one at a time). `run` returns whether
/// the flow succeeded. Returns when `stop` is set, or after one pass with `once`.
pub fn watch(triggers: &[Trigger], flow_dir: &Path, opts: &WatchOptions, log: &mut dyn FnMut(String), run: &mut dyn FnMut(&WatchEvent) -> bool) -> Result<()> {
    if triggers.is_empty() {
        bail!("the flow has no triggers: add a \"triggers\" list (see the skill's scheduling guide)");
    }
    // Files already handled (or present at start), by path -> (size, modified).
    let mut seen: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
    // Files waiting to settle: path -> (key, first time this key was seen).
    let mut pending: HashMap<PathBuf, ((u64, SystemTime), Instant)> = HashMap::new();
    let mut next_mail: Vec<Instant> = vec![Instant::now(); triggers.len()];
    for (i, t) in triggers.iter().enumerate() {
        match t {
            Trigger::File(f) => {
                // A watched folder that does not exist yet is created, so files can be dropped in.
                let folder = abs(flow_dir, &f.folder);
                if !folder.exists() {
                    std::fs::create_dir_all(&folder).map_err(|e| anyhow!("cannot create watched folder {}: {e}", folder.display()))?;
                    log(format!("created {}", folder.display()));
                }
                let files = list(f, flow_dir)?;
                if !f.include_existing && !opts.once {
                    for p in &files {
                        if let Some(k) = file_key(p) {
                            seen.insert(p.clone(), k);
                        }
                    }
                }
                log(format!("watching {} for {} ({} file(s) there now)", abs(flow_dir, &f.folder).display(), f.pattern, files.len()));
            }
            Trigger::Email(m) => {
                log(format!(
                    "checking {} mail every {} min",
                    if m.source.eq_ignore_ascii_case("outlook") { "Outlook" } else { m.username.as_str() },
                    m.every_minutes.max(1)
                ));
                next_mail[i] = Instant::now();
            }
        }
    }
    loop {
        for (i, t) in triggers.iter().enumerate() {
            if opts.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match t {
                Trigger::File(f) => {
                    let files = match list(f, flow_dir) {
                        Ok(v) => v,
                        Err(e) => {
                            log(format!("file trigger: {e:#}"));
                            continue;
                        }
                    };
                    for p in files {
                        let Some(key) = file_key(&p) else { continue };
                        if seen.get(&p) == Some(&key) {
                            continue;
                        }
                        let settle = Duration::from_millis(f.settle_ms);
                        let ready = if opts.once {
                            !in_use(&p)
                        } else {
                            match pending.get(&p) {
                                Some((k, since)) if *k == key => since.elapsed() >= settle && !in_use(&p),
                                _ => {
                                    pending.insert(p.clone(), (key, Instant::now()));
                                    settle.is_zero() && !in_use(&p)
                                }
                            }
                        };
                        if !ready {
                            continue;
                        }
                        pending.remove(&p);
                        seen.insert(p.clone(), key);
                        log(format!("new file: {}", p.display()));
                        let ev = WatchEvent { trigger: i, variable: f.variable.clone(), value: json!(p.display().to_string()), file: Some(p.clone()) };
                        let ok = run(&ev);
                        match move_handled(&p, f, flow_dir, ok) {
                            Ok(Some(to)) => {
                                seen.remove(&p);
                                log(format!("moved to {}", to.display()));
                            }
                            Ok(None) => {}
                            Err(e) => log(format!("{e:#}")),
                        }
                    }
                }
                Trigger::Email(m) => {
                    if Instant::now() < next_mail[i] {
                        continue;
                    }
                    next_mail[i] = Instant::now() + Duration::from_secs(60 * m.every_minutes.max(1) as u64);
                    match check_mail(m, flow_dir) {
                        Ok(mails) => {
                            for mail in mails {
                                log(format!("new mail: {}", mail.get("subject").and_then(Value::as_str).unwrap_or("")));
                                run(&WatchEvent { trigger: i, variable: m.variable.clone(), value: mail, file: None });
                            }
                        }
                        Err(e) => log(format!("mail trigger: {e:#}")),
                    }
                }
            }
        }
        if opts.once {
            return Ok(());
        }
        // Sleep in small slices so stopping stays responsive.
        let end = Instant::now() + opts.poll;
        while Instant::now() < end {
            if opts.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Unread matching mail, marked read (and moved) so it does not trigger again.
fn check_mail(m: &EmailTrigger, flow_dir: &Path) -> Result<Vec<Value>> {
    let q = crate::mailread::Query {
        folder: m.folder.clone(),
        unread_only: true,
        from: m.from.clone(),
        subject_contains: m.subject_contains.clone(),
        since: None,
        max: 50,
        newest_first: false,
        mark_read: true,
        move_to: m.move_to.clone(),
        save_dir: if m.save_attachments_to.trim().is_empty() { None } else { Some(abs(flow_dir, &m.save_attachments_to)) },
        attachment_filter: m.attachment_filter.clone(),
    };
    if m.source.eq_ignore_ascii_case("outlook") {
        return crate::mailread::read_outlook(&q, &m.account);
    }
    let (host, port) = match (m.imap_host.trim(), crate::mailread::imap_defaults(&m.provider)) {
        (h, _) if !h.is_empty() => (h.to_string(), if m.imap_port == 0 { 993 } else { m.imap_port }),
        (_, Some((h, p))) => (h.to_string(), if m.imap_port == 0 { p } else { m.imap_port }),
        _ => bail!("set imap_host (or provider gmail / outlook365 / yahoo / zoho)"),
    };
    let password = crate::mail::secrets::get(&m.password_secret)?;
    crate::mailread::read_imap(&crate::mailread::ImapSettings { host, port, username: m.username.clone(), password }, &q)
}

/// Problems with a flow's triggers, for `validate`.
pub fn check(triggers: &[Trigger]) -> Vec<String> {
    let mut out = vec![];
    for (i, t) in triggers.iter().enumerate() {
        let at = format!("triggers[{i}]");
        match t {
            Trigger::File(f) => {
                if f.folder.trim().is_empty() {
                    out.push(format!("{at}: file trigger needs a folder"));
                }
                if f.variable.trim().is_empty() {
                    out.push(format!("{at}: set variable (it receives the file path)"));
                }
            }
            Trigger::Email(m) => {
                let outlook = m.source.eq_ignore_ascii_case("outlook");
                if !outlook && !m.source.eq_ignore_ascii_case("imap") {
                    out.push(format!("{at}: source must be imap or outlook"));
                }
                if !outlook && (m.username.trim().is_empty() || m.password_secret.trim().is_empty()) {
                    out.push(format!("{at}: an IMAP trigger needs username and password_secret"));
                }
                if !outlook && m.imap_host.trim().is_empty() && crate::mailread::imap_defaults(&m.provider).is_none() {
                    out.push(format!("{at}: unknown provider `{}`; set imap_host", m.provider));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn file_trigger_runs_once_per_file_and_moves_it() {
        let dir = std::env::temp_dir().join(format!("afm_watch_{}", std::process::id()));
        let inbox = dir.join("in");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("old.pdf"), "old").unwrap();
        std::fs::write(inbox.join("a.pdf"), "A").unwrap();
        std::fs::write(inbox.join("skip.txt"), "x").unwrap();
        let t = Trigger::File(FileTrigger {
            folder: "in".into(),
            pattern: "*.pdf".into(),
            move_to: "done".into(),
            move_failed_to: "failed".into(),
            settle_ms: 0,
            ..FileTrigger::default()
        });
        let opts = WatchOptions { once: true, stop: Arc::new(AtomicBool::new(false)), poll: Duration::from_millis(50) };
        let mut got = vec![];
        watch(&[t], &dir, &opts, &mut |_| {}, &mut |e| {
            let name = e.value.as_str().unwrap().rsplit(['\\', '/']).next().unwrap().to_string();
            got.push(name.clone());
            name != "old.pdf" // pretend the run for old.pdf fails
        })
        .unwrap();
        assert_eq!(got, ["a.pdf", "old.pdf"]);
        assert!(inbox.join("done").join("a.pdf").exists());
        assert!(inbox.join("failed").join("old.pdf").exists());
        assert!(inbox.join("skip.txt").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn continuous_watch_ignores_existing_and_waits_for_new() {
        let dir = std::env::temp_dir().join(format!("afm_watch2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("existing.csv"), "1").unwrap();
        let t = Trigger::File(FileTrigger { folder: ".".into(), pattern: "*.csv".into(), settle_ms: 200, ..FileTrigger::default() });
        let stop = Arc::new(AtomicBool::new(false));
        let opts = WatchOptions { once: false, stop: stop.clone(), poll: Duration::from_millis(100) };
        let d2 = dir.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::write(d2.join("new.csv"), "2").unwrap();
        });
        let mut got = vec![];
        let started = Instant::now();
        watch(&[t], &dir, &opts, &mut |_| {}, &mut |e| {
            got.push(e.value.as_str().unwrap().to_string());
            stop.store(true, Ordering::Relaxed);
            true
        })
        .unwrap();
        writer.join().unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].ends_with("new.csv"), "{got:?}");
        assert!(started.elapsed() >= Duration::from_millis(450), "waited for the file to settle");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn checks() {
        let bad = [Trigger::File(FileTrigger::default()), Trigger::Email(EmailTrigger { provider: "nope".into(), ..EmailTrigger::default() })];
        let msgs = check(&bad).join("\n");
        assert!(msgs.contains("needs a folder"), "{msgs}");
        assert!(msgs.contains("needs username"), "{msgs}");
        assert!(msgs.contains("unknown provider"), "{msgs}");
        assert!(check(&[Trigger::Email(EmailTrigger { source: "outlook".into(), ..EmailTrigger::default() })]).is_empty());
    }
}
