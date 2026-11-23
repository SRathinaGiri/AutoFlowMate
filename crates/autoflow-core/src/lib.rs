//! AutoFlowMate core: flow model, validation and execution engine.

pub mod actions;
pub mod ai;
pub mod data;
pub mod db;
pub mod dialogs;
pub mod documents;
pub mod engine;
pub mod failsafe;
pub mod flow;
pub mod http;
pub mod input;
pub mod mail;
pub mod mailread;
pub mod ocr;
pub mod presets;
pub mod pseudo;
pub mod recorder;
pub mod redact;
pub mod schedule;
pub mod templates;
pub mod textops;
#[cfg(windows)]
pub mod uia;
pub mod validate;
pub mod vars;
pub mod vision;
pub mod watch;
pub mod web;

pub use actions::{catalog, Action, ActionInfo};

/// Folders that hold the program: where it was started from, and, when that is a link (winget
/// starts portable apps through links in its Links folder), the folder of the real program,
/// which is where `lang\` and `templates\` are.
pub fn program_dirs() -> Vec<std::path::PathBuf> {
    let mut v = vec![];
    let Ok(exe) = std::env::current_exe() else { return v };
    if let Some(d) = exe.parent() {
        v.push(d.to_path_buf());
    }
    if let Ok(real) = std::fs::canonicalize(&exe) {
        // canonicalize gives \\?\C:\...; plain paths are friendlier in messages.
        let s = real.to_string_lossy();
        let real = match s.strip_prefix(r"\\?\") {
            Some(rest) if !rest.starts_with("UNC") => std::path::PathBuf::from(rest),
            _ => real.clone(),
        };
        if let Some(d) = real.parent() {
            if !v.iter().any(|x| x == d) {
                v.push(d.to_path_buf());
            }
        }
    }
    v
}
/// True when AutoFlowMate runs from an MSIX package (Microsoft Store). Its program folder is
/// read-only and its path changes with every update.
pub fn packaged() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
        use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
        let mut len = 0u32;
        // Without a package this fails with APPMODEL_ERROR_NO_PACKAGE; with one it asks for a buffer.
        let r = unsafe { GetCurrentPackageFullName(&mut len, None) };
        r != APPMODEL_ERROR_NO_PACKAGE
    }
    #[cfg(not(windows))]
    false
}

/// The command-line program to put in scheduled tasks: `autoflow.exe` next to the running
/// program, or, in the MSIX package, its `autoflow` alias, whose path stays the same across
/// updates.
pub fn cli_exe() -> std::path::PathBuf {
    if packaged() {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            return std::path::PathBuf::from(l).join("Microsoft").join("WindowsApps").join("autoflow.exe");
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map(|d| d.join("autoflow.exe")).unwrap_or_default()
}

/// The user's Documents folder (wherever Windows keeps it, for example in OneDrive).
pub fn documents_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Some(s) = s {
                return Some(std::path::PathBuf::from(s));
            }
        }
    }
    std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Documents"))
}

/// Copies a folder and everything in it; files already in `to` are kept.
pub fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else if !dest.exists() {
            std::fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

pub use engine::{run_flow, Debugger, Event, RunOptions, RunResult};
pub use flow::{Flow, OnError, Step};
pub use validate::{validate_json, Issue, Severity};
