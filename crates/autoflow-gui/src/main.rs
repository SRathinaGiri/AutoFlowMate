//! AutoFlowMate Studio — visual editor and runner for flow JSON files.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flowchart;
mod form;
mod i18n;
mod icons;
mod look;
mod schema_info;
mod settings;
mod smart;
mod tree;
mod variables;

use crate::i18n::tr;
use autoflow_core::{catalog, run_flow, validate_json, Action, ActionInfo, Debugger, Event, Flow, RunOptions, Severity, Step};
use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use tree::{DragItem, ListPath, Selection, StepPath};

#[derive(PartialEq, Clone, Copy)]
enum View {
    Flowchart,
    List,
    Json,
}

/// Result of a "Snip image" request: saved file path relative to the flow folder.
type SnipResult = Result<String, String>;

/// Steps from a finished recording.
type RecordResult = Result<Vec<Step>, String>;

/// Result of "Pick element": screen position and, when found, a UI Automation selector.
type PickResult = Result<(i32, i32, Option<String>), String>;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    // autoflow-studio --language-template FILE: every text to translate, with empty values.
    if args.get(1).map(String::as_str) == Some("--language-template") {
        let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "template.json".into()));
        match i18n::write_template(&out, &schema_info::SchemaInfo::load()) {
            Ok(n) => println!("wrote {n} texts to {}", out.display()),
            Err(e) => eprintln!("cannot write {}: {e}", out.display()),
        }
        return Ok(());
    }
    let path = args.get(1).map(PathBuf::from);
    // Opens maximised (not full screen, so the title bar and taskbar stay usable).
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1320.0, 820.0]).with_min_inner_size([900.0, 500.0]).with_maximized(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "AutoFlowMate Studio",
        options,
        Box::new(move |cc| {
            let prefs = look::UiPrefs::load();
            let _ = i18n::set_language(&prefs.language);
            look::apply_fonts(&cc.egui_ctx, &prefs);
            look::apply_style(&cc.egui_ctx, &prefs);
            Ok(Box::new(Studio::new(path)))
        }),
    )
}

enum LogKind {
    Info,
    Warn,
    Error,
    Ok,
}

struct Running {
    rx: Receiver<Event>,
    stop: Arc<AtomicBool>,
    current: Option<String>,
    debug: Arc<Debugger>,
    /// Paused before this step, with the variable values at that moment.
    paused: Option<(String, String, Map<String, Value>)>,
}

/// Undo / redo by whole-flow snapshots. Quick successive edits (typing) form one step.
#[derive(Default)]
struct History {
    undo: Vec<String>,
    redo: Vec<String>,
    snap: String,
    last_change: Option<std::time::Instant>,
}

const HISTORY_LIMIT: usize = 200;

/// What to do once the user has answered "save changes?".
#[derive(Clone)]
enum Pending {
    New,
    Open(Option<PathBuf>),
    Template(PathBuf),
    Close,
}

/// What "Try it" produced: variable names and values, or the error.
type TryOutcome = Result<Vec<(String, String)>, String>;

struct Studio {
    flow: Flow,
    path: Option<PathBuf>,
    dirty: bool,
    sel: Selection,
    catalog: Vec<ActionInfo>,
    help: HashMap<String, String>,
    json_buffers: HashMap<String, String>,
    buffers_for: Option<StepPath>,
    flow_vars_buf: String,
    step_error: Option<String>,
    log: Vec<(LogKind, String)>,
    running: Option<Running>,
    failed_steps: Vec<String>,
    dry_run: bool,
    minimize_on_run: bool,
    pick_rx: Option<Receiver<PickResult>>,
    snip_rx: Option<(StepPath, Receiver<SnipResult>)>,
    web_pick_rx: Option<(StepPath, Receiver<Result<autoflow_core::web::PickedElement, String>>)>,
    palette_filter: String,
    last_title: String,
    view: View,
    zoom: f32,
    settings_buf: HashMap<String, String>,
    sched: Option<SchedUi>,
    icon_tex: Option<egui::TextureHandle>,
    /// Splash screen is shown until this time (seconds since start), or until clicked.
    splash_until: Option<f64>,
    show_about: bool,
    right_tab: RightTab,
    last_sel: Selection,
    vars_ui: variables::VarsUi,
    table_cache: HashMap<String, (Option<std::time::SystemTime>, form::TableInfo)>,
    preview: Option<(String, Result<Value, String>)>,
    /// Result of "Try it" for a step: the step it belongs to and what it produced.
    try_result: Option<(StepPath, TryOutcome)>,
    secret_dialog: Option<SecretDialog>,
    history: History,
    breakpoints: HashSet<String>,
    pending: Option<Pending>,
    allow_close: bool,
    /// A recording in progress: its stop flag and where the steps arrive.
    recording: Option<(Arc<AtomicBool>, Receiver<RecordResult>)>,
    /// Presets, ODBC sources (for the property editors).
    lists: smart::Lists,
    /// Results of editor buttons (loaded models, connection tests).
    cache: smart::Cache,
    /// The Settings window, when open.
    settings_ui: Option<settings::SettingsUi>,
    /// "How recording works" is shown before recording starts.
    record_prompt: bool,
    /// File > New from template: the templates found (None = window closed).
    template_list: Option<Vec<autoflow_core::templates::TemplateInfo>>,
    /// File > Save as template: the name being typed (None = window closed).
    template_name: Option<String>,
    /// Per-action field descriptions, allowed values and defaults from the schema.
    schema: schema_info::SchemaInfo,
    /// Font, text size and palette behaviour (View menu).
    prefs: look::UiPrefs,
    /// The palette category that is open when only one may be.
    open_category: Option<String>,
    /// One-frame request to open (true) or close (false) every palette category.
    palette_all: Option<bool>,
}

#[derive(PartialEq, Clone, Copy)]
enum RightTab {
    Step,
    Variables,
    Flow,
}

const ICON_PNG: &[u8] = include_bytes!("../../../assets/autoflowmate.png");
/// Banner shown on the splash screen and in About (made by scripts/brand_assets.py).
const SPLASH_PNG: &[u8] = include_bytes!("../../../assets/splash.png");
const GITHUB_URL: &str = "https://github.com/SRathinaGiri/AutoFlowMate";
const SPLASH_SECONDS: f64 = 2.5;

/// "Set password" dialog: the value goes straight to Windows Credential Manager.
struct SecretDialog {
    name: String,
    first: String,
    second: String,
    error: Option<String>,
}

/// State of the "Schedule" window.
struct SchedUi {
    name: String,
    kind: usize,
    time: String,
    date: String,
    days: [bool; 7],
    day_of_month: u8,
    minutes: u32,
    tasks: Vec<autoflow_core::schedule::TaskInfo>,
    message: Option<(bool, String)>,
    /// Run the flow's triggers (autoflow watch) instead of the flow.
    watch: bool,
    /// The flow has triggers, so the watch option is offered.
    has_triggers: bool,
}

const SCHED_KINDS: [&str; 7] = ["Daily", "Weekdays (Mon-Fri)", "Weekly", "Monthly", "Once", "Every N minutes", "At log-on"];

impl Studio {
    fn new(path: Option<PathBuf>) -> Self {
        let mut s = Self {
            flow: Flow::default(),
            path: None,
            dirty: false,
            sel: Selection::None,
            catalog: catalog(),
            help: form::schema_help(),
            json_buffers: HashMap::new(),
            buffers_for: None,
            flow_vars_buf: "{}".into(),
            step_error: None,
            log: vec![],
            running: None,
            failed_steps: vec![],
            dry_run: false,
            minimize_on_run: false,
            pick_rx: None,
            snip_rx: None,
            web_pick_rx: None,
            palette_filter: String::new(),
            last_title: String::new(),
            view: View::Flowchart,
            zoom: 1.0,
            settings_buf: HashMap::new(),
            sched: None,
            icon_tex: None,
            splash_until: Some(-1.0),
            show_about: false,
            right_tab: RightTab::Flow,
            last_sel: Selection::None,
            vars_ui: variables::VarsUi::default(),
            table_cache: HashMap::new(),
            preview: None,
            try_result: None,
            secret_dialog: None,
            history: History::default(),
            breakpoints: HashSet::new(),
            pending: None,
            allow_close: false,
            recording: None,
            lists: smart::Lists::load(),
            cache: smart::Cache::default(),
            settings_ui: None,
            record_prompt: false,
            template_list: None,
            template_name: None,
            schema: schema_info::SchemaInfo::load(),
            prefs: look::UiPrefs::load(),
            open_category: Some("Flow".into()),
            palette_all: None,
        };
        if let Some(p) = path {
            s.open_path(p);
        }
        s.sync_vars_buf();
        s.reset_history();
        s
    }

    // ---------------------------------------------------------------- undo

    fn reset_history(&mut self) {
        self.history = History { snap: self.flow.to_json(), ..History::default() };
    }

