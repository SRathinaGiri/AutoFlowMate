//! Sending e-mail: SMTP (Gmail, Microsoft 365, any server) and the Outlook
//! desktop app. Passwords live in Windows Credential Manager (`secrets`), never
//! in flow files.

use anyhow::{anyhow, bail, Context as _, Result};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Passwords and tokens kept in Windows Credential Manager under "AutoFlowMate".
pub mod secrets {
    use anyhow::{anyhow, Result};

    const SERVICE: &str = "AutoFlowMate";

    fn entry(name: &str) -> Result<keyring::Entry> {
        let n = name.trim();
        if n.is_empty() {
            return Err(anyhow!("secret name is empty"));
        }
        keyring::Entry::new(SERVICE, n).map_err(|e| anyhow!("credential store: {e}"))
    }

    pub fn set(name: &str, value: &str) -> Result<()> {
        entry(name)?.set_password(value).map_err(|e| anyhow!("cannot store secret `{name}`: {e}"))
    }

    pub fn get(name: &str) -> Result<String> {
        entry(name)?.get_password().map_err(|e| match e {
            keyring::Error::NoEntry => anyhow!("no password stored under `{name}` - run `autoflow secret set {name}` (or use the 🔑 button in Studio)"),
            other => anyhow!("cannot read secret `{name}`: {other}"),
        })
    }

    pub fn exists(name: &str) -> bool {
        entry(name).and_then(|e| e.get_password().map_err(|x| anyhow!("{x}"))).is_ok()
    }

