---
name: autoflowmate
description: Automate Windows desktop and web tasks (files, programs, windows, mouse, keyboard, UI elements, web pages by element id, image recognition, on-screen text via OCR, dialogs, clipboard, screenshots), data work (Excel/CSV/PDF and PDF tables in, CSV/Excel/Word out, Word templates, zip, SQLite and ODBC databases, text/date/maths helpers), e-mail (send, read and save attachments), web APIs and AI models (LM Studio, Ollama, OpenAI, Gemini, Claude — e.g. classify comments) by writing AutoFlowMate flow JSON (or recording it with `autoflow record`) and running it with the `autoflow` CLI, on a schedule or whenever a file or e-mail arrives, — a PyAutoGUI-like toolkit with a fail-safe. Use when the user asks to automate a repetitive Windows task (including slow apps such as Tally), build an RPA flow or bot, click or type into a desktop app, find/click an image or text on screen, process files, mail or database rows in bulk, or edit/debug an existing AutoFlowMate `.json` flow.
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
| `autoflow mouse [--once]` | Live mouse X/Y and pixel colour (PyAutoGUI displayMousePosition) |
| `autoflow locate image.png [--confidence 0.9] [--all]` | Test whether an image is found on screen right now |
| `autoflow web-pick <url>` | Open a page, Ctrl+click an element, get its `element_id` / `selector` |
| `autoflow ocr [--window T] [--image P] [--find TEXT]` | What Windows OCR reads on screen / in a window / in a picture (to test `ocr_*`); `--languages` lists OCR languages |
| `autoflow secret set NAME` / `check NAME` / `delete NAME` | Store a password or API key (hidden prompt) in Windows Credential Manager (`password_secret`, `api_key_secret`, `bearer_secret`) |
| `autoflow record new.json` | Record the user's clicks and typing as a flow (Ctrl+Alt+R stops); passwords are never recorded |
| `autoflow watch flow.json [--once]` | Run the flow for each new file / e-mail listed in its `triggers` |
| `autoflow schedule add flow.json --when "daily 09:00"` | Run a flow automatically (Windows Task Scheduler); `--when logon --watch` starts the watcher at log-on |
| `autoflow schedule list` / `run NAME` / `remove NAME` | See next/last run and result, start now, delete |
| `autoflow run flow.json --log run.log` | Also append a timestamped log of the run |
| `autoflow schema` | JSON Schema for flow files |
| `autoflow new flow.json` | Empty flow |

If `autoflow` is not on PATH, use `target\release\autoflow.exe` (or `target\debug\autoflow.exe`) in the
AutoFlowMate repo; build with `cargo build --release`.

Full action/field reference: [references/actions.md](references/actions.md). Read it before using an
action for the first time. The authoritative list is `autoflow actions --json` and
`autoflow template <action>`; trust them over memory. Never invent fields: an unknown field is
ignored at run time, so treat an "unknown field" warning as an error and fix it (a misspelt
`send_now`, `overwrite` or privacy option silently changes behaviour).

## Workflow

1. **Understand the task.** Which app, which windows, which files. Ask the user for anything
   only they know (paths, credentials are entered by them, not you).
2. **Discover the UI** (desktop apps): `autoflow windows --json` for window titles. For controls,
   have the user point at the control and run `autoflow inspect --delay 5`, or use `inspect --at X Y`.
   Prefer the returned `selector` (it uses `automation_id` when the app provides one).
3. **Write the flow** (see format below). Give each step a short unique `id`.
4. `autoflow validate flow.json --json` → fix every `error`; read every `warning`.
5. `autoflow run flow.json --dry-run` → check the plan (no files/inputs touched; waits are
   skipped and `while` loops make one pass). Verification of files the flow *would* create
   fails in a dry run — that is expected, not a bug to fix.
