//! Executes flows. Runs synchronously on the calling thread and reports
//! progress through an event callback, so the CLI prints events and the GUI
//! runs the engine on a worker thread and forwards events over a channel.

use anyhow::{anyhow, bail, Context as _, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::actions::*;
use crate::flow::{Flow, OnError, Step};
use crate::input::Input;
use crate::vars::Vars;

/// Progress reported while a flow runs.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    StepStarted { id: String, action: String, depth: usize },
    StepFinished { id: String, action: String, ok: bool, error: Option<String>, millis: u64 },
    Log { level: LogLevel, message: String },
    Finished { success: bool, message: String },
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Log side-effecting actions instead of performing them.
    pub dry_run: bool,
    /// Set to true from another thread to cancel the run.
    pub stop: Arc<AtomicBool>,
    /// Pause after every step (useful to watch a flow).
    pub step_delay_ms: u64,
    /// Abort when the mouse is pushed into the top-left screen corner.
    pub failsafe: bool,
    /// Folder used for `${flow_dir}` and relative paths. Usually the flow file's folder.
    pub flow_dir: PathBuf,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            stop: Arc::new(AtomicBool::new(false)),
            step_delay_ms: 0,
            failsafe: true,
            flow_dir: std::env::current_dir().unwrap_or_default(),
        }
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
    let vars = Vars { map: Map::new(), flow_dir: opts.flow_dir.display().to_string() };
    let mut ctx = Context { vars, opts, sink, input: None, #[cfg(windows)] uia: None, depth: 0 };
    let started = Instant::now();
    let init = init_vars(&mut ctx.vars, &flow.variables, overrides);
    let (success, message) = match init.and_then(|_| ctx.run_steps(&flow.steps, 0)) {
        Ok(Ctl::Next) | Ok(Ctl::Break) => (true, format!("completed in {:.1}s", started.elapsed().as_secs_f64())),
        Ok(Ctl::Stop { success, message }) => (success, if message.is_empty() { "stopped".into() } else { message }),
        Err(e) => (false, format!("{e:#}")),
    };
    (ctx.sink)(Event::Finished { success, message: message.clone() });
    RunResult { success, message, variables: ctx.vars.map }
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
    sink: &'a mut dyn FnMut(Event),
    input: Option<Input>,
    #[cfg(windows)]
    uia: Option<crate::uia::Uia>,
    depth: usize,
}

const MAX_SUBFLOW_DEPTH: usize = 16;