    /// Called once per frame: record a snapshot when the flow changed.
    fn track_history(&mut self) {
        let now = self.flow.to_json();
        if now == self.history.snap {
            return;
        }
        let h = &mut self.history;
        let burst = h.last_change.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(700));
        if !burst {
            h.undo.push(std::mem::take(&mut h.snap));
            if h.undo.len() > HISTORY_LIMIT {
                h.undo.remove(0);
            }
        }
        h.redo.clear();
        h.snap = now;
        h.last_change = Some(std::time::Instant::now());
    }

    fn undo(&mut self) {
        if let Some(prev) = self.history.undo.pop() {
            let cur = std::mem::replace(&mut self.history.snap, prev.clone());
            self.history.redo.push(cur);
            self.restore(&prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.history.redo.pop() {
            let cur = std::mem::replace(&mut self.history.snap, next.clone());
            self.history.undo.push(cur);
            self.restore(&next);
        }
    }

    fn restore(&mut self, json: &str) {
        match serde_json::from_str::<Flow>(json) {
            Ok(f) => {
                self.flow = f;
                self.dirty = true;
                self.history.last_change = None;
                self.json_buffers.clear();
                self.buffers_for = None;
                self.settings_buf.clear();
                self.sync_vars_buf();
                if let Selection::Step(p) = &self.sel {
                    if tree::step_ref(&self.flow.steps, p).is_none() {
                        self.sel = Selection::None;
                    }
                } else {
                    self.sel = Selection::None;
                }
            }
            Err(e) => self.info(LogKind::Error, format!("undo failed: {e}")),
        }
    }

    // ---------------------------------------------------------------- unsaved changes

    /// Do `p` now, or first ask whether to save the changes.
    fn guard(&mut self, p: Pending, ctx: &egui::Context) {
        if self.dirty {
            self.pending = Some(p);
        } else {
            self.proceed(p, ctx);
        }
    }

    fn proceed(&mut self, p: Pending, ctx: &egui::Context) {
        match p {
            Pending::New => self.new_flow(),
            Pending::Open(None) => self.open_dialog(),
            Pending::Open(Some(path)) => self.open_path(path),
            Pending::Template(path) => self.start_from_template(path),
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn unsaved_window(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.clone() else { return };
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "this flow".into());
        let mut answer: Option<u8> = None;
        egui::Window::new(tr("Unsaved changes")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("Save the changes to {name} first?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("💾 Save")).strong()).clicked() {
                    answer = Some(0);
                }
                if ui.button(tr("Don't save")).clicked() {
                    answer = Some(1);
                }
                if ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(2);
                }
            });
        });
        match answer {
            Some(0) => {
                self.save(false);
                self.pending = None;
                if !self.dirty {
                    self.proceed(p, ctx);
                }
            }
            Some(1) => {
                self.pending = None;
                self.dirty = false;
                self.proceed(p, ctx);
            }
            Some(_) => self.pending = None,
            None => {}
        }
    }

    // ---------------------------------------------------------------- breakpoints

    fn toggle_breakpoint(&mut self, p: &StepPath) {
        if tree::step_ref(&self.flow.steps, p).is_some_and(|s| s.id.is_empty()) {
            self.flow.assign_ids();
        }
        let Some(id) = tree::step_ref(&self.flow.steps, p).map(|s| s.id.clone()) else { return };
        if !self.breakpoints.remove(&id) {
            self.breakpoints.insert(id.clone());
        }
        if let Some(r) = &self.running {
            *r.debug.breakpoints.lock().unwrap() = self.breakpoints.clone();
        }
    }

    /// Start recording clicks and keys; the window is minimised until Ctrl+Alt+R.
    fn start_record(&mut self, ctx: &egui::Context) {
        if self.recording.is_some() || self.running.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let s2 = stop.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            // Give the window time to minimise first.
            std::thread::sleep(std::time::Duration::from_millis(800));
            let r = autoflow_core::recorder::record(Default::default(), s2, &mut |_| {}).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
            repaint.request_repaint();
        });
        self.recording = Some((stop, rx));
        self.info(LogKind::Info, "recording: use the app as usual; stop with the ■ Stop recording window or Ctrl+Alt+R");
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    /// Explains recording, and how to stop it, before it starts.
    fn record_prompt_window(&mut self, ctx: &egui::Context) {
        if !self.record_prompt {
            return;
        }
        let mut go = None;
        egui::Window::new(tr("⏺ Record your actions")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(tr("Recording starts when you press OK. Studio is minimised, and the steps are added at the selected place."));
            ui.add_space(6.0);
            ui.label(tr("•  Use the program you want to automate as usual: click, type, use shortcuts."));
            ui.label(
                RichText::new(tr("•  To stop: click  ■ Stop recording  in the small window at the top right of the screen, or press  Ctrl+Alt+R.")).strong(),
            );
            ui.label(tr("•  While recording, the mouse pointer is a cross."));
            ui.label(tr("•  What you type into password boxes is never recorded; the flow will ask for it when it runs."));
            ui.label(
                RichText::new(tr("•  Clicks on program buttons in the taskbar are NOT recorded. Open programs from Start or Search instead (recorded as the Windows key and Win+S), or start them in the flow with a Run program step."))
                    .strong()
                    .color(crate::look::err()),
            );
            ui.label(tr("•  Clicks on AutoFlowMate's own windows are not recorded."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("OK, start recording")).strong()).clicked() {
                    go = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    go = Some(false);
                }
            });
        });
        match go {
            Some(true) => {
                self.record_prompt = false;
                self.start_record(ctx);
            }
            Some(false) => self.record_prompt = false,
            None => {}
        }
    }

    fn poll_record(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.recording else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.recording = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        match result {
            Ok(steps) if steps.is_empty() => self.info(LogKind::Warn, "nothing was recorded"),
            Ok(steps) => {
                let n = steps.len();
                for s in steps {
                    self.add_step(s);
                }
                self.info(LogKind::Ok, format!("recorded {n} step(s); check them, then Validate"));
            }
            Err(e) => self.info(LogKind::Error, format!("recording failed: {e}")),
        }
    }

    fn is_paused(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.paused.is_some())
    }

    /// Continue a paused run, to the next step only or to the next breakpoint.
    fn resume(&mut self, step: bool, ctx: &egui::Context) {
        if let Some(r) = self.running.as_mut() {
            if r.paused.take().is_some() {
                r.debug.resume(step);
                if self.minimize_on_run && !self.dry_run && !step {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
        }
    }

    fn paused_window(&mut self, ctx: &egui::Context) {
        let Some((id, action, vars)) = self.running.as_ref().and_then(|r| r.paused.clone()) else { return };
        let mut act: Option<bool> = None;
        let mut stop = false;
        egui::Window::new(tr("⏸ Paused")).default_pos([380.0, 120.0]).default_width(380.0).show(ctx, |ui| {
            ui.label(RichText::new(format!("Before step {id}  ({} {action})", icons::action_icon(&action))).strong());
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("▶ Continue (F5)")).color(crate::look::ok())).clicked() {
                    act = Some(false);
                }
                if ui.button(tr("⏭ Step (F10)")).on_hover_text(tr("Run this step, then pause again")).clicked() {
                    act = Some(true);
                }
                if ui.button(RichText::new(tr("■ Stop")).color(crate::look::err())).clicked() {
                    stop = true;
                }
            });
            ui.separator();
            ui.weak(tr("Variables now:"));
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                egui::Grid::new("paused_vars").striped(true).num_columns(2).show(ui, |ui| {
                    for (k, v) in &vars {
                        ui.monospace(k);
                        let mut text = match v {
                            Value::String(s) => format!("\"{s}\""),
                            other => other.to_string(),
                        };
                        if text.chars().count() > 120 {
                            text = text.chars().take(120).collect::<String>() + "…";
                        }
                        ui.label(RichText::new(text).monospace()).on_hover_text(serde_json::to_string_pretty(v).unwrap_or_default());
                        ui.end_row();
                    }
                });
            });
        });
        if let Some(step) = act {
            self.resume(step, ctx);
        }
        if stop {
            self.stop();
        }
    }

    fn info(&mut self, kind: LogKind, msg: impl Into<String>) {
        self.log.push((kind, msg.into()));
    }

    fn sync_vars_buf(&mut self) {
        self.flow_vars_buf = serde_json::to_string_pretty(&self.flow.variables).unwrap_or_else(|_| "{}".into());
    }

    // ---------------------------------------------------------------- files

    fn open_path(&mut self, p: PathBuf) {
        match Flow::load(&p) {
            Ok(f) => {
                self.flow = f;
                self.info(LogKind::Info, format!("opened {}", p.display()));
                self.path = Some(p);
                self.dirty = false;
                self.sel = Selection::None;
                self.breakpoints.clear();
                self.sync_vars_buf();
                self.reset_history();
            }
            Err(e) => self.info(LogKind::Error, format!("{e:#}")),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Flow", &["json"]).pick_file() {
            self.open_path(p);
        }
    }

    fn save(&mut self, save_as: bool) {
        let path = match (&self.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("Flow", &["json"]).set_file_name(format!("{}.json", slug(&self.flow.name))).save_file(),
        };
        let Some(p) = path else { return };
        self.flow.assign_ids();
        match self.flow.save(&p) {
            Ok(()) => {
                self.info(LogKind::Ok, format!("saved {}", p.display()));
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.info(LogKind::Error, format!("save failed: {e:#}")),
        }
    }

    /// Start a new, unsaved flow from a template (the template itself is never changed).
    fn start_from_template(&mut self, p: PathBuf) {
        match Flow::load(&p) {
            Ok(f) => {
                self.new_flow();
                self.info(
                    LogKind::Info,
                    format!("{} \"{}\" - {}", tr("started from the template"), f.name, tr("set the values in the Variables tab, then Save")),
                );
                self.flow = f;
                self.dirty = true;
                self.sync_vars_buf();
                self.reset_history();
            }
            Err(e) => self.info(LogKind::Error, format!("{e:#}")),
        }
    }

    fn templates_window(&mut self, ctx: &egui::Context) {
        let Some(list) = &self.template_list else { return };
        let mut pick: Option<PathBuf> = None;
        let mut open = true;
        let mut close = false;
        egui::Window::new(tr("📋 New from template")).open(&mut open).default_width(640.0).default_height(520.0).collapsible(false).show(ctx, |ui| {
            ui.label(tr("Start from a ready-made flow. The template is copied; set its values in the Variables tab and save it under a new name."));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                if list.is_empty() {
                    ui.label(tr("No templates found."));
                }
                let mut shown_own = false;
                for t in list {
                    if t.own && !shown_own {
                        shown_own = true;
                        ui.separator();
                        ui.label(RichText::new(tr("Your templates")).strong());
                    }
                    ui.group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new(tr("Use")).strong()).clicked() {
                                pick = Some(t.path.clone());
                            }
                            ui.label(RichText::new(&t.name).strong());
                        });
                        if !t.description.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&t.description).weak()).wrap());
                        }
                    });
                }
            });
            ui.horizontal(|ui| {
                if let Some(d) = autoflow_core::templates::user_dir() {
                    if ui.button(tr("📂 Open my templates folder")).clicked() {
                        let _ = std::fs::create_dir_all(&d);
                        let _ = std::process::Command::new("explorer").arg(&d).spawn();
                    }
                }
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if !open || close {
            self.template_list = None;
        }
        if let Some(p) = pick {
            self.template_list = None;
            self.guard(Pending::Template(p), ctx);
        }
    }

    fn save_template_window(&mut self, ctx: &egui::Context) {
        let Some(name) = self.template_name.as_mut() else { return };
        let mut done: Option<bool> = None;
        egui::Window::new(tr("💾 Save as template")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(tr("Save this flow as a template you can start new flows from (File > New from template)."));
            ui.horizontal(|ui| {
                ui.label(tr("Name"));
                ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
            });
            ui.weak(tr("Tip: say what to change in the flow's description; it is shown in the template list."));
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("💾 Save")).strong()).clicked() {
                    done = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let name = self.template_name.take().unwrap_or_default();
                let mut flow = self.flow.clone();
                flow.assign_ids();
                if !name.trim().is_empty() {
                    flow.name = name.trim().to_string();
                }
                match autoflow_core::templates::save_own(&flow.to_json(), &flow.name) {
                    Ok(p) => self.info(LogKind::Ok, format!("{} {}", tr("saved the template"), p.display())),
                    Err(e) => self.info(LogKind::Error, format!("{e:#}")),
                }
            }
            Some(false) => self.template_name = None,
            None => {}
        }
    }

    fn new_flow(&mut self) {
        self.flow = Flow::default();
        self.path = None;
        self.dirty = false;
        self.sel = Selection::None;
        self.breakpoints.clear();
        self.sync_vars_buf();
        self.reset_history();
    }

    // ---------------------------------------------------------------- run

    fn validate(&mut self) -> bool {
        let (parsed, issues) = validate_json(&self.flow.to_json());
        for i in &issues {
            let kind = if i.severity == Severity::Error { LogKind::Error } else { LogKind::Warn };
            self.info(kind, format!("{}: {}", i.location, i.message));
        }
        let ok = parsed.is_some() && !issues.iter().any(|i| i.severity == Severity::Error);
        if ok {
            self.info(LogKind::Ok, format!("validation passed ({} warnings)", issues.len()));
        }
        ok
    }

    fn run(&mut self, ctx: &egui::Context, stepping: bool) {
        if self.running.is_some() {
            return;
        }
        self.flow.assign_ids();
        self.log.clear();
        self.failed_steps.clear();
        if !self.validate() {
            self.info(LogKind::Error, "fix the errors above before running");
            return;
        }
        let flow = self.flow.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flow_dir = self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        // Timing and fail-safe come from flow.settings (edited in the toolbar / flow settings).
        let debug = Arc::new(Debugger::new(self.breakpoints.iter().cloned(), stepping));
        let opts = RunOptions { dry_run: self.dry_run, stop: stop.clone(), flow_dir, debug: Some(debug.clone()), ..RunOptions::default() };
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let mut sink = |e: Event| {
                let _ = tx.send(e);
                repaint.request_repaint();
            };
            run_flow(&flow, Map::new(), &opts, &mut sink);
        });
        if self.minimize_on_run && !self.dry_run && !stepping {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.info(
            LogKind::Info,
            match (self.dry_run, stepping) {
                (_, true) => "step-by-step run started: F10 runs the next step, F5 continues",
                (true, _) => "dry run started",
                _ => "run started",
            },
        );
        self.running = Some(Running { rx, stop, current: None, debug, paused: None });
    }

    fn poll_run(&mut self, ctx: &egui::Context) {
        let Some(r) = self.running.as_mut() else { return };
        let mut finished = false;
        let mut out = vec![];
        while let Ok(e) = r.rx.try_recv() {
            match e {
                Event::StepStarted { id, action, depth } => {
                    out.push((LogKind::Info, format!("{}▶ [{id}] {action}", "   ".repeat(depth))));
                    r.current = Some(id);
                }
                Event::StepFinished { id, ok: false, error, .. } => {
                    out.push((LogKind::Error, format!("✖ [{id}] {}", error.unwrap_or_default())));
                    self.failed_steps.push(id);
                }
                Event::StepFinished { .. } => {}
                Event::Log { level, message } => {
                    let k = match level {
                        autoflow_core::actions::LogLevel::Info => LogKind::Info,
                        autoflow_core::actions::LogLevel::Warn => LogKind::Warn,
                        autoflow_core::actions::LogLevel::Error => LogKind::Error,
                    };
                    out.push((k, format!("    {message}")));
                }
                Event::Finished { success, message } => {
                    out.push((if success { LogKind::Ok } else { LogKind::Error }, format!("{} {message}", if success { "✔ SUCCESS" } else { "✖ FAILED" })));
                    finished = true;
                }
                Event::Paused { id, action, variables } => {
                    out.push((LogKind::Warn, format!("⏸ paused before [{id}] {action}")));
                    r.current = Some(id.clone());
                    r.paused = Some((id, action, variables));
                    // Show the editor so the user can look around.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        self.log.extend(out);
        if finished {
            self.running = None;
            if self.minimize_on_run {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    fn stop(&mut self) {
        if let Some(r) = &mut self.running {
            r.stop.store(true, Ordering::Relaxed);
            r.paused = None;
            r.debug.resume(false);
        }
    }

    // ---------------------------------------------------------------- pick

    fn start_pick(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let res = (|| -> anyhow::Result<(i32, i32, Option<String>)> {
                let (x, y) = autoflow_core::input::Input::new()?.position()?;
                #[cfg(windows)]
                let info = autoflow_core::uia::Uia::new().and_then(|u| u.inspect_point(x, y)).ok().map(|i| serde_json::to_string(&i).unwrap_or_default());
                #[cfg(not(windows))]
                let info = None;
                Ok((x, y, info))
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(res);
            repaint.request_repaint();
        });
        self.pick_rx = Some(rx);
        self.info(LogKind::Info, "Pick: point at the target within 3 seconds…");
    }

    fn poll_pick(&mut self) {
        let Some(rx) = &self.pick_rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        self.pick_rx = None;
        let (x, y, info) = match res {
            Ok(v) => v,
            Err(e) => return self.info(LogKind::Error, format!("pick failed: {e}")),
        };
        let element: Option<Value> = info.and_then(|s| serde_json::from_str(&s).ok());
        self.info(LogKind::Info, format!("picked ({x}, {y}) {}", element.as_ref().map(|e| e.to_string()).unwrap_or_default()));

        // Apply to the selected step if it can use it.
        if let Selection::Step(p) = self.sel.clone() {
            if let Some(step) = tree::step_mut(&mut self.flow.steps, &p) {
                let mut v = serde_json::to_value(&*step).unwrap();
                let o = v.as_object_mut().unwrap();
                let mut applied = false;
                if o.contains_key("automation_id") {
                    if let Some(e) = &element {
                        for k in ["window", "automation_id", "control_type", "class_name"] {
                            o.insert(k.into(), e.get(k).cloned().unwrap_or(Value::String(String::new())));
                        }
                        let has_id = e.get("automation_id").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
                        o.insert("name".into(), if has_id { Value::String(String::new()) } else { e.get("name").cloned().unwrap_or_default() });
                        applied = true;
                    }
                } else if o.contains_key("x") && o.contains_key("y") {
                    o.insert("x".into(), Value::from(x));
                    o.insert("y".into(), Value::from(y));
                    applied = true;
                }
                if applied {
                    if let Ok(s) = serde_json::from_value::<Step>(v) {
                        *step = s;
                        self.dirty = true;
                        self.buffers_for = None;
                        self.info(LogKind::Ok, "applied to selected step");
                        return;
                    }
                }
            }
        }
        // Otherwise insert a new step.
        let step = match &element {
            Some(e) if e.get("automation_id").is_some() => {
                let mut t = serde_json::json!({"action": "ui_click", "invoke": true});
                let has_id = e.get("automation_id").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
                for k in ["window", "automation_id", "control_type"] {
                    t[k] = e.get(k).cloned().unwrap_or_default();
                }
                if !has_id {
                    t["name"] = e.get("name").cloned().unwrap_or_default();
                }
                serde_json::from_value::<Step>(t).ok()
            }
            _ => serde_json::from_value::<Step>(serde_json::json!({"action": "mouse_click", "x": x, "y": y})).ok(),
        };
        if let Some(s) = step {
            self.add_step(s);
        }
    }

    // ---------------------------------------------------------------- editing

    fn add_step(&mut self, step: Step) {
        let p = tree::insert(&mut self.flow.steps, &self.sel, step);
        self.flow.assign_ids();
        self.sel = Selection::Step(p);
        self.dirty = true;
    }

    fn apply(&mut self, op: TreeOp) {
        match op {
            TreeOp::Breakpoint(p) => self.toggle_breakpoint(&p),
            TreeOp::Select(s) => self.sel = s,
            TreeOp::Toggle(p) => {
                if let Some(s) = tree::step_mut(&mut self.flow.steps, &p) {
                    s.enabled = !s.enabled;
                    self.dirty = true;
                }
            }
            TreeOp::Move(p, delta) => {
                if let Some(list) = tree::list_mut(&mut self.flow.steps, &p.parents) {
                    let j = p.index as isize + delta;
                    if j >= 0 && (j as usize) < list.len() {
                        list.swap(p.index, j as usize);
                        self.sel = Selection::Step(StepPath { parents: p.parents, index: j as usize });
                        self.dirty = true;
                    }
                }
            }
            TreeOp::Delete(p) => {
                if let Some(list) = tree::list_mut(&mut self.flow.steps, &p.parents) {
                    if p.index < list.len() {
                        list.remove(p.index);
                        self.sel = Selection::None;
                        self.dirty = true;
                    }
                }
            }
            TreeOp::Duplicate(p) => {
                if let Some(list) = tree::list_mut(&mut self.flow.steps, &p.parents) {
                    if let Some(s) = list.get(p.index).cloned() {
                        let mut s = s;
                        autoflow_core::flow::visit_mut(std::slice::from_mut(&mut s), &mut |x| x.id.clear());
                        list.insert(p.index + 1, s);
                        self.flow.assign_ids();
                        self.sel = Selection::Step(StepPath { parents: p.parents, index: p.index + 1 });
                        self.dirty = true;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- UI pieces

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Wraps onto a second row when the window is narrow or the text is large.
        ui.horizontal_wrapped(|ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(tr("File"), |ui| {
                    if ui.button(tr("New")).clicked() {
                        self.guard(Pending::New, &ctx);
                        ui.close();
                    }
                    if ui
                        .button(tr("📋 New from template…"))
                        .on_hover_text(tr("Start from a ready-made flow: redaction, pseudonymisation, mail merge, invoices to Excel..."))
                        .clicked()
                    {
                        self.template_list = Some(autoflow_core::templates::list());
                        ui.close();
                    }
                    if ui.button(tr("Open…  (Ctrl+O)")).clicked() {
                        self.guard(Pending::Open(None), &ctx);
                        ui.close();
                    }
                    if ui.button(tr("Save  (Ctrl+S)")).clicked() {
                        self.save(false);
                        ui.close();
                    }
                    if ui.button(tr("Save as…")).clicked() {
                        self.save(true);
                        ui.close();
                    }
                    if ui.button(tr("Save as template…")).on_hover_text(tr("Keep this flow as a starting point for new flows")).clicked() {
                        self.template_name = Some(self.flow.name.clone());
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tr("⚙ Settings…")).on_hover_text(tr("AI and e-mail provider lists")).clicked() {
                        self.settings_ui = Some(settings::SettingsUi::new());
                        ui.close();
                    }
                });
                ui.menu_button(tr("Edit"), |ui| {
                    if ui.add_enabled(!self.history.undo.is_empty(), egui::Button::new(tr("Undo  (Ctrl+Z)"))).clicked() {
                        self.undo();
                        ui.close();
                    }
                    if ui.add_enabled(!self.history.redo.is_empty(), egui::Button::new(tr("Redo  (Ctrl+Y)"))).clicked() {
                        self.redo();
                        ui.close();
                    }
                    ui.separator();
                    let sel_step = if let Selection::Step(p) = &self.sel { Some(p.clone()) } else { None };
                    if ui.add_enabled(sel_step.is_some(), egui::Button::new(tr("Toggle breakpoint  (F9)"))).clicked() {
                        if let Some(p) = sel_step {
                            self.toggle_breakpoint(&p);
                        }
                        ui.close();
                    }
                    if ui.add_enabled(!self.breakpoints.is_empty(), egui::Button::new(tr("Clear all breakpoints"))).clicked() {
                        self.breakpoints.clear();
                        if let Some(r) = &self.running {
                            r.debug.breakpoints.lock().unwrap().clear();
                        }
                        ui.close();
                    }
                });
                ui.menu_button(tr("View"), |ui| {
                    let before = self.prefs.clone();
                    ui.label(RichText::new(tr("Text size")).strong());
                    for (scale, label) in look::SCALES {
                        ui.radio_value(&mut self.prefs.text_scale, scale, tr(label));
                    }
                    ui.separator();
                    ui.checkbox(&mut self.prefs.system_font, tr("Windows font (Segoe UI)")).on_hover_text(tr("Off = egui's built-in font"));
                    ui.checkbox(&mut self.prefs.one_category, tr("Palette: one category open at a time"));
                    ui.separator();
                    ui.label(RichText::new(tr("Language")).strong());
                    for (code, name) in i18n::available() {
                        ui.radio_value(&mut self.prefs.language, code, name);
                    }
                    ui.weak(tr("Language files are in the lang folder next to the program."));
                    if self.prefs != before {
                        if self.prefs.system_font != before.system_font {
                            look::apply_fonts(&ctx, &self.prefs);
                        }
                        if self.prefs.language != before.language {
                            if let Err(e) = i18n::set_language(&self.prefs.language) {
                                self.info(LogKind::Error, e);
                            }
                        }
                        look::apply_style(&ctx, &self.prefs);
                        self.prefs.save();
                    }
                });
                ui.menu_button(tr("Help"), |ui| {
                    if ui.button(tr("About AutoFlowMate")).clicked() {
                        self.show_about = true;
                        ui.close();
                    }
                    ui.hyperlink_to(tr("Project on GitHub"), GITHUB_URL);
                    ui.hyperlink_to(tr("Action reference"), format!("{GITHUB_URL}/blob/main/skills/autoflowmate/references/actions.md"));
                });
            });
            ui.separator();
            let running = self.running.is_some();
            let paused = self.is_paused();
            if ui.add_enabled(!self.history.undo.is_empty(), egui::Button::new("⟲")).on_hover_text(tr("Undo (Ctrl+Z)")).clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.history.redo.is_empty(), egui::Button::new("⟳")).on_hover_text(tr("Redo (Ctrl+Y)")).clicked() {
                self.redo();
            }
            ui.separator();
            let run_label = if paused { tr("▶ Continue (F5)") } else { tr("▶ Run (F5)") };
            if ui.add_enabled(!running || paused, egui::Button::new(RichText::new(run_label).color(crate::look::ok()))).clicked() {
                if paused {
                    self.resume(false, &ctx)
                } else {
                    self.run(&ctx, false)
                }
            }
            if ui
                .add_enabled(!running || paused, egui::Button::new(tr("⏭ Step (F10)")))
                .on_hover_text(tr("Run one step at a time, pausing before each (shows the variables). Breakpoints: F9 on a step."))
                .clicked()
            {
                if paused {
                    self.resume(true, &ctx)
                } else {
                    self.run(&ctx, true)
                }
            }
            if ui.add_enabled(running, egui::Button::new(RichText::new(tr("■ Stop")).color(crate::look::err()))).clicked() {
                self.stop();
            }
            let recording = self.recording.is_some();
            if ui
                .add_enabled(
                    !running && !recording,
                    egui::Button::new(RichText::new(if recording { tr("⏺ Recording…") } else { tr("⏺ Record") }).color(crate::look::err())),
                )
                .on_hover_text(tr(
                    "Record your clicks and typing as steps (inserted at the selection). Press Ctrl+Alt+R to stop. Typing in password boxes is never saved.",
                ))
                .clicked()
            {
                self.record_prompt = true;
            }
            ui.checkbox(&mut self.dry_run, tr("Dry run"));
            ui.checkbox(&mut self.minimize_on_run, tr("Minimize while running"));
            ui.separator();
            // Run-wide settings, saved in the flow (settings.*).
            let s = &mut self.flow.settings;
            let mut changed = false;
            ui.label(tr("Key delay")).on_hover_text(tr("settings.key_delay_ms: pause between keystrokes. Raise to 150-250 ms for slow apps such as Tally."));
            changed |= ui.add(egui::DragValue::new(&mut s.key_delay_ms).range(0..=2000).suffix(" ms").speed(5.0)).changed();
            ui.label(tr("Step delay"))
                .on_hover_text(tr("settings.step_delay_ms: pause after every action step (if / loops / try add none themselves; their inner steps do)."));
            changed |= ui.add(egui::DragValue::new(&mut s.step_delay_ms).range(0..=10000).suffix(" ms").speed(10.0)).changed();
            changed |= ui
                .checkbox(&mut s.failsafe, tr("Fail-safe"))
                .on_hover_text(format!(
                    "Abort the run when the mouse touches a screen corner (PyAutoGUI FAILSAFE). The hotkey {} always aborts.",
                    if s.abort_hotkey.is_empty() { "(none)" } else { &s.abort_hotkey }
                ))
                .changed();
            self.dirty |= changed;
            ui.separator();
            if ui.button(tr("⏰ Schedule…")).on_hover_text(tr("Run this flow automatically with Windows Task Scheduler")).clicked() {
                self.open_schedule();
            }
            if ui.button(tr("✔ Validate")).clicked() {
                self.log.clear();
                self.validate();
            }
            let picking = self.pick_rx.is_some();
            if ui
                .add_enabled(!picking, egui::Button::new(if picking { tr("⌖ Picking…") } else { tr("⌖ Pick element") }))
                .on_hover_text(tr("After 3 s, capture the UI element / position under the mouse. Fills the selected ui_* or mouse step, or adds a new step."))
                .clicked()
            {
                self.start_pick(&ctx);
            }
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Actions"));
        ui.add(egui::TextEdit::singleline(&mut self.palette_filter).hint_text(tr("filter…")));
        ui.horizontal(|ui| {
            if ui.small_button(tr("⊟ Collapse all")).clicked() {
                self.palette_all = Some(false);
                self.open_category = None;
            }
            if ui.small_button(tr("⊞ Expand all")).on_hover_text(tr("Turns off “one category at a time” until you open a category")).clicked() {
                self.palette_all = Some(true);
            }
        });
        ui.weak(tr("Drag onto the flowchart, or click to insert at the selection"));
        ui.separator();
        let mut add: Option<String> = None;
        let filter = self.palette_filter.to_lowercase();
        egui::ScrollArea::vertical().id_salt("palette").show(ui, |ui| {
            let mut cats: Vec<&str> = vec![];
            for a in &self.catalog {
                if !cats.contains(&a.category) {
                    cats.push(a.category);
                }
            }
            for cat in cats {
                let items: Vec<&ActionInfo> = self
                    .catalog
                    .iter()
                    .filter(|a| a.category == cat && (filter.is_empty() || a.name.contains(&filter) || a.description.to_lowercase().contains(&filter)))
                    .collect();
                if items.is_empty() {
                    continue;
                }
                // Searching shows every match; otherwise one category at a time (View menu), or free.
                let open = if !filter.is_empty() {
                    Some(true)
                } else if let Some(all) = self.palette_all {
                    Some(all)
                } else if self.prefs.one_category {
                    Some(self.open_category.as_deref() == Some(cat))
                } else {
                    None
                };
                let resp = egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::category_icon(cat), tr(cat))).strong())
                    .id_salt(("palette_cat", cat))
                    .default_open(cat == "Flow")
                    .open(open)
                    .show(ui, |ui| {
                        let w = ui.available_width();
                        for a in items {
                            let r = ui
                                .add(
                                    egui::Button::new(format!("{}  {}", icons::action_icon(a.name), a.name))
                                        .right_text("")
                                        .min_size(egui::vec2(w, 24.0))
                                        .sense(egui::Sense::click_and_drag()),
                                )
                                .on_hover_text(tr(a.description));
                            r.dnd_set_drag_payload(DragItem::New(a.name.to_string()));
                            if r.clicked() {
                                add = Some(a.name.to_string());
                            }
                        }
                    });
                if resp.header_response.clicked() && filter.is_empty() {
                    let was_open = self.open_category.as_deref() == Some(cat);
                    self.open_category = if was_open { None } else { Some(cat.to_string()) };
                    // After "Expand all", clicking a header goes back to one-at-a-time.
                    self.palette_all = None;
                }
            }
        });
        // "Collapse all" applies once; "Expand all" stays until a header is clicked.
        if self.palette_all == Some(false) {
            self.palette_all = None;
        }
        if let Some(name) = add {
            if let Some(action) = Action::template(&name) {
                self.add_step(Step::new(action));
            }
        }
    }

    fn steps_view(&mut self, ui: &mut egui::Ui) {
        // Wraps onto a second row when the text is large, so nothing is pushed off screen.
        ui.horizontal_wrapped(|ui| {
            let title = if self.flow.name.is_empty() { tr("Untitled flow") } else { self.flow.name.clone() };
            ui.heading(title);
            if ui.small_button(tr("flow settings")).clicked() {
                self.sel = Selection::None;
            }
        });
        // View switch and zoom on their own row, so a long flow name never splits them.
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.view, View::Flowchart, tr("Flowchart"));
            ui.selectable_value(&mut self.view, View::List, tr("List"));
            ui.selectable_value(&mut self.view, View::Json, "JSON");
            if self.view == View::Flowchart {
                ui.separator();
                if ui.small_button("−").on_hover_text(tr("Zoom out")).clicked() {
                    self.zoom = (self.zoom - 0.1).max(0.4);
                }
                ui.label(format!("{:.0}%", self.zoom * 100.0));
                if ui.small_button("+").on_hover_text(tr("Zoom in")).clicked() {
                    self.zoom = (self.zoom + 0.1).min(2.0);
                }
            }
        });
        ui.separator();
        let current = self.running.as_ref().and_then(|r| r.current.clone());
        let paused_at = self.running.as_ref().and_then(|r| r.paused.as_ref().map(|p| p.0.clone()));
        match self.view {
            View::Flowchart => {
                let mut ops = vec![];
                // Ctrl + mouse wheel zooms.
                let zd = ui.input(|i| if i.modifiers.command { i.zoom_delta() } else { 1.0 });
                if zd != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
                    self.zoom = (self.zoom * zd).clamp(0.4, 2.0);
                }
                egui::ScrollArea::both().id_salt("chart").auto_shrink([false, false]).show(ui, |ui| {
                    let chart = flowchart::Chart {
                        sel: &self.sel,
                        breakpoints: &self.breakpoints,
                        paused: paused_at.as_deref(),
                        current: current.as_deref(),
                        failed: &self.failed_steps,
                        zoom: self.zoom,
                        color: cat_color,
                        summary: summarize,
                    };
                    chart.show(ui, &self.flow.steps, &mut ops);
                });
                for op in ops {
                    match op {
                        flowchart::ChartOp::Select(s) => self.apply(TreeOp::Select(s)),
                        flowchart::ChartOp::Toggle(p) => self.apply(TreeOp::Toggle(p)),
                        flowchart::ChartOp::Delete(p) => self.apply(TreeOp::Delete(p)),
                        flowchart::ChartOp::Duplicate(p) => self.apply(TreeOp::Duplicate(p)),
                        flowchart::ChartOp::Drop(item, lp, idx) => self.drop_item(item, lp, idx),
                        flowchart::ChartOp::Breakpoint(p) => self.toggle_breakpoint(&p),
                    }
                }
            }
            View::List => {
                let mut ops = vec![];
                egui::ScrollArea::vertical().id_salt("steps").auto_shrink([false, false]).show(ui, |ui| {
                    if self.flow.steps.is_empty() {
                        ui.add_space(20.0);
                        ui.weak(tr("No steps yet. Pick an action on the left, or use ⌖ Pick element."));
                    }
                    let ctx = RowCtx { sel: &self.sel, current: current.as_deref(), failed: &self.failed_steps, breakpoints: &self.breakpoints };
                    draw_list(ui, &self.flow.steps, &vec![], 0, &ctx, &mut ops);
                });
                for op in ops {
                    self.apply(op);
                }
            }
            View::Json => {
                let mut text = self.flow.to_json();
                egui::ScrollArea::vertical().id_salt("json").show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                });
            }
        }
    }

    /// Handle a drag-and-drop onto a flowchart connector.
    fn drop_item(&mut self, item: DragItem, lp: ListPath, idx: usize) {
        match item {
            DragItem::New(name) => {
                if let Some(action) = Action::template(&name) {
                    self.sel = Selection::Gap(lp, idx);
                    self.add_step(Step::new(action));
                }
            }
            DragItem::Move(from) => match tree::move_step(&mut self.flow.steps, &from, &lp, idx) {
                Some(p) => {
                    self.sel = Selection::Step(p);
                    self.dirty = true;
                    self.buffers_for = None;
                }
                None => self.info(LogKind::Warn, "a step cannot be moved inside itself"),
            },
        }
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        // Selecting a step jumps to the Step tab.
        if self.sel != self.last_sel {
            if matches!(self.sel, Selection::Step(_)) {
                self.right_tab = RightTab::Step;
            }
            self.last_sel = self.sel.clone();
        }
        ui.horizontal(|ui| {
            let n_vars = self.flow.variables.len();
            ui.selectable_value(&mut self.right_tab, RightTab::Step, RichText::new(tr("⚙ Step")).strong());
            ui.selectable_value(&mut self.right_tab, RightTab::Variables, RichText::new(format!("𝑥 {} ({n_vars})", tr("Variables"))).strong());
            ui.selectable_value(&mut self.right_tab, RightTab::Flow, RichText::new(tr("📄 Flow")).strong());
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("props").auto_shrink([false, false]).show(ui, |ui| match self.right_tab {
            RightTab::Step => match self.sel.clone() {
                Selection::Step(p) => self.step_properties(ui, &p),
                Selection::List(_) | Selection::Gap(..) => {
                    ui.heading(tr("Insertion point"));
                    ui.label(tr("Click an action in the palette to insert it here, or drag one onto any connector."));
                }
                Selection::None => {
                    ui.add_space(20.0);
                    ui.weak(tr("Select a step in the flowchart to edit it here."));
                }
            },
            RightTab::Variables => {
                let dir = self.flow_dir();
                if variables::show(ui, &mut self.flow, &mut self.vars_ui, dir.as_deref()) {
                    self.dirty = true;
                    self.buffers_for = None;
                }
            }
            RightTab::Flow => self.flow_properties(ui),
        });
    }

    /// Run one step on its own with the flow's variables; returns its output variables.
    fn try_step(&self, p: &StepPath) -> Result<Vec<(String, String)>, String> {
        let step = tree::step_ref(&self.flow.steps, p).cloned().ok_or("step not found")?;
        let v = serde_json::to_value(&step).unwrap_or_default();
        let outputs: Vec<String> = v
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(k, _)| k.ends_with("_var") || (matches!(step.action.name(), "set_variable" | "increment" | "list_append") && *k == "name"))
                    .filter_map(|(_, x)| x.as_str().map(str::to_string))
                    .filter(|n| !n.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let flow = Flow { variables: self.flow.variables.clone(), variable_types: self.flow.variable_types.clone(), steps: vec![step], ..Flow::default() };
        let opts = RunOptions { flow_dir: self.flow_dir().unwrap_or_default(), failsafe: Some(false), ..RunOptions::default() };
        let r = run_flow(&flow, Map::new(), &opts, &mut |_| {});
        if !r.success {
            let mut m = r.message;
            if m.contains("unknown variable") {
                m.push_str(" (it is set by an earlier step: give it a sample value in the Variables tab to try this step)");
            }
            return Err(m);
        }
        Ok(outputs
            .into_iter()
            .map(|n| {
                let shown = match r.variables.get(&n) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => serde_json::to_string_pretty(other).unwrap_or_default(),
                    None => "(not set)".into(),
                };
                (n, shown)
            })
            .collect())
    }

    fn flow_dir(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Variables a field can use: declared ones (with types) and those created by steps.
    fn var_infos(&self) -> Vec<form::VarInfo> {
        autoflow_core::validate::defined_variables(&self.flow)
            .into_iter()
            .map(|n| {
                let t = self.flow.variable_types.get(&n).copied().or_else(|| self.flow.variables.get(&n).map(autoflow_core::flow::VarType::guess));
                (n, t)
            })
            .collect()
    }

    fn step_properties(&mut self, ui: &mut egui::Ui, p: &StepPath) {
        if self.buffers_for.as_ref() != Some(p) {
            self.json_buffers.clear();
            self.buffers_for = Some(p.clone());
            self.step_error = None;
        }
        let Some(step) = tree::step_ref(&self.flow.steps, p) else {
            self.sel = Selection::None;
            return;
        };
        let name = step.action.name();
        let category = step.action.category();
        let desc = self.catalog.iter().find(|a| a.name == name).map(|a| a.description).unwrap_or("");
        ui.heading(RichText::new(format!("{}  {name}", icons::action_icon(name))).strong());
        ui.label(tr(desc));
        ui.separator();

        let mut v = serde_json::to_value(step).unwrap();
        // Make sure common fields are always visible.
        if let Some(o) = v.as_object_mut() {
            for (k, d) in [
                ("id", Value::from("")),
                ("comment", Value::from("")),
                ("on_error", Value::from("stop")),
                ("retries", Value::from(0)),
                ("retry_delay_ms", Value::from(1000)),
                ("delay_before_ms", Value::from(0)),
                ("delay_after_ms", Value::from(0)),
            ] {
                o.entry(k).or_insert(d);
            }
        }
        let obj = v.as_object_mut().unwrap();
        // Action fields first, common fields after.
        let common = ["id", "enabled", "comment", "on_error", "retries", "retry_delay_ms", "delay_before_ms", "delay_after_ms"];
        let mut action_part: Map<String, Value> = obj.iter().filter(|(k, _)| !common.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut common_part: Map<String, Value> = obj.iter().filter(|(k, _)| common.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();

        // Action-specific field help first, generic help (common fields, condition) as fallback.
        let mut help: HashMap<String, String> =
            autoflow_core::flow::action_field_help(name).into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect();
        for (k, v) in &self.help {
            help.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let vars = self.var_infos();
        let dir = self.flow_dir();
        let table = if matches!(name, "read_excel" | "read_csv") { Some(self.table_info(&action_part, name)) } else { None };
        let mut form = form::Form {
            help: &help,
            json_buffers: &mut self.json_buffers,
            action: name.to_string(),
            vars: &vars,
            flow_dir: dir.as_deref(),
            new_var: None,
            table: table.as_ref(),
            secret_request: None,
            lists: &self.lists,
            cache: &mut self.cache,
            ctx: Map::new(),
            open_settings: false,
            fields: self.schema.fields(name),
        };
        let mut changed = form.object(ui, "a", &mut action_part);
        ui.add_space(8.0);
        egui::CollapsingHeader::new(tr("Step settings (id, errors, retries, delays)")).default_open(false).show(ui, |ui| {
            changed |= form.object(ui, "c", &mut common_part);
        });
        // "➕ New variable…" chosen in a field: declare it now.
        if let Some(name) = form.secret_request.take() {
            self.secret_dialog = Some(SecretDialog { name, first: String::new(), second: String::new(), error: None });
        }
        if form.open_settings {
            self.settings_ui = Some(settings::SettingsUi::new());
        }
        if let Some((vname, vtype)) = form.new_var.take() {
            variables::add_variable(&mut self.flow, &vname, vtype);
            self.vars_ui.highlight = Some(vname.clone());
            self.info(LogKind::Ok, format!("created variable `{vname}` ({}) - set its value in the Variables tab", vtype.label()));
            self.dirty = true;
        }

        if changed {
            let mut merged = common_part;
            merged.extend(action_part);
            match serde_json::from_value::<Step>(Value::Object(merged)) {
                Ok(new) => {
                    if let Some(s) = tree::step_mut(&mut self.flow.steps, p) {
                        *s = new;
                        self.dirty = true;
                        self.step_error = None;
                    }
                }
                Err(e) => self.step_error = Some(e.to_string()),
            }
        }
        if let Some(e) = &self.step_error {
            ui.colored_label(crate::look::err(), e);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(tr("⬆ Up")).clicked() {
                self.apply(TreeOp::Move(p.clone(), -1));
            }
            if ui.button(tr("⬇ Down")).clicked() {
                self.apply(TreeOp::Move(p.clone(), 1));
            }
            if ui.button(tr("⧉ Duplicate")).clicked() {
                self.apply(TreeOp::Duplicate(p.clone()));
            }
            if ui.button(RichText::new(tr("🗑 Delete")).color(crate::look::err())).clicked() {
                self.apply(TreeOp::Delete(p.clone()));
            }
        });
        if matches!(category, "Text" | "Date & Math") || matches!(name, "set_variable" | "increment" | "list_append" | "redact_text") {
            ui.add_space(6.0);
            if ui
                .button(RichText::new(tr("▶ Try it")).strong())
                .on_hover_text(tr("Run just this step now, with the flow's variables, and show what it gives. Nothing else in the flow runs."))
                .clicked()
            {
                let r = self.try_step(p);
                self.try_result = Some((p.clone(), r));
            }
            if let Some((tp, r)) = &self.try_result {
                if tp == p {
                    match r {
                        Ok(vals) if vals.is_empty() => {
                            ui.colored_label(crate::look::ok(), tr("✔ ran (no output variable)"));
                        }
                        Ok(vals) => {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                for (k, v) in vals {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(format!("{k} =")).strong());
                                        ui.label(RichText::new(v).monospace().color(crate::look::ok()));
                                    });
                                }
                            });
                        }
                        Err(e) => {
                            ui.colored_label(crate::look::err(), format!("✖ {e}"));
                        }
                    }
                }
            }
        }
        if matches!(name, "read_excel" | "read_csv" | "read_pdf") {
            ui.add_space(6.0);
            if ui
                .button(tr("👁 Preview data"))
                .on_hover_text(tr("Read the file now with these settings and show the first rows (what the flow will get)"))
                .clicked()
            {
                self.preview_data(p);
            }
        }
        let targets_element = name.starts_with("browser_") && !matches!(name, "browser_open" | "browser_navigate" | "browser_eval" | "browser_close");
        if targets_element {
            ui.add_space(6.0);
            let busy = self.web_pick_rx.is_some();
            if ui
                .add_enabled(!busy, egui::Button::new(if busy { "🎯 Waiting for Ctrl+click in the browser…" } else { "🎯 Pick on page" }))
                .on_hover_text(tr("Opens the flow's page (from its browser_open step). Hold Ctrl and click the element: its id (or a selector) is filled in here. No developer tools needed."))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_web_pick(&ctx, p.clone());
            }
        }
        if name.starts_with("image_") {
            ui.add_space(6.0);
            let busy = self.snip_rx.is_some();
            if ui
                .add_enabled(!busy, egui::Button::new(if busy { "📷 Snipping…" } else { "📷 Snip image" }))
                .on_hover_text(tr("Opens the Windows snipping tool; the snip is saved under images/ next to the flow and set as `image`."))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_snip(&ctx, p.clone());
            }
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(tr("Step JSON")).default_open(false).show(ui, |ui| {
            if let Some(s) = tree::step_ref(&self.flow.steps, p) {
                let mut text = serde_json::to_string_pretty(s).unwrap_or_default();
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
            }
        });
    }

    fn flow_properties(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Flow settings"));
        ui.separator();
        egui::Grid::new("flowgrid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(tr("name"));
            self.dirty |= ui.text_edit_singleline(&mut self.flow.name).changed();
            ui.end_row();
            ui.label(tr("description"));
            self.dirty |= ui.add(egui::TextEdit::multiline(&mut self.flow.description).desired_rows(3)).changed();
            ui.end_row();
        });
        ui.add_space(4.0);
        if ui.link(format!("𝑥 {} variable(s) — edit in the Variables tab", self.flow.variables.len())).clicked() {
            self.right_tab = RightTab::Variables;
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Run settings")).strong());
        ui.weak(tr("Timing and safety for every run of this flow (PyAutoGUI PAUSE / FAILSAFE)."));
        let mut v = serde_json::to_value(&self.flow.settings).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            let help = self.schema.help("Settings").clone();
            let mut f = form::Form {
                help: &help,
                json_buffers: &mut self.settings_buf,
                action: String::new(),
                vars: &[],
                flow_dir: None,
                new_var: None,
                table: None,
                secret_request: None,
                lists: &self.lists,
                cache: &mut self.cache,
                ctx: Map::new(),
                open_settings: false,
                fields: self.schema.fields("Settings"),
            };
            if f.object(ui, "settings", o) {
                if let Ok(s) = serde_json::from_value(v) {
                    self.flow.settings = s;
                    self.dirty = true;
                }
            }
        }
        ui.add_space(10.0);
        self.triggers_ui(ui);
        ui.add_space(8.0);
        ui.label(format!("File: {}", self.path.as_ref().map(|p| p.display().to_string()).unwrap_or("(not saved)".into())));
    }

    /// Triggers: new files in a folder or new e-mail start the flow while `autoflow watch` runs.
    fn triggers_ui(&mut self, ui: &mut egui::Ui) {
        use autoflow_core::watch::{EmailTrigger, FileTrigger, Trigger};
        ui.label(RichText::new(tr("Triggers")).strong());
        ui.weak(tr("Start this flow automatically for each new file or e-mail. Run `autoflow watch flow.json`, or tick “Watch the flow's triggers” in ⏰ Schedule… (at log-on)."));
        let dir = self.flow_dir();
        let mut remove = None;
        let mut secret = None;
        for i in 0..self.flow.triggers.len() {
            let (title, def) = match &self.flow.triggers[i] {
                Trigger::File(_) => ("📂 New file", "FileTrigger"),
                Trigger::Email(_) => ("📨 New e-mail", "EmailTrigger"),
            };
            let help = self.schema.help(def).clone();
            egui::CollapsingHeader::new(RichText::new(format!("{title} (trigger {})", i + 1)).strong()).id_salt(("trigger", i)).default_open(true).show(
                ui,
                |ui| {
                    let mut v = serde_json::to_value(&self.flow.triggers[i]).unwrap_or_default();
                    let Some(o) = v.as_object_mut() else { return };
                    let kind = o.remove("kind").unwrap_or_default();
                    let mut f = form::Form {
                        help: &help,
                        json_buffers: &mut self.settings_buf,
                        action: format!("trigger_{}", kind.as_str().unwrap_or("")),
                        vars: &[],
                        flow_dir: dir.as_deref(),
                        new_var: None,
                        table: None,
                        secret_request: None,
                        lists: &self.lists,
                        cache: &mut self.cache,
                        ctx: Map::new(),
                        open_settings: false,
                        fields: self.schema.fields(def),
                    };
                    if f.object(ui, &format!("triggers.{i}"), o) {
                        o.insert("kind".into(), kind);
                        if let Ok(t) = serde_json::from_value(v) {
                            self.flow.triggers[i] = t;
                            self.dirty = true;
                        }
                    }
                    if let Some(n) = f.secret_request.take() {
                        secret.get_or_insert(n);
                    }
                    if ui.small_button(tr("🗑 Remove trigger")).clicked() {
                        remove = Some(i);
                    }
                },
            );
        }
        if let Some(i) = remove {
            self.flow.triggers.remove(i);
            self.settings_buf.clear();
            self.dirty = true;
        }
        if let Some(name) = secret {
            self.secret_dialog = Some(SecretDialog { name, first: String::new(), second: String::new(), error: None });
        }
        ui.horizontal(|ui| {
            if ui.button(tr("➕ File trigger")).clicked() {
                self.flow.triggers.push(Trigger::File(FileTrigger { folder: "inbox".into(), ..FileTrigger::default() }));
                self.dirty = true;
            }
            if ui.button(tr("➕ E-mail trigger")).clicked() {
                self.flow.triggers.push(Trigger::Email(EmailTrigger::default()));
                self.dirty = true;
            }
        });
    }

    // ---------------------------------------------------------------- branding

    /// The banner image (splash and About), loaded once.
    fn banner(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.icon_tex
            .get_or_insert_with(|| {
                let img = eframe::icon_data::from_png_bytes(SPLASH_PNG)
                    .map(|i| egui::ColorImage::from_rgba_unmultiplied([i.width as usize, i.height as usize], &i.rgba))
                    .unwrap_or_else(|_| egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
                ctx.load_texture("banner", img, egui::TextureOptions::LINEAR)
            })
            .clone()
    }

    /// Banner, version, tagline and GitHub link (splash and About).
    fn brand(ui: &mut egui::Ui, tex: &egui::TextureHandle, width: f32) {
        ui.vertical_centered(|ui| {
            let [w, h] = tex.size();
            let img = egui::Image::new(tex).fit_to_exact_size(egui::vec2(width, width * h as f32 / w.max(1) as f32)).corner_radius(10.0);
            ui.add(img);
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).weak());
            ui.add_space(6.0);
            ui.label(tr("Windows desktop & web automation (RPA) — flows as JSON"));
            ui.label(RichText::new(tr("Visual flowchart · CLI · AI-agent skill · scheduling")).weak());
            ui.add_space(8.0);
            ui.hyperlink_to(RichText::new(GITHUB_URL).size(14.0), GITHUB_URL);
        });
    }

    fn splash(&mut self, ctx: &egui::Context) {
        let Some(mut until) = self.splash_until else { return };
        let now = ctx.input(|i| i.time);
        if until < 0.0 {
            // Start the timer once the window is actually shown and focused
            // (eframe renders a few frames before the window becomes visible).
            let focused = ctx.input(|i| i.viewport().focused) == Some(true);
            if focused || now > 15.0 {
                until = now + SPLASH_SECONDS;
                self.splash_until = Some(until);
            } else {
                until = f64::INFINITY;
            }
        }
        let clicked = ctx.input(|i| i.pointer.any_click() || i.key_pressed(egui::Key::Escape));
        if now >= until || (clicked && now > until - SPLASH_SECONDS + 0.3) {
            self.splash_until = None;
            return;
        }
        let tex = self.banner(ctx);
        // Dim the editor behind the splash.
        let screen = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::NULL.with("splash_dim"))).rect_filled(
            screen,
            0.0,
            Color32::from_black_alpha(140),
        );
        egui::Area::new(egui::Id::NULL.with("splash")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            egui::Frame::window(ui.style()).inner_margin(egui::Margin::same(28)).corner_radius(16.0).show(ui, |ui| {
                ui.set_width(720.0);
                Self::brand(ui, &tex, 720.0);
                ui.add_space(10.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(tr("click anywhere to start")).small().weak()));
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let tex = self.banner(ctx);
        let mut open = true;
        egui::Window::new(tr("About AutoFlowMate")).open(&mut open).resizable(false).collapsible(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ctx,
            |ui| {
                ui.set_width(480.0);
                Self::brand(ui, &tex, 480.0);
                ui.separator();
                ui.label(RichText::new(tr("Shortcuts")).strong());
                ui.label(tr("F5 run / continue · F10 step · F9 breakpoint · Ctrl+Z / Ctrl+Y undo / redo"));
                ui.label(tr("Ctrl+S save · Ctrl+O open · Ctrl+wheel zoom flowchart"));
                ui.label(tr("Abort a run: move the mouse into a screen corner, or press the abort hotkey (default Ctrl+Alt+Q)"));
                ui.add_space(6.0);
                ui.label(RichText::new(tr("Built with")).strong());
                ui.label(RichText::new(tr("Rust · egui/eframe · enigo · uiautomation · headless_chrome · xcap · calamine · rust_xlsxwriter · lettre · imap · ureq · rusqlite · odbc-api · Windows OCR")).weak());
                ui.add_space(6.0);
                ui.label(RichText::new(tr("MIT licence: free to use, change and share, without warranty.")).weak());
            },
        );
        self.show_about = open;
    }

    // ---------------------------------------------------------------- scheduling

    fn open_schedule(&mut self) {
        if self.path.is_none() || self.dirty {
            self.save(false);
        }
        let Some(path) = self.path.clone() else {
            self.info(LogKind::Warn, "save the flow before scheduling it");
            return;
        };
        let mut days = [false; 7];
        days[autoflow_core::schedule::today_weekday() as usize] = true;
        let date = autoflow_core::vars::Vars::default().interpolate("${date}").unwrap_or_default();
        let mut s = SchedUi {
            name: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            kind: 0,
            time: "09:00".into(),
            date,
            days,
            day_of_month: 1,
            minutes: 30,
            tasks: vec![],
            message: None,
            watch: !self.flow.triggers.is_empty(),
            has_triggers: !self.flow.triggers.is_empty(),
        };
        refresh_tasks(&mut s, &path);
        self.sched = Some(s);
    }

    fn schedule_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else { return };
        let Some(mut s) = self.sched.take() else { return };
        let mut open = true;
        egui::Window::new(tr("⏰ Schedule this flow")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.label(RichText::new(path.display().to_string()).weak());
            ui.add_space(4.0);
            egui::Grid::new("schedgrid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(tr("Task name"));
                ui.text_edit_singleline(&mut s.name);
                ui.end_row();
                ui.label(tr("Run"));
                egui::ComboBox::from_id_salt("schedkind").selected_text(SCHED_KINDS[s.kind]).show_ui(ui, |ui| {
                    for (i, k) in SCHED_KINDS.iter().enumerate() {
                        ui.selectable_value(&mut s.kind, i, *k);
                    }
                });
                ui.end_row();
                match s.kind {
                    2 => {
                        ui.label(tr("Days"));
                        ui.horizontal(|ui| {
                            for (i, d) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
                                ui.checkbox(&mut s.days[i], *d);
                            }
                        });
                        ui.end_row();
                    }
                    3 => {
                        ui.label(tr("Day of month"));
                        ui.add(egui::DragValue::new(&mut s.day_of_month).range(1..=31));
                        ui.end_row();
                    }
                    4 => {
                        ui.label(tr("Date"));
                        ui.add(egui::TextEdit::singleline(&mut s.date).hint_text(tr("YYYY-MM-DD")).desired_width(110.0));
                        ui.end_row();
                    }
                    5 => {
                        ui.label(tr("Every"));
                        ui.add(egui::DragValue::new(&mut s.minutes).range(1..=1440).suffix(" min"));
                        ui.end_row();
                    }
                    _ => {}
                }
                if s.kind != 5 && s.kind != 6 {
                    ui.label(tr("Time"));
                    ui.add(egui::TextEdit::singleline(&mut s.time).hint_text(tr("HH:MM (24 h)")).desired_width(70.0));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr(
                    "Desktop flows run only while you are logged on with the screen unlocked. Each run is logged to logs\\<flow>.log next to the flow.",
                ))
                .small()
                .weak(),
            );
            if s.has_triggers {
                ui.checkbox(&mut s.watch, tr("Watch the flow's triggers (new files / e-mail)"))
                    .on_hover_text(tr("Runs `autoflow watch`: at log-on it keeps watching all day; on other schedules it checks once each time."));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("Create / replace task")).strong()).clicked() {
                    s.message = Some(create_task(&s, &path));
                    refresh_tasks(&mut s, &path);
                }
                if ui.button(tr("Refresh")).clicked() {
                    refresh_tasks(&mut s, &path);
                }
                if ui.button(tr("Open log")).clicked() {
                    let log = autoflow_core::schedule::log_path(&path);
                    if log.exists() {
                        let _ = std::process::Command::new("notepad").arg(&log).spawn();
                    } else {
                        s.message = Some((false, "no log yet - the flow has not run on a schedule".into()));
                    }
                }
            });
            if let Some((ok, m)) = &s.message {
                ui.colored_label(if *ok { crate::look::ok() } else { crate::look::err() }, m);
            }
            ui.separator();
            ui.label(RichText::new(tr("Tasks for this flow")).strong());
            if s.tasks.is_empty() {
                ui.weak(tr("none"));
            }
            let mut action: Option<(bool, String)> = None;
            for t in &s.tasks {
                let when = t.description.split(" | ").nth(1).unwrap_or("");
                ui.label(RichText::new(format!("{}  —  {when}", t.name)).strong());
                ui.weak(format!(
                    "next: {}    last: {} ({})",
                    if t.next_run.is_empty() { "-" } else { &t.next_run },
                    if t.last_run.is_empty() { "-" } else { &t.last_run },
                    t.last_result_text()
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr("Run now")).clicked() {
                        action = Some((true, t.name.clone()));
                    }
                    if ui.button(tr("Delete")).clicked() {
                        action = Some((false, t.name.clone()));
                    }
                });
                ui.add_space(4.0);
            }
            if let Some((run, name)) = action {
                let r = if run { autoflow_core::schedule::run_now(&name) } else { autoflow_core::schedule::remove(&name) };
                s.message = Some(match r {
                    Ok(()) => (true, format!("{} \"{name}\"", if run { "started" } else { "deleted" })),
                    Err(e) => (false, e.to_string()),
                });
                if run {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                }
                refresh_tasks(&mut s, &path);
            }
        });
        if open {
            self.sched = Some(s);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            let (x, y) = autoflow_core::failsafe::cursor().unwrap_or((0, 0));
            ui.monospace(format!("Mouse X {x:>5}  Y {y:>5}"));
            if autoflow_core::failsafe::corner_hit(x, y) {
                ui.colored_label(crate::look::err(), tr("fail-safe corner"));
            }
            ui.separator();
            let s = &self.flow.settings;
            let mut abort = vec![];
            if s.failsafe {
                abort.push("mouse to a screen corner".to_string());
            }
            if !s.abort_hotkey.is_empty() {
                abort.push(s.abort_hotkey.clone());
            }
            ui.weak(if abort.is_empty() { "Abort: Stop button only".to_string() } else { format!("Abort: {}", abort.join(" or ")) });
            ui.separator();
            ui.weak(format!("key delay {} ms · input pause {} ms · step delay {} ms", s.key_delay_ms, s.input_pause_ms, s.step_delay_ms));
        });
    }

    /// Use the Windows snipping tool, then save the clipboard image next to the flow.
    fn start_snip(&mut self, ctx: &egui::Context, p: StepPath) {
        let dir = self.path.as_ref().and_then(|f| f.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let id = tree::step_ref(&self.flow.steps, &p).map(|s| s.id.clone()).unwrap_or_default();
        let name = format!("images/{}_{}.png", if id.is_empty() { "img".into() } else { id }, chrono_stamp());
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let res = (|| -> anyhow::Result<String> {
                let mut cb = arboard::Clipboard::new()?;
                let before = cb.get_image().ok().map(|i| i.bytes.len());
                let _ = cb.clear();
                std::process::Command::new("explorer").arg("ms-screenclip:").spawn()?;
                let start = std::time::Instant::now();
                while start.elapsed() < std::time::Duration::from_secs(60) {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    if let Ok(img) = cb.get_image() {
                        if Some(img.bytes.len()) != before || before.is_none() {
                            let out = dir.join(&name);
                            std::fs::create_dir_all(out.parent().unwrap())?;
                            autoflow_core::vision::save_rgba_png(&out, img.width as u32, img.height as u32, img.bytes.into_owned())?;
                            return Ok(name.clone());
                        }
                    }
                }
                anyhow::bail!("no snip within 60 s")
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(res);
            repaint.request_repaint();
        });
        self.snip_rx = Some((p, rx));
        self.info(LogKind::Info, "Snip: drag a rectangle around the target (Esc cancels)…");
    }

    /// Flow variables with their initial values resolved (for previews and pickers).
    fn resolved_vars(&self) -> autoflow_core::vars::Vars {
        let mut vars = autoflow_core::vars::Vars { map: Map::new(), flow_dir: self.flow_dir().map(|d| d.display().to_string()).unwrap_or_default() };
        for (k, v) in &self.flow.variables {
            if let Ok(r) = vars.resolve_value(v) {
                vars.map.insert(k.clone(), r);
            }
        }
        vars
    }

    /// Resolve a path field like the engine does (variables, relative to the flow folder).
    fn resolve_path(&self, raw: &str) -> Option<PathBuf> {
        let t = self.resolved_vars().interpolate(raw).ok()?;
        if t.trim().is_empty() {
            return None;
        }
        let p = PathBuf::from(t.trim());
        Some(if p.is_absolute() { p } else { self.flow_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default()).join(p) })
    }

    /// Sheets and headers of the file a read_excel / read_csv step uses (cached by file time).
    fn table_info(&mut self, fields: &Map<String, Value>, action: &str) -> form::TableInfo {
        let s = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let Some(path) = self.resolve_path(&s("path")) else { return form::TableInfo::default() };
        let sheet = self.resolved_vars().interpolate(&s("sheet")).unwrap_or_default();
        let delim = s("delimiter");
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let key = format!("{}|{action}|{sheet}|{delim}", path.display());
        if let Some((m, info)) = self.table_cache.get(&key) {
            if *m == modified {
                return info.clone();
            }
        }
        let mut info = form::TableInfo::default();
        if !path.exists() {
            info.error = Some(format!("file not found: {}", path.display()));
        } else if action == "read_excel" {
            match autoflow_core::data::sheet_names(&path) {
                Ok(names) => info.sheets = names,
                Err(e) => info.error = Some(e.to_string()),
            }
            if let Ok((_, raw)) = autoflow_core::data::read_excel_raw(&path, &sheet, "") {
                info.headers = autoflow_core::data::headers(&raw);
            }
        } else if let Ok(raw) = autoflow_core::data::read_csv_raw(&path, &delim) {
            info.headers = autoflow_core::data::headers(&raw);
        }
        self.table_cache.insert(key, (modified, info.clone()));
        info
    }

    /// Run the selected data step's import now and show the result.
    fn preview_data(&mut self, p: &StepPath) {
        let Some(step) = tree::step_ref(&self.flow.steps, p) else { return };
        let vars = self.resolved_vars();
        let txt = |s: &str| vars.interpolate(s).unwrap_or_else(|_| s.to_string());
        let result: Result<(String, Value), String> = (|| match &step.action {
            Action::ReadExcel(a) => {
                let path = self.resolve_path(&a.path).ok_or("set the workbook path first")?;
                let (used, raw) = autoflow_core::data::read_excel_raw(&path, &txt(&a.sheet), &txt(&a.range)).map_err(|e| e.to_string())?;
                let v = autoflow_core::data::shape(raw, a.has_header, &txt(&a.column), a.skip_empty).map_err(|e| e.to_string())?;
                Ok((format!("{} [{used}]", path.display()), v))
            }
            Action::ReadCsv(a) => {
                let path = self.resolve_path(&a.path).ok_or("set the CSV path first")?;
                let raw = autoflow_core::data::read_csv_raw(&path, &txt(&a.delimiter)).map_err(|e| e.to_string())?;
                let v = autoflow_core::data::shape(raw, a.has_header, &txt(&a.column), a.skip_empty).map_err(|e| e.to_string())?;
                Ok((path.display().to_string(), v))
            }
            Action::ReadPdf(a) => {
                let path = self.resolve_path(&a.path).ok_or("set the PDF path first")?;
                let (text, n) = autoflow_core::data::read_pdf(&path, &txt(&a.pages), a.stream_order).map_err(|e| e.to_string())?;
                Ok((format!("{} ({n} pages)", path.display()), Value::String(text)))
            }
            _ => Err("not a data step".into()),
        })();
        self.preview = Some(match result {
            Ok((title, v)) => (title, Ok(v)),
            Err(e) => ("Preview".into(), Err(e)),
        });
    }

    fn secret_window(&mut self, ctx: &egui::Context) {
        let Some(d) = self.secret_dialog.as_mut() else { return };
        let mut open = true;
        let mut done: Option<String> = None;
        egui::Window::new(tr("🔑 Store password")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ctx,
            |ui| {
                ui.label(format!("Password for `{}`", d.name));
                ui.weak(tr("Saved in Windows Credential Manager (AutoFlowMate). It is never written to the flow file or the log."));
                ui.weak(tr("Gmail: create an App Password (Google Account → Security → 2-Step Verification → App passwords)."));
                ui.add_space(6.0);
                egui::Grid::new("secret_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr("Password"));
                    ui.add_sized([240.0, 22.0], egui::TextEdit::singleline(&mut d.first).password(true));
                    ui.end_row();
                    ui.label(tr("Again"));
                    ui.add_sized([240.0, 22.0], egui::TextEdit::singleline(&mut d.second).password(true));
                    ui.end_row();
                });
                if let Some(e) = &d.error {
                    ui.colored_label(crate::look::err(), e);
                }
                ui.horizontal(|ui| {
                    if ui.button(RichText::new(tr("Store")).strong()).clicked() {
                        if d.first.is_empty() {
                            d.error = Some("enter the password".into());
                        } else if d.first != d.second {
                            d.error = Some("the two entries differ".into());
                        } else {
                            match autoflow_core::mail::secrets::set(&d.name, &d.first) {
                                Ok(()) => done = Some(d.name.clone()),
                                Err(e) => d.error = Some(e.to_string()),
                            }
                        }
                    }
                    if autoflow_core::mail::secrets::exists(&d.name) && ui.button(tr("Delete stored password")).clicked() {
                        match autoflow_core::mail::secrets::delete(&d.name) {
                            Ok(()) => done = Some(format!("{} (deleted)", d.name)),
                            Err(e) => d.error = Some(e.to_string()),
                        }
                    }
                });
            },
        );
        if let Some(name) = done {
            self.info(LogKind::Ok, format!("password `{name}` stored in Windows Credential Manager"));
            self.secret_dialog = None;
        } else if !open {
            self.secret_dialog = None;
        }
    }

    fn preview_window(&mut self, ctx: &egui::Context) {
        let Some((title, res)) = &self.preview else { return };
        let mut open = true;
        egui::Window::new(tr("👁 Data preview")).open(&mut open).default_size([760.0, 420.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(title).weak());
            ui.separator();
            match res {
                Err(e) => {
                    ui.colored_label(crate::look::err(), e);
                }
                Ok(Value::String(text)) => {
                    ui.label(format!("{} characters", text.chars().count()));
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        let mut t = text.chars().take(20_000).collect::<String>();
                        ui.add(egui::TextEdit::multiline(&mut t).code_editor().desired_width(f32::INFINITY));
                    });
                }
                Ok(Value::Array(items)) => {
                    let n = items.len();
                    ui.label(format!("{n} item(s){}", if n > 50 { " — showing the first 50" } else { "" }));
                    egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                        let first = items.first();
                        let headers: Vec<String> = match first {
                            Some(Value::Object(o)) => o.keys().cloned().collect(),
                            Some(Value::Array(a)) => (0..a.len()).map(|i| format!("{}", i + 1)).collect(),
                            _ => vec!["value".into()],
                        };
                        egui::Grid::new("preview_grid").striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                            ui.strong("#");
                            for h in &headers {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for (i, item) in items.iter().take(50).enumerate() {
                                ui.weak(format!("{}", i + 1));
                                match item {
                                    Value::Object(o) => {
                                        for h in &headers {
                                            ui.label(o.get(h).map(autoflow_core::vars::to_text).unwrap_or_default());
                                        }
                                    }
                                    Value::Array(a) => {
                                        for j in 0..headers.len() {
                                            ui.label(a.get(j).map(autoflow_core::vars::to_text).unwrap_or_default());
                                        }
                                    }
                                    other => {
                                        ui.label(autoflow_core::vars::to_text(other));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
                Ok(other) => {
                    ui.label(other.to_string());
                }
            }
            ui.add_space(6.0);
            ui.weak(tr("Records are used in loops as ${row.Header}; a single column gives a plain list for for_each."));
        });
        if !open {
            self.preview = None;
        }
    }

    /// URL of the flow's first browser_open step, with flow variables filled in.
    fn flow_start_url(&self) -> String {
        let mut url = String::new();
        autoflow_core::flow::visit(&self.flow.steps, &mut |s| {
            if url.is_empty() {
                if let Action::BrowserOpen(a) = &s.action {
                    url = a.url.clone();
                }
            }
        });
        let mut vars = autoflow_core::vars::Vars { map: Map::new(), flow_dir: self.flow_dir().map(|d| d.display().to_string()).unwrap_or_default() };
        for (k, v) in &self.flow.variables {
            if let Ok(r) = vars.resolve_value(v) {
                vars.map.insert(k.clone(), r);
            }
        }
        let url = vars.interpolate(&url).unwrap_or(url);
        let base = self.flow_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if url.trim().is_empty() {
            String::new()
        } else {
            autoflow_core::web::to_url(&url, &base)
        }
    }

    fn start_web_pick(&mut self, ctx: &egui::Context, p: StepPath) {
        let url = self.flow_start_url();
        if url.is_empty() {
            self.info(LogKind::Warn, "add a browser_open step with the page URL first - the picker opens that page");
            return;
        }
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let r = autoflow_core::web::pick_element(&url, "edge", 300_000, &|| false).map_err(|e| e.to_string());
            let _ = tx.send(r);
            repaint.request_repaint();
        });
        self.web_pick_rx = Some((p, rx));
        self.info(LogKind::Info, "Pick on page: a browser opens - type/log in as needed, then hold Ctrl and click the element");
    }

    fn poll_web_pick(&mut self) {
        let Some((p, rx)) = &self.web_pick_rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        let p = p.clone();
        self.web_pick_rx = None;
        match res {
            Ok(el) => {
                if let Some(step) = tree::step_mut(&mut self.flow.steps, &p) {
                    let mut v = serde_json::to_value(&*step).unwrap();
                    if el.id.is_empty() {
                        v["selector"] = Value::String(el.selector.clone());
                        v["element_id"] = Value::String(String::new());
                    } else {
                        v["element_id"] = Value::String(el.id.clone());
                        v["selector"] = Value::String(String::new());
                    }
                    if let Ok(s) = serde_json::from_value::<Step>(v) {
                        *step = s;
                        self.dirty = true;
                        self.buffers_for = None;
                    }
                }
                let what = if el.id.is_empty() { format!("selector {}", el.selector) } else { format!("element_id \"{}\"", el.id) };
                self.info(LogKind::Ok, format!("picked <{}> {what} {}", el.tag, if el.text.is_empty() { String::new() } else { format!("(\"{}\")", el.text) }));
            }
            Err(e) => self.info(LogKind::Error, format!("pick on page failed: {e}")),
        }
    }

    fn poll_snip(&mut self) {
        let Some((p, rx)) = &self.snip_rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        let p = p.clone();
        self.snip_rx = None;
        match res {
            Ok(file) => {
                if let Some(step) = tree::step_mut(&mut self.flow.steps, &p) {
                    let mut v = serde_json::to_value(&*step).unwrap();
                    v["image"] = Value::String(file.clone());
                    if let Ok(s) = serde_json::from_value::<Step>(v) {
                        *step = s;
                        self.dirty = true;
                        self.buffers_for = None;
                    }
                }
                self.info(
                    LogKind::Ok,
                    format!("saved snip as {file}{}", if self.path.is_none() { " (save the flow so the relative path resolves)" } else { "" }),
                );
            }
            Err(e) => self.info(LogKind::Error, format!("snip failed: {e}")),
        }
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong(tr("Run log"));
            if ui.small_button(tr("clear")).clicked() {
                self.log.clear();
            }
        });
        egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for (k, m) in &self.log {
                let c = match k {
                    LogKind::Info => ui.visuals().text_color(),
                    LogKind::Warn => crate::look::warn(),
                    LogKind::Error => crate::look::err(),
                    LogKind::Ok => crate::look::ok(),
                };
                ui.label(RichText::new(m).monospace().color(c));
            }
        });
    }
}

impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        look::set_dark(ui.visuals().dark_mode);
        self.poll_run(&ctx);
        self.poll_record(&ctx);
        self.poll_pick();
        self.poll_snip();
        self.poll_web_pick();
        // Keep the live mouse position in the status bar fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.pick_rx.is_some() || self.snip_rx.is_some() { 150 } else { 250 }));

        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }

        // Shortcuts. Undo/redo only when no text box has the focus (it has its own undo).
        let typing = ctx.memory(|m| m.focused().is_some());
        let (save, open, run, step, bp, undo, redo) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::F10),
                i.key_pressed(egui::Key::F9),
                !typing && i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                !typing && i.modifiers.command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if save {
            self.save(false);
        }
        if open {
            self.guard(Pending::Open(None), &ctx);
        }
        let paused = self.is_paused();
        if run {
            if paused {
                self.resume(false, &ctx)
            } else {
                self.run(&ctx, false)
            }
        }
        if step {
            if paused {
                self.resume(true, &ctx)
            } else {
                self.run(&ctx, true)
            }
        }
        if bp {
            if let Selection::Step(p) = self.sel.clone() {
                self.toggle_breakpoint(&p);
            }
        }
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }

        // Window title
        let title = format!(
            "{}{} — AutoFlowMate Studio",
            if self.dirty { "● " } else { "" },
            self.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "untitled".into())
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        self.schedule_window(&ctx);
        self.about_window(&ctx);
        self.preview_window(&ctx);
        self.secret_window(&ctx);
        self.paused_window(&ctx);
        self.record_prompt_window(&ctx);
        self.templates_window(&ctx);
        self.save_template_window(&ctx);
        if settings::show(&ctx, &mut self.settings_ui) {
            self.lists.presets = autoflow_core::presets::Presets::load();
        }
        self.unsaved_window(&ctx);
        self.splash(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").exact_size(22.0).show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("log").resizable(true).default_size(170.0).show(ui, |ui| self.log_view(ui));
        egui::Panel::left("palette").resizable(true).default_size(190.0).show(ui, |ui| self.palette(ui));
        egui::Panel::right("props").resizable(true).default_size(400.0).show(ui, |ui| self.properties(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.steps_view(ui));

        // Label that follows the pointer while dragging an action or step.
        if let Some(item) = egui::DragAndDrop::payload::<DragItem>(&ctx) {
            if let Some(pos) = ctx.pointer_interact_pos() {
                let text = match &*item {
                    DragItem::New(n) => format!("+ {n}"),
                    DragItem::Move(p) => tree::step_ref(&self.flow.steps, p).map(|s| format!("↕ {}", s.action.name())).unwrap_or_default(),
                };
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("dnd_ghost")));
                let r = painter.text(pos + egui::vec2(14.0, 10.0), egui::Align2::LEFT_TOP, &text, egui::FontId::proportional(14.0), Color32::WHITE);
                painter.rect_filled(r.expand(5.0), 4.0, Color32::from_rgba_unmultiplied(40, 90, 160, 220));
                painter.text(pos + egui::vec2(14.0, 10.0), egui::Align2::LEFT_TOP, &text, egui::FontId::proportional(14.0), Color32::WHITE);
            }
        }
        self.track_history();
    }
}

