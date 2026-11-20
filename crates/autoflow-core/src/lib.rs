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
pub mod recorder;
pub mod schedule;
pub mod textops;
#[cfg(windows)]
pub mod uia;
pub mod validate;
pub mod vars;
pub mod vision;
pub mod watch;
pub mod web;

pub use actions::{catalog, Action, ActionInfo};
pub use engine::{run_flow, Debugger, Event, RunOptions, RunResult};
pub use flow::{Flow, OnError, Step};
pub use validate::{validate_json, Issue, Severity};