impl<'a> Context<'a> {
    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        (self.sink)(Event::Log { level, message: message.into() });
    }

    fn cancelled(&self) -> bool {
        self.opts.stop.load(Ordering::Relaxed)
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancelled() {
            return Err(Cancelled("run cancelled by user".into()).into());
        }
        Ok(())
    }

    /// Sleep in small slices so cancellation stays responsive.
    fn sleep(&self, ms: u64) -> Result<()> {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            self.check_cancel()?;
            std::thread::sleep((end - Instant::now()).min(Duration::from_millis(50)));
        }
        Ok(())
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
        (self.sink)(Event::StepStarted { id: id.clone(), action: action.clone(), depth });
        let start = Instant::now();
        let is_container = !step.action.children().is_empty();
        let attempts = if is_container { 1 } else { step.retries + 1 };

        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            match self.exec(&step.action, depth) {
                Ok(ctl) => break Ok(ctl),
                Err(e) if e.is::<Cancelled>() || e.is::<Handled>() => break Err(e),
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
                (self.sink)(Event::StepFinished { id, action, ok: true, error: None, millis });
                if self.opts.step_delay_ms > 0 {
                    self.sleep(self.opts.step_delay_ms)?;
                }
                Ok(ctl)
            }
            Err(e) if e.is::<Cancelled>() || e.is::<Handled>() => {
                (self.sink)(Event::StepFinished { id, action, ok: false, error: Some(format!("{e:#}")), millis });
                Err(e)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                (self.sink)(Event::StepFinished { id: id.clone(), action: action.clone(), ok: false, error: Some(msg.clone()), millis });
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

    fn input(&mut self) -> Result<&mut Input> {
        if self.opts.failsafe {
            let inp = match self.input.take() {
                Some(i) => i,
                None => Input::new()?,
            };
            let pos = inp.position().unwrap_or((1, 1));
            self.input = Some(inp);
            if pos == (0, 0) {
                return Err(Cancelled("fail-safe triggered: mouse moved to the top-left corner".into()).into());
            }
        } else if self.input.is_none() {
            self.input = Some(Input::new()?);
        }
        Ok(self.input.as_mut().unwrap())
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
            Op::Equals => match nums { (Ok(a), Ok(b)) => a == b, _ => l == r },
            Op::NotEquals => match nums { (Ok(a), Ok(b)) => a != b, _ => l != r },
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
                let cur: f64 = if cur.trim().is_empty() { 0.0 } else { cur.trim().parse().map_err(|_| anyhow!("variable `{name}` is not a number: `{cur}`"))? };
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
                    let i = self.input()?;
                    if a.relative { i.move_by(x, y)? } else { i.move_to(x, y)? }
                }
            }
            Action::MouseClick(a) => {
                let pos = match (&a.x, &a.y) {
                    (Some(x), Some(y)) => Some((self.int(x)? as i32, self.int(y)? as i32)),
                    (None, None) => None,
                    _ => bail!("mouse_click needs both x and y, or neither"),
                };
                if !self.dry(format!("{:?} click x{} at {pos:?}", a.button, a.clicks)) {
                    let i = self.input()?;
                    if let Some((x, y)) = pos {
                        i.move_to(x, y)?;
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    i.click(a.button, a.clicks)?;
                }
            }
            Action::MouseScroll(a) => {
                let n = self.int(&a.amount)? as i32;
                if !self.dry(format!("scroll {n}")) {
                    self.input()?.scroll(n, a.horizontal)?;
                }
            }
            Action::MouseDrag(a) => {
                let from = (self.int(&a.from_x)? as i32, self.int(&a.from_y)? as i32);
                let to = (self.int(&a.to_x)? as i32, self.int(&a.to_y)? as i32);
                if !self.dry(format!("drag {from:?} -> {to:?}")) {
                    self.input()?.drag(from, to, a.button)?;
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
                if !self.dry(format!("type {} chars", t.chars().count())) {
                    self.input()?.type_text_mode(&t, a.delay_ms, a.mode)?;
                }
            }
            Action::KeyPress(a) => {
                let keys = self.text(&a.keys)?;
                crate::input::parse_combo(&keys)?; // validate even in dry-run
                if !self.dry(format!("press {keys} x{}", a.times.max(1))) {
                    for n in 0..a.times.max(1) {
                        if n > 0 {
                            std::thread::sleep(Duration::from_millis(30));
                        }
                        self.input()?.press_combo(&keys)?;
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
                        i.type_text(&text, 0)?;
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
                    screenshot(&p, a.monitor, &window)?;
                }
            }

            #[cfg(not(windows))]
            other => bail!("action `{}` is only supported on Windows", other.name()),
        }
        Ok(Ctl::Next)
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
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            cmd.spawn().with_context(|| format!("start {program}"))?;
            return Ok(());
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        let mut child = hidden(&mut cmd).spawn().with_context(|| format!("start {program}"))?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out_t = std::thread::spawn(move || { let mut b = Vec::new(); std::io::Read::read_to_end(&mut stdout, &mut b).ok(); b });
        let err_t = std::thread::spawn(move || { let mut b = Vec::new(); std::io::Read::read_to_end(&mut stderr, &mut b).ok(); b });
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
        let declared: Map<String, Value> = sub
            .variables
            .iter()
            .filter(|(k, _)| !vars.map.contains_key(*k) && !passed.contains_key(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        init_vars(&mut vars, &declared, Map::new())?;
        vars.map.extend(passed);
        let opts = RunOptions { flow_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(), ..self.opts.clone() };
        self.log(LogLevel::Info, format!("running sub-flow {}", path.display()));
        let (ctl, map) = {
            let mut child = Context {
                vars,
                opts: &opts,
                sink: &mut *self.sink,
                input: self.input.take(),
                #[cfg(windows)]
                uia: self.uia.take(),
                depth: self.depth + 1,
            };
            let ctl = child.run_steps(&sub.steps, 0);
            self.input = child.input.take();
            #[cfg(windows)]
            {
                self.uia = child.uia.take();
            }
            (ctl, child.vars.map)
        };
        if a.share_variables {
            self.vars.map = map;
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
        t.ok()
            .map(|t| json!(chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()))
            .unwrap_or(Value::Null)
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

fn screenshot(path: &Path, monitor: u32, window: &str) -> Result<()> {
    let img = if !window.is_empty() {
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