// -------------------------------------------------------------------- tree rows

enum TreeOp {
    Breakpoint(StepPath),
    Select(Selection),
    Toggle(StepPath),
    Move(StepPath, isize),
    Delete(StepPath),
    Duplicate(StepPath),
}

struct RowCtx<'a> {
    sel: &'a Selection,
    current: Option<&'a str>,
    failed: &'a [String],
    breakpoints: &'a HashSet<String>,
}

fn draw_list(ui: &mut egui::Ui, steps: &[Step], parents: &ListPath, depth: usize, cx: &RowCtx, ops: &mut Vec<TreeOp>) {
    for (i, step) in steps.iter().enumerate() {
        let path = StepPath { parents: parents.clone(), index: i };
        let selected = *cx.sel == Selection::Step(path.clone());
        let running = cx.current == Some(step.id.as_str()) && !step.id.is_empty();
        let failed = cx.failed.iter().any(|f| f == &step.id);
        ui.horizontal(|ui| {
            let bp = !step.id.is_empty() && cx.breakpoints.contains(&step.id);
            let dot = RichText::new("●").color(if bp { Color32::from_rgb(220, 60, 60) } else { Color32::from_gray(90).gamma_multiply(0.35) });
            if ui.add(egui::Label::new(dot).sense(egui::Sense::click())).on_hover_text(tr("Breakpoint: pause before this step (F9)")).clicked() {
                ops.push(TreeOp::Breakpoint(path.clone()));
            }
            ui.add_space(depth as f32 * 22.0);
            let mut en = step.enabled;
            if ui.checkbox(&mut en, "").on_hover_text(tr("enabled")).changed() {
                ops.push(TreeOp::Toggle(path.clone()));
            }
            let marker = if running {
                "▶ "
            } else if failed {
                "✖ "
            } else {
                ""
            };
            let mut label = RichText::new(format!("{marker}{} {}", icons::action_icon(step.action.name()), step.action.name())).strong();
            if !step.enabled {
                label = label.strikethrough().weak();
            }
            let summary = summarize(step);
            let text = format!("{}  {}", step.id, summary);
            let r1 = ui.selectable_label(selected, label);
            let r2 = ui.selectable_label(selected, RichText::new(text).weak());
            if r1.clicked() || r2.clicked() {
                ops.push(TreeOp::Select(Selection::Step(path.clone())));
            }
            if running {
                ui.colored_label(Color32::from_rgb(90, 160, 240), "running");
            }
            if !step.comment.is_empty() {
                ui.weak(format!("// {}", step.comment));
            }
        });
        let labels = step.action.child_labels();
        for (l, children) in step.action.children().into_iter().enumerate() {
            let mut lp = parents.clone();
            lp.push((i, l));
            let sel = *cx.sel == Selection::List(lp.clone());
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 22.0 + 26.0);
                if ui
                    .selectable_label(sel, RichText::new(format!("{}:  ({} steps)", labels[l], children.len())).italics())
                    .on_hover_text(tr("Select, then click an action to add it here"))
                    .clicked()
                {
                    ops.push(TreeOp::Select(Selection::List(lp.clone())));
                }
            });
            draw_list(ui, children, &lp, depth + 2, cx, ops);
        }
    }
}

