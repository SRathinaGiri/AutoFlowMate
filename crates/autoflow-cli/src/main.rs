//! `autoflow` — run, validate and author AutoFlowMate flows from the command line.
//! Every command supports `--json` output so AI agents can drive it.

use anyhow::{bail, Context as _, Result};
use autoflow_core::{catalog, run_flow, validate_json, Event, Flow, RunOptions, Severity, Step};
use clap::{Parser, Subcommand};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "autoflow", version, about = "AutoFlowMate - Windows RPA flows as JSON")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a flow file.
    Run {
        flow: PathBuf,
        /// Override a variable: --var name=value (repeatable; JSON values allowed).
        #[arg(long = "var", value_name = "NAME=VALUE")]
        vars: Vec<String>,
        /// Log what would happen without touching files, mouse, keyboard or windows.
        #[arg(long)]
        dry_run: bool,
        /// Print events as JSON lines (for agents/tools).
        #[arg(long)]
        json: bool,
        /// Override settings.step_delay_ms (pause after every step).
        #[arg(long, value_name = "MS")]
        step_delay: Option<u64>,
        /// Override settings.key_delay_ms (delay between keystrokes).
        #[arg(long, value_name = "MS")]
        key_delay: Option<u64>,
        /// Override settings.input_pause_ms (pause after every mouse/keyboard action).
        #[arg(long, value_name = "MS")]
        input_pause: Option<u64>,
        /// Disable the corner fail-safe for this run (the abort hotkey still works).
        #[arg(long)]
        no_failsafe: bool,
        /// Skip validation before running.
        #[arg(long)]
        no_validate: bool,
        /// Append a timestamped log of the run to this file.
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
        /// Mode used by scheduled tasks: hide the console window and log to
        /// <flow folder>\logs\<flow name>.log (unless --log is given).
        #[arg(long)]
        scheduled: bool,
    },
    /// Keep running and start the flow for each new file / e-mail listed in its "triggers".
    /// Stop with Ctrl+C. To start it at log-on: `autoflow schedule add flow.json --when logon --watch`.
    Watch {
        flow: PathBuf,
        /// Check once and exit (files already there count as new); for a task that runs every N minutes.
        #[arg(long)]
        once: bool,
        /// Variables passed to every run: --var name=value (repeatable).
        #[arg(long = "var", value_name = "NAME=VALUE")]
        vars: Vec<String>,
        /// How often to look at watched folders, in seconds.
        #[arg(long, default_value_t = 2)]
        poll_seconds: u64,
        /// Append a timestamped log to this file.
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
        /// Mode used by scheduled tasks: hide the console and log to <flow folder>\logs\<flow name>.log.
        #[arg(long)]
        scheduled: bool,
    },
    /// Record your clicks and typing as a new flow. Press Ctrl+Alt+R to stop.
    /// Clicks on named controls become ui_click steps; typing into password boxes is never saved.
    Record {
        /// Flow file to write.
        out: PathBuf,
        /// Do not add wait steps for pauses.
        #[arg(long)]
        no_waits: bool,
        /// Replace OUT if it exists.
        #[arg(long)]
        force: bool,
        /// Seconds to wait before recording starts (time to switch to the app).
        #[arg(long, default_value_t = 3)]
        delay: u64,
    },
    /// Store passwords (e.g. a Gmail App Password) in Windows Credential Manager for send_email.
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// Run flows automatically with Windows Task Scheduler.
    Schedule {
        #[command(subcommand)]
        cmd: ScheduleCmd,
    },
    /// Check a flow file for errors without running it.
    Validate {
        flow: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List all actions with descriptions and default parameters.
    Actions {
        #[arg(long)]
        json: bool,
        /// Only this category (Flow, File, Program, Window, Mouse, Keyboard, UI Element, Clipboard, Screen).
        #[arg(long)]
        category: Option<String>,
    },
    /// Print a step template for an action, ready to paste into a flow.
    Template { action: String },
    /// Print the JSON Schema of flow files.
    Schema {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate the Markdown action reference (used by the AI skill).
    Docs {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Create a new flow file: empty, or a copy of a template (see `autoflow templates`).
    New {
        file: PathBuf,
        #[arg(long, default_value = "New flow")]
        name: String,
        /// Start from this template (its file name, e.g. redact_folder, or its title).
        #[arg(long)]
        template: Option<String>,
    },
    /// List the flow templates you can start from (built-in and your own).
    Templates,
    /// Show the UI element under the mouse pointer (to build ui_* selectors).
    Inspect {
        /// Seconds to wait before capturing, so you can point at something.
        #[arg(long, default_value_t = 3)]
        delay: u64,
        /// Use this position instead of the mouse pointer.
        #[arg(long, num_args = 2, value_names = ["X", "Y"])]
        at: Option<Vec<i32>>,
    },
    /// List visible top-level windows (titles to use in window_* actions).
    Windows {
        #[arg(long)]
        json: bool,
    },
    /// Live mouse position and pixel colour (PyAutoGUI displayMousePosition). Ctrl+C to quit.
    Mouse {
        /// Print one reading and exit.
        #[arg(long)]
        once: bool,
    },
    /// Open a web page and Ctrl+click an element to get its element_id / selector
    /// (no developer tools needed).
    WebPick {
        /// Page to open (URL or local .html file).
        url: String,
        /// "edge" (default) or "chrome".
        #[arg(long, default_value = "edge")]
        browser: String,
        /// Give up after this many seconds.
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },
    /// Find an image on screen and print the matches (to test image_* steps).
    Locate {
        image: PathBuf,
        #[arg(long, default_value_t = 0.9)]
        confidence: f64,
        /// Report every match, not just the best.
        #[arg(long)]
        all: bool,
    },
    /// Read text with Windows OCR (to test ocr_* steps): the primary screen, a window or a picture.
    Ocr {
        /// Read this picture file instead of the screen.
        #[arg(long)]
        image: Option<PathBuf>,
        /// Read this window (part of its title).
        #[arg(long)]
        window: Option<String>,
        /// Also report where this text is (screen coordinates).
        #[arg(long)]
        find: Option<String>,
        /// Language tag, e.g. en-US (default: the Windows display languages).
        #[arg(long, default_value = "")]
        language: String,
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
        /// List the installed OCR languages and exit.
        #[arg(long)]
        languages: bool,
    },
}

#[derive(Subcommand)]
enum SecretCmd {
    /// Store a password under NAME (asked for twice, not shown). Flows use `password_secret: "NAME"`.
    Set { name: String },
    /// Tell whether a password is stored under NAME.
    Check { name: String },
    /// Delete the password stored under NAME.
    Delete { name: String },
    /// Make a new random key for pseudonymize under NAME (default pseudonym_key) and show it once
    /// so you can back it up. Refuses to replace an existing key.
    NewKey {
        #[arg(default_value = "pseudonym_key")]
        name: String,
    },
    /// Show what is stored under NAME, to back up a pseudonymisation key (asks first).
    Show { name: String },
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// Create (or replace) a scheduled task for a flow.
    ///
    /// WHEN examples: "daily 09:00", "weekdays 18:30", "weekly mon,thu 09:00",
    /// "monthly 1 09:00", "once 2026-11-20 09:00", "every 30m", "every 2h", "logon".
    Add {
        flow: PathBuf,
        /// When to run (quote it), e.g. "daily 09:00".
        #[arg(long = "when", value_name = "WHEN")]
        when: String,
        /// Task name (default: the flow file name).
        #[arg(long)]
        name: Option<String>,
        /// Variables passed to every run: --var name=value (repeatable).
        #[arg(long = "var", value_name = "NAME=VALUE")]
        vars: Vec<String>,
        /// Stop a run that takes longer than this.
        #[arg(long, default_value_t = 120)]
        time_limit_minutes: u32,
        /// Print the task XML instead of creating the task.
        #[arg(long)]
        print_xml: bool,
        /// Run `autoflow watch` (the flow's triggers) instead of the flow itself. With --when logon
        /// the watcher runs all day; with "every 30m" it checks once each time.
        #[arg(long)]
        watch: bool,
    },
    /// List AutoFlowMate tasks with next/last run and result.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Start a task now (to test it).
    Run { name: String },
    /// Delete a task.
    Remove { name: String },
}

fn main() -> ExitCode {
    // Physical-pixel coordinates everywhere (mouse, screenshots, UI Automation, image search).
    #[cfg(windows)]
    let _ = enigo::set_dpi_awareness();
    let cli = Cli::parse();
    match real_main(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn real_main(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Schedule { cmd } => schedule_cmd(cmd),
        Cmd::Secret { cmd } => {
            use autoflow_core::mail::secrets;
            match cmd {
                SecretCmd::Set { name } => {
                    let a = rpassword::prompt_password(format!("Password for `{name}` (not shown): "))?;
                    if a.is_empty() {
                        bail!("empty password, nothing stored");
                    }
                    let b = rpassword::prompt_password("Type it again: ")?;
                    if a != b {
                        bail!("the two entries differ, nothing stored");
                    }
                    secrets::set(&name, &a)?;
                    println!("stored `{name}` in Windows Credential Manager (AutoFlowMate:{name})");
                    println!("use it in send_email with  \"password_secret\": \"{name}\"");
                }
                SecretCmd::Check { name } => {
                    if secrets::exists(&name) {
                        println!("`{name}` is stored");
                    } else {
                        println!("`{name}` is not stored - run `autoflow secret set {name}`");
                        return Ok(ExitCode::from(1));
                    }
                }
                SecretCmd::Delete { name } => {
                    secrets::delete(&name)?;
                    println!("deleted `{name}`");
                }
                SecretCmd::NewKey { name } => {
                    if secrets::exists(&name) {
                        bail!("`{name}` already exists; replacing it would make its vault unreadable (delete it first if you are sure)");
                    }
                    let key = autoflow_core::pseudo::new_key_text();
                    secrets::set(&name, &key)?;
                    println!("stored a new key under `{name}`. Back it up somewhere safe (a password manager), then clear this screen:");
                    println!("{key}");
                    println!("to restore it on another PC:  autoflow secret set {name}   (paste the key)");
                }
                SecretCmd::Show { name } => {
                    eprint!("This shows the secret `{name}` on screen. Type yes to continue: ");
                    let mut answer = String::new();
                    std::io::stdin().read_line(&mut answer)?;
                    if answer.trim() != "yes" {
                        bail!("not shown");
                    }
                    println!("{}", secrets::get(&name)?);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Run { flow, vars, dry_run, json, step_delay, key_delay, input_pause, no_failsafe, no_validate, log, scheduled } => {
            if scheduled {
                hide_console();
            }
            let log_file = log.or_else(|| scheduled.then(|| autoflow_core::schedule::log_path(&abs(&flow))));
            let mut logger = match &log_file {
                Some(p) => Some(RunLog::open(p, &flow, scheduled)?),
                None => None,
            };
            let text = match std::fs::read_to_string(&flow) {
                Ok(t) => t,
                Err(e) => {
                    if let Some(l) = logger.as_mut() {
                        l.line(&format!("ERROR cannot read flow: {e}"));
                    }
                    return Err(e.into());
                }
            };
            let (parsed, issues) = validate_json(&text);
            let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
            if !no_validate || parsed.is_none() {
                for i in &issues {
                    if json {
                        println!("{}", json!({"event": "validation", "severity": i.severity, "location": i.location, "message": i.message}));
                    } else {
                        eprintln!("{:?} {}: {}", i.severity, i.location, i.message);
                    }
                    if let Some(l) = logger.as_mut() {
                        l.line(&format!("{:?} {}: {}", i.severity, i.location, i.message));
                    }
                }
                if has_errors {
                    if let Some(l) = logger.as_mut() {
                        l.line("RESULT invalid flow (exit 2)");
                    }
                    return Ok(ExitCode::from(2));
                }
            }
            let mut flow_doc = parsed.expect("parsed when no errors");
            flow_doc.assign_ids();
            let mut overrides = Map::new();
            for v in vars {
                let (k, val) = autoflow_core::vars::parse_override(&v)?;
                overrides.insert(k, val);
            }
            let opts = RunOptions {
                dry_run,
                step_delay_ms: step_delay,
                key_delay_ms: key_delay,
                input_pause_ms: input_pause,
                failsafe: if no_failsafe { Some(false) } else { None },
                flow_dir: flow_dir(&flow),
                ..RunOptions::default()
            };
            let mut sink = |e: Event| {
                print_event(&e, json);
                if let Some(l) = logger.as_mut() {
                    l.event(&e);
                }
            };
            let result = run_flow(&flow_doc, overrides, &opts, &mut sink);
            if json {
                println!("{}", json!({"event": "result", "success": result.success, "message": result.message, "variables": result.variables}));
            }
            if let Some(p) = &log_file {
                if !json && !scheduled {
                    eprintln!("log: {}", p.display());
                }
            }
            Ok(if result.success { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
        Cmd::Watch { flow, once, vars, poll_seconds, log, scheduled } => {
            if scheduled {
                hide_console();
            }
            let log_file = log.or_else(|| scheduled.then(|| autoflow_core::schedule::log_path(&abs(&flow))));
            let mut logger = match &log_file {
                Some(p) => Some(RunLog::open(p, &flow, scheduled)?),
                None => None,
            };
            let text = std::fs::read_to_string(&flow)?;
            let (parsed, issues) = validate_json(&text);
            for i in &issues {
                eprintln!("{:?} {}: {}", i.severity, i.location, i.message);
                if let Some(l) = logger.as_mut() {
                    l.line(&format!("{:?} {}: {}", i.severity, i.location, i.message));
                }
            }
            let Some(mut flow_doc) = parsed.filter(|_| !issues.iter().any(|i| i.severity == Severity::Error)) else {
                return Ok(ExitCode::from(2));
            };
            flow_doc.assign_ids();
            let mut base = Map::new();
            for v in vars {
                let (k, val) = autoflow_core::vars::parse_override(&v)?;
                base.insert(k, val);
            }
            let dir = flow_dir(&flow);
            let opts = autoflow_core::watch::WatchOptions {
                once,
                stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                poll: std::time::Duration::from_secs(poll_seconds.max(1)),
            };
            let logger = std::cell::RefCell::new(logger);
            let say = |s: String| {
                println!("[watch] {s}");
                if let Some(l) = logger.borrow_mut().as_mut() {
                    l.line(&format!("WATCH {s}"));
                }
            };
            let (mut runs, mut failures) = (0u32, 0u32);
            let mut log_fn = |s: String| say(s);
            let mut run_fn = |ev: &autoflow_core::watch::WatchEvent| {
                let mut overrides = base.clone();
                overrides.insert(ev.variable.clone(), ev.value.clone());
                let ropts = RunOptions { flow_dir: dir.clone(), ..RunOptions::default() };
                let mut sink = |e: Event| {
                    print_event(&e, false);
                    if let Some(l) = logger.borrow_mut().as_mut() {
                        l.event(&e);
                    }
                };
                let r = run_flow(&flow_doc, overrides, &ropts, &mut sink);
                runs += 1;
                if !r.success {
                    failures += 1;
                }
                r.success
            };
            let result = autoflow_core::watch::watch(&flow_doc.triggers, &dir, &opts, &mut log_fn, &mut run_fn);
            say(format!("stopped after {runs} run(s), {failures} failed"));
            result?;
            Ok(if failures > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        Cmd::Record { out, no_waits, force, delay } => {
            if out.exists() && !force {
                bail!("{} exists; use --force to replace it", out.display());
            }
            for s in (1..=delay).rev() {
                eprintln!("recording starts in {s}...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            eprintln!("recording: use the app as usual; stop with the ■ Stop recording window (top right) or Ctrl+Alt+R");
            let opts = autoflow_core::recorder::RecordOptions { min_wait_ms: if no_waits { 0 } else { 1500 }, ..Default::default() };
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let steps = autoflow_core::recorder::record(opts, stop, &mut |s| eprintln!("  + {s}"))?;
            let mut flow = Flow {
                name: format!("Recorded {}", autoflow_core::vars::Vars::default().interpolate("${now:dd-MM-yyyy HH:mm}").unwrap_or_default()),
                description: "Recorded with `autoflow record`. Check the steps, add waits for slow screens, and validate before running.".into(),
                steps,
                ..Flow::default()
            };
            flow.assign_ids();
            flow.save(&out)?;
            eprintln!("saved {} step(s) to {}", flow.steps.len(), out.display());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Validate { flow, json } => {
            let text = std::fs::read_to_string(&flow)?;
            let (parsed, issues) = validate_json(&text);
            let ok = parsed.is_some() && !issues.iter().any(|i| i.severity == Severity::Error);
            if json {
                println!("{}", serde_json::to_string_pretty(&json!({"valid": ok, "issues": issues}))?);
            } else {
                for i in &issues {
                    println!("{:<8} {}: {}", format!("{:?}", i.severity), i.location, i.message);
                }
                if ok {
                    let n = parsed.map(|f| count_steps(&f.steps)).unwrap_or(0);
                    println!("OK: {} ({n} steps, {} warnings)", flow.display(), issues.len());
                }
            }
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(2) })
        }
        Cmd::Actions { json, category } => {
            let list: Vec<_> = catalog().into_iter().filter(|a| category.as_ref().map(|c| a.category.eq_ignore_ascii_case(c)).unwrap_or(true)).collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                let mut last = "";
                for a in &list {
                    if a.category != last {
                        println!("\n{}", a.category);
                        last = a.category;
                    }
                    println!("  {:<18} {}", a.name, a.description);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Template { action } => {
            let Some(info) = catalog().into_iter().find(|a| a.name == action) else {
                bail!("unknown action `{action}`; run `autoflow actions`");
            };
            println!("{}", serde_json::to_string_pretty(&info.template)?);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Schema { output } => {
            let s = serde_json::to_string_pretty(&autoflow_core::flow::json_schema())?;
            match output {
                Some(p) => std::fs::write(p, s)?,
                None => println!("{s}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Docs { output } => {
            let md = action_docs();
            match output {
                Some(p) => std::fs::write(p, md)?,
                None => print!("{md}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::New { file, name, template } => {
            if file.exists() {
                bail!("{} already exists", file.display());
            }
            if let Some(t) = template {
                let info = autoflow_core::templates::find(&t).ok_or_else(|| anyhow::anyhow!("no template `{t}`; see `autoflow templates`"))?;
                std::fs::write(&file, autoflow_core::templates::read(&info)?)?;
                println!("created {} from the template \"{}\"", file.display(), info.name);
            } else {
                let flow = Flow { name, ..Flow::default() };
                flow.save(&file)?;
                println!("created {}", file.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Templates => {
            let all = autoflow_core::templates::list();
            if all.is_empty() {
                println!("no templates found");
            }
            for t in all {
                println!("{:<28} {}{}", t.id, t.name, if t.own { "  (yours)" } else { "" });
            }
            println!("use one:  autoflow new my_flow.json --template <name>");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Inspect { delay, at } => inspect(delay, at),
        Cmd::Windows { json } => windows(json),
        Cmd::Mouse { once } => {
            use std::io::Write;
            loop {
                let (x, y) = autoflow_core::failsafe::cursor().unwrap_or((0, 0));
                let color = autoflow_core::vision::pixel(x, y).map(autoflow_core::vision::hex).unwrap_or_else(|_| "?".into());
                let corner = if autoflow_core::failsafe::corner_hit(x, y) { "  <- fail-safe corner" } else { "" };
                if once {
                    println!("{}", json!({"x": x, "y": y, "color": color}));
                    return Ok(ExitCode::SUCCESS);
                }
                print!("\rX: {x:>5}  Y: {y:>5}  RGB: {color}{corner}            ");
                std::io::stdout().flush().ok();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        Cmd::WebPick { url, browser, timeout } => {
            let url = autoflow_core::web::to_url(&url, &std::env::current_dir()?);
            eprintln!("Opening {url} - hold Ctrl and click the element you want…");
            let p = autoflow_core::web::pick_element(&url, &browser, timeout * 1000, &|| false)?;
            let target = if p.id.is_empty() { json!({"selector": p.selector}) } else { json!({"element_id": p.id}) };
            println!("{}", serde_json::to_string_pretty(&json!({"element": p, "use_in_step": target}))?);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Locate { image, confidence, all } => {
            let t = autoflow_core::vision::load_template(&image)?;
            let started = std::time::Instant::now();
            let found = autoflow_core::vision::locate(&t, None, confidence, all)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"found": !found.is_empty(), "matches": found, "millis": started.elapsed().as_millis() as u64}))?
            );
            Ok(if found.is_empty() { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        Cmd::Ocr { image, window, find, language, scale, languages } => {
            if languages {
                for l in autoflow_core::ocr::languages() {
                    println!("{l}");
                }
                return Ok(ExitCode::SUCCESS);
            }
            let (img, ox, oy) = if let Some(p) = image {
                (xcap::image::open(&p).with_context(|| format!("open {}", p.display()))?.to_rgba8(), 0, 0)
            } else if let Some(t) = window {
                let want = t.to_lowercase();
                let w = xcap::Window::all()?
                    .into_iter()
                    .find(|w| w.title().map(|x| x.to_lowercase().contains(&want)).unwrap_or(false) && !w.is_minimized().unwrap_or(true))
                    .ok_or_else(|| anyhow::anyhow!("no visible window titled \"{t}\""))?;
                (w.capture_image()?, w.x()?, w.y()?)
            } else {
                let m = xcap::Monitor::all()?.into_iter().find(|m| m.is_primary().unwrap_or(false)).ok_or_else(|| anyhow::anyhow!("no monitor"))?;
                (m.capture_image()?, m.x()?, m.y()?)
            };
            let started = std::time::Instant::now();
            let out = autoflow_core::ocr::recognize(&img, &language, scale)?;
            println!("{}", out.text);
            eprintln!("({} lines in {} ms)", out.lines.len(), started.elapsed().as_millis());
            if let Some(f) = find {
                let hits: Vec<_> = autoflow_core::ocr::find(&out, &f, true)
                    .iter()
                    .map(|h| {
                        let (cx, cy) = h.center();
                        json!({"x": ox + cx.round() as i32, "y": oy + cy.round() as i32})
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json!({"text": f, "found": !hits.is_empty(), "centres": hits}))?);
                return Ok(if hits.is_empty() { ExitCode::from(1) } else { ExitCode::SUCCESS });
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Markdown reference of every action: description, fields with their help
/// text (from the JSON schema) and a default template.
fn action_docs() -> String {
    let mut md = String::from(
        "# AutoFlowMate action reference\n\n\
         Generated by `autoflow docs`. Every step is a JSON object with `\"action\"` plus the fields below.\n\
         All fields are optional in JSON (defaults shown in the template), but `autoflow validate` reports required ones.\n\
         String fields support `${variable}` interpolation.\n\n\
         Common step fields: `id`, `enabled` (default true), `comment`, `on_error` (`stop`|`continue`), `retries`, `retry_delay_ms` (default 1000).\n",
    );
    let mut last = "";
    for a in catalog() {
        if a.category != last {
            md.push_str(&format!("\n## {}\n", a.category));
            last = a.category;
        }
        md.push_str(&format!("\n### `{}`\n\n{}\n\n", a.name, a.description));
        let help = autoflow_core::flow::action_field_help(a.name);
        if let Some(o) = a.template.as_object() {
            let fields: Vec<_> = o.iter().filter(|(k, _)| k.as_str() != "action").collect();
            if !fields.is_empty() {
                md.push_str("| field | default | description |\n|---|---|---|\n");
                for (k, v) in fields {
                    let d = help.get(k).and_then(Value::as_str).unwrap_or("").replace('|', "\\|");
                    md.push_str(&format!("| `{k}` | `{}` | {d} |\n", v.to_string().replace('|', "\\|")));
                }
                md.push('\n');
            }
        }
        md.push_str(&format!("```json\n{}\n```\n", serde_json::to_string(&a.template).unwrap()));
    }
    md
}

/// Absolute path without the `\\?\` prefix.
fn abs(path: &Path) -> PathBuf {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(path));
    PathBuf::from(p.display().to_string().trim_start_matches(r"\\?\"))
}

/// Appends a plain-text record of a run to a log file.
struct RunLog {
    file: std::fs::File,
}

impl RunLog {
    fn open(path: &Path, flow: &Path, scheduled: bool) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Keep logs from growing forever: rotate at 5 MB.
        if std::fs::metadata(path).map(|m| m.len() > 5_000_000).unwrap_or(false) {
            let _ = std::fs::rename(path, path.with_extension("log.old"));
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        let mut l = Self { file };
        l.line(&format!("===== {} {}", if scheduled { "scheduled run of" } else { "run of" }, flow.display()));
        Ok(l)
    }

    fn line(&mut self, s: &str) {
        use std::io::Write;
        let ts = autoflow_core::vars::Vars::default().interpolate("${now}").unwrap_or_default();
        let _ = writeln!(self.file, "{ts}  {s}");
    }

    fn event(&mut self, e: &Event) {
        match e {
            Event::StepStarted { .. } | Event::Paused { .. } => {}
            Event::StepFinished { id, action, ok: true, millis, .. } => self.line(&format!("ok    [{id}] {action} ({millis} ms)")),
            Event::StepFinished { id, action, error, .. } => self.line(&format!("FAIL  [{id}] {action}: {}", error.as_deref().unwrap_or(""))),
            Event::Log { level, message } => self.line(&format!("{:<5} {message}", format!("{level:?}").to_uppercase())),
            Event::Finished { success, message } => self.line(&format!("RESULT {} {message}", if *success { "SUCCESS" } else { "FAILED" })),
        }
    }
}

/// Hide this process's console window (scheduled runs must not cover the app being automated).
fn hide_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleWindow() -> isize;
        }
        #[link(name = "user32")]
        extern "system" {
            fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        }
        unsafe {
            let h = GetConsoleWindow();
            if h != 0 {
                ShowWindow(h, 0);
            }
        }
    }
}

fn schedule_cmd(cmd: ScheduleCmd) -> Result<ExitCode> {
    use autoflow_core::schedule::{self, TaskSpec, Trigger};
    match cmd {
        ScheduleCmd::Add { flow, when, name, vars, time_limit_minutes, print_xml, watch } => {
            let flow = abs(&flow);
            // Refuse to schedule a broken flow.
            let (parsed, issues) = validate_json(&std::fs::read_to_string(&flow)?);
            if parsed.is_none() || issues.iter().any(|i| i.severity == Severity::Error) {
                for i in &issues {
                    eprintln!("{:?} {}: {}", i.severity, i.location, i.message);
                }
                bail!("fix the flow before scheduling it (autoflow validate)");
            }
            let mut extra = vec![];
            for v in vars {
                autoflow_core::vars::parse_override(&v)?;
                extra.push("--var".to_string());
                extra.push(v);
            }
            let name = name.unwrap_or_else(|| flow.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            let spec = TaskSpec {
                name: schedule::sanitize_name(&name),
                flow: flow.clone(),
                trigger: Trigger::parse(&when)?,
                exe: abs(&std::env::current_exe()?),
                extra_args: extra,
                time_limit_minutes,
                watch,
            };
            if print_xml {
                print!("{}", schedule::task_xml(&spec));
                return Ok(ExitCode::SUCCESS);
            }
            schedule::create(&spec)?;
            println!("scheduled \"{}\": {}", spec.name, spec.trigger.describe());
            println!("  runs:  {} run \"{}\" --scheduled", spec.exe.display(), flow.display());
            println!("  log:   {}", schedule::log_path(&flow).display());
            println!("  note:  desktop flows only run while you are logged on and the screen is unlocked");
            Ok(ExitCode::SUCCESS)
        }
        ScheduleCmd::List { json } => {
            let tasks = schedule::list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else if tasks.is_empty() {
                println!("no AutoFlowMate tasks");
            } else {
                for t in tasks {
                    let desc = t.description.trim_start_matches("AutoFlowMate | ");
                    println!("{}  [{}]", t.name, t.state);
                    println!("    {desc}");
                    println!(
                        "    next: {}   last: {} ({})",
                        if t.next_run.is_empty() { "-" } else { &t.next_run },
                        if t.last_run.is_empty() { "-" } else { &t.last_run },
                        t.last_result_text()
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        ScheduleCmd::Run { name } => {
            schedule::run_now(&name)?;
            println!("started \"{name}\" (check `autoflow schedule list` and the flow's logs folder)");
            Ok(ExitCode::SUCCESS)
        }
        ScheduleCmd::Remove { name } => {
            schedule::remove(&name)?;
            println!("removed \"{name}\"");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn flow_dir(path: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    // canonicalize adds the \\?\ prefix on Windows; strip it for friendlier paths.
    PathBuf::from(dir.display().to_string().trim_start_matches(r"\\?\"))
}

fn count_steps(steps: &[Step]) -> usize {
    let mut n = 0;
    autoflow_core::flow::visit(steps, &mut |_| n += 1);
    n
}

fn print_event(e: &Event, json: bool) {
    if json {
        println!("{}", serde_json::to_string(e).unwrap());
        return;
    }
    match e {
        Event::StepStarted { id, action, depth } => println!("{}> [{id}] {action}", "  ".repeat(*depth)),
        Event::StepFinished { id, ok: false, error, .. } => println!("  x [{id}] {}", error.as_deref().unwrap_or("")),
        Event::StepFinished { .. } | Event::Paused { .. } => {}
        Event::Log { level, message } => println!("  {:?}: {message}", level),
        Event::Finished { success, message } => println!("{} {message}", if *success { "SUCCESS" } else { "FAILED" }),
    }
}

#[cfg(windows)]
fn inspect(delay: u64, at: Option<Vec<i32>>) -> Result<ExitCode> {
    use autoflow_core::uia::Uia;
    let (x, y) = match at {
        Some(v) => (v[0], v[1]),
        None => {
            for s in (1..=delay).rev() {
                eprintln!("capturing in {s}... point at the element");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            autoflow_core::input::Input::new()?.position()?
        }
    };
    let info = Uia::new()?.inspect_point(x, y)?;
    let selector = serde_json::to_value(info.to_selector())?;
    println!("{}", serde_json::to_string_pretty(&json!({"element": info, "selector": selector}))?);
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(windows))]
fn inspect(_: u64, _: Option<Vec<i32>>) -> Result<ExitCode> {
    bail!("inspect is only available on Windows")
}

#[cfg(windows)]
fn windows(json: bool) -> Result<ExitCode> {
    let uia = autoflow_core::uia::Uia::new()?;
    let titles: Vec<String> = uia.windows("", false)?.into_iter().filter_map(|w| w.get_name().ok()).filter(|t| !t.trim().is_empty()).collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&Value::from(titles))?);
    } else {
        titles.iter().for_each(|t| println!("{t}"));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(windows))]
fn windows(_: bool) -> Result<ExitCode> {
    bail!("windows is only available on Windows")
}
