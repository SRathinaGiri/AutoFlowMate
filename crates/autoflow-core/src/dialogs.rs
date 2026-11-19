//! Simple user dialogs (PyAutoGUI alert / confirm / prompt / password).

use anyhow::{bail, Result};

use crate::actions::{DialogButtons, DialogIcon};

/// Show a message box; returns the clicked button: ok, cancel, yes, no, retry.
#[cfg(windows)]
pub fn message_box(title: &str, text: &str, buttons: DialogButtons, icon: DialogIcon) -> Result<String> {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let b = match buttons {
        DialogButtons::Ok => 0x0,
        DialogButtons::OkCancel => 0x1,
        DialogButtons::YesNoCancel => 0x3,
        DialogButtons::YesNo => 0x4,
        DialogButtons::RetryCancel => 0x5,
    };
    let i = match icon {
        DialogIcon::Error => 0x10,
        DialogIcon::Question => 0x20,
        DialogIcon::Warning => 0x30,
        DialogIcon::Info => 0x40,
        DialogIcon::None => 0,
    };
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const MB_TOPMOST: u32 = 0x4_0000;
    let (t, c) = (wide(text), wide(title));
    let r = unsafe { MessageBoxW(0, t.as_ptr(), c.as_ptr(), b | i | MB_SETFOREGROUND | MB_TOPMOST) };
    Ok(match r {
        1 => "ok",
        2 => "cancel",
        4 => "retry",
        6 => "yes",
        7 => "no",
        0 => bail!("message box could not be shown"),
        _ => "cancel",
    }
    .into())
}

#[cfg(not(windows))]
pub fn message_box(_: &str, _: &str, _: DialogButtons, _: DialogIcon) -> Result<String> {
    bail!("message_box is only available on Windows")
}

const INPUT_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$f = New-Object Windows.Forms.Form
$f.Text = $env:AF_TITLE; $f.StartPosition = 'CenterScreen'; $f.TopMost = $true
$f.FormBorderStyle = 'FixedDialog'; $f.MaximizeBox = $false; $f.MinimizeBox = $false
$f.AutoScaleMode = 'Dpi'
# Optional picture (e.g. a CAPTCHA), shown enlarged above the text box.
$top = 10; $w = 400; $pic = $null
if ($env:AF_IMAGE -and (Test-Path $env:AF_IMAGE)) {
  $bytes = [IO.File]::ReadAllBytes($env:AF_IMAGE)
  $img = [Drawing.Image]::FromStream((New-Object IO.MemoryStream(,$bytes)))
  $scale = [Math]::Min(3.0, [Math]::Max(1.0, 360.0 / [Math]::Max(1, $img.Width)))
  $pw = [int]($img.Width * $scale); $ph = [int]($img.Height * $scale)
  $w = [Math]::Max(400, $pw + 24)
  $pic = New-Object Windows.Forms.PictureBox; $pic.Image = $img; $pic.SizeMode = 'Zoom'
  $pic.BorderStyle = 'FixedSingle'; $pic.SetBounds([int](($w - $pw) / 2), 10, $pw, $ph)
  $top = 20 + $ph
}
$f.ClientSize = New-Object Drawing.Size($w, ($top + 140))
$l = New-Object Windows.Forms.Label; $l.Text = $env:AF_PROMPT; $l.SetBounds(12, $top, ($w - 24), 44)
$t = New-Object Windows.Forms.TextBox; $t.Text = $env:AF_DEFAULT; $t.SetBounds(12, ($top + 50), ($w - 24), 24)
$t.Font = New-Object Drawing.Font('Segoe UI', 12)
if ($env:AF_PASSWORD -eq '1') { $t.UseSystemPasswordChar = $true }
$ok = New-Object Windows.Forms.Button; $ok.Text = 'OK'; $ok.SetBounds(($w - 180), ($top + 95), 80, 30); $ok.DialogResult = 'OK'
$c = New-Object Windows.Forms.Button; $c.Text = 'Cancel'; $c.SetBounds(($w - 92), ($top + 95), 80, 30); $c.DialogResult = 'Cancel'
$f.Controls.AddRange(@($l, $t, $ok, $c)); if ($pic) { $f.Controls.Add($pic) }
$f.AcceptButton = $ok; $f.CancelButton = $c
$f.Add_Shown({ $f.Activate(); $t.Focus() })
if ($f.ShowDialog() -eq 'OK') {
  [Console]::OutputEncoding = [Text.Encoding]::UTF8
  [Console]::Out.Write($t.Text); exit 0
} else { exit 1 }
"#;

/// Ask for a line of text. Returns None when cancelled.
pub fn input_box(title: &str, prompt: &str, default: &str, password: bool, image: Option<&std::path::Path>) -> Result<Option<String>> {
    if let Some(p) = image {
        if !p.exists() {
            bail!("input_box image not found: {}", p.display());
        }
    }
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-Command", INPUT_SCRIPT])
        .env("AF_TITLE", title)
        .env("AF_PROMPT", prompt)
        .env("AF_DEFAULT", default)
        .env("AF_IMAGE", image.map(|p| p.display().to_string()).unwrap_or_default())
        .env("AF_PASSWORD", if password { "1" } else { "0" })
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window; the form still shows
    }
    let out = cmd.output()?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).to_string())),
        Some(1) => Ok(None),
        _ => bail!("input box failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
    }
}
