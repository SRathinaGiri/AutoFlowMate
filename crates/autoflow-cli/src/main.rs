//! `autoflow` — run, validate and author AutoFlowMate flows from the command line.
//! Every command supports `--json` output so AI agents can drive it.

use anyhow::{bail, Result};
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
    /// Create a new, empty flow file.
    New {
        file: PathBuf,
        #[arg(long, default_value = "New flow")]
        name: String,
    },
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
    /// Find an image on screen and print the matches (to test image_* steps).
    Locate {
        image: PathBuf,
        #[arg(long, default_value_t = 0.9)]
        confidence: f64,
        /// Report every match, not just the best.
        #[arg(long)]
        all: bool,
    },
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
        Cmd::Run { flow, vars, dry_run, json, step_delay, key_delay, input_pause, no_failsafe, no_validate } => {
            let text = std::fs::read_to_string(&flow)?;
            let (parsed, issues) = validate_json(&text);
            let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
            if !no_validate || parsed.is_none() {
                for i in &issues {
                    if json {
                        println!("{}", json!({"event": "validation", "severity": i.severity, "location": i.location, "message": i.message}));
                    } else {
                        eprintln!("{:?} {}: {}", i.severity, i.location, i.message);
                    }
                }
                if has_errors {
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
            let mut sink = |e: Event| print_event(&e, json);
            let result = run_flow(&flow_doc, overrides, &opts, &mut sink);
            if json {
                println!("{}", json!({"event": "result", "success": result.success, "message": result.message, "variables": result.variables}));
            }
            Ok(if result.success { ExitCode::SUCCESS } else { ExitCode::from(1) })
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
            let list: Vec<_> = catalog()
                .into_iter()
                .filter(|a| category.as_ref().map(|c| a.category.eq_ignore_ascii_case(c)).unwrap_or(true))
                .collect();
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
        Cmd::New { file, name } => {
            if file.exists() {
                bail!("{} already exists", file.display());
            }
            let flow = Flow { name, ..Flow::default() };
            flow.save(&file)?;
            println!("created {}", file.display());
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
        Cmd::Locate { image, confidence, all } => {
            let t = autoflow_core::vision::load_template(&image)?;
            let started = std::time::Instant::now();
            let found = autoflow_core::vision::locate(&t, None, confidence, all)?;
            println!("{}", serde_json::to_string_pretty(&json!({"found": !found.is_empty(), "matches": found, "millis": started.elapsed().as_millis() as u64}))?);
            Ok(if found.is_empty() { ExitCode::from(1) } else { ExitCode::SUCCESS })
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
        Event::StepFinished { .. } => {}
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
    let titles: Vec<String> = uia
        .windows("", false)?
        .into_iter()
        .filter_map(|w| w.get_name().ok())
        .filter(|t| !t.trim().is_empty())
        .collect();
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