    pub fn delete(name: &str) -> Result<()> {
        entry(name)?.delete_credential().map_err(|e| anyhow!("cannot delete secret `{name}`: {e}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Plain connection upgraded with STARTTLS (port 587).
    StartTls,
    /// TLS from the first byte (port 465).
    Tls,
    /// No encryption (internal relays / local test servers only).
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub password: String,
    pub timeout: Duration,
}

/// SMTP host, port and security for a mail provider preset (`gmail`, `outlook365`...;
/// Studio: File > Settings > E-mail providers).
pub fn provider_defaults(provider: &str) -> Option<(String, u16, Security)> {
    provider_defaults_in(&crate::presets::Presets::load(), provider)
}

pub fn provider_defaults_in(presets: &crate::presets::Presets, provider: &str) -> Option<(String, u16, Security)> {
    let p = presets.find_mail(provider).filter(|p| !p.smtp_host.trim().is_empty())?;
    let sec = match p.smtp_security.trim().to_ascii_lowercase().as_str() {
        "tls" | "ssl" => Security::Tls,
        "none" | "plain" => Security::None,
        _ => Security::StartTls,
    };
    let port = if p.smtp_port != 0 {
        p.smtp_port
    } else {
        match sec {
            Security::Tls => 465,
            Security::None => 25,
            Security::StartTls => 587,
        }
    };
    Some((p.smtp_host.trim().to_string(), port, sec))
}

#[derive(Debug, Clone, Default)]
pub struct Mail {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub html: bool,
    pub attachments: Vec<PathBuf>,
}

/// "a@x.com; b@y.com, Name <c@z.com>" -> list.
pub fn split_addresses(s: &str) -> Vec<String> {
    s.split([',', ';', '\n']).map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect()
}

fn mailbox(a: &str) -> Result<Mailbox> {
    a.parse::<Mailbox>().map_err(|e| anyhow!("bad e-mail address `{a}`: {e}"))
}

fn content_type(p: &Path) -> ContentType {
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let t = match ext.as_str() {
        "pdf" => "application/pdf",
        "xml" => "application/xml",
        "json" => "application/json",
        "csv" => "text/csv",
        "txt" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "zip" => "application/zip",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    };
    ContentType::parse(t).unwrap_or(ContentType::TEXT_PLAIN)
}

/// Build the MIME message (also used by tests).
pub fn build_message(m: &Mail) -> Result<Message> {
    if m.to.is_empty() && m.cc.is_empty() && m.bcc.is_empty() {
        bail!("no recipients");
    }
    let mut b = Message::builder().from(mailbox(&m.from)?).subject(m.subject.clone());
    for a in &m.to {
        b = b.to(mailbox(a)?);
    }
    for a in &m.cc {
        b = b.cc(mailbox(a)?);
    }
    for a in &m.bcc {
        b = b.bcc(mailbox(a)?);
    }
    let body = if m.html { SinglePart::html(m.body.clone()) } else { SinglePart::plain(m.body.clone()) };
    let msg = if m.attachments.is_empty() {
        b.singlepart(body)
    } else {
        let mut mp = MultiPart::mixed().singlepart(body);
        for p in &m.attachments {
            let bytes = std::fs::read(p).with_context(|| format!("cannot read attachment {}", p.display()))?;
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "attachment".into());
            mp = mp.singlepart(Attachment::new(name).body(bytes, content_type(p)));
        }
        b.multipart(mp)
    };
    msg.map_err(|e| anyhow!("cannot build the e-mail: {e}"))
}

/// Send through an SMTP server. Returns the server's reply.
pub fn send_smtp(s: &SmtpSettings, m: &Mail) -> Result<String> {
    let msg = build_message(m)?;
    let builder = match s.security {
        Security::StartTls => SmtpTransport::starttls_relay(&s.host).map_err(|e| anyhow!("SMTP {}: {e}", s.host))?,
        Security::Tls => SmtpTransport::relay(&s.host).map_err(|e| anyhow!("SMTP {}: {e}", s.host))?,
        Security::None => SmtpTransport::builder_dangerous(&s.host),
    };
    let mut builder = builder.port(s.port).timeout(Some(s.timeout));
    if !s.username.trim().is_empty() {
        builder = builder.credentials(Credentials::new(s.username.clone(), s.password.clone()));
    }
    let resp = builder.build().send(&msg).map_err(|e| {
        let text = e.to_string();
        let hint = if text.contains("535") || text.to_lowercase().contains("authentication") {
            " (check the user name and the stored password; Gmail needs an App Password, see SKILL/README)"
        } else {
            ""
        };
        anyhow!("sending via {}:{} failed: {text}{hint}", s.host, s.port)
    })?;
    Ok(resp.message().collect::<Vec<_>>().join(" "))
}

const OUTLOOK_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$ol = New-Object -ComObject Outlook.Application
$m = $ol.CreateItem(0)
$m.To = $env:AF_TO
if ($env:AF_CC) { $m.CC = $env:AF_CC }
if ($env:AF_BCC) { $m.BCC = $env:AF_BCC }
$m.Subject = $env:AF_SUBJECT
if ($env:AF_HTML -eq '1') { $m.HTMLBody = $env:AF_BODY } else { $m.Body = $env:AF_BODY }
foreach ($a in ($env:AF_ATTACH -split "`n")) { if ($a.Trim()) { [void]$m.Attachments.Add($a.Trim()) } }
if ($env:AF_ACCOUNT) {
  $acc = $null
  foreach ($x in $ol.Session.Accounts) { if ($x.SmtpAddress -eq $env:AF_ACCOUNT) { $acc = $x } }
  if (-not $acc) { throw "Outlook has no account $($env:AF_ACCOUNT)" }
  $m.SendUsingAccount = $acc
}
if ($env:AF_SEND -eq '1') { $m.Send(); [Console]::Out.Write('sent') } else { $m.Display(); [Console]::Out.Write('draft') }
"#;

/// Create the mail in the Outlook desktop app (classic Outlook) and send it,
/// or show it as a draft. Uses the account(s) already signed in to Outlook.
pub fn outlook_send(m: &Mail, send_now: bool, account: &str) -> Result<String> {
    for p in &m.attachments {
        if !p.exists() {
            bail!("attachment not found: {}", p.display());
        }
    }
    let attach: Vec<String> = m.attachments.iter().map(|p| p.display().to_string()).collect();
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", OUTLOOK_SCRIPT])
        .env("AF_TO", m.to.join("; "))
        .env("AF_CC", m.cc.join("; "))
        .env("AF_BCC", m.bcc.join("; "))
        .env("AF_SUBJECT", &m.subject)
        .env("AF_BODY", &m.body)
        .env("AF_HTML", if m.html { "1" } else { "0" })
        .env("AF_ATTACH", attach.join("\n"))
        .env("AF_SEND", if send_now { "1" } else { "0" })
        .env("AF_ACCOUNT", account.trim())
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("cannot start PowerShell")?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        bail!("Outlook could not create the mail: {first} (classic Outlook must be installed; the new Outlook app cannot be automated)");
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_providers() {
        assert_eq!(split_addresses("a@x.com; b@y.com,  ,c@z.com"), ["a@x.com", "b@y.com", "c@z.com"]);
        let p = crate::presets::Presets::builtin();
        assert_eq!(provider_defaults_in(&p, "Gmail"), Some(("smtp.gmail.com".to_string(), 587, Security::StartTls)));
        assert!(provider_defaults_in(&p, "custom").is_none());
    }

    #[test]
    fn builds_message_with_attachment() {
        let dir = std::env::temp_dir().join(format!("afm_mail_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("BalanceSheet.xml");
        std::fs::write(&f, "<ENVELOPE/>").unwrap();
        let m = Mail {
            from: "AutoFlowMate <bot@example.com>".into(),
            to: vec!["boss@example.com".into()],
            subject: "Balance Sheet".into(),
            body: "Attached.".into(),
            attachments: vec![f],
            ..Mail::default()
        };
        let raw = String::from_utf8(build_message(&m).unwrap().formatted()).unwrap();
        assert!(raw.contains("Subject: Balance Sheet"));
        assert!(raw.contains("BalanceSheet.xml"));
        assert!(raw.contains("application/xml"));
        assert!(build_message(&Mail { to: vec!["not an address".into()], ..m.clone() }).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
