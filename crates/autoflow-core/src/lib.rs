//! AutoFlowMate core: flow model, validation and execution engine.

pub mod actions;
pub mod data;
pub mod dialogs;
pub mod engine;
pub mod failsafe;
pub mod flow;
pub mod schedule;
pub mod vision;
pub mod web;
pub mod input;
pub mod validate;
pub mod vars;
#[cfg(windows)]
pub mod uia;

pub use actions::{catalog, Action, ActionInfo};
pub use engine::{run_flow, Event, RunOptions, RunResult};
pub use flow::{Flow, OnError, Step};
pub use validate::{validate_json, Issue, Severity};
