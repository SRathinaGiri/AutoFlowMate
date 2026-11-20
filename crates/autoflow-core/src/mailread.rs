//! Reading e-mail: IMAP (Gmail, Zoho, Yahoo, any IMAP server) and the
//! Outlook desktop app. Messages become records:
//! {uid, from, from_name, to, subject, date, body, attachments, attachment_names}.

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Which messages to read and what to do with them.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub folder: String,
    pub unread_only: bool,
    pub from: String,
    pub subject_contains: String,
    pub since: Option<chrono::NaiveDate>,
    pub max: usize,
    pub newest_first: bool,
    pub mark_read: bool,
    pub move_to: String,
    /// Save attachments here (None = only list their names).
    pub save_dir: Option<PathBuf>,
    /// Glob such as "*.pdf" (empty = every attachment).
    pub attachment_filter: String,
}

#[derive(Debug, Clone)]
pub struct ImapSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// IMAP server for well-known providers.
/// IMAP server for a mail provider preset (Studio: File > Settings > E-mail providers).
pub fn imap_defaults(provider: &str) -> Option<(String, u16)> {
    let presets = crate::presets::Presets::load();
    let p = presets.find_mail(provider).filter(|p| !p.imap_host.trim().is_empty())?;
    Some((p.imap_host.trim().to_string(), if p.imap_port == 0 { 993 } else { p.imap_port }))
}

fn imap_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The IMAP SEARCH criteria for a query.
pub fn search_criteria(q: &Query) -> String {
    let mut parts = vec![];
    if q.unread_only {
        parts.push("UNSEEN".to_string());
    }
    if !q.from.trim().is_empty() {
        parts.push(format!("FROM {}", imap_quote(q.from.trim())));
    }
    if !q.subject_contains.trim().is_empty() {
        parts.push(format!("SUBJECT {}", imap_quote(q.subject_contains.trim())));
    }
    if let Some(d) = q.since {
        parts.push(format!("SINCE {}", d.format("%-d-%b-%Y")));
    }
    if parts.is_empty() {
        "ALL".into()
    } else {
        parts.join(" ")
    }
}

pub fn read_imap(s: &ImapSettings, q: &Query) -> Result<Vec<Value>> {
    // TLS is required, except for a server on this PC (e.g. a local mail bridge).
    let local = matches!(s.host.trim().to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1");
    let mode = if local { imap::ConnectionMode::Auto } else { imap::ConnectionMode::AutoTls };
    let client = imap::ClientBuilder::new(s.host.as_str(), s.port).mode(mode).connect().map_err(|e| anyhow!("cannot connect to {}:{}: {e}", s.host, s.port))?;
    let mut session = client.login(&s.username, &s.password).map_err(|(e, _)| {
        anyhow!("IMAP login to {} failed: {e} (check the user name and stored password; Gmail needs IMAP enabled and an App Password)", s.host)
    })?;
    let folder = if q.folder.trim().is_empty() { "INBOX" } else { q.folder.trim() };
    session.select(folder).map_err(|e| anyhow!("cannot open folder `{folder}`: {e}"))?;
    let mut uids: Vec<u32> = session.uid_search(search_criteria(q)).context("IMAP search failed")?.into_iter().collect();
    uids.sort_unstable();
    if q.newest_first {
        uids.reverse();
    }
    uids.truncate(q.max.max(1));
    let mut out = vec![];
    for uid in &uids {
        let fetches = session.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").with_context(|| format!("fetch message {uid}"))?;
        for f in fetches.iter() {
            if let Some(body) = f.body() {
                let mut rec = parse_message(body, q)?;
                rec["uid"] = json!(f.uid.unwrap_or(*uid));
                out.push(rec);
            }
        }
    }
    if !uids.is_empty() {
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        if q.mark_read {
            session.uid_store(&set, "+FLAGS (\\Seen)").context("cannot mark messages as read")?;
        }
        if !q.move_to.trim().is_empty() {
            session.uid_mv(&set, q.move_to.trim()).map_err(|e| anyhow!("cannot move messages to `{}`: {e}", q.move_to.trim()))?;
        }
    }
    session.logout().ok();
    Ok(out)
}

/// Turn a raw RFC 5322 message into a record, saving wanted attachments.
pub fn parse_message(raw: &[u8], q: &Query) -> Result<Value> {
    let m = mail_parser::MessageParser::new().parse(raw).ok_or_else(|| anyhow!("unreadable message"))?;
    let (from, from_name) =
        m.from().and_then(|l| l.first()).map(|a| (a.address().unwrap_or("").to_string(), a.name().unwrap_or("").to_string())).unwrap_or_default();
    let to: Vec<String> = m.to().map(|l| l.mailboxes().filter_map(|a| a.address().map(str::to_string)).collect()).unwrap_or_default();
    let date = m
        .date()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(&d.to_rfc3339()).ok())
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let body = match m.body_text(0) {
        Some(t) => t.into_owned(),
        None => m.body_html(0).map(|h| mail_parser::html_to_text(&h)).unwrap_or_default(),
    };
    let filter = glob::Pattern::new(&q.attachment_filter.trim().to_lowercase()).ok().filter(|_| !q.attachment_filter.trim().is_empty());
    let (mut saved, mut names) = (vec![], vec![]);
    for part in m.attachments() {
        let name = part.attachment_name().map(str::to_string).unwrap_or_else(|| format!("attachment{}", names.len() + 1));
        if filter.as_ref().is_some_and(|f| !f.matches(&name.to_lowercase())) {
            continue;
        }
        names.push(name.clone());
        if let Some(dir) = &q.save_dir {
            let p = unique_path(dir, &safe_name(&name));
            std::fs::create_dir_all(dir)?;
            std::fs::write(&p, part.decoded()).with_context(|| format!("save {}", p.display()))?;
            saved.push(p.display().to_string());
        }
    }
    Ok(json!({
        "from": from, "from_name": from_name, "to": to.join(", "),
        "subject": m.subject().unwrap_or(""), "date": date, "body": body,
        "attachments": saved, "attachment_names": names,
    }))
}