6. **Confirmation gate.** Before the first real run, if the flow does any of the following, tell
   the user exactly what it will do and wait for their yes. Ask again when that part changes:
   - deletes or overwrites files (`file_delete`, `dir_delete`, `file_move`, `overwrite: true`);
   - changes a database (`db_execute`);
   - changes a mailbox (`read_email` / `outlook_read` with `mark_read` or `move_to`);
   - sends e-mail (`send_email`, `outlook_send` with `send_now`);
   - submits forms or posts data to websites and APIs (`browser_click` on a submit, `http_request`
     other than GET);
   - sends the user's data to a cloud AI provider (`ai_prompt` with openai / gemini / anthropic);
   - kills programs (`kill_program`);
   - redacts or pseudonymises data (`redact_files`, `pseudonymize`, `depseudonymize`): say what
     goes where and that results need a human spot-check; never set `continue_on_problems`
     without the user's explicit agreement;
   - runs a program, script or shell command (`run_program`, PowerShell, `cmd /c`, `.bat`):
     judge it by its **effect** and apply the same rules as above (a `Remove-Item` is a delete,
     an `Invoke-WebRequest -Method Post` is a post). Prefer the native action, which the
     validator and dry run understand.

   A dry run never needs this; "run it" from the user for a flow you just described counts as the yes.
7. `autoflow run flow.json --json` → read the `step_finished` events (`ok`, `error`, `millis`)
   and the final `result` event (`success`, `message`, `variables`; typed passwords show as
   `••••••`). Fix and repeat.
8. Hand the user the file; they can open it in AutoFlowMate Studio to view/edit/run. Studio has
   undo/redo, breakpoints (F9 on a step) and step-by-step running (F10) that shows every
   variable's value before each step — suggest it when the user wants to see what a flow does.

## Flow format

