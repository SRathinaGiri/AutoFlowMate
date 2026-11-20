//! Executes flows. Runs synchronously on the calling thread and reports
//! progress through an event callback, so the CLI prints events and the GUI
//! runs the engine on a worker thread and forwards events over a channel.

use anyhow::{anyhow, bail, Context as _, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::actions::*;
use crate::flow::{Flow, OnError, Settings, Step};
use crate::input::Input;
use crate::vars::Vars;

/// Progress reported while a flow runs.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    StepStarted {
        id: String,
        action: String,
        depth: usize,
    },
    StepFinished {
        id: String,
        action: String,
        ok: bool,
        error: Option<String>,
        millis: u64,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    Finished {
        success: bool,
        message: String,
    },
    /// Waiting before step `id` (breakpoint or step-by-step); `variables` are the current values.
    Paused {
        id: String,
        action: String,
        variables: Map<String, Value>,
    },
}

/// Breakpoints and step-by-step running, shared between the run thread and the editor.
#[derive(Debug, Default)]
pub struct Debugger {
    /// Step ids to pause before.
    pub breakpoints: Mutex<std::collections::HashSet<String>>,
    /// Pause before every step.
    pub stepping: AtomicBool,
    go: Mutex<bool>,
    wake: std::sync::Condvar,
}

impl Debugger {
    pub fn new(breakpoints: impl IntoIterator<Item = String>, stepping: bool) -> Self {
        Self { breakpoints: Mutex::new(breakpoints.into_iter().collect()), stepping: AtomicBool::new(stepping), ..Default::default() }
    }

    fn should_pause(&self, id: &str) -> bool {
        self.stepping.load(Ordering::Relaxed) || self.breakpoints.lock().map(|b| b.contains(id)).unwrap_or(false)
    }

    /// Let a paused run go on: to the next step only (`step`), or until the next breakpoint.
    pub fn resume(&self, step: bool) {
        self.stepping.store(step, Ordering::Relaxed);
        *self.go.lock().unwrap() = true;
        self.wake.notify_all();
    }

    /// Block until resumed or the run is stopped.
    /// Get ready to pause. Called before the Paused event is sent, so a resume that arrives
    /// before `wait` starts is not lost.
    fn arm(&self) {
        *self.go.lock().unwrap() = false;
    }

    /// Block until resumed (since the last `arm`) or the run is stopped.
    fn wait(&self, stop: &AtomicBool) {
        let mut go = self.go.lock().unwrap();
        while !*go && !stop.load(Ordering::Relaxed) {
            go = self.wake.wait_timeout(go, Duration::from_millis(50)).unwrap().0;
        }
    }
}

/// How to run a flow. `None` fields use the flow's own `settings`.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Log side-effecting actions instead of performing them.
    pub dry_run: bool,
    /// Set to true from another thread to cancel the run.
    pub stop: Arc<AtomicBool>,
    /// Override settings.step_delay_ms.
    pub step_delay_ms: Option<u64>,
    /// Override settings.key_delay_ms.
    pub key_delay_ms: Option<u64>,
    /// Override settings.input_pause_ms.
    pub input_pause_ms: Option<u64>,
    /// Override settings.failsafe.
    pub failsafe: Option<bool>,
    /// Folder used for `${flow_dir}` and relative paths. Usually the flow file's folder.
    pub flow_dir: PathBuf,
    /// Breakpoints / step-by-step (the editor's debugger); None = run straight through.
    pub debug: Option<Arc<Debugger>>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            stop: Arc::new(AtomicBool::new(false)),
            step_delay_ms: None,
            key_delay_ms: None,
            input_pause_ms: None,
            failsafe: None,
            flow_dir: std::env::current_dir().unwrap_or_default(),
            debug: None,
        }
    }
}

impl RunOptions {
    /// The flow's settings with this run's overrides applied.
    pub fn effective(&self, s: &Settings) -> Settings {
        let mut s = s.clone();
        if let Some(v) = self.step_delay_ms {
            s.step_delay_ms = v;
        }
        if let Some(v) = self.key_delay_ms {
            s.key_delay_ms = v;
        }
        if let Some(v) = self.input_pause_ms {
            s.input_pause_ms = v;
        }
        if let Some(v) = self.failsafe {
            s.failsafe = v;
        }
        s
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub success: bool,
    pub message: String,
    pub variables: Map<String, Value>,
}

/// Run a flow to completion. `overrides` replace the flow's initial variables.
pub fn run_flow(flow: &Flow, overrides: Map<String, Value>, opts: &RunOptions, sink: &mut dyn FnMut(Event)) -> RunResult {
    let settings = opts.effective(&flow.settings);
    let reason: crate::failsafe::Reason = Arc::new(Mutex::new(None));
    let vars = Vars { map: Map::new(), flow_dir: opts.flow_dir.display().to_string() };
    let started = Instant::now();

    // Fail-safe watcher (corners + abort hotkey) for real runs.
    let guard = if opts.dry_run {
        Ok(None)
    } else {
        let hotkey = settings.abort_hotkey.clone();
        crate::failsafe::start(opts.stop.clone(), reason.clone(), settings.failsafe, &hotkey).map(Some)
    };

    let mut ctx = Context {
        vars,
        opts,
        settings,
        reason,
        sink,
        input: None,
        #[cfg(windows)]
        uia: None,
        web: None,
        dbs: Default::default(),
        secret_vars: Default::default(),
        depth: 0,
    };
    let result = guard.and_then(|g| {
        if g.is_some() {
            let s = &ctx.settings;
            let mut parts = vec![];
            if s.failsafe {
                parts.push("move the mouse into a screen corner".to_string());
            }
            if !s.abort_hotkey.trim().is_empty() {
                parts.push(format!("press {}", s.abort_hotkey));
            }
            if !parts.is_empty() {
                ctx.log(LogLevel::Info, format!("to abort: {}", parts.join(" or ")));
            }
        }
        let r = init_vars(&mut ctx.vars, &flow.variables, overrides).and_then(|_| ctx.run_steps(&flow.steps, 0));
        drop(g);
        r
    });
    // Release anything key_down/mouse_down left pressed.
    ctx.input = None;
    ctx.web = None;
    let (success, message) = match result {
        Ok(Ctl::Next) | Ok(Ctl::Break) => (true, format!("completed in {:.1}s", started.elapsed().as_secs_f64())),
        Ok(Ctl::Stop { success, message }) => (success, if message.is_empty() { "stopped".into() } else { message }),
        Err(e) => (false, format!("{e:#}")),
    };
    // Typed passwords stay usable inside the run but never leave it: events, --json output, callers.
    let secrets = ctx.secret_values();
    let message = mask_text(&message, &secrets);
    ctx.emit(Event::Finished { success, message: message.clone() });
    let variables = match mask_value(Value::Object(std::mem::take(&mut ctx.vars.map)), &secrets) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    RunResult { success, message, variables }
}

/// Text shown instead of a typed password.
pub const MASK: &str = "••••••";

fn secret_values(vars: &Map<String, Value>, names: &std::collections::HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = names.iter().filter_map(|n| vars.get(n)).filter_map(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string).collect();
    // Longest first, so a password containing another one is masked whole.
    v.sort_by_key(|s| std::cmp::Reverse(s.len()));
    v
}

fn mask_text(s: &str, secrets: &[String]) -> String {
    let mut out = s.to_string();
    for sec in secrets {
        if out.contains(sec.as_str()) {
            out = out.replace(sec.as_str(), MASK);
        }
    }
    out
}

/// Mask passwords anywhere in a value, including copies inside other variables and records.
fn mask_value(v: Value, secrets: &[String]) -> Value {
    match v {
        Value::String(s) => Value::String(mask_text(&s, secrets)),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| mask_value(x, secrets)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, mask_value(x, secrets))).collect()),
        other => other,
    }
}

fn mask_event(e: Event, secrets: &[String]) -> Event {
    match e {
        Event::Log { level, message } => Event::Log { level, message: mask_text(&message, secrets) },
        Event::StepFinished { id, action, ok, error, millis } => Event::StepFinished { id, action, ok, error: error.map(|m| mask_text(&m, secrets)), millis },
        Event::Finished { success, message } => Event::Finished { success, message: mask_text(&message, secrets) },
        Event::Paused { id, action, variables } => Event::Paused {
            id,
            action,
            variables: match mask_value(Value::Object(variables), secrets) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        },
        other => other,
    }
}

/// Set initial variables in declaration order, so later ones may reference
/// earlier ones and built-ins (`"out": "${flow_dir}\\out"`). Overrides win and
/// are resolved too.
fn init_vars(vars: &mut Vars, declared: &Map<String, Value>, overrides: Map<String, Value>) -> Result<()> {
    for (k, v) in declared {
        if overrides.contains_key(k) {
            continue;
        }
        let r = vars.resolve_value(v).with_context(|| format!("variable `{k}`"))?;
        vars.map.insert(k.clone(), r);
    }
    for (k, v) in overrides {
        let r = vars.resolve_value(&v).with_context(|| format!("variable `{k}`"))?;
        vars.map.insert(k, r);
    }
    Ok(())
}

/// Control signal returned by step execution.
enum Ctl {
    Next,
    Break,
    Stop { success: bool, message: String },
}

/// Marks errors that already went through a step's error handling, so outer
/// control-flow steps pass them through instead of retrying.
#[derive(Debug)]
struct Handled(String);
impl std::fmt::Display for Handled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Handled {}

#[derive(Debug)]
struct Cancelled(String);
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Cancelled {}

struct Context<'a> {
    vars: Vars,
    opts: &'a RunOptions,
    settings: Settings,
    reason: crate::failsafe::Reason,
    sink: &'a mut dyn FnMut(Event),
    input: Option<Input>,
    #[cfg(windows)]
    uia: Option<crate::uia::Uia>,
    web: Option<crate::web::WebSession>,
    /// Open database connections, by connection string (reused across steps).
    dbs: std::collections::HashMap<String, crate::db::Conn>,
    /// Variables holding typed passwords (masked when the debugger shows values).
    secret_vars: std::collections::HashSet<String>,
    depth: usize,
}

const MAX_SUBFLOW_DEPTH: usize = 16;