/// A file name without characters Windows does not allow.
pub fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        "attachment".into()
    } else {
        s
    }
}

/// `dir/name`, or `dir/name (2).ext` etc. when that file already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap()
}

const OUTLOOK_READ_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ol = New-Object -ComObject Outlook.Application
$ns = $ol.GetNamespace('MAPI')
$parts = @($env:AF_FOLDER -split '[/\\]' | Where-Object { $_ })
if ($env:AF_ACCOUNT) {
  $root = $ns.Folders.Item($env:AF_ACCOUNT)
  if (-not $parts -or $parts[0] -eq 'Inbox') { $folder = $root.Store.GetDefaultFolder(6); $parts = @($parts | Select-Object -Skip 1) } else { $folder = $root }
} else {
  $folder = $ns.GetDefaultFolder(6)
  if ($parts -and $parts[0] -eq 'Inbox') { $parts = @($parts | Select-Object -Skip 1) }
}
foreach ($p in $parts) { $folder = $folder.Folders.Item($p) }
$dest = $null
if ($env:AF_MOVE) {
  $dest = $ns.GetDefaultFolder(6)
  foreach ($p in @($env:AF_MOVE -split '[/\\]' | Where-Object { $_ -and $_ -ne 'Inbox' })) { $dest = $dest.Folders.Item($p) }
}
$items = $folder.Items
$items.Sort('[ReceivedTime]', ($env:AF_NEWEST -eq '1'))
$since = $null
if ($env:AF_SINCE) { $since = [datetime]::ParseExact($env:AF_SINCE, 'yyyy-MM-dd', $null) }
$max = [int]$env:AF_MAX
$picked = @()
foreach ($it in $items) {
  if ($picked.Count -ge $max) { break }
  if ($it.Class -ne 43) { continue }
  if ($env:AF_UNREAD -eq '1' -and -not $it.UnRead) { continue }
  if ($since -and $it.ReceivedTime -lt $since) { if ($env:AF_NEWEST -eq '1') { break } else { continue } }
  $addr = $it.SenderEmailAddress
  if ($it.SenderEmailType -eq 'EX') { try { $addr = $it.Sender.GetExchangeUser().PrimarySmtpAddress } catch {} }
  if ($env:AF_FROM -and -not ("$addr $($it.SenderName)" -like "*$($env:AF_FROM)*")) { continue }
  if ($env:AF_SUBJECT -and -not ($it.Subject -like "*$($env:AF_SUBJECT)*")) { continue }
  $picked += ,@($it, $addr)
}
$out = @()
foreach ($pair in $picked) {
  $it = $pair[0]
  $saved = @(); $names = @()
  foreach ($a in $it.Attachments) {
    $n = $a.FileName
    if ($env:AF_FILTER -and -not ($n -like $env:AF_FILTER)) { continue }
    $names += $n
    if ($env:AF_SAVE) {
      [void](New-Item -ItemType Directory -Force -Path $env:AF_SAVE)
      $safe = ($n -replace '[<>:"/\\|?*]', '_')
      $target = Join-Path $env:AF_SAVE $safe
      $i = 2
      while (Test-Path -LiteralPath $target) {
        $target = Join-Path $env:AF_SAVE ("{0} ({1}){2}" -f [IO.Path]::GetFileNameWithoutExtension($safe), $i, [IO.Path]::GetExtension($safe)); $i++
      }
      $a.SaveAsFile($target); $saved += $target
    }
  }
  $out += [pscustomobject]@{
    entry_id = $it.EntryID; from = $pair[1]; from_name = $it.SenderName; to = $it.To; subject = $it.Subject
    date = $it.ReceivedTime.ToString('yyyy-MM-dd HH:mm:ss'); body = $it.Body
    attachments = @($saved); attachment_names = @($names)
  }
  if ($env:AF_MARK -eq '1') { $it.UnRead = $false; $it.Save() }
  if ($dest) { [void]$it.Move($dest) }
}
ConvertTo-Json -InputObject @($out) -Depth 4 -Compress
"#;

