---
name: autoflowmate
description: Automate Windows desktop tasks (files, programs, windows, mouse, keyboard, UI elements, clipboard, screenshots) by writing AutoFlowMate flow JSON and running it with the `autoflow` CLI. Use when the user asks to automate a repetitive Windows task, build an RPA flow or bot, click or type into a desktop app, organise or move files on a schedule-able script, or edit/debug an existing AutoFlowMate `.json` flow.
---

# AutoFlowMate (Windows RPA)

AutoFlowMate runs **flows**: JSON files listing steps such as `file_copy`, `window_activate`,
`ui_click`, `type_text`. You write the JSON, check it with `autoflow validate`, try it with
`--dry-run`, then run it. The same file opens in the visual editor (`autoflow-studio flow.json`).

## Tools

| Command | Purpose |
|---|---|
| `autoflow actions [--json] [--category File]` | List every action with its description |
| `autoflow template <action>` | Print a step with all fields and defaults |
| `autoflow validate flow.json [--json]` | Static checks (unknown actions/fields, missing fields, bad keys, undefined variables) |
| `autoflow run flow.json [--var k=v]... [--dry-run] [--json]` | Run. Exit code 0 ok, 1 failed, 2 invalid |
| `autoflow windows [--json]` | Titles of open top-level windows |
| `autoflow inspect --at X Y` / `autoflow inspect --delay 3` | Describe the UI element at a point, plus a ready selector |
| `autoflow schema` | JSON Schema for flow files |
| `autoflow new flow.json` | Empty flow |

If `autoflow` is not on PATH, use `target\release\autoflow.exe` (or `target\debug\autoflow.exe`) in the
AutoFlowMate repo; build with `cargo build --release`.

Full action/field reference: [references/actions.md](references/actions.md). Read it before using an
action for the first time. Never invent fields: unknown fields are ignored (validate warns).

## Workflow

1. **Understand the task.** Which app, which windows, which files. Ask the user for anything
   only they know (paths, credentials are entered by them, not you).
2. **Discover the UI** (desktop apps): `autoflow windows --json` for window titles. For controls,
   have the user point at the control and run `autoflow inspect --delay 5`, or use `inspect --at X Y`.
   Prefer the returned `selector` (it uses `automation_id` when the app provides one).
3. **Write the flow** (see format below). Give each step a short unique `id`.
4. `autoflow validate flow.json --json` → fix every `error`; read every `warning`.
5. `autoflow run flow.json --dry-run` → check the plan (no files/inputs touched).
6. `autoflow run flow.json --json` → read the `step_finished` events (`ok`, `error`, `millis`)
   and the final `result` event (`success`, `message`, `variables`). Fix and repeat.
7. Hand the user the file; they can open it in AutoFlowMate Studio to view/edit/run.

## Flow format

```json
{
  "$schema": "../schema/flow.schema.json",
  "name": "Daily report",
  "description": "What it does",
  "version": 1,
  "variables": { "out": "${flow_dir}\\out", "count": 0 },
  "steps": [
    { "id": "mk", "action": "dir_create", "path": "${out}" },
    { "id": "list", "action": "dir_list", "path": "C:\\Data", "pattern": "*.csv", "output_var": "files" },
    { "id": "each", "action": "for_each", "items": "${files}", "item_var": "f", "steps": [
        { "id": "cp", "action": "file_copy", "source": "${f}", "destination": "${out}\\", "overwrite": true },
        { "id": "inc", "action": "increment", "name": "count" }
    ]},
    { "id": "done", "action": "log", "message": "copied ${count} files" }
  ]
}
```

Every step: `action` + that action's fields, plus optional common fields
`id`, `enabled`, `comment`, `on_error` (`stop` default | `continue`), `retries`, `retry_delay_ms`.

### Variables

- `${name}` inside any string. `${obj.key}` and `${list.0}` reach into objects/arrays.
- A string that is exactly `"${x}"` keeps x's type (arrays stay arrays — use this for `for_each.items`).
- Built-ins: `${now}` `${date}` `${time}` `${timestamp}` (20260131_140509) `${unix}` `${flow_dir}`
  `${cwd}` `${env.USERPROFILE}` (any env var) `${newline}` `${tab}`. `$${` = literal `${`.
- Flow `variables` are resolved in order, so later ones may use earlier ones and built-ins.
- Steps that produce values take an `output_var` (`file_read`, `dir_list`, `ui_get_text`, ...).
  `file_exists` / `window_exists` / `ui_exists` store `"true"`/`"false"`; test with `op: is_true`.
