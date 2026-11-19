//! AutoFlowMate Studio — visual editor and runner for flow JSON files.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flowchart;
mod form;
mod tree;

use autoflow_core::{catalog, run_flow, validate_json, Action, ActionInfo, Event, Flow, RunOptions, Severity, Step};
use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value};
use std::collections::HashMap;
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

fn main() -> eframe::Result {
    let path = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1320.0, 820.0]).with_min_inner_size([900.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("AutoFlowMate Studio", options, Box::new(move |_cc| Ok(Box::new(Studio::new(path)))))
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
}

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
    pick_rx: Option<Receiver<Result<(i32, i32, Option<String>), String>>>,
    snip_rx: Option<(StepPath, Receiver<SnipResult>)>,
    palette_filter: String,
    last_title: String,
    view: View,
    zoom: f32,
    settings_buf: HashMap<String, String>,
}

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
            palette_filter: String::new(),
            last_title: String::new(),
            view: View::Flowchart,
            zoom: 1.0,
            settings_buf: HashMap::new(),
        };
        if let Some(p) = path {
            s.open_path(p);
        }
        s.sync_vars_buf();
        s
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
                self.sync_vars_buf();
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
            _ => rfd::FileDialog::new()
                .add_filter("Flow", &["json"])
                .set_file_name(format!("{}.json", slug(&self.flow.name)))
                .save_file(),
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

    fn new_flow(&mut self) {
        self.flow = Flow::default();
        self.path = None;
        self.dirty = false;
        self.sel = Selection::None;
        self.sync_vars_buf();
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

    fn run(&mut self, ctx: &egui::Context) {
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
        let flow_dir = self
            .path
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        // Timing and fail-safe come from flow.settings (edited in the toolbar / flow settings).
        let opts = RunOptions { dry_run: self.dry_run, stop: stop.clone(), flow_dir, ..RunOptions::default() };
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let mut sink = |e: Event| {
                let _ = tx.send(e);
                repaint.request_repaint();
            };
            run_flow(&flow, Map::new(), &opts, &mut sink);
        });
        if self.minimize_on_run && !self.dry_run {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.info(LogKind::Info, if self.dry_run { "dry run started" } else { "run started" });
        self.running = Some(Running { rx, stop, current: None });
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
        if let Some(r) = &self.running {
            r.stop.store(true, Ordering::Relaxed);
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
                let info = autoflow_core::uia::Uia::new()
                    .and_then(|u| u.inspect_point(x, y))
                    .ok()
                    .map(|i| serde_json::to_string(&i).unwrap_or_default());
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
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.new_flow();
                    ui.close();
                }
                if ui.button("Open…  (Ctrl+O)").clicked() {
                    self.open_dialog();
                    ui.close();
                }
                if ui.button("Save  (Ctrl+S)").clicked() {
                    self.save(false);
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    self.save(true);
                    ui.close();
                }
            });
            ui.separator();
            let running = self.running.is_some();
            if ui.add_enabled(!running, egui::Button::new(RichText::new("▶ Run (F5)").color(Color32::from_rgb(80, 180, 90)))).clicked() {
                self.run(&ctx);
            }
            if ui.add_enabled(running, egui::Button::new(RichText::new("■ Stop").color(Color32::from_rgb(220, 80, 80)))).clicked() {
                self.stop();
            }
            ui.checkbox(&mut self.dry_run, "Dry run");
            ui.checkbox(&mut self.minimize_on_run, "Minimize while running");
            ui.separator();
            // Run-wide settings, saved in the flow (settings.*).
            let s = &mut self.flow.settings;
            let mut changed = false;
            ui.label("Key delay").on_hover_text("settings.key_delay_ms: pause between keystrokes. Raise to 150-250 ms for slow apps such as Tally.");
            changed |= ui.add(egui::DragValue::new(&mut s.key_delay_ms).range(0..=2000).suffix(" ms").speed(5.0)).changed();
            ui.label("Step delay").on_hover_text("settings.step_delay_ms: pause after every step.");
            changed |= ui.add(egui::DragValue::new(&mut s.step_delay_ms).range(0..=10000).suffix(" ms").speed(10.0)).changed();
            changed |= ui
                .checkbox(&mut s.failsafe, "Fail-safe")
                .on_hover_text(format!(
                    "Abort the run when the mouse touches a screen corner (PyAutoGUI FAILSAFE). The hotkey {} always aborts.",
                    if s.abort_hotkey.is_empty() { "(none)" } else { &s.abort_hotkey }
                ))
                .changed();
            self.dirty |= changed;
            ui.separator();
            if ui.button("✔ Validate").clicked() {
                self.log.clear();
                self.validate();
            }
            let picking = self.pick_rx.is_some();
            if ui
                .add_enabled(!picking, egui::Button::new(if picking { "⌖ Picking…" } else { "⌖ Pick element" }))
                .on_hover_text("After 3 s, capture the UI element / position under the mouse. Fills the selected ui_* or mouse step, or adds a new step.")
                .clicked()
            {
                self.start_pick(&ctx);
            }
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        ui.heading("Actions");
        ui.add(egui::TextEdit::singleline(&mut self.palette_filter).hint_text("filter…"));
        ui.weak("Drag onto the flowchart, or click to insert at the selection");
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
                egui::CollapsingHeader::new(RichText::new(cat).strong().color(cat_color(cat))).default_open(true).show(ui, |ui| {
                    for a in items {
                        let r = ui.add(egui::Button::new(a.name).sense(egui::Sense::click_and_drag())).on_hover_text(a.description);
                        r.dnd_set_drag_payload(DragItem::New(a.name.to_string()));
                        if r.clicked() {
                            add = Some(a.name.to_string());
                        }
                    }
                });
            }
        });
        if let Some(name) = add {
            if let Some(action) = Action::template(&name) {
                self.add_step(Step::new(action));
            }
        }
    }

    fn steps_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(if self.flow.name.is_empty() { "Untitled flow" } else { &self.flow.name });
            if ui.small_button("flow settings").clicked() {
                self.sel = Selection::None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.view, View::Json, "JSON");
                ui.selectable_value(&mut self.view, View::List, "List");
                ui.selectable_value(&mut self.view, View::Flowchart, "Flowchart");
                if self.view == View::Flowchart {
                    ui.separator();
                    if ui.small_button("+").clicked() {
                        self.zoom = (self.zoom + 0.1).min(2.0);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.small_button("−").clicked() {
                        self.zoom = (self.zoom - 0.1).max(0.4);
                    }
                }
            });
        });
        ui.separator();
        let current = self.running.as_ref().and_then(|r| r.current.clone());
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
                    }
                }
            }
            View::List => {
                let mut ops = vec![];
                egui::ScrollArea::vertical().id_salt("steps").auto_shrink([false, false]).show(ui, |ui| {
                    if self.flow.steps.is_empty() {
                        ui.add_space(20.0);
                        ui.weak("No steps yet. Pick an action on the left, or use ⌖ Pick element.");
                    }
                    let ctx = RowCtx { sel: &self.sel, current: current.as_deref(), failed: &self.failed_steps };
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
        egui::ScrollArea::vertical().id_salt("props").show(ui, |ui| match self.sel.clone() {
            Selection::Step(p) => self.step_properties(ui, &p),
            Selection::List(_) | Selection::Gap(..) => {
                ui.heading("Insertion point");
                ui.label("Click an action in the palette to insert it here, or drag one onto any connector.");
            }
            Selection::None => self.flow_properties(ui),
        });
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
        let desc = self.catalog.iter().find(|a| a.name == name).map(|a| a.description).unwrap_or("");
        ui.heading(RichText::new(name).color(cat_color(step.action.category())));
        ui.label(desc);
        ui.separator();

        let mut v = serde_json::to_value(step).unwrap();
        // Make sure common fields are always visible.
        if let Some(o) = v.as_object_mut() {
            for (k, d) in [("id", Value::from("")), ("comment", Value::from("")), ("on_error", Value::from("stop")), ("retries", Value::from(0)), ("retry_delay_ms", Value::from(1000))] {
                o.entry(k).or_insert(d);
            }
        }
        let obj = v.as_object_mut().unwrap();
        // Action fields first, common fields after.
        let common = ["id", "enabled", "comment", "on_error", "retries", "retry_delay_ms"];
        let mut action_part: Map<String, Value> = obj.iter().filter(|(k, _)| !common.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut common_part: Map<String, Value> = obj.iter().filter(|(k, _)| common.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();

        // Action-specific field help first, generic help (common fields, condition) as fallback.
        let mut help: HashMap<String, String> = autoflow_core::flow::action_field_help(name)
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect();
        for (k, v) in &self.help {
            help.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let mut form = form::Form { help: &help, json_buffers: &mut self.json_buffers, action: name.to_string() };
        let mut changed = form.object(ui, "a", &mut action_part);
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Step settings").default_open(false).show(ui, |ui| {
            changed |= form.object(ui, "c", &mut common_part);
        });

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
            ui.colored_label(Color32::from_rgb(220, 80, 80), e);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("⬆ Up").clicked() {
                self.apply(TreeOp::Move(p.clone(), -1));
            }
            if ui.button("⬇ Down").clicked() {
                self.apply(TreeOp::Move(p.clone(), 1));
            }
            if ui.button("⧉ Duplicate").clicked() {
                self.apply(TreeOp::Duplicate(p.clone()));
            }
            if ui.button(RichText::new("🗑 Delete").color(Color32::from_rgb(220, 80, 80))).clicked() {
                self.apply(TreeOp::Delete(p.clone()));
            }
        });
        if name.starts_with("image_") {
            ui.add_space(6.0);
            let busy = self.snip_rx.is_some();
            if ui
                .add_enabled(!busy, egui::Button::new(if busy { "📷 Snipping…" } else { "📷 Snip image" }))
                .on_hover_text("Opens the Windows snipping tool; the snip is saved under images/ next to the flow and set as `image`.")
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_snip(&ctx, p.clone());
            }
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new("Step JSON").default_open(false).show(ui, |ui| {
            if let Some(s) = tree::step_ref(&self.flow.steps, p) {
                let mut text = serde_json::to_string_pretty(s).unwrap_or_default();
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
            }
        });
    }

    fn flow_properties(&mut self, ui: &mut egui::Ui) {
        ui.heading("Flow settings");
        ui.separator();
        egui::Grid::new("flowgrid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("name");
            self.dirty |= ui.text_edit_singleline(&mut self.flow.name).changed();
            ui.end_row();
            ui.label("description");
            self.dirty |= ui.add(egui::TextEdit::multiline(&mut self.flow.description).desired_rows(3)).changed();
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Variables (JSON object)").strong());
        ui.weak("Use them as ${name}. Built-ins: ${now} ${date} ${time} ${timestamp} ${flow_dir} ${env.USERPROFILE} ${newline}");
        let resp = ui.add(egui::TextEdit::multiline(&mut self.flow_vars_buf).code_editor().desired_rows(8).desired_width(f32::INFINITY));
        match serde_json::from_str::<Map<String, Value>>(&self.flow_vars_buf) {
            Ok(m) => {
                if resp.changed() {
                    self.flow.variables = m;
                    self.dirty = true;
                }
            }
            Err(e) => {
                ui.colored_label(Color32::from_rgb(220, 80, 80), format!("JSON: {e}"));
            }
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Run settings").strong());
        ui.weak("Timing and safety for every run of this flow (PyAutoGUI PAUSE / FAILSAFE).");
        let mut v = serde_json::to_value(&self.flow.settings).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            let help: HashMap<String, String> = autoflow_core::flow::json_schema()
                .pointer("/$defs/Settings/properties")
                .and_then(Value::as_object)
                .map(|p| p.iter().filter_map(|(k, x)| x.get("description").and_then(Value::as_str).map(|d| (k.clone(), d.to_string()))).collect())
                .unwrap_or_default();
            let mut f = form::Form { help: &help, json_buffers: &mut self.settings_buf, action: String::new() };
            if f.object(ui, "settings", o) {
                if let Ok(s) = serde_json::from_value(v) {
                    self.flow.settings = s;
                    self.dirty = true;
                }
            }
        }
        ui.add_space(8.0);
        ui.label(format!("File: {}", self.path.as_ref().map(|p| p.display().to_string()).unwrap_or("(not saved)".into())));
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            let (x, y) = autoflow_core::failsafe::cursor().unwrap_or((0, 0));
            ui.monospace(format!("Mouse X {x:>5}  Y {y:>5}"));
            if autoflow_core::failsafe::corner_hit(x, y) {
                ui.colored_label(Color32::from_rgb(230, 90, 90), "fail-safe corner");
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
        let dir = self
            .path
            .as_ref()
            .and_then(|f| f.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
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
                self.info(LogKind::Ok, format!("saved snip as {file}{}", if self.path.is_none() { " (save the flow so the relative path resolves)" } else { "" }));
            }
            Err(e) => self.info(LogKind::Error, format!("snip failed: {e}")),
        }
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Run log");
            if ui.small_button("clear").clicked() {
                self.log.clear();
            }
        });
        egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for (k, m) in &self.log {
                let c = match k {
                    LogKind::Info => ui.visuals().text_color(),
                    LogKind::Warn => Color32::from_rgb(220, 170, 50),
                    LogKind::Error => Color32::from_rgb(230, 90, 90),
                    LogKind::Ok => Color32::from_rgb(90, 190, 100),
                };
                ui.label(RichText::new(m).monospace().color(c));
            }
        });
    }
}

impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_run(&ctx);
        self.poll_pick();
        self.poll_snip();
        // Keep the live mouse position in the status bar fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.pick_rx.is_some() || self.snip_rx.is_some() { 150 } else { 250 }));

        // Shortcuts
        let (save, open, run) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.key_pressed(egui::Key::F5),
            )
        });
        if save {
            self.save(false);
        }
        if open {
            self.open_dialog();
        }
        if run {
            self.run(&ctx);
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
    }
}

// -------------------------------------------------------------------- tree rows

enum TreeOp {
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
}

fn draw_list(ui: &mut egui::Ui, steps: &[Step], parents: &ListPath, depth: usize, cx: &RowCtx, ops: &mut Vec<TreeOp>) {
    for (i, step) in steps.iter().enumerate() {
        let path = StepPath { parents: parents.clone(), index: i };
        let selected = *cx.sel == Selection::Step(path.clone());
        let running = cx.current == Some(step.id.as_str()) && !step.id.is_empty();
        let failed = cx.failed.iter().any(|f| f == &step.id);
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 22.0);
            let mut en = step.enabled;
            if ui.checkbox(&mut en, "").on_hover_text("enabled").changed() {
                ops.push(TreeOp::Toggle(path.clone()));
            }
            let marker = if running { "▶ " } else if failed { "✖ " } else { "" };
            let mut label = RichText::new(format!("{marker}{}", step.action.name())).strong().color(cat_color(step.action.category()));
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
                if ui.selectable_label(sel, RichText::new(format!("{}:  ({} steps)", labels[l], children.len())).italics()).on_hover_text("Select, then click an action to add it here").clicked() {
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
        if matches!(k.as_str(), "action" | "then" | "else" | "steps" | "timeout_ms" | "exact" | "name_contains" | "invoke") {
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
        _ => Color32::from_rgb(160, 160, 170),
    }
}

fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs:x}")
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if t.is_empty() { "flow".into() } else { t }
}