```json
{
  "$schema": "../schema/flow.schema.json",
  "name": "Daily report",
  "description": "What it does",
  "version": 1,
  "settings": { "key_delay_ms": 100, "input_pause_ms": 100, "step_delay_ms": 0,
                "mouse_move_ms": 0, "failsafe": true, "abort_hotkey": "ctrl+alt+q" },
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
`id`, `enabled`, `comment`, `on_error` (`stop` default | `continue`), `retries`, `retry_delay_ms`,
`delay_before_ms`, `delay_after_ms`.

### Timing (settings)

`settings` applies to the whole run (CLI flags override for one run):

| setting | default | meaning | CLI override |
|---|---|---|---|
| `key_delay_ms` | 100 | between keystrokes (`type_text`) and repeated `key_press` | `--key-delay` |
| `input_pause_ms` | 100 | pause after every mouse/keyboard action (PyAutoGUI `PAUSE`) | `--input-pause` |
| `step_delay_ms` | 0 | pause after every action step (not after `if`/loops/`try` themselves; the steps inside get it) | `--step-delay` |
| `mouse_move_ms` | 0 | default duration of mouse moves | — |
| `failsafe` | true | abort when the mouse is pushed into any screen corner | `--no-failsafe` |
| `abort_hotkey` | `ctrl+alt+q` | abort at any time; `""` disables | — |

Slow or busy apps (Tally, ERP clients, remote desktops) drop keys: raise `key_delay_ms` to
150–250 and `input_pause_ms` to 200–500, or set `delay_ms` on a single `type_text` /
`interval_ms` on a `key_press`. Use `delay_before_ms`/`delay_after_ms` on individual steps that
trigger slow screens.

### Variables

- `${name}` inside any string. `${obj.key}` and `${list.0}` reach into objects/arrays.
- A string that is exactly `"${x}"` keeps x's type (arrays stay arrays — use this for `for_each.items`).
- Built-ins: `${now}` `${date}` `${time}` `${timestamp}` (20260131_140509) `${unix}` (seconds)
  `${flow_dir}` `${cwd}` `${env.USERPROFILE}` (any env var) `${newline}` `${tab}`. `$${` = literal `${`.
- Custom date/time: `${now:FORMAT}` with Excel/.NET-style tokens `yyyy yy MMMM MMM MM dd dddd ddd
  HH` (24 h) `hh` (12 h) `mm ss fff tt`, e.g. `${now:yyyyMMdd_HHmmss}` → `20261119_102333`,
  `${now:dd-MM-yyyy}` → `19-11-2026`. A pattern containing `%` is used as strftime. `MM` = month,
  `mm` = minutes. Timestamps are evaluated when used; to reuse one value, store it once in
  `variables` (`"stamp": "${now:yyyyMMdd_HHmmss}"`) and refer to `${stamp}`.
- Flow `variables` are resolved in order, so later ones may use earlier ones and built-ins.
- Optionally declare each variable's type in `variable_types` (same names): `text`, `integer`,
  `number`, `boolean`, `date` (YYYY-MM-DD), `time` (HH:MM), `datetime`, `url`, `file`, `folder`,
  `list`. Studio then shows a matching editor (date picker, Browse buttons, …) and offers the
  variable in fields of that kind; `validate` warns when a value does not fit its type. Use
  names made of letters, digits and `_`.
  ```json
  "variables":      { "report_date": "2026-11-19", "input_file": "D:\\data\\sales.xlsx", "retries": 3 },
  "variable_types": { "report_date": "date", "input_file": "file", "retries": "integer" }
  ```
- Steps that produce values take an `output_var` (`file_read`, `dir_list`, `ui_get_text`, ...).
  `file_exists` / `window_exists` / `ui_exists` store `"true"`/`"false"`; test with `op: is_true`.
- After a step fails with `on_error: continue`, `${last_error}` holds the message.
- Override at run time: `--var name=value` (JSON values like `[1,2]` or `5` are parsed).

### Control flow

- `if`: `condition` + `then` + `else` step lists.
- `repeat` (`times`, `index_var`), `while` (`condition`, `max_iterations`), `for_each`
  (`items`, `item_var`, `index_var`) with a `steps` list. `break` leaves the innermost loop.
- `stop` ends the flow (`success` true/false). `run_flow` runs another flow file.
- `try` runs `steps`; on failure it runs `catch` (message in `${error}`) and carries on;
  `finally` always runs. See *Error handling* below.
- Conditions: `{ "left": "${x}", "op": "...", "right": "..." , "ignore_case": false }`.
  Ops: equals, not_equals, contains, not_contains, starts_with, ends_with, greater_than,
  less_than, greater_or_equal, less_or_equal (numeric when both sides are numbers),
  is_empty, not_empty, is_true, is_false, path_exists, matches (wildcards `*` `?`).
- Arithmetic: `calculate` (`expression`, `decimals`); `increment` for counters. Dates:
  `date_calc`. Text: `text_*`. See [references/data.md](references/data.md).

## Error handling: try / catch / finally

```json
{ "id": "safe", "action": "try",
  "steps":   [ { "id": "pdf", "action": "read_pdf", "path": "${file}", "output_var": "text" } ],
  "catch":   [ { "id": "warn", "action": "log", "level": "warn", "message": "skipped ${file}: ${error}" } ],
  "finally": [ { "id": "tidy", "action": "browser_close" } ] }
```

When a step inside `steps` fails, the rest of `steps` is skipped, the message goes to
`error_var` (default `error`) and `catch` runs; the flow then continues after the `try`. `finally`
always runs (success, failure, `break` or `stop`). Use it inside `for_each` so one bad item does not
stop the batch; use `on_error: continue` for a single optional step. Stopping the run (fail-safe,
Stop button) is never caught.

## Topic guides (read before using these actions)

The details for each area are in `references/`. Read the matching file before writing steps from it:

| Task | Read |
|---|---|
| The user gives numbered keystrokes / waits for a desktop app (Tally…) | [references/keystroke-recipe.md](references/keystroke-recipe.md) |
| Excel / CSV / PDF in, CSV / Excel out, `list_append`, text, dates, `calculate` | [references/data.md](references/data.md) |
| `ai_prompt` (classify, summarise, extract), `http_request` | [references/ai-and-apis.md](references/ai-and-apis.md) |
| Sending or reading e-mail, attachments | [references/email.md](references/email.md) |
| SQLite / ODBC (SQL Server, Access, Excel, Tally) | [references/databases.md](references/databases.md) |
| Tables in PDFs, Word read / templates / reports, zip | [references/documents.md](references/documents.md) |
| Redacting personal data (PAN, Aadhaar, names...) in documents or text; pseudonymising IDs with a reversible vault | [references/privacy.md](references/privacy.md) |
| `ocr_*`: text on screen | [references/ocr.md](references/ocr.md) |
| Web pages: `browser_*`, element ids, CAPTCHAs | [references/web.md](references/web.md) |
| Running a flow on a schedule, or when a file / e-mail arrives (`triggers`, `autoflow watch`) | [references/scheduling.md](references/scheduling.md) |
| Ready-made example flows to copy from | [references/examples.md](references/examples.md) |
| Every action and field | [references/actions.md](references/actions.md) |

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
4. **Image recognition** (`image_click`, `image_wait`, `image_wait_vanish`, `image_find`,
   `image_find_all`) for apps without accessible controls (Tally, games, remote desktops, custom
   canvases). `image` is a PNG of the target (the user snips it with Win+Shift+S or Studio's
   "📷 Snip image"); `confidence` 0.9 is a good start; give a `region` to speed it up. Verify with
   `autoflow locate img.png`. `pixel_get` / `pixel_matches` check a single colour.
5. **On-screen text (OCR)** (`ocr_click`, `ocr_find`, `ocr_read`) when there are no controls
   but the target has a visible label: survives theme/zoom changes better than images. See
   [references/ocr.md](references/ocr.md).
6. **Mouse coordinates** (`mouse_click` x/y, `mouse_move` with `duration_ms`/`tween`,
   `mouse_drag`, `mouse_down`/`mouse_up`) — last resort; breaks when windows move or DPI changes.
   Use `window_set_state` maximized first to stabilise layouts. `screen_size` gives the resolution.

Ask the user with `message_box` (ok/ok_cancel/yes_no…, answer in `output_var`) or `input_box`
(`password: true` masks input) when a flow needs a decision or a value at run time.
`key_down`/`key_up` hold modifiers (e.g. shift while clicking).

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

## Scheduling (summary)

Use `autoflow schedule add flow.json --when "weekdays 18:30"` (Windows Task Scheduler), never a
polling loop. Desktop flows need the PC logged in and unlocked; scheduled flows must not use
`message_box`/`input_box`. Details: [references/scheduling.md](references/scheduling.md).

## Safety

- **Fail-safe:** pushing the mouse into **any screen corner** aborts a run (even mid-typing),
  and so does the **abort hotkey** (`ctrl+alt+q` by default). Studio also has Stop. Don't
  disable these unless the user asks; never move the mouse into a corner on purpose.
- Use `--dry-run` before running anything that deletes, moves, kills programs or types.
- `file_delete` accepts wildcards: double-check the path; prefer `missing_ok` over broad patterns.
- Never put passwords, API keys or tokens in flow files. Store them with `autoflow secret set NAME`
  and refer to the name (`password_secret`, `api_key_secret`, `bearer_secret`, `PWD={password}`).
- Sending data to a cloud AI provider or web API shares it with that company: confirm with the
  user first when it is customer, personal or financial data.
- `db_execute` changes databases: test against a copy or with `--dry-run`, and use `WHERE`
  carefully. `read_email` with `move_to` / `mark_read` changes the mailbox.
- Keyboard/mouse steps act on whatever has focus; tell the user not to use the PC while a
  desktop flow runs.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `unknown action` / `unknown field` | Check `autoflow actions`, `autoflow template <action>` |
| `window "X" not found` | `autoflow windows`; use a shorter substring; raise `timeout_ms` |
| `element not found` | Re-`inspect`; drop `name` if it changes; add `control_type`; set `name_contains` |
| Typed text has missing characters | Activate window first; raise `settings.key_delay_ms` (Tally: 150–250); try `mode: "paste"` |
| `image ... not found` | `autoflow locate img.png --confidence 0.8`; re-snip at the current zoom/theme; add `region` |
| Run aborted with "fail-safe triggered" | The mouse touched a corner; keep coordinates away from corners |
| Shortcut did nothing | Add `wait` 300–500 ms before it; the app may still be loading |
| `variable ... is never set` warning | Define it in `variables` or pass `--var` |
| `unknown variable` while running | A `${x}` was used before any step set it (e.g. inside `if` after a failed step): declare it in `variables` with a default |
| `cannot reach the local AI server` | Start LM Studio's server (Developer tab) and load a model, or set `provider` |
| `the model replied ..., which is none of the labels` | Make the `system` text clearer, add `retries`, or add a label such as `Unknown` |
| IMAP `login failed` | Gmail: enable IMAP, use an App Password stored with `autoflow secret set`; Microsoft 365: use `outlook_read` |
| `SQL error` / placeholder count error | One `?` per value in `params`; quote table names with spaces like `[Sheet1$]` |
| OCR text `not found` | `autoflow ocr --window ...` to see what it reads; maximise the window; raise `scale` to 3; match fewer words |
