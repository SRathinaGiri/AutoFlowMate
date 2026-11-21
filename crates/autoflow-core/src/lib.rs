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
pub use engine::{run_flow, Debugger, Event, RunOptions, RunResult};
pub use flow::{Flow, OnError, Step};
pub use validate::{validate_json, Issue, Severity};