/// Short description of a step's key parameters.
fn summarize(step: &Step) -> String {
    let v = serde_json::to_value(&step.action).unwrap_or_default();
    let Some(o) = v.as_object() else { return String::new() };
    let mut parts = vec![];
    for (k, val) in o {
        if matches!(k.as_str(), "action" | "then" | "else" | "steps" | "catch" | "finally" | "timeout_ms" | "exact" | "name_contains" | "invoke") {
            continue;
        }
        let s = match val {
            Value::String(s) if !s.is_empty() => format!("{k}={s}"),
            Value::Number(n) => format!("{k}={n}"),
            Value::Bool(true) => k.clone(),
            Value::Object(c) if k == "condition" => format!(
                "{} {} {}",
                c.get("left").and_then(Value::as_str).unwrap_or(""),
                c.get("op").and_then(Value::as_str).unwrap_or(""),
                c.get("right").and_then(Value::as_str).unwrap_or("")
            ),
            Value::Array(a) if !a.is_empty() => format!("{k}=[{}]", a.len()),
            _ => continue,
        };
        parts.push(s);
        if parts.len() >= 3 {
            break;
        }
    }
    let mut s = parts.join("  ");
    if s.chars().count() > 90 {
        s = s.chars().take(90).collect::<String>() + "…";
    }
    s
}