- After a step fails with `on_error: continue`, `${last_error}` holds the message.
- Override at run time: `--var name=value` (JSON values like `[1,2]` or `5` are parsed).

### Control flow

- `if`: `condition` + `then` + `else` step lists.
- `repeat` (`times`, `index_var`), `while` (`condition`, `max_iterations`), `for_each`
  (`items`, `item_var`, `index_var`) with a `steps` list. `break` leaves the innermost loop.
- `stop` ends the flow (`success` true/false). `run_flow` runs another flow file.
- Conditions: `{ "left": "${x}", "op": "...", "right": "..." , "ignore_case": false }`.
  Ops: equals, not_equals, contains, not_contains, starts_with, ends_with, greater_than,
  less_than, greater_or_equal, less_or_equal (numeric when both sides are numbers),
  is_empty, not_empty, is_true, is_false, path_exists, matches (wildcards `*` `?`).
- Arithmetic: only `increment` (`name`, `by`). Use counters for loops.

## Desktop automation: choosing the right action

Prefer, in this order (most robust first):

1. **File/program actions** (`file_*`, `dir_*`, `run_program` with `wait: true` and
   `output_var`) — no UI at all. Use them whenever the task allows (e.g. PowerShell via
   `run_program` with `program: "powershell"`, `args: ["-NoProfile","-Command","..."]`).
2. **UI Automation** (`ui_click`, `ui_set_text`, `ui_get_text`, `ui_wait`, `ui_exists`).
   Finds controls by `window` (title contains) + `automation_id` / `name` / `control_type` /
   `class_name`. Independent of screen resolution and window position. `ui_click` with
   `invoke: true` presses buttons without moving the mouse. `ui_get_text` on a `Document`
   or `Edit` reads back what is in an editor — use it to **verify** results.
3. **Keyboard** (`key_press` with combos like `ctrl+s`, `alt+f4`, `win+r`, `f5`; `type_text`)
   after `window_activate`. `type_text` modes: `keys` (default, reliable), `unicode`
   (non-Latin text), `paste` (long text, via clipboard).
4. **Mouse coordinates** (`mouse_click` x/y, `mouse_drag`) — last resort; breaks when
   windows move or DPI changes. Use `window_set_state` maximized first to stabilise layouts.

Robustness rules:

- Always `window_activate` (it waits up to `timeout_ms`) before keyboard input.
- Wait for state instead of sleeping: `window_wait`, `ui_wait`, `window_wait_close`.
  Use `wait` only for short settling (200–1000 ms) after an action whose effect you cannot detect.
- Window titles change ("Untitled - Notepad" → "*notes - Notepad"); match a stable part.
  Dialogs owned by a window (e.g. "Save as") are found by their own title too.
- Add `retries` (and `retry_delay_ms`) to steps that touch flaky UI.
- End with a verification step (`file_exists`, `ui_get_text` + `if`, `screenshot`) and a `stop`
  with `success: false` if the check fails, so failures are visible.
- Don't close or discard windows/tabs/documents the flow did not create.

## Safety

- Moving the mouse to the **top-left screen corner aborts** a run (fail-safe). Studio also has Stop.
- Use `--dry-run` before running anything that deletes, moves, kills programs or types.
- `file_delete` accepts wildcards: double-check the path; prefer `missing_ok` over broad patterns.
- Never put passwords or secrets in flow files; ask the user to type them or pass them at run
  time with `--var` from their own secure source.
- Keyboard/mouse steps act on whatever has focus; tell the user not to use the PC while a
  desktop flow runs.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `unknown action` / `unknown field` | Check `autoflow actions`, `autoflow template <action>` |
| `window "X" not found` | `autoflow windows`; use a shorter substring; raise `timeout_ms` |
| `element not found` | Re-`inspect`; drop `name` if it changes; add `control_type`; set `name_contains` |
| Typed text has missing characters | Activate window first; increase `delay_ms`; try `mode: "paste"` |
| Shortcut did nothing | Add `wait` 300–500 ms before it; the app may still be loading |
| `variable ... is never set` warning | Define it in `variables` or pass `--var` |

## Examples

See the repo's `examples/` folder:
`file_organizer.json` (files, loops, conditions, report), `calculator_uia.json` (UI Automation,
read-back verification), `notepad_keyboard.json` (keyboard, Save-As dialog, file check).
