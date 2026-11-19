# Architecture

```
          ┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
          │ autoflow-studio  │     │   autoflow CLI   │     │   AI agent       │
          │ (egui editor)    │     │ run/validate/... │     │ (SKILL.md)       │
          └────────┬─────────┘     └────────┬─────────┘     └────────┬─────────┘
                   │      flow JSON (serde) │  events (JSON lines)   │ shells out to CLI
                   └───────────────┬────────┴────────────────────────┘
                          ┌────────▼─────────┐
                          │  autoflow-core   │
                          │ flow · actions   │
                          │ validate · vars  │
                          │ engine           │
                          └──┬─────┬──────┬──┘
                    enigo ───┘     │      └─── std::fs / process / glob
             (mouse, keyboard)     │
                         uiautomation (UIA COM)      xcap · arboard
```

## Core ideas

- **One source of truth for actions.** `actions/mod.rs` defines each action as a struct with
  `#[serde(default)]`, registered in the `actions!` macro list. From that the code derives
  the `Action` enum (`"action": "name"` tag), the palette/CLI catalog, default templates,
  the JSON Schema (schemars, doc comments become descriptions) and the generated Markdown reference.
- **Flat step JSON.** `Step` has common fields (`id`, `enabled`, `on_error`, `retries`, ...) and
  flattens the action, so a step reads `{"id":"s1","action":"file_copy","source":...}`.
  This is easy for people and LLMs to write.
- **Engine** (`engine.rs`) walks steps recursively. It returns a control signal (`Next` / `Break` /
  `Stop`) for loops, applies retries and `on_error` per step, and reports `Event`s through a
  callback. The CLI prints them, and the GUI forwards them over a channel from a worker thread.
  Cancellation is an `AtomicBool`, checked between steps and during waits. The fail-safe
  (mouse in the top-left corner) is checked before every input action.
- **Variables** (`vars.rs`): a JSON map with `${...}` interpolation, dotted access and built-ins.
  Initial variables resolve in order.
- **Validation** (`validate.rs`) works on raw JSON plus the parsed flow. It reports unknown
  actions with suggestions, unknown or misspelled fields with the valid list, per-step type errors,
  required fields, bad key combos and control types, `break` outside loops, and undefined
  variables. The messages are written for agents to act on.
- **Settings and fail-safe** (`flow.rs` `Settings`, `failsafe.rs`). A watcher thread polls the
  cursor and the abort hotkey every 20 ms (`GetCursorPos` / `GetAsyncKeyState`) and sets the
  run's stop flag with a reason. Typing, smooth moves, drags and waits check that flag between
  keystrokes and slices, so an abort takes effect within about 20 ms. A corner or hotkey that is
  already active when the run starts is ignored until it is released.
- **Vision** (`vision.rs`). Screen capture goes through xcap, per monitor or region. Template
  matching is zero-mean NCC (OpenCV `TM_CCOEFF_NORMED`): a coarse pass on a ¼–½ scale pyramid
  level, then refinement at full resolution around the candidates. It uses integral images for
  window statistics and splits rows across threads. The process is per-monitor DPI-aware, so
  mouse, screenshots, UIA and image coordinates are all physical pixels.
- **Flowchart** (`autoflow-gui/src/flowchart.rs`). It is a structured layout of the step tree, not
  a free-form node graph: every flow stays valid JSON, with no dangling edges. Sizes are measured
  recursively. `if` lays out two branch columns that merge, and loops get a frame with a
  loop-back arrow. Connectors are egui drag-and-drop targets carrying `(list path, index)`.
- **Web** (`web.rs`) starts Edge or Chrome itself with `--remote-debugging-port` on a free port
  and a private temporary profile. It reads `/json/version` to get the DevTools WebSocket URL,
  then attaches with `headless_chrome::Browser::connect`. The browser is not launched through
  headless_chrome because that waits for a stderr line that Edge on Windows never prints, and
  `msedge.exe` is a launcher that exits immediately. Elements are found by
  `[id="…"]` or a CSS selector. Typing uses `Input.insertText`, and the browser is closed with
  `Browser.close` and its profile deleted when the run ends.
- **Scheduling** (`schedule.rs`) registers tasks in Task Scheduler's `\AutoFlowMate\` folder
  from Task XML via `schtasks /Create /XML`. XML is used rather than option strings because its
  dates are locale-independent and there is no 261-character `/TR` limit. Tasks use an
  `InteractiveToken` principal, so they run only while the user is logged on, which desktop
  input requires. Other settings: `IgnoreNew` for overlapping runs, `StartWhenAvailable`, and
  battery allowed. Listing uses PowerShell's `Get-ScheduledTask` because it returns structured,
  locale-independent fields. Scheduled runs call `autoflow run --scheduled`, which hides the
  console and logs to `logs\<flow>.log`.
- **Windows specifics** are behind `cfg(windows)` (`uia.rs`, window and `ui_*` actions), so the
  core still compiles elsewhere. That leaves room for Linux/macOS backends later.

## Adding an action

1. Add a struct with `#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]`
   and `#[serde(default)]`, documenting each field with `///`.
2. Register it in the `actions!` list (category, name, variant, description).
3. Handle it in `Context::exec` (`engine.rs`); respect `self.dry(...)` for side effects.
4. Add required-field checks in `validate.rs::check_action`.
5. Run `autoflow schema -o schema/flow.schema.json` and
   `autoflow docs -o skills/autoflowmate/references/actions.md`.

The GUI form, palette and CLI pick up the action with no further changes.

## Roadmap

| Area | Idea | Likely crate/approach |
|---|---|---|
| Web | XPath targets, multiple tabs, file upload/download, cookies, attach to the user's logged-in browser, full-page screenshots | `headless_chrome` CDP calls |
| Vision | OCR (`ocr_read`, `text_click`), multi-scale image search | Windows.Media.Ocr via `windows` crate |
| Recorder | Record mouse/keyboard/UIA events into steps | low-level hooks (`rdev`) plus `inspect_point` per click |
| Data | `excel_read/write`, `csv_read`, JSON/regex helpers, arithmetic expressions | `calamine`, `rust_xlsxwriter`, `csv`, `evalexpr` |
| Triggers | File-arrival / folder-watch trigger (`autoflow watch`, started at log-on by a scheduled task), e-mail trigger | polling or `notify` crate |
| Agents | MCP server exposing run/validate/inspect/windows as tools | `rmcp` |
| Studio | Undo/redo, unsaved-changes prompt on close, breakpoints and step-through, variable watch | egui |