/// Read messages from a folder of the Outlook desktop app (classic Outlook).
pub fn read_outlook(q: &Query, account: &str) -> Result<Vec<Value>> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", OUTLOOK_READ_SCRIPT])
        .env("AF_FOLDER", q.folder.trim())
        .env("AF_ACCOUNT", account.trim())
        .env("AF_UNREAD", if q.unread_only { "1" } else { "0" })
        .env("AF_FROM", q.from.trim())
        .env("AF_SUBJECT", q.subject_contains.trim())
        .env("AF_SINCE", q.since.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default())
        .env("AF_MAX", q.max.max(1).to_string())
        .env("AF_NEWEST", if q.newest_first { "1" } else { "0" })
        .env("AF_MARK", if q.mark_read { "1" } else { "0" })
        .env("AF_MOVE", q.move_to.trim())
        .env("AF_SAVE", q.save_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
        .env("AF_FILTER", q.attachment_filter.trim())
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("cannot start PowerShell")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        bail!("reading Outlook failed: {first} (classic Outlook must be installed; check the folder name)");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return Ok(vec![]);
    }
    match serde_json::from_str::<Value>(text).with_context(|| format!("unexpected Outlook output: {}", text.chars().take(200).collect::<String>()))? {
        Value::Array(a) => Ok(a),
        other => Ok(vec![other]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria() {
        let q = Query {
            unread_only: true,
            from: "gst@gov.in".into(),
            subject_contains: "Return \"3B\"".into(),
            since: chrono::NaiveDate::from_ymd_opt(2026, 1, 5),
            ..Query::default()
        };
        assert_eq!(search_criteria(&q), r#"UNSEEN FROM "gst@gov.in" SUBJECT "Return \"3B\"" SINCE 5-Jan-2026"#);
        assert_eq!(search_criteria(&Query::default()), "ALL");
    }

    #[test]
    fn parses_and_saves_attachments() {
        let raw = b"From: GST Portal <noreply@gst.gov.in>\r\nTo: me@example.com\r\nSubject: Invoice 42\r\n\
Date: Thu, 19 Nov 2026 10:00:00 +0530\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlease find the invoice.\r\n\
--b1\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"inv:42.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n\
--b1\r\nContent-Type: text/csv\r\nContent-Disposition: attachment; filename=\"data.csv\"\r\n\r\na,b\r\n--b1--\r\n";
        let dir = std::env::temp_dir().join(format!("afm_mailread_{}", std::process::id()));
        let q = Query { save_dir: Some(dir.clone()), attachment_filter: "*.PDF".into(), ..Query::default() };
        let r = parse_message(raw, &q).unwrap();
        assert_eq!(r["from"], "noreply@gst.gov.in");
        assert_eq!(r["from_name"], "GST Portal");
        assert_eq!(r["subject"], "Invoice 42");
        assert!(r["body"].as_str().unwrap().starts_with("Please find the invoice."));
        assert_eq!(r["attachment_names"], json!(["inv:42.pdf"]));
        let saved = r["attachments"][0].as_str().unwrap();
        assert!(saved.ends_with("inv_42.pdf"));
        assert_eq!(std::fs::read(saved).unwrap(), b"%PDF-1.4\n");
        // a second save does not overwrite
        let r2 = parse_message(raw, &q).unwrap();
        assert!(r2["attachments"][0].as_str().unwrap().ends_with("inv_42 (2).pdf"));
        std::fs::remove_dir_all(dir).ok();
    }
}