fn cat_color(cat: &str) -> Color32 {
    match cat {
        "Flow" => Color32::from_rgb(170, 130, 230),
        "File" => Color32::from_rgb(90, 170, 220),
        "Program" => Color32::from_rgb(220, 150, 80),
        "Window" => Color32::from_rgb(100, 190, 170),
        "Mouse" => Color32::from_rgb(230, 110, 140),
        "Keyboard" => Color32::from_rgb(220, 190, 70),
        "UI Element" => Color32::from_rgb(110, 200, 110),
        "Web" => Color32::from_rgb(60, 150, 235),
        "Data" => Color32::from_rgb(30, 160, 120),
        "Email" => Color32::from_rgb(200, 80, 60),
        "Internet" => Color32::from_rgb(70, 130, 200),
        "Database" => Color32::from_rgb(120, 140, 90),
        "Documents" => Color32::from_rgb(70, 110, 200),
        "Privacy" => Color32::from_rgb(90, 90, 110),
        "AI" => Color32::from_rgb(160, 90, 230),
        "Text" => Color32::from_rgb(190, 150, 90),
        "Date & Math" => Color32::from_rgb(80, 170, 200),
        "Image" => Color32::from_rgb(235, 120, 60),
        "OCR" => Color32::from_rgb(210, 140, 40),
        "Screen" => Color32::from_rgb(150, 120, 220),
        "Dialog" => Color32::from_rgb(200, 100, 180),
        "Clipboard" => Color32::from_rgb(120, 160, 120),
        _ => Color32::from_rgb(160, 160, 170),
    }
}