impl<'a> Context<'a> {
    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.emit(Event::Log { level, message: message.into() });
    }

    /// The values of variables that hold typed passwords.
    fn secret_values(&self) -> Vec<String> {
        secret_values(&self.vars.map, &self.secret_vars)
    }

    /// Report an event, with any typed password replaced by a mask.
    fn emit(&mut self, e: Event) {
        let secrets = self.secret_values();
        let e = if secrets.is_empty() { e } else { mask_event(e, &secrets) };
        (self.sink)(e);
    }

    fn cancelled(&self) -> bool {
        self.opts.stop.load(Ordering::Relaxed)
    }

    fn cancel_error(&self) -> anyhow::Error {
        let why = self.reason.lock().ok().and_then(|r| r.clone()).unwrap_or_else(|| "run cancelled by user".into());
        Cancelled(why).into()
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancelled() {
            return Err(self.cancel_error());
        }
        Ok(())
    }

    /// Sleep in small slices so cancellation stays responsive.
    fn sleep(&self, ms: u64) -> Result<()> {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            self.check_cancel()?;
            std::thread::sleep((end - Instant::now()).min(Duration::from_millis(20)));
        }
        self.check_cancel()
    }

    fn run_steps(&mut self, steps: &[Step], depth: usize) -> Result<Ctl> {
        for step in steps {
            match self.run_step(step, depth)? {
                Ctl::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Ctl::Next)
    }

    fn run_step(&mut self, step: &Step, depth: usize) -> Result<Ctl> {
        if !step.enabled {
            return Ok(Ctl::Next);
        }
        self.check_cancel()?;
        let id = step.id.clone();
        let action = step.action.name().to_string();
        if let Some(d) = self.opts.debug.clone() {
            if d.should_pause(&id) {
                let variables = self.vars.map.clone();
                d.arm();
                self.emit(Event::Paused { id: id.clone(), action: action.clone(), variables });
                d.wait(&self.opts.stop);
                self.check_cancel()?;
            }
        }
        self.emit(Event::StepStarted { id: id.clone(), action: action.clone(), depth });
        let dry = self.opts.dry_run;
        if step.delay_before_ms > 0 && !dry {
            self.sleep(step.delay_before_ms)?;
        }
        let start = Instant::now();
        let is_container = !step.action.children().is_empty();
        let attempts = if is_container { 1 } else { step.retries + 1 };

        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            match self.exec(&step.action, depth) {
                Ok(ctl) => break Ok(ctl),
                Err(e) if e.is::<Cancelled>() || e.is::<Handled>() => break Err(e),
                // Interrupted by the fail-safe/stop while typing, moving, waiting...
                Err(_) if self.cancelled() => break Err(self.cancel_error()),
                Err(e) if attempt < attempts => {
                    self.log(LogLevel::Warn, format!("[{id}] {action} failed (attempt {attempt}/{attempts}): {e:#}; retrying"));
                    self.sleep(step.retry_delay_ms)?;
                }
                Err(e) => break Err(e),
            }
        };
        let millis = start.elapsed().as_millis() as u64;

        match result {
            Ok(ctl) => {
                self.emit(Event::StepFinished { id, action, ok: true, error: None, millis });
                if !dry {
                    // PyAutoGUI-style PAUSE after input, then the per-step delays.
                    let mut pause = step.delay_after_ms + self.settings.step_delay_ms;
                    if step.action.uses_input() {
                        pause += self.settings.input_pause_ms;
                    }
                    if pause > 0 && !is_container {
                        self.sleep(pause)?;
                    }
                }
                Ok(ctl)
            }
            Err(e) if e.is::<Cancelled>() || e.is::<Handled>() => {
                self.emit(Event::StepFinished { id, action, ok: false, error: Some(format!("{e:#}")), millis });
                Err(e)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.emit(Event::StepFinished { id: id.clone(), action: action.clone(), ok: false, error: Some(msg.clone()), millis });
                match step.on_error {
                    OnError::Continue => {
                        self.log(LogLevel::Warn, format!("[{id}] {action} failed, continuing: {msg}"));
                        self.vars.set_str("last_error", msg).ok();
                        Ok(Ctl::Next)
                    }
                    OnError::Stop => {
                        self.log(LogLevel::Error, format!("[{id}] {action} failed: {msg}"));
                        Err(Handled(format!("step {id} ({action}) failed: {msg}")).into())
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ helpers

    fn text(&self, s: &str) -> Result<String> {
        self.vars.interpolate(s)
    }

    fn int(&self, v: &Int) -> Result<i64> {
        match v {
            Int::Num(n) => Ok(*n),
            Int::Text(s) => {
                let t = self.text(s)?;
                t.trim().parse::<f64>().map(|f| f.round() as i64).map_err(|_| anyhow!("`{t}` is not a number"))
            }
        }
    }

    fn opt_point(&self, x: &Option<Int>, y: &Option<Int>, what: &str) -> Result<Option<(i32, i32)>> {
        match (x, y) {
            (Some(x), Some(y)) => Ok(Some((self.int(x)? as i32, self.int(y)? as i32))),
            (None, None) => Ok(None),
            _ => bail!("{what} needs both x and y, or neither"),
        }
    }

    /// Interpolate a path and make it absolute relative to the flow folder.
    fn path(&self, s: &str) -> Result<PathBuf> {
        let t = self.text(s)?;
        if t.trim().is_empty() {
            bail!("path is empty");
        }
        let p = PathBuf::from(t.trim());
        Ok(if p.is_absolute() { p } else { self.opts.flow_dir.join(p) })
    }

    fn set_var(&mut self, name: &str, v: Value) -> Result<()> {
        if name.trim().is_empty() {
            return Ok(()); // output not wanted
        }
        self.vars.set(name, v)
    }

    /// The input device. The fail-safe watcher thread sets the stop flag;
    /// checking it here keeps every input action responsive to it.
    fn input(&mut self) -> Result<&mut Input> {
        self.check_cancel()?;
        if self.input.is_none() {
            self.input = Some(Input::new()?);
        }
        Ok(self.input.as_mut().unwrap())
    }

    /// Move with the step's duration, or the flow default.
    fn move_mouse(&mut self, x: i32, y: i32, duration: Option<u64>, tween: Tween) -> Result<()> {
        let ms = duration.unwrap_or(self.settings.mouse_move_ms);
        let stop = self.opts.stop.clone();
        self.input()?.move_smooth(x, y, ms, tween, &stop)
    }

    #[cfg(windows)]
    fn uia(&mut self) -> Result<&crate::uia::Uia> {
        if self.uia.is_none() {
            self.uia = Some(crate::uia::Uia::new()?);
        }
        Ok(self.uia.as_ref().unwrap())
    }

    fn eval(&self, c: &Condition) -> Result<bool> {
        let mut l = self.text(&c.left)?;
        let mut r = self.text(&c.right)?;
        if c.ignore_case {
            l = l.to_lowercase();
            r = r.to_lowercase();
        }
        let nums = (l.trim().parse::<f64>(), r.trim().parse::<f64>());
        let cmp = |ord: fn(std::cmp::Ordering) -> bool| -> bool {
            match nums {
                (Ok(a), Ok(b)) => a.partial_cmp(&b).map(ord).unwrap_or(false),
                _ => ord(l.cmp(&r)),
            }
        };
        Ok(match c.op {
            Op::Equals => match nums {
                (Ok(a), Ok(b)) => a == b,
                _ => l == r,
            },
            Op::NotEquals => match nums {
                (Ok(a), Ok(b)) => a != b,
                _ => l != r,
            },
            Op::Contains => l.contains(&r),
            Op::NotContains => !l.contains(&r),
            Op::StartsWith => l.starts_with(&r),
            Op::EndsWith => l.ends_with(&r),
            Op::GreaterThan => cmp(|o| o.is_gt()),
            Op::LessThan => cmp(|o| o.is_lt()),
            Op::GreaterOrEqual => cmp(|o| o.is_ge()),
            Op::LessOrEqual => cmp(|o| o.is_le()),
            Op::IsEmpty => l.trim().is_empty(),
            Op::NotEmpty => !l.trim().is_empty(),
            Op::IsTrue => matches!(l.trim().to_lowercase().as_str(), "true" | "yes" | "1"),
            Op::IsFalse => matches!(l.trim().to_lowercase().as_str(), "false" | "no" | "0" | ""),
            Op::PathExists => {
                let p = PathBuf::from(l.trim());
                let p = if p.is_absolute() { p } else { self.opts.flow_dir.join(p) };
                p.exists()
            }
            Op::Matches => glob::Pattern::new(&r).map_err(|e| anyhow!("bad pattern `{r}`: {e}"))?.matches(&l),
        })
    }

    fn dry(&mut self, what: String) -> bool {
        if self.opts.dry_run {
            self.log(LogLevel::Info, format!("[dry-run] would {what}"));
        }
        self.opts.dry_run
    }

    // ------------------------------------------------------------ actions

    fn exec(&mut self, action: &Action, depth: usize) -> Result<Ctl> {
        match action {
            // ---------------- flow
            Action::SetVariable(a) => {
                let v = self.vars.resolve_value(&a.value)?;
                self.vars.set(&self.text(&a.name)?, v)?;
            }
            Action::Increment(a) => {
                let name = self.text(&a.name)?;
                let cur = self.vars.get(&name).map(|v| crate::vars::to_text(&v)).unwrap_or_default();
                let cur: f64 =
                    if cur.trim().is_empty() { 0.0 } else { cur.trim().parse().map_err(|_| anyhow!("variable `{name}` is not a number: `{cur}`"))? };
                let by = self.int(&a.by)? as f64;
                let n = cur + by;
                let v = if n.fract() == 0.0 { json!(n as i64) } else { json!(n) };
                self.vars.set(&name, v)?;
            }
            Action::Log(a) => {
                let m = self.text(&a.message)?;
                self.log(a.level, m);
            }
            Action::Wait(a) => {
                let ms = self.int(&a.ms)?.max(0) as u64;
                if !self.dry(format!("wait {ms} ms")) {
                    self.sleep(ms)?;
                }
            }
            Action::If(a) => {
                let steps = if self.eval(&a.condition)? { &a.then } else { &a.otherwise };
                return self.run_steps(steps, depth + 1);
            }
            Action::Repeat(a) => {
                let n = self.int(&a.times)?;
                for i in 0..n.max(0) {
                    self.set_var(&a.index_var, json!(i))?;
                    match self.run_steps(&a.steps, depth + 1)? {
                        Ctl::Next => {}
                        Ctl::Break => break,
                        stop => return Ok(stop),
                    }
                }
            }
            Action::While(a) => {
                let mut n = 0;
                while self.eval(&a.condition)? {
                    if n >= a.max_iterations {
                        bail!("while loop reached max_iterations ({})", a.max_iterations);
                    }
                    n += 1;
                    match self.run_steps(&a.steps, depth + 1)? {
                        Ctl::Next => {}
                        Ctl::Break => break,
                        stop => return Ok(stop),
                    }
                    // Waits are skipped in a dry run, so loops that poll for real-world
                    // state (files, windows, the clock) would spin; one pass shows the plan.
                    if self.opts.dry_run {
                        self.log(LogLevel::Info, "[dry-run] while: stopping after one pass");
                        break;
                    }
                }
            }
            Action::ForEach(a) => {
                let items = match self.vars.resolve_value(&a.items)? {
                    Value::Array(v) => v,
                    Value::Null => vec![],
                    Value::String(s) => s.lines().filter(|l| !l.trim().is_empty()).map(|l| json!(l)).collect(),
                    other => vec![other],
                };
                for (i, item) in items.into_iter().enumerate() {
                    self.set_var(&a.item_var, item)?;
                    self.set_var(&a.index_var, json!(i))?;
                    match self.run_steps(&a.steps, depth + 1)? {
                        Ctl::Next => {}
                        Ctl::Break => break,
                        stop => return Ok(stop),
                    }
                }
            }
            Action::Try(a) => {
                let outcome = match self.run_steps(&a.steps, depth + 1) {
                    Ok(ctl) => Ok(ctl),
                    Err(e) if e.is::<Cancelled>() => return Err(e),
                    Err(e) => {
                        let msg = match e.downcast_ref::<Handled>() {
                            Some(Handled(m)) => m.clone(),
                            None => format!("{e:#}"),
                        };
                        self.log(LogLevel::Warn, format!("caught: {msg}"));
                        self.set_var(&a.error_var, json!(msg))?;
                        self.vars.set_str("last_error", msg).ok();
                        self.run_steps(&a.catch, depth + 1)
                    }
                };
                // `finally` runs whether the steps succeeded, failed or broke out of a loop.
                let fin = self.run_steps(&a.finally, depth + 1)?;
                let ctl = outcome?;
                return Ok(if matches!(fin, Ctl::Next) { ctl } else { fin });
            }
            Action::Break(_) => return Ok(Ctl::Break),
            Action::Stop(a) => return Ok(Ctl::Stop { success: a.success, message: self.text(&a.message)? }),
            Action::RunFlow(a) => self.run_subflow(a)?,

            // ---------------- files
            Action::FileCopy(a) => self.copy_move(a, false)?,
            Action::FileMove(a) => self.copy_move(a, true)?,
            Action::FileDelete(a) => {
                let paths = self.expand(&a.path)?;
                if paths.is_empty() && !a.missing_ok {
                    bail!("nothing to delete at {}", self.text(&a.path)?);
                }
                for p in paths {
                    if !self.dry(format!("delete {}", p.display())) {
                        std::fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
                    }
                }
            }
            Action::FileWrite(a) => {
                let p = self.path(&a.path)?;
                let content = self.text(&a.content)?;
                if !self.dry(format!("write {} chars to {}", content.len(), p.display())) {
                    if let Some(dir) = p.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    if a.append {
                        use std::io::Write;
                        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p)?;
                        f.write_all(content.as_bytes())?;
                    } else {
                        std::fs::write(&p, content).with_context(|| format!("write {}", p.display()))?;
                    }
                }
            }
            Action::FileRead(a) => {
                let p = self.path(&a.path)?;
                let bytes = std::fs::read(&p).with_context(|| format!("read {}", p.display()))?;
                self.set_var(&a.output_var, json!(String::from_utf8_lossy(&bytes)))?;
            }
            Action::FileExists(a) => {
                let exists = self.path(&a.path)?.exists();
                self.set_var(&a.output_var, json!(exists.to_string()))?;
            }
            Action::FileInfo(a) => {
                let p = self.path(&a.path)?;
                let info = file_info(&p);
                self.set_var(&a.output_var, info)?;
            }
            Action::DirCreate(a) => {
                let p = self.path(&a.path)?;
                if !self.dry(format!("create folder {}", p.display())) {
                    std::fs::create_dir_all(&p).with_context(|| format!("create {}", p.display()))?;
                }
            }
            Action::DirDelete(a) => {
                let p = self.path(&a.path)?;
                if !p.exists() {
                    if a.missing_ok {
                        return Ok(Ctl::Next);
                    }
                    bail!("folder {} does not exist", p.display());
                }
                if !self.dry(format!("delete folder {}", p.display())) {
                    if a.recursive { std::fs::remove_dir_all(&p) } else { std::fs::remove_dir(&p) }
                        .with_context(|| format!("delete folder {}", p.display()))?;
                }
            }
            Action::DirList(a) => {
                let dir = self.path(&a.path)?;
                let pat = self.text(&a.pattern)?;
                let pat = if pat.is_empty() { "*".to_string() } else { pat };
                let base = glob::Pattern::escape(&dir.display().to_string());
                let full = if a.recursive { format!("{base}/**/{pat}") } else { format!("{base}/{pat}") };
                let mut out = Vec::new();
                for entry in glob::glob(&full).map_err(|e| anyhow!("bad pattern: {e}"))?.flatten() {
                    let is_dir = entry.is_dir();
                    if (is_dir && a.include_dirs) || (!is_dir && a.include_files) {
                        out.push(entry.display().to_string());
                    }
                }
                out.sort();
                self.set_var(&a.output_var, json!(out))?;
            }

            // ---------------- data import
            Action::ReadExcel(a) => {
                let p = self.path(&a.path)?;
                let (sheet, range, column) = (self.text(&a.sheet)?, self.text(&a.range)?, self.text(&a.column)?);
                let (used, raw) = crate::data::read_excel_raw(&p, &sheet, &range)?;
                let v = crate::data::shape(raw, a.has_header, &column, a.skip_empty)?;
                let n = v.as_array().map(Vec::len).unwrap_or(0);
                self.log(LogLevel::Info, format!("read {n} {} from {} [{used}]", if column.is_empty() { "row(s)" } else { "value(s)" }, p.display()));
                self.set_var(&a.output_var, v)?;
                self.set_var(&a.count_var, json!(n))?;
            }
            Action::ReadCsv(a) => {
                let p = self.path(&a.path)?;
                let (delim, column) = (self.text(&a.delimiter)?, self.text(&a.column)?);
                let raw = crate::data::read_csv_raw(&p, &delim)?;
                let v = crate::data::shape(raw, a.has_header, &column, a.skip_empty)?;
                let n = v.as_array().map(Vec::len).unwrap_or(0);
                self.log(LogLevel::Info, format!("read {n} {} from {}", if column.is_empty() { "row(s)" } else { "value(s)" }, p.display()));
                self.set_var(&a.output_var, v)?;
                self.set_var(&a.count_var, json!(n))?;
            }
            Action::ReadPdf(a) => {
                let p = self.path(&a.path)?;
                let (text, pages) = crate::data::read_pdf(&p, &self.text(&a.pages)?, a.stream_order)?;
                self.log(LogLevel::Info, format!("read {} characters from {} ({pages} pages)", text.chars().count(), p.display()));
                self.set_var(&a.output_var, json!(text))?;
                self.set_var(&a.page_count_var, json!(pages))?;
            }
            Action::ListAppend(a) => {
                let name = self.text(&a.name)?;
                if name.trim().is_empty() {
                    bail!("list_append needs a list variable name");
                }
                let v = self.vars.resolve_value(&a.value)?;
                let mut list = match self.vars.get(&name) {
                    None | Some(Value::Null) => vec![],
                    Some(Value::String(s)) if s.is_empty() => vec![],
                    Some(Value::Array(l)) => l,
                    Some(other) => bail!("variable `{name}` is not a list (it holds {})", crate::vars::to_text(&other)),
                };
                list.push(v);
                self.vars.set(&name, Value::Array(list))?;
            }
            Action::WriteCsv(a) => {
                let p = self.path(&a.path)?;
                let data = self.vars.resolve_value(&a.data)?;
                let cols: Vec<String> = a.columns.iter().map(|c| self.text(c)).collect::<Result<_>>()?;
                let table = crate::data::to_table(&data, &cols)?;
                let what = format!("{} {} row(s) to {}", if a.append { "append" } else { "write" }, table.rows.len(), p.display());
                if !self.dry(what.clone()) {
                    let n = crate::data::write_csv(&p, &table, &self.text(&a.delimiter)?, a.header, a.append, a.excel_bom, a.escape_formulas)?;
                    self.log(LogLevel::Info, format!("{} {n} row(s) to {}", if a.append { "appended" } else { "wrote" }, p.display()));
                }
                self.set_var(&a.count_var, json!(table.rows.len()))?;
            }
            Action::WriteExcel(a) => {
                let p = self.path(&a.path)?;
                let data = self.vars.resolve_value(&a.data)?;
                let cols: Vec<String> = a.columns.iter().map(|c| self.text(c)).collect::<Result<_>>()?;
                let table = crate::data::to_table(&data, &cols)?;
                let sheet = self.text(&a.sheet)?;
                if !self.dry(format!("write {} row(s) to {} [{sheet}]", table.rows.len(), p.display())) {
                    let n = crate::data::write_excel(&p, &sheet, &table, a.header, a.autofit, a.allow_formulas)?;
                    self.log(LogLevel::Info, format!("wrote {n} row(s) to {} [{sheet}]", p.display()));
                }
                self.set_var(&a.count_var, json!(table.rows.len()))?;
            }

            // ---------------- documents
            Action::ReadPdfTable(a) => {
                let p = self.path(&a.path)?;
                let pages = crate::documents::parse_pages(&self.text(&a.pages)?)?;
                let raw = crate::documents::pdf_table(&p, &pages, a.gap, a.min_columns as usize)?;
                let v = crate::data::shape(raw, a.has_header, &self.text(&a.column)?, true)?;
                let n = v.as_array().map(Vec::len).unwrap_or(0);
                self.log(LogLevel::Info, format!("read {n} table row(s) from {}", p.display()));
                self.set_var(&a.count_var, json!(n))?;
                self.set_var(&a.output_var, v)?;
            }
            Action::WordRead(a) => {
                let p = self.path(&a.path)?;
                let (text, tables) = crate::documents::word_read(&p)?;
                let mut shaped = vec![];
                for t in tables {
                    let rows = t.into_iter().map(|r| r.into_iter().map(Value::String).collect()).collect();
                    shaped.push(crate::data::shape(rows, a.has_header, "", true)?);
                }
                self.log(LogLevel::Info, format!("read {} characters and {} table(s) from {}", text.chars().count(), shaped.len(), p.display()));
                self.set_var(&a.tables_var, Value::Array(shaped))?;
                self.set_var(&a.output_var, json!(text))?;
            }
            Action::WordFill(a) => {
                let (tpl, out) = (self.path(&a.template)?, self.path(&a.path)?);
                if tpl == out {
                    bail!("path must differ from the template, so the template stays reusable");
                }
                let mut values = Map::new();
                for (k, v) in &a.values {
                    values.insert(k.clone(), self.vars.resolve_value(v)?);
                }
                if !self.dry(format!("fill {} into {}", tpl.display(), out.display())) {
                    let missing = crate::documents::word_fill(&tpl, &out, &values)?;
                    if !missing.is_empty() {
                        let list = missing.iter().map(|m| format!("{{{{{m}}}}}")).collect::<Vec<_>>().join(", ");
                        if a.missing_ok {
                            self.log(LogLevel::Warn, format!("no value for {list}; left as is"));
                        } else {
                            std::fs::remove_file(&out).ok();
                            bail!("no value for {list} (add them to values, or set missing_ok)");
                        }
                    }
                    self.log(LogLevel::Info, format!("wrote {}", out.display()));
                }
            }
            Action::WordWrite(a) => {
                let p = self.path(&a.path)?;
                let data = self.vars.resolve_value(&a.data)?;
                let table = match &data {
                    Value::Null => None,
                    Value::String(s) if s.trim().is_empty() => None,
                    d => {
                        let cols: Vec<String> = a.columns.iter().map(|c| self.text(c)).collect::<Result<_>>()?;
                        Some(crate::data::to_table(d, &cols)?)
                    }
                };
                let (title, text) = (self.text(&a.title)?, self.text(&a.text)?);
                if !self.dry(format!("write {}", p.display())) {
                    crate::documents::word_write(&p, &title, &text, table.as_ref())?;
                    self.log(LogLevel::Info, format!("wrote {}", p.display()));
                }
            }
            Action::ZipCreate(a) => {
                let zip = self.path(&a.path)?;
                let mut sources = vec![];
                for s in &a.sources {
                    let t = self.text(s)?;
                    if t.trim().is_empty() {
                        continue;
                    }
                    if t.contains('*') || t.contains('?') {
                        let found = self.expand(s)?;
                        if found.is_empty() {
                            bail!("nothing matches {t}");
                        }
                        sources.extend(found);
                    } else {
                        sources.push(self.path(s)?);
                    }
                }
                if !self.dry(format!("zip {} item(s) into {}", sources.len(), zip.display())) {
                    let n = crate::documents::zip_create(&zip, &sources)?;
                    self.log(LogLevel::Info, format!("zipped {n} file(s) into {}", zip.display()));
                    self.set_var(&a.count_var, json!(n))?;
                }
            }
            Action::ZipExtract(a) => {
                let zip = self.path(&a.path)?;
                let dest = match self.text(&a.destination)? {
                    d if d.trim().is_empty() => zip.with_extension(""),
                    _ => self.path(&a.destination)?,
                };
                if !self.dry(format!("unzip {} into {}", zip.display(), dest.display())) {
                    let files = crate::documents::zip_extract(&zip, &dest, &self.text(&a.pattern)?, a.overwrite)?;
                    self.log(LogLevel::Info, format!("unzipped {} file(s) into {}", files.len(), dest.display()));
                    self.set_var(&a.output_var, json!(files))?;
                }
            }

            // ---------------- database
            Action::DbQuery(a) => {
                let sql = self.text(&a.sql)?;
                let params = self.db_params(&a.params)?;
                // A dry run reads existing databases but never creates a SQLite file.
                if self.opts.dry_run {
                    let conn = self.text(&a.db.connection)?;
                    if let Some(f) = crate::db::sqlite_file(&conn, &self.opts.flow_dir) {
                        if f != ":memory:" && !Path::new(&f).exists() {
                            let short: String = sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
                            self.dry(format!("query {f} (not created yet): {short}"));
                            self.set_var(&a.count_var, json!(0))?;
                            self.set_var(&a.output_var, json!([]))?;
                            return Ok(Ctl::Next);
                        }
                    }
                }
                let conn = self.db(&a.db)?;
                let rows = crate::db::query(conn, &sql, &params, a.max_rows as usize)?;
                self.log(LogLevel::Info, format!("query returned {} row(s)", rows.len()));
                self.set_var(&a.count_var, json!(rows.len()))?;
                self.set_var(&a.output_var, Value::Array(rows))?;
            }
            Action::DbExecute(a) => {
                let sql = self.text(&a.sql)?;
                let params = self.db_params(&a.params)?;
                let short: String = sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
                if !self.dry(format!("run SQL: {short}")) {
                    let conn = self.db(&a.db)?;
                    let n = crate::db::execute(conn, &sql, &params)?;
                    self.log(LogLevel::Info, format!("{n} row(s) changed: {short}"));
                    self.set_var(&a.affected_var, json!(n))?;
                }
            }

            // ---------------- internet
            Action::HttpRequest(a) => self.http_request(a)?,

            // ---------------- AI
            Action::AiPrompt(a) => self.ai_prompt(a)?,

            // ---------------- text
            Action::TextTransform(a) => {
                use crate::textops::*;
                let s = self.text(&a.input)?;
                let v = match a.operation {
                    TextOperation::Trim => json!(s.trim()),
                    TextOperation::Upper => json!(s.to_uppercase()),
                    TextOperation::Lower => json!(s.to_lowercase()),
                    TextOperation::Title => json!(title_case(&s)),
                    TextOperation::Length => json!(s.chars().count()),
                    TextOperation::UrlEncode => json!(url_encode(&s)),
                    TextOperation::RemoveSpaces => json!(s.chars().filter(|c| !c.is_whitespace()).collect::<String>()),
                };
                self.set_var(&a.output_var, v)?;
            }
            Action::TextReplace(a) => {
                let s = self.text(&a.input)?;
                let find = self.text(&a.find)?;
                let with = self.text(&a.replace_with)?;
                let out = if find.is_empty() {
                    s
                } else if a.regex || a.ignore_case {
                    let pat = if a.regex { find } else { regex::escape(&find) };
                    let re = regex::RegexBuilder::new(&pat).case_insensitive(a.ignore_case).build().map_err(|e| anyhow!("bad pattern: {e}"))?;
                    let with = if a.regex { with } else { with.replace('$', "$$") };
                    re.replace_all(&s, with.as_str()).into_owned()
                } else {
                    s.replace(&find, &with)
                };
                self.set_var(&a.output_var, json!(out))?;
            }
            Action::TextExtract(a) => {
                let s = self.text(&a.input)?;
                let pat = self.text(&a.pattern)?;
                let re = regex::RegexBuilder::new(&pat).case_insensitive(a.ignore_case).build().map_err(|e| anyhow!("bad pattern `{pat}`: {e}"))?;
                let g = a.group as usize;
                if g >= re.captures_len() {
                    bail!("pattern `{pat}` has no group {g} (it has {})", re.captures_len() - 1);
                }
                let pick = |c: regex::Captures| c.get(g).map(|m| m.as_str().to_string()).unwrap_or_default();
                let (v, found) = if a.all {
                    let all: Vec<String> = re.captures_iter(&s).map(pick).collect();
                    let f = !all.is_empty();
                    (json!(all), f)
                } else {
                    match re.captures(&s) {
                        Some(c) => (json!(pick(c)), true),
                        None => (json!(""), false),
                    }
                };
                self.set_var(&a.output_var, v)?;
                self.set_var(&a.found_var, json!(found.to_string()))?;
            }
            Action::TextSplit(a) => {
                let s = self.text(&a.input)?;
                let sep = unescape(&self.text(&a.separator)?);
                let mut parts: Vec<String> = if sep.is_empty() {
                    s.chars().map(String::from).collect()
                } else if a.regex {
                    regex::Regex::new(&sep).map_err(|e| anyhow!("bad pattern: {e}"))?.split(&s).map(str::to_string).collect()
                } else if sep == "\n" {
                    s.lines().map(str::to_string).collect()
                } else {
                    s.split(sep.as_str()).map(str::to_string).collect()
                };
                if a.trim {
                    parts = parts.into_iter().map(|p| p.trim().to_string()).collect();
                }
                if a.skip_empty {
                    parts.retain(|p| !p.trim().is_empty());
                }
                self.set_var(&a.output_var, json!(parts))?;
            }
            Action::TextJoin(a) => {
                let items = match self.vars.resolve_value(&a.items)? {
                    Value::Array(v) => v,
                    Value::Null => vec![],
                    other => vec![other],
                };
                let sep = unescape(&self.text(&a.separator)?);
                let s = items.iter().map(crate::vars::to_text).collect::<Vec<_>>().join(&sep);
                self.set_var(&a.output_var, json!(s))?;
            }

            // ---------------- dates and maths
            Action::DateCalc(a) => {
                use crate::textops::*;
                let fmt_in = self.text(&a.input_format)?;
                let d = parse_date(&self.text(&a.date)?, &fmt_in)?;
                let fmt = self.text(&a.format)?;
                let v = match a.operation {
                    DateOperation::Format => json!(format_date(&d, &fmt)),
                    DateOperation::Add => json!(format_date(&add(d, self.int(&a.amount)?, &self.text(&a.unit)?)?, &fmt)),
                    DateOperation::Diff => json!(diff(d, parse_date(&self.text(&a.other_date)?, &fmt_in)?, &self.text(&a.unit)?)?),
                    DateOperation::Parts => parts(&d),
                };
                self.set_var(&a.output_var, v)?;
            }
            Action::Calculate(a) => {
                let e = self.text(&a.expression)?;
                let n = crate::textops::calculate(&e)?;
                self.set_var(&a.output_var, crate::textops::number_value(n, a.decimals))?;
            }

            // ---------------- email
            Action::SendEmail(a) => {
                let (to, cc, bcc) = (self.text(&a.to)?, self.text(&a.cc)?, self.text(&a.bcc)?);
                let username = self.text(&a.username)?;
                let from = match self.text(&a.from)? {
                    f if f.trim().is_empty() => username.clone(),
                    f => f,
                };
                let attachments: Vec<PathBuf> = a.attachments.iter().filter(|s| !s.trim().is_empty()).map(|s| self.path(s)).collect::<Result<_>>()?;
                for p in &attachments {
                    if !p.exists() {
                        bail!("attachment not found: {}", p.display());
                    }
                }
                let mail = crate::mail::Mail {
                    from,
                    to: crate::mail::split_addresses(&to),
                    cc: crate::mail::split_addresses(&cc),
                    bcc: crate::mail::split_addresses(&bcc),
                    subject: self.text(&a.subject)?,
                    body: self.text(&a.body)?,
                    html: a.html,
                    attachments,
                };
                let provider = self.text(&a.provider)?;
                let (host, port, security) = match crate::mail::provider_defaults(&provider) {
                    Some((h, p, s)) if a.smtp_host.trim().is_empty() => (h.to_string(), if a.smtp_port == 0 { p } else { a.smtp_port }, s),
                    _ => {
                        let sec = match a.security.trim().to_ascii_lowercase().as_str() {
                            "tls" | "ssl" => crate::mail::Security::Tls,
                            "none" | "plain" => crate::mail::Security::None,
                            _ => crate::mail::Security::StartTls,
                        };
                        let default_port = match sec {
                            crate::mail::Security::Tls => 465,
                            crate::mail::Security::None => 25,
                            crate::mail::Security::StartTls => 587,
                        };
                        let host = self.text(&a.smtp_host)?;
                        if host.trim().is_empty() {
                            bail!("set smtp_host, or a provider from File > Settings > E-mail providers (gmail, outlook365...)");
                        }
                        (host, if a.smtp_port == 0 { default_port } else { a.smtp_port }, sec)
                    }
                };
                let what = format!("e-mail \"{}\" to {} with {} attachment(s) via {host}:{port}", mail.subject, mail.to.join(", "), mail.attachments.len());
                if !self.dry(format!("send {what}")) {
                    // The password is read from Windows Credential Manager only now, and never logged.
                    let password = if username.trim().is_empty() { String::new() } else { crate::mail::secrets::get(&self.text(&a.password_secret)?)? };
                    let settings =
                        crate::mail::SmtpSettings { host, port, security, username, password, timeout: Duration::from_millis(a.timeout_ms.max(5_000)) };
                    let reply = crate::mail::send_smtp(&settings, &mail)?;
                    self.log(LogLevel::Info, format!("sent {what} ({reply})"));
                }
            }
            Action::ReadEmail(a) => {
                let q = self.mail_query(&a.filter)?;
                let provider = self.text(&a.provider)?;
                let (host, port) = match (self.text(&a.imap_host)?, crate::mailread::imap_defaults(&provider)) {
                    (h, _) if !h.trim().is_empty() => (h.trim().to_string(), if a.imap_port == 0 { 993 } else { a.imap_port }),
                    (_, Some((h, p))) => (h.to_string(), if a.imap_port == 0 { p } else { a.imap_port }),
                    _ => bail!("set imap_host, or a provider from File > Settings > E-mail providers (gmail, outlook365...)"),
                };
                let username = self.text(&a.username)?;
                let password = crate::mail::secrets::get(&self.text(&a.password_secret)?)?;
                let settings = crate::mailread::ImapSettings { host: host.clone(), port, username: username.clone(), password };
                let mails = crate::mailread::read_imap(&settings, &q)?;
                self.log(LogLevel::Info, format!("read {} message(s) from {username} / {}", mails.len(), q.folder));
                self.mail_dry_notes(&a.filter)?;
                self.set_var(&a.filter.count_var, json!(mails.len()))?;
                self.set_var(&a.filter.output_var, Value::Array(mails))?;
            }
            Action::OutlookRead(a) => {
                let q = self.mail_query(&a.filter)?;
                let mails = crate::mailread::read_outlook(&q, &self.text(&a.account)?)?;
                self.log(LogLevel::Info, format!("read {} message(s) from Outlook {}", mails.len(), q.folder));
                self.mail_dry_notes(&a.filter)?;
                self.set_var(&a.filter.count_var, json!(mails.len()))?;
                self.set_var(&a.filter.output_var, Value::Array(mails))?;
            }
            Action::OutlookSend(a) => {
                let attachments: Vec<PathBuf> = a.attachments.iter().filter(|s| !s.trim().is_empty()).map(|s| self.path(s)).collect::<Result<_>>()?;
                let mail = crate::mail::Mail {
                    from: String::new(),
                    to: crate::mail::split_addresses(&self.text(&a.to)?),
                    cc: crate::mail::split_addresses(&self.text(&a.cc)?),
                    bcc: crate::mail::split_addresses(&self.text(&a.bcc)?),
                    subject: self.text(&a.subject)?,
                    body: self.text(&a.body)?,
                    html: a.html,
                    attachments,
                };
                let what = format!("Outlook e-mail \"{}\" to {} with {} attachment(s)", mail.subject, mail.to.join(", "), mail.attachments.len());
                if !self.dry(format!("{} {what}", if a.send_now { "send" } else { "open a draft of" })) {
                    let r = crate::mail::outlook_send(&mail, a.send_now, &self.text(&a.account)?)?;
                    self.log(LogLevel::Info, format!("{} {what}", if r == "sent" { "sent" } else { "opened draft:" }));
                }
            }

            // ---------------- programs
            Action::RunProgram(a) => self.run_program(a)?,
            Action::KillProgram(a) => {
                let name = self.text(&a.name)?;
                if !self.dry(format!("kill {name}")) {
                    let out = hidden(Command::new("taskkill").args(["/IM", &name, "/F"])).output()?;
                    if !out.status.success() && !a.missing_ok {
                        bail!("no running process named {name}");
                    }
                }
            }
            Action::Open(a) => {
                let t = self.text(&a.target)?;
                if !self.dry(format!("open {t}")) {
                    open_target(&t)?;
                }
            }

            // ---------------- windows
            #[cfg(windows)]
            Action::WindowActivate(a) => {
                let title = self.text(&a.title)?;
                if !self.dry(format!("activate window \"{title}\"")) {
                    let stop = self.opts.stop.clone();
                    let uia = self.uia()?;
                    let w = uia.wait_window(&title, a.exact, a.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                    uia.activate(&w)?;
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
            #[cfg(windows)]
            Action::WindowWait(a) => {
                let title = self.text(&a.title)?;
                if !self.dry(format!("wait for window \"{title}\"")) {
                    let stop = self.opts.stop.clone();
                    self.uia()?.wait_window(&title, a.exact, a.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                }
            }
            #[cfg(windows)]
            Action::WindowWaitClose(a) => {
                let title = self.text(&a.title)?;
                if !self.dry(format!("wait for window \"{title}\" to close")) {
                    let stop = self.opts.stop.clone();
                    self.uia()?.wait_window_closed(&title, a.exact, a.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                }
            }
            #[cfg(windows)]
            Action::WindowClose(a) => {
                let title = self.text(&a.title)?;
                if !self.dry(format!("close window \"{title}\"")) {
                    let stop = self.opts.stop.clone();
                    let uia = self.uia()?;
                    let w = uia.wait_window(&title, a.exact, a.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                    uia.close(&w)?;
                }
            }
            #[cfg(windows)]
            Action::WindowSetState(a) => {
                let title = self.text(&a.window.title)?;
                if !self.dry(format!("set window \"{title}\" to {:?}", a.state)) {
                    let stop = self.opts.stop.clone();
                    let uia = self.uia()?;
                    let w = uia.wait_window(&title, a.window.exact, a.window.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                    uia.set_state(&w, a.state)?;
                }
            }
            #[cfg(windows)]
            Action::WindowExists(a) => {
                let title = self.text(&a.window.title)?;
                let found = !self.uia()?.windows(&title, a.window.exact)?.is_empty();
                self.set_var(&a.output_var, json!(found.to_string()))?;
            }

            // ---------------- mouse
            Action::MouseMove(a) => {
                let (x, y) = (self.int(&a.x)? as i32, self.int(&a.y)? as i32);
                if !self.dry(format!("move mouse {} ({x}, {y})", if a.relative { "by" } else { "to" })) {
                    let (tx, ty) = if a.relative {
                        let (cx, cy) = self.input()?.position()?;
                        (cx + x, cy + y)
                    } else {
                        (x, y)
                    };
                    self.move_mouse(tx, ty, a.duration_ms, a.tween)?;
                }
            }
            Action::MouseClick(a) => {
                let pos = self.opt_point(&a.x, &a.y, "mouse_click")?;
                if !self.dry(format!("{:?} click x{} at {pos:?}", a.button, a.clicks)) {
                    if let Some((x, y)) = pos {
                        self.move_mouse(x, y, a.duration_ms, a.tween)?;
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    self.input()?.click_with(a.button, a.clicks, a.interval_ms)?;
                }
            }
            Action::MouseDown(a) | Action::MouseUp(a) => {
                let down = matches!(action, Action::MouseDown(_));
                let pos = self.opt_point(&a.x, &a.y, action.name())?;
                if !self.dry(format!("{} {:?} button at {pos:?}", if down { "press" } else { "release" }, a.button)) {
                    if let Some((x, y)) = pos {
                        self.move_mouse(x, y, None, Tween::Linear)?;
                    }
                    let i = self.input()?;
                    if down {
                        i.button_down(a.button)?
                    } else {
                        i.button_up(a.button)?
                    }
                }
            }
            Action::MouseScroll(a) => {
                let n = self.int(&a.amount)? as i32;
                let pos = self.opt_point(&a.x, &a.y, "mouse_scroll")?;
                if !self.dry(format!("scroll {n}")) {
                    if let Some((x, y)) = pos {
                        self.move_mouse(x, y, None, Tween::Linear)?;
                    }
                    self.input()?.scroll(n, a.horizontal)?;
                }
            }
            Action::MouseDrag(a) => {
                let from = match self.opt_point(&a.from_x, &a.from_y, "mouse_drag from")? {
                    Some(p) => p,
                    None if self.opts.dry_run => (0, 0),
                    None => self.input()?.position()?,
                };
                let (tx, ty) = (self.int(&a.to_x)? as i32, self.int(&a.to_y)? as i32);
                let to = if a.relative { (from.0 + tx, from.1 + ty) } else { (tx, ty) };
                if !self.dry(format!("drag {from:?} -> {to:?}")) {
                    let stop = self.opts.stop.clone();
                    self.input()?.drag_smooth(from, to, a.button, a.duration_ms, a.tween, &stop)?;
                }
            }
            Action::MousePosition(a) => {
                let (x, y) = match self.input.as_ref() {
                    Some(i) => i.position()?,
                    None => Input::new()?.position()?,
                };
                self.set_var(&a.x_var, json!(x))?;
                self.set_var(&a.y_var, json!(y))?;
            }

            // ---------------- keyboard
            Action::TypeText(a) => {
                let t = self.text(&a.text)?;
                let delay = a.delay_ms.unwrap_or(self.settings.key_delay_ms);
                if !self.dry(format!("type {} chars ({delay} ms apart)", t.chars().count())) {
                    let stop = self.opts.stop.clone();
                    self.input()?.type_text_mode(&t, delay, a.mode, &stop)?;
                }
            }
            Action::KeyPress(a) => {
                let keys = self.text(&a.keys)?;
                crate::input::parse_combo(&keys)?; // validate even in dry-run
                let interval = a.interval_ms.unwrap_or(self.settings.key_delay_ms);
                if !self.dry(format!("press {keys} x{}", a.times.max(1))) {
                    for n in 0..a.times.max(1) {
                        if n > 0 {
                            self.sleep(interval)?;
                        }
                        self.input()?.press_combo(&keys)?;
                    }
                }
            }
            Action::KeyDown(a) | Action::KeyUp(a) => {
                let keys = self.text(&a.keys)?;
                crate::input::parse_combo(&keys)?;
                let down = matches!(action, Action::KeyDown(_));
                if !self.dry(format!("{} {keys}", if down { "hold" } else { "release" })) {
                    let i = self.input()?;
                    if down {
                        i.key_down(&keys)?
                    } else {
                        i.key_up(&keys)?
                    }
                }
            }
            // ---------------- UI Automation
            #[cfg(windows)]
            Action::UiClick(a) => {
                let sel = self.selector(&a.target)?;
                if !self.dry(format!("click element {sel:?}")) {
                    self.input()?; // fail-safe check
                    let stop = self.opts.stop.clone();
                    let uia = self.uia()?;
                    let e = uia.find(&sel, &|| stop.load(Ordering::Relaxed))?;
                    uia.click(&e, a.invoke, a.double)?;
                }
            }
            #[cfg(windows)]
            Action::UiSetText(a) => {
                let sel = self.selector(&a.target)?;
                let text = self.text(&a.text)?;
                if !self.dry(format!("set text of element {sel:?}")) {
                    let stop = self.opts.stop.clone();
                    let uia = self.uia()?;
                    let e = uia.find(&sel, &|| stop.load(Ordering::Relaxed))?;
                    if !uia.set_value(&e, &text)? {
                        // No Value pattern (e.g. rich text editors): focus, select all, type.
                        e.set_focus().map_err(|err| anyhow!("cannot focus element: {err}"))?;
                        let i = self.input()?;
                        i.press_combo("ctrl+a")?;
                        i.press_combo("delete")?;
                        let (delay, stop) = (self.settings.key_delay_ms, self.opts.stop.clone());
                        self.input()?.type_text_mode(&text, delay, TypeMode::Keys, &stop)?;
                    }
                }
            }
            #[cfg(windows)]
            Action::UiGetText(a) => {
                let sel = self.selector(&a.target)?;
                let stop = self.opts.stop.clone();
                let uia = self.uia()?;
                let e = uia.find(&sel, &|| stop.load(Ordering::Relaxed))?;
                let t = uia.get_text(&e);
                self.set_var(&a.output_var, json!(t))?;
            }
            #[cfg(windows)]
            Action::UiWait(a) => {
                let sel = self.selector(&a.target)?;
                let stop = self.opts.stop.clone();
                self.uia()?.find(&sel, &|| stop.load(Ordering::Relaxed))?;
            }
            #[cfg(windows)]
            Action::UiExists(a) => {
                let sel = self.selector(&a.target)?;
                let stop = self.opts.stop.clone();
                let found = self.uia()?.find(&sel, &|| stop.load(Ordering::Relaxed)).is_ok();
                self.check_cancel()?;
                self.set_var(&a.output_var, json!(found.to_string()))?;
            }

            // ---------------- clipboard & screen
            Action::ClipboardSet(a) => {
                let t = self.text(&a.text)?;
                if !self.dry("set clipboard".into()) {
                    arboard::Clipboard::new()?.set_text(t)?;
                }
            }
            Action::ClipboardGet(a) => {
                let t = arboard::Clipboard::new()?.get_text().unwrap_or_default();
                self.set_var(&a.output_var, json!(t))?;
            }
            Action::Screenshot(a) => {
                let p = self.path(&a.path)?;
                let window = self.text(&a.window)?;
                if !self.dry(format!("save screenshot to {}", p.display())) {
                    screenshot(&p, a.monitor, &window, a.region)?;
                }
            }
            Action::ScreenSize(a) => {
                let (w, h) = crate::failsafe::screen_size();
                self.set_var(&a.width_var, json!(w))?;
                self.set_var(&a.height_var, json!(h))?;
            }
            Action::PixelGet(a) => {
                let (x, y) = (self.int(&a.x)? as i32, self.int(&a.y)? as i32);
                let c = crate::vision::pixel(x, y)?;
                self.set_var(&a.output_var, json!(crate::vision::hex(c)))?;
            }
            Action::PixelMatches(a) => {
                let (x, y) = (self.int(&a.x)? as i32, self.int(&a.y)? as i32);
                let want = crate::vision::parse_hex(&self.text(&a.color)?)?;
                let got = crate::vision::pixel(x, y)?;
                let ok = want.iter().zip(got).all(|(w, g)| w.abs_diff(g) <= a.tolerance);
                self.set_var(&a.output_var, json!(ok.to_string()))?;
            }

            // ---------------- image recognition
            Action::ImageFind(a) => {
                let m = self.find_image(&a.target, false, false)?.into_iter().next();
                self.set_var(&a.found_var, json!(m.is_some().to_string()))?;
                if let Some(m) = m {
                    self.set_var(&a.x_var, json!(m.x))?;
                    self.set_var(&a.y_var, json!(m.y))?;
                }
            }
            Action::ImageFindAll(a) => {
                let all = self.find_image(&a.target, true, false)?;
                self.set_var(&a.output_var, serde_json::to_value(all)?)?;
            }
            Action::ImageWait(a) => {
                if self.opts.dry_run {
                    self.dry(format!("wait for image {}", a.target.image));
                } else {
                    self.find_image(&a.target, false, true)?;
                }
            }
            Action::ImageWaitVanish(a) => {
                if !self.dry(format!("wait for image {} to disappear", a.target.image)) {
                    let t = self.image_template(&a.target)?;
                    let start = Instant::now();
                    while !crate::vision::locate(&t, a.target.region, a.target.confidence, false)?.is_empty() {
                        if start.elapsed() >= Duration::from_millis(a.target.timeout_ms) {
                            bail!("image {} still visible after {} ms", a.target.image, a.target.timeout_ms);
                        }
                        self.sleep(200)?;
                    }
                }
            }
            Action::ImageClick(a) => {
                if !self.dry(format!("click image {}", a.target.image)) {
                    let m = self.find_image(&a.target, false, true)?[0];
                    let (x, y) = (m.x + a.offset_x, m.y + a.offset_y);
                    self.move_mouse(x, y, None, Tween::Linear)?;
                    std::thread::sleep(Duration::from_millis(30));
                    self.input()?.click_with(a.button, a.clicks, 60)?;
                }
            }

            // ---------------- OCR
            Action::OcrRead(a) => {
                let (img, ox, oy) = self.ocr_capture(&a.source)?;
                let out = crate::ocr::recognize(&img, &self.text(&a.source.language)?, a.source.scale)?;
                self.log(LogLevel::Info, format!("OCR read {} line(s), {} characters", out.lines.len(), out.text.chars().count()));
                self.set_var(&a.lines_var, json!(out.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>()))?;
                self.set_var(&a.words_var, crate::ocr::words_json(&out, ox, oy))?;
                self.set_var(&a.output_var, json!(out.text))?;
            }
            Action::OcrFind(a) => {
                let at = self.ocr_locate(a)?;
                self.set_var(&a.found_var, json!(at.is_some().to_string()))?;
                if let Some((x, y)) = at {
                    self.set_var(&a.x_var, json!(x))?;
                    self.set_var(&a.y_var, json!(y))?;
                }
            }
            Action::OcrClick(a) => {
                if !a.find.source.image.trim().is_empty() {
                    bail!("ocr_click reads the screen; remove `image`");
                }
                if !self.dry(format!("click the text \"{}\"", a.find.text)) {
                    let Some((x, y)) = self.ocr_locate(&a.find)? else {
                        bail!("text \"{}\" not found on screen within {} ms", self.text(&a.find.text)?, a.find.timeout_ms);
                    };
                    self.set_var(&a.find.found_var, json!("true"))?;
                    self.set_var(&a.find.x_var, json!(x))?;
                    self.set_var(&a.find.y_var, json!(y))?;
                    self.move_mouse(x + a.offset_x, y + a.offset_y, None, Tween::Linear)?;
                    std::thread::sleep(Duration::from_millis(30));
                    self.input()?.click_with(a.button, a.clicks, 60)?;
                }
            }

            // ---------------- web
            Action::BrowserOpen(a) => {
                let url = crate::web::to_url(&self.text(&a.url)?, &self.opts.flow_dir);
                if !self.dry(format!("open {url} in {}", a.browser)) {
                    self.web = None; // close a previous browser first
                    let w = crate::web::WebSession::open(&self.text(&a.browser)?, a.headless, (a.width, a.height), a.keep_open)?;
                    if !url.trim().is_empty() {
                        w.navigate(&url, a.timeout_ms)?;
                    }
                    let title = w.title();
                    self.web = Some(w);
                    self.log(LogLevel::Info, format!("browser opened: {title}"));
                }
            }
            Action::BrowserNavigate(a) => {
                let url = crate::web::to_url(&self.text(&a.url)?, &self.opts.flow_dir);
                if !self.dry(format!("go to {url}")) {
                    self.web()?.navigate(&url, a.timeout_ms)?;
                }
            }
            Action::BrowserType(a) => {
                let css = self.web_css(&a.target)?;
                let text = self.text(&a.text)?;
                if !self.dry(format!("type {} chars into {css}", text.chars().count())) {
                    self.web()?.type_text(&css, &text, a.clear, a.press_enter, a.target.timeout_ms)?;
                }
            }
            Action::BrowserClick(a) => {
                let css = self.web_css(&a.target)?;
                if !self.dry(format!("click {css}")) {
                    self.web()?.click(&css, a.target.timeout_ms)?;
                }
            }
            Action::BrowserWait(a) => {
                let css = self.web_css(&a.target)?;
                if !self.dry(format!("wait for {css}")) {
                    self.web()?.wait(&css, a.target.timeout_ms)?;
                }
            }
            Action::BrowserGetText(a) => {
                let css = self.web_css(&a.target)?;
                if self.dry(format!("read {css}")) {
                    self.set_var(&a.output_var, json!(""))?;
                } else {
                    let attr = self.text(&a.attribute)?;
                    let v = self.web()?.get(&css, &attr, a.target.timeout_ms)?;
                    self.set_var(&a.output_var, json!(v))?;
                }
            }
            Action::BrowserScreenshot(a) => {
                let p = self.path(&a.path)?;
                let (id, sel) = (self.text(&a.element_id)?, self.text(&a.selector)?);
                let css = if id.trim().is_empty() && sel.trim().is_empty() { None } else { Some(crate::web::css(&id, &sel)?) };
                if !self.dry(format!("save page screenshot to {}", p.display())) {
                    let timeout = if a.timeout_ms == 0 { 10_000 } else { a.timeout_ms };
                    self.web()?.screenshot(&p, css.as_deref(), a.full_page, timeout)?;
                }
            }
            Action::BrowserWaitText(a) => {
                let text = self.text(&a.text)?;
                let (id, sel) = (self.text(&a.element_id)?, self.text(&a.selector)?);
                let css = if id.trim().is_empty() && sel.trim().is_empty() { None } else { Some(crate::web::css(&id, &sel)?) };
                if self.dry(format!("wait for text \"{text}\"")) {
                    self.set_var(&a.output_var, json!(""))?;
                } else {
                    let stop = self.opts.stop.clone();
                    let got = self.web()?.wait_text(&text, css.as_deref(), a.ignore_case, a.timeout_ms, &|| stop.load(Ordering::Relaxed))?;
                    self.set_var(&a.output_var, json!(got))?;
                }
            }
            Action::BrowserEval(a) => {
                let script = self.text(&a.script)?;
                if self.dry("run JavaScript".into()) {
                    self.set_var(&a.output_var, Value::Null)?;
                } else {
                    let v = self.web()?.eval(&script)?;
                    self.set_var(&a.output_var, v)?;
                }
            }
            Action::BrowserClose(_) => {
                if !self.dry("close the browser".into()) {
                    if let Some(w) = self.web.take() {
                        w.close();
                    }
                }
            }

            // ---------------- dialogs
            Action::MessageBox(a) => {
                let (title, text) = (self.text(&a.title)?, self.text(&a.text)?);
                if self.dry(format!("show message \"{text}\"")) {
                    self.set_var(&a.output_var, json!("ok"))?;
                } else {
                    let answer = crate::dialogs::message_box(&title, &text, a.buttons, a.icon)?;
                    self.log(LogLevel::Info, format!("user answered: {answer}"));
                    self.set_var(&a.output_var, json!(answer))?;
                }
            }
            Action::InputBox(a) => {
                let (title, prompt, default) = (self.text(&a.title)?, self.text(&a.prompt)?, self.text(&a.default)?);
                if self.dry(format!("ask \"{prompt}\"")) {
                    self.set_var(&a.output_var, json!(default))?;
                } else {
                    let image = if a.image.trim().is_empty() { None } else { Some(self.path(&a.image)?) };
                    match crate::dialogs::input_box(&title, &prompt, &default, a.password, image.as_deref())? {
                        Some(v) => {
                            if a.password {
                                self.secret_vars.insert(self.text(&a.output_var)?);
                            }
                            self.set_var(&a.output_var, json!(v))?
                        }
                        None if a.fail_on_cancel => bail!("cancelled by the user"),
                        None => self.set_var(&a.output_var, json!(""))?,
                    }
                }
            }

            #[cfg(not(windows))]
            other => bail!("action `{}` is only supported on Windows", other.name()),
        }
        Ok(Ctl::Next)
    }

    fn db_params(&self, params: &[Value]) -> Result<Vec<Value>> {
        params.iter().map(|p| self.vars.resolve_value(p)).collect()
    }

    /// The open connection for this connection string (opened on first use).
    fn db(&mut self, d: &DbConnection) -> Result<&mut crate::db::Conn> {
        let mut conn = self.text(&d.connection)?;
        if conn.trim().is_empty() {
            bail!("set the database connection (a .db file or an ODBC connection string)");
        }
        if conn.contains("{password}") {
            let name = self.text(&d.password_secret)?;
            if name.trim().is_empty() {
                bail!("the connection contains {{password}}: set password_secret");
            }
            conn = conn.replace("{password}", &crate::mail::secrets::get(name.trim())?);
        }
        if !self.dbs.contains_key(&conn) {
            let c = crate::db::open(&conn, &self.opts.flow_dir, self.opts.dry_run)?;
            let shown = crate::db::redact(&self.text(&d.connection)?);
            self.log(LogLevel::Info, format!("connected to {shown}"));
            self.dbs.insert(conn.clone(), c);
        }
        Ok(self.dbs.get_mut(&conn).unwrap())
    }

    /// In a dry run, say what reading the mail would have changed.
    fn mail_dry_notes(&mut self, f: &MailFilter) -> Result<()> {
        if !self.opts.dry_run {
            return Ok(());
        }
        let save = self.text(&f.save_attachments_to)?;
        if !save.trim().is_empty() {
            self.dry(format!("save attachments to {save}"));
        }
        if f.mark_read {
            self.dry("mark the messages read".into());
        }
        let mv = self.text(&f.move_to)?;
        if !mv.trim().is_empty() {
            self.dry(format!("move the messages to {mv}"));
        }
        Ok(())
    }

    fn mail_query(&self, f: &MailFilter) -> Result<crate::mailread::Query> {
        let since = match self.text(&f.since)? {
            s if s.trim().is_empty() => None,
            s => Some(crate::textops::parse_date(&s, "")?.date()),
        };
        let save = self.text(&f.save_attachments_to)?;
        Ok(crate::mailread::Query {
            folder: self.text(&f.folder)?,
            unread_only: f.unread_only,
            from: self.text(&f.from)?,
            subject_contains: self.text(&f.subject_contains)?,
            since,
            max: f.max.max(1) as usize,
            newest_first: f.newest_first,
            mark_read: f.mark_read && !self.opts.dry_run,
            move_to: if self.opts.dry_run { String::new() } else { self.text(&f.move_to)? },
            // A dry run reads messages but saves no attachments.
            save_dir: if save.trim().is_empty() || self.opts.dry_run { None } else { Some(self.path(&save)?) },
            attachment_filter: self.text(&f.attachment_filter)?,
        })
    }

    /// The picture OCR reads, and where its top-left corner is on screen.
    fn ocr_capture(&self, s: &OcrSource) -> Result<(xcap::image::RgbaImage, i32, i32)> {
        if !s.image.trim().is_empty() {
            let p = self.path(&s.image)?;
            let img = xcap::image::open(&p).with_context(|| format!("cannot open image {}", p.display()))?;
            return Ok((img.to_rgba8(), 0, 0));
        }
        if let Some(r) = s.region {
            return crate::vision::capture(Some(r))?.into_iter().next().ok_or_else(|| anyhow!("capture failed"));
        }
        let window = self.text(&s.window)?;
        if !window.trim().is_empty() {
            let want = window.to_lowercase();
            let w = xcap::Window::all()?
                .into_iter()
                .find(|w| w.title().map(|t| t.to_lowercase().contains(&want)).unwrap_or(false) && !w.is_minimized().unwrap_or(true))
                .ok_or_else(|| anyhow!("no visible window titled \"{window}\""))?;
            return Ok((w.capture_image()?, w.x()?, w.y()?));
        }
        let mut monitors = xcap::Monitor::all()?;
        monitors.sort_by_key(|m| !m.is_primary().unwrap_or(false));
        let m = monitors.get(s.monitor as usize).ok_or_else(|| anyhow!("monitor {} not found ({} available)", s.monitor, monitors.len()))?;
        Ok((m.capture_image()?, m.x()?, m.y()?))
    }

    /// Look for text until found or timed out. Returns the screen centre of the chosen match.
    fn ocr_locate(&mut self, a: &OcrFind) -> Result<Option<(i32, i32)>> {
        let needle = self.text(&a.text)?;
        if needle.trim().is_empty() {
            bail!("set the text to look for");
        }
        let lang = self.text(&a.source.language)?;
        let end = Instant::now() + Duration::from_millis(a.timeout_ms);
        loop {
            self.check_cancel()?;
            let (img, ox, oy) = self.ocr_capture(&a.source)?;
            let out = crate::ocr::recognize(&img, &lang, a.source.scale)?;
            let mut hits = crate::ocr::find(&out, &needle, a.ignore_case);
            hits.sort_by_key(|p| (p.y as i32 / 8, p.x as i32));
            if let Some(h) = hits.get(a.occurrence.max(1) as usize - 1) {
                let (cx, cy) = h.center();
                return Ok(Some((ox + cx.round() as i32, oy + cy.round() as i32)));
            }
            if Instant::now() >= end || !a.source.image.trim().is_empty() {
                let seen: String = out.text.replace('\n', " | ").chars().take(160).collect();
                self.log(LogLevel::Info, format!("text \"{needle}\" not found; OCR saw: {seen}"));
                return Ok(None);
            }
            self.sleep(300)?;
        }
    }

    fn http_request(&mut self, a: &HttpRequest) -> Result<()> {
        let method = match self.text(&a.method)?.trim().to_ascii_uppercase() {
            m if m.is_empty() => "GET".to_string(),
            m => m,
        };
        let url = self.text(&a.url)?;
        if url.trim().is_empty() {
            bail!("http_request needs a url");
        }
        let mut headers: Vec<(String, String)> = Vec::new();
        for (k, v) in &a.headers {
            headers.push((k.clone(), crate::vars::to_text(&self.vars.resolve_value(v)?)));
        }
        let has = |h: &[(String, String)], name: &str| h.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
        let body = match self.vars.resolve_value(&a.body)? {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) => Some(s.into_bytes()),
            v => {
                if !has(&headers, "content-type") {
                    headers.push(("Content-Type".into(), "application/json".into()));
                }
                Some(serde_json::to_vec(&v)?)
            }
        };
        let save_to = if a.save_to.trim().is_empty() { None } else { Some(self.path(&a.save_to)?) };
        let read_only = matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS");
        if (!read_only || save_to.is_some()) && self.dry(format!("{method} {url}")) {
            return Ok(());
        }
        if !a.bearer_secret.trim().is_empty() && !has(&headers, "authorization") {
            let token = crate::mail::secrets::get(&self.text(&a.bearer_secret)?)?;
            headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
        let r = crate::http::send(&method, &url, &headers, body, Duration::from_millis(a.timeout_ms.max(1_000)))?;
        self.set_var(&a.status_var, json!(r.status))?;
        if r.status >= 400 && a.fail_on_error_status {
            bail!("{method} {url} failed: {}", r.error_summary());
        }
        self.log(LogLevel::Info, format!("{method} {url} -> {} ({} bytes)", r.status, r.body.len()));
        if let Some(p) = save_to {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&p, &r.body).with_context(|| format!("write {}", p.display()))?;
            self.set_var(&a.output_var, json!(p.display().to_string()))?;
            return Ok(());
        }
        let value = match r.json() {
            Some(v) if a.parse_json && (r.is_json() || matches!(v, Value::Object(_) | Value::Array(_))) => v,
            _ => json!(r.text()),
        };
        self.set_var(&a.output_var, value)
    }

    fn ai_prompt(&mut self, a: &AiPrompt) -> Result<()> {
        let provider_name = self.text(&a.provider)?;
        let preset = crate::ai::resolve(&provider_name)?;
        let labels: Vec<String> = a.labels.iter().map(|l| self.text(l)).collect::<Result<Vec<_>>>()?.into_iter().filter(|l| !l.trim().is_empty()).collect();
        let mut system = self.text(&a.system)?;
        if !labels.is_empty() {
            system = format!("{}{}{}", system.trim_end(), if system.trim().is_empty() { "" } else { "\n\n" }, crate::ai::label_instruction(&labels));
        } else if a.json {
            system = format!("{}{}Reply with a single JSON value and nothing else.", system.trim_end(), if system.trim().is_empty() { "" } else { "\n\n" });
        }
        let prompt = self.text(&a.prompt)?;
        if prompt.trim().is_empty() {
            bail!("ai_prompt needs a prompt");
        }
        if self.dry(format!("ask {}: {}", provider_name.trim(), prompt.chars().take(80).collect::<String>())) {
            let v = labels.first().map(|l| json!(l)).unwrap_or_else(|| json!("[dry-run reply]"));
            return self.set_var(&a.output_var, v);
        }
        // Empty fields fall back to the provider preset (server, default model, key name).
        let or = |v: String, d: &str| if v.trim().is_empty() { d.to_string() } else { v.trim().to_string() };
        let secret = or(self.text(&a.api_key_secret)?, &preset.key_secret);
        let api_key = if secret.is_empty() { String::new() } else { crate::mail::secrets::get(&secret)? };
        let req = crate::ai::Request {
            api: preset.api,
            model: or(self.text(&a.model)?, &preset.default_model),
            base_url: or(self.text(&a.base_url)?, &preset.base_url),
            needs_key: preset.needs_key,
            api_key,
            system,
            prompt,
            temperature: a.temperature,
            max_tokens: a.max_tokens.max(16),
            json: a.json && labels.is_empty(),
            timeout: Duration::from_millis(a.timeout_ms.max(5_000)),
        };
        let started = Instant::now();
        let (reply, model) = crate::ai::complete(&req)?;
        let secs = started.elapsed().as_secs_f32();
        let value = if !labels.is_empty() {
            match crate::ai::pick_label(&reply, &labels) {
                Some(l) => json!(l),
                None => bail!("the model replied `{}`, which is none of the labels {}", reply.chars().take(120).collect::<String>(), labels.join(", ")),
            }
        } else if a.json {
            crate::ai::extract_json(&reply).ok_or_else(|| anyhow!("the model did not reply with JSON: {}", reply.chars().take(200).collect::<String>()))?
        } else {
            json!(reply)
        };
        let shown = crate::vars::to_text(&value);
        self.log(LogLevel::Info, format!("{model} ({secs:.1}s): {}", shown.chars().take(120).collect::<String>()));
        self.set_var(&a.output_var, value)
    }

    fn web(&self) -> Result<&crate::web::WebSession> {
        self.web.as_ref().ok_or_else(|| anyhow!("no browser is open: add a browser_open step first"))
    }

    fn web_css(&self, t: &WebTarget) -> Result<String> {
        crate::web::css(&self.text(&t.element_id)?, &self.text(&t.selector)?)
    }

    fn image_template(&self, t: &ImageTarget) -> Result<xcap::image::GrayImage> {
        let p = self.path(&t.image)?;
        if !p.exists() {
            bail!("image file not found: {}", p.display());
        }
        crate::vision::load_template(&p)
    }

    /// Locate an image, retrying until `timeout_ms`. With `required`, failing to find it is an error.
    fn find_image(&mut self, t: &ImageTarget, all: bool, required: bool) -> Result<Vec<crate::vision::Match>> {
        let tmpl = self.image_template(t)?;
        let shown = self.text(&t.image)?;
        if !(0.0..=1.0).contains(&t.confidence) {
            bail!("confidence must be between 0 and 1");
        }
        let start = Instant::now();
        loop {
            let found = crate::vision::locate(&tmpl, t.region, t.confidence, all)?;
            if !found.is_empty() {
                let m = found[0];
                self.log(LogLevel::Info, format!("found {shown} at ({}, {}) score {:.3}", m.x, m.y, m.score));
                return Ok(found);
            }
            if start.elapsed() >= Duration::from_millis(t.timeout_ms) {
                break;
            }
            self.sleep(250)?;
        }
        if required {
            bail!("image {shown} not found on screen within {} ms (confidence {})", t.timeout_ms, t.confidence);
        }
        Ok(vec![])
    }

    fn selector(&self, s: &Selector) -> Result<Selector> {
        Ok(Selector {
            window: self.text(&s.window)?,
            name: self.text(&s.name)?,
            name_contains: s.name_contains,
            automation_id: self.text(&s.automation_id)?,
            class_name: self.text(&s.class_name)?,
            control_type: self.text(&s.control_type)?,
            timeout_ms: s.timeout_ms,
        })
    }

    /// Expand a path that may contain wildcards into existing files.
    fn expand(&self, raw: &str) -> Result<Vec<PathBuf>> {
        let p = self.path(raw)?;
        let s = p.display().to_string();
        if s.contains('*') || s.contains('?') {
            Ok(glob::glob(&s).map_err(|e| anyhow!("bad pattern: {e}"))?.flatten().filter(|p| p.is_file()).collect())
        } else if p.exists() {
            Ok(vec![p])
        } else {
            Ok(vec![])
        }
    }

    fn copy_move(&mut self, a: &CopyMove, is_move: bool) -> Result<()> {
        let verb = if is_move { "move" } else { "copy" };
        let sources = self.expand(&a.source)?;
        if sources.is_empty() {
            bail!("source not found: {}", self.text(&a.source)?);
        }
        let dest = self.path(&a.destination)?;
        // A trailing slash or a wildcard source means "into this folder".
        let into_dir = self.text(&a.destination)?.trim().ends_with(['\\', '/']);
        let many = sources.len() > 1 || self.text(&a.source)?.contains(['*', '?']);
        for src in sources {
            let target = if dest.is_dir() || many || into_dir {
                dest.join(src.file_name().ok_or_else(|| anyhow!("bad source {}", src.display()))?)
            } else {
                dest.clone()
            };
            if target.exists() && !a.overwrite {
                bail!("{} already exists (set overwrite: true)", target.display());
            }
            if self.dry(format!("{verb} {} -> {}", src.display(), target.display())) {
                continue;
            }
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if is_move {
                if target.exists() {
                    std::fs::remove_file(&target).ok();
                }
                if std::fs::rename(&src, &target).is_err() {
                    std::fs::copy(&src, &target).with_context(|| format!("move {}", src.display()))?;
                    std::fs::remove_file(&src)?;
                }
            } else {
                std::fs::copy(&src, &target).with_context(|| format!("copy {} -> {}", src.display(), target.display()))?;
            }
        }
        Ok(())
    }

    fn run_program(&mut self, a: &RunProgram) -> Result<()> {
        let program = self.text(&a.program)?;
        let args: Vec<String> = a.args.iter().map(|s| self.text(s)).collect::<Result<_>>()?;
        if self.dry(format!("run {program} {}", args.join(" "))) {
            return Ok(());
        }
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        if !a.working_dir.is_empty() {
            cmd.current_dir(self.path(&a.working_dir)?);
        }
        if !a.wait {
            // Detach stdio so a long-running program does not hold our output pipes open.
            // Windows children inherit every inheritable handle, so also stop our
            // own stdio from being inheritable.
            no_inherit_std_handles();
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            cmd.spawn().with_context(|| format!("start {program}"))?;
            return Ok(());
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        let mut child = hidden(&mut cmd).spawn().with_context(|| format!("start {program}"))?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut stdout, &mut b).ok();
            b
        });
        let err_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut stderr, &mut b).ok();
            b
        });
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if self.cancelled() || (a.timeout_ms > 0 && start.elapsed() >= Duration::from_millis(a.timeout_ms)) {
                let _ = child.kill();
                self.check_cancel()?;
                bail!("{program} timed out after {} ms", a.timeout_ms);
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let out = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).to_string();
        let err = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).to_string();
        let code = status.code().unwrap_or(-1);
        self.set_var(&a.output_var, json!(out))?;
        self.set_var(&a.exit_code_var, json!(code))?;
        if a.check_exit_code && code != 0 {
            bail!("{program} exited with code {code}: {}", err.trim());
        }
        Ok(())
    }

    fn run_subflow(&mut self, a: &RunFlow) -> Result<()> {
        if self.depth >= MAX_SUBFLOW_DEPTH {
            bail!("sub-flows nested too deeply (limit {MAX_SUBFLOW_DEPTH})");
        }
        let path = self.path(&a.path)?;
        let sub = Flow::load(&path)?;
        let mut passed = Map::new();
        for (k, v) in &a.variables {
            passed.insert(k.clone(), self.vars.resolve_value(v)?);
        }
        let mut vars = Vars {
            map: if a.share_variables { self.vars.map.clone() } else { Map::new() },
            flow_dir: path.parent().unwrap_or(Path::new(".")).display().to_string(),
        };
        let declared: Map<String, Value> =
            sub.variables.iter().filter(|(k, _)| !vars.map.contains_key(*k) && !passed.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        init_vars(&mut vars, &declared, Map::new())?;
        vars.map.extend(passed);
        // Breakpoints refer to this flow's step ids, so a sub-flow runs straight through.
        let opts = RunOptions { flow_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(), debug: None, ..self.opts.clone() };
        self.log(LogLevel::Info, format!("running sub-flow {}", path.display()));
        let (ctl, map, secrets) = {
            let mut child = Context {
                vars,
                opts: &opts,
                settings: self.settings.clone(),
                reason: self.reason.clone(),
                sink: &mut *self.sink,
                input: self.input.take(),
                #[cfg(windows)]
                uia: self.uia.take(),
                web: self.web.take(),
                dbs: std::mem::take(&mut self.dbs),
                secret_vars: self.secret_vars.clone(),
                depth: self.depth + 1,
            };
            let ctl = child.run_steps(&sub.steps, 0);
            self.input = child.input.take();
            #[cfg(windows)]
            {
                self.uia = child.uia.take();
            }
            {
                self.web = child.web.take();
                self.dbs = std::mem::take(&mut child.dbs);
            }
            (ctl, child.vars.map, child.secret_vars)
        };
        if a.share_variables {
            self.vars.map = map;
            // Passwords typed in the sub-flow stay masked in this flow's output too.
            self.secret_vars.extend(secrets);
        }
        match ctl? {
            Ctl::Stop { success: false, message } => bail!("sub-flow failed: {message}"),
            _ => Ok(()),
        }
    }
}

fn file_info(p: &Path) -> Value {
    let meta = std::fs::metadata(p).ok();
    let ts = |t: std::io::Result<std::time::SystemTime>| -> Value {
        t.ok().map(|t| json!(chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string())).unwrap_or(Value::Null)
    };
    json!({
        "path": p.display().to_string(),
        "name": p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "stem": p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "extension": p.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "parent": p.parent().map(|s| s.display().to_string()).unwrap_or_default(),
        "exists": meta.is_some(),
        "is_dir": meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
        "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
        "modified": meta.as_ref().map(|m| ts(m.modified())).unwrap_or(Value::Null),
        "created": meta.as_ref().map(|m| ts(m.created())).unwrap_or(Value::Null),
    })
}

/// Make this process's stdin/stdout/stderr non-inheritable, so programs started
/// without waiting (e.g. an app the flow launches) don't keep the caller's pipes
/// open, which would make `autoflow run ... | ...` hang until that app exits.
fn no_inherit_std_handles() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(which: u32) -> isize;
            fn SetHandleInformation(h: isize, mask: u32, flags: u32) -> i32;
        }
        const HANDLE_FLAG_INHERIT: u32 = 1;
        for which in [-10i32, -11, -12] {
            unsafe {
                let h = GetStdHandle(which as u32);
                if h != 0 && h != -1 {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }
}

/// Do not flash a console window for helper processes.
fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn open_target(t: &str) -> Result<()> {
    #[cfg(windows)]
    {
        if t.contains("://") || t.starts_with("mailto:") {
            Command::new("rundll32").args(["url.dll,FileProtocolHandler", t]).spawn()?;
        } else {
            Command::new("explorer").arg(t).spawn()?;
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(t).spawn()?;
    }
    Ok(())
}

fn screenshot(path: &Path, monitor: u32, window: &str, region: Option<Region>) -> Result<()> {
    let img = if let Some(r) = region {
        crate::vision::capture(Some(r))?.into_iter().next().ok_or_else(|| anyhow!("capture failed"))?.0
    } else if !window.is_empty() {
        let want = window.to_lowercase();
        let w = xcap::Window::all()?
            .into_iter()
            .find(|w| w.title().map(|t| t.to_lowercase().contains(&want)).unwrap_or(false) && !w.is_minimized().unwrap_or(true))
            .ok_or_else(|| anyhow!("no visible window titled \"{window}\""))?;
        w.capture_image()?
    } else {
        let mut monitors = xcap::Monitor::all()?;
        // index 0 = primary
        monitors.sort_by_key(|m| !m.is_primary().unwrap_or(false));
        let m = monitors.get(monitor as usize).ok_or_else(|| anyhow!("monitor {monitor} not found ({} available)", monitors.len()))?;
        m.capture_image()?
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    img.save(path).with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

/// "\n", "\t" and "\\" typed in a separator field mean newline, tab and backslash.
fn unescape(s: &str) -> String {
    s.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_masked_everywhere() {
        let vars: Map<String, Value> = serde_json::from_value(json!({
            "pw": "s3cret!", "copy": "s3cret!", "login": {"user": "me", "pass": "s3cret!"}, "other": "fine"
        }))
        .unwrap();
        let names: std::collections::HashSet<String> = ["pw".to_string()].into();
        let secrets = secret_values(&vars, &names);
        let masked = mask_value(Value::Object(vars), &secrets);
        assert_eq!(masked["pw"], MASK);
        assert_eq!(masked["copy"], MASK, "copies of the password are masked too");
        assert_eq!(masked["login"]["pass"], MASK);
        assert_eq!(masked["other"], "fine");
        match mask_event(Event::Log { level: LogLevel::Info, message: "logging in with s3cret!".into() }, &secrets) {
            Event::Log { message, .. } => assert_eq!(message, format!("logging in with {MASK}")),
            _ => unreachable!(),
        }
        assert!(secret_values(&Map::new(), &names).is_empty());
    }
}