fn refresh_tasks(s: &mut SchedUi, flow: &std::path::Path) {
    match autoflow_core::schedule::for_flow(flow) {
        Ok(t) => s.tasks = t,
        Err(e) => s.message = Some((false, format!("cannot list tasks: {e}"))),
    }
}

/// Build the trigger from the form and register the task.
fn create_task(s: &SchedUi, flow: &std::path::Path) -> (bool, String) {
    use autoflow_core::schedule::{self, TaskSpec, Trigger};
    const D: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let when = match s.kind {
        0 => format!("daily {}", s.time.trim()),
        1 => format!("weekdays {}", s.time.trim()),
        2 => {
            let days: Vec<&str> = (0..7).filter(|i| s.days[*i]).map(|i| D[i]).collect();
            if days.is_empty() {
                return (false, "pick at least one day".into());
            }
            format!("weekly {} {}", days.join(","), s.time.trim())
        }
        3 => format!("monthly {} {}", s.day_of_month, s.time.trim()),
        4 => format!("once {} {}", s.date.trim(), s.time.trim()),
        5 => format!("every {}m", s.minutes),
        _ => "logon".into(),
    };
    let trigger = match Trigger::parse(&when) {
        Ok(t) => t,
        Err(e) => return (false, e.to_string()),
    };
    // The task runs the CLI that sits next to Studio.
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("autoflow.exe"))).unwrap_or_default();
    let spec = TaskSpec {
        name: schedule::sanitize_name(&s.name),
        flow: flow.to_path_buf(),
        trigger,
        exe,
        extra_args: vec![],
        time_limit_minutes: 120,
        watch: s.watch,
    };
    match schedule::create(&spec) {
        Ok(()) => (true, format!("scheduled \"{}\": {}", spec.name, spec.trigger.describe())),
        Err(e) => (false, e.to_string()),
    }
}

fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs:x}")
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if t.is_empty() {
        "flow".into()
    } else {
        t
    }
}
