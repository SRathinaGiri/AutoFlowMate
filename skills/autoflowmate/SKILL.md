---
name: autoflowmate
description: Automate Windows desktop and web tasks (files, programs, windows, mouse, keyboard, UI elements, web pages by element id, image recognition, on-screen text via OCR, dialogs, clipboard, screenshots), data work (Excel/CSV/PDF in, CSV/Excel out, SQLite and ODBC databases, text/date/maths helpers), e-mail (send, read and save attachments), web APIs and AI models (LM Studio, Ollama, OpenAI, Gemini, Claude — e.g. classify comments) by writing AutoFlowMate flow JSON and running it with the `autoflow` CLI — a PyAutoGUI-like toolkit with a fail-safe. Use when the user asks to automate a repetitive Windows task (including slow apps such as Tally), build an RPA flow or bot, click or type into a desktop app, find/click an image or text on screen, process files, mail or database rows in bulk, or edit/debug an existing AutoFlowMate `.json` flow.
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
| `autoflow schedule add flow.json --when "daily 09:00"` | Run a flow automatically (Windows Task Scheduler) |
| `autoflow schedule list` / `run NAME` / `remove NAME` | See next/last run and result, start now, delete |
| `autoflow run flow.json --log run.log` | Also append a timestamped log of the run |
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
5. `autoflow run flow.json --dry-run` → check the plan (no files/inputs touched; waits are
   skipped and `while` loops make one pass). Verification of files the flow *would* create
   fails in a dry run — that is expected, not a bug to fix.
6. `autoflow run flow.json --json` → read the `step_finished` events (`ok`, `error`, `millis`)
   and the final `result` event (`success`, `message`, `variables`). Fix and repeat.
7. Hand the user the file; they can open it in AutoFlowMate Studio to view/edit/run. Studio has
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
| `step_delay_ms` | 0 | pause after every step | `--step-delay` |
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
  `date_calc`. Text: `text_*`. See *Text, dates and maths* below.

## Recipe: a numbered list of keystrokes → a flow

Users often describe a desktop task as numbered steps ("Open X, wait 20 s, press T, press Down
6 times…"). Translate it **one instruction per step, in order**, keeping the user's numbering in
the ids (`s01_open`, `s07_down6`), then apply the robustness additions below.

| Instruction | Step |
|---|---|
| Open / start `C:\App\app.exe` | `{"action":"run_program","program":"C:\\App\\app.exe","working_dir":"C:\\App"}` (put the path in `variables`) |
| Wait N seconds | `{"action":"wait","ms":N000}` |
| Press T / Enter / Escape / F5 | `{"action":"key_press","keys":"t"}` · `"enter"` · `"esc"` · `"f5"` |
| Press Ctrl + E, Alt+F4, Ctrl+Shift+S | `"keys":"ctrl+e"` · `"alt+f4"` · `"ctrl+shift+s"` |
| Press Down arrow 6 times | `{"action":"key_press","keys":"down","times":6}` (arrows: `up down left right`) |
| Type "text" | `{"action":"type_text","text":"text"}` |
| …with a timestamp `yyyyMMdd_HHmmss` | `"text":"Report_${now:yyyyMMdd_HHmmss}.xml"` (or `${timestamp}` for exactly that format) |
| Wait/pause N s between **all** steps | `settings.step_delay_ms: N000` — not a `wait` between every step |
| Click at x,y | `{"action":"mouse_click","x":X,"y":Y}` |

Rules:

- "Between all steps" delays go in `settings.step_delay_ms`; explicit waits in the list stay as
  `wait` steps (they add to the step delay — mention that to the user).
- Keyboard-driven business apps (Tally, ERP, terminal emulators) need `settings.key_delay_ms`
  150–250 so no keystroke is lost.
- Insert `window_activate` (title substring of the app) right before the first keystroke, so
  keys can never go to another window. Tell the user you added it.
- Key names are lowercase; a single letter `t` is a plain keypress (no Shift). Uppercase text
  belongs in `type_text`.
- Put file names/paths in `variables` so they appear once and can be overridden with `--var`.
- Unclear instruction (which window? 12 h or 24 h clock — `hh` vs `HH`?)? Pick the safe/likely
  meaning, say which you chose, and keep going.

What to deliver by default: the faithful translation **plus** upgrades 2–4 below (they need
nothing from the user and only replace fixed sleeps or add checks). Apply upgrade 1 only when the
user gives you an image of the ready screen (or the app exposes a window title/UI element to
wait for); otherwise keep their fixed wait and explain how to upgrade it. Always list every step
you added or changed in your reply.

More decisions you will meet:

- **Save dialogs:** if the user names the output folder, type the full path
  (`${out_dir}\\${file}`) into the file-name box — apps remember the last folder, so a bare
  name may land elsewhere. Verify exactly that path.
- **Verify before or after closing the app?** Before closing (as in the Tally example) leaves the
  app open on the failing screen — best when a person will look. For unattended/scheduled runs,
  close the app first and verify last, so nothing is left running.
- **`message_box` / `input_box` wait for a click** — good for attended runs and loud failures,
  wrong for unattended ones (the run hangs). Use `stop` with `success: false` (exit code 1)
  for unattended failure reporting; add a message box only if the user wants a pop-up.
- **Timestamps:** use a single token for the whole stamp (`${now:dd-MM-yyyy_HHmm}`, not two
  `${now:…}` tokens) and define it once in `variables` — variables are evaluated once at the
  start, so the typed name and the checked path always match.
- **Numbers from variables:** numeric fields such as `ms`, `times`, `x`, `y`, `by`, `amount`
  accept a `"${var}"` string, e.g. `{ "action": "wait", "ms": "${pause}" }`.
- **Window titles** are matched as a case-insensitive substring of the top-level window (and
  its dialogs). If unsure, ask the user to run `autoflow windows` with the app open; splash
  screens may have a different title, so prefer the main window's stable word.

Robustness upgrades:

1. **Wait for a screen instead of a fixed startup sleep.** If the app shows an identifiable
   screen when it is ready, wait for it: `image_wait` with a PNG of a stable part of that screen
   (ask the user to snip it with Win+Shift+S or Studio's "📷 Snip image" and save it under
   `images/` next to the flow), or `ui_wait` / `window_wait` when the app exposes UI Automation
   elements or a distinct window title. Keep a generous `timeout_ms` (60000 for slow starts).
2. **Wait for an output file instead of a fixed sleep**, with a clock-based deadline (independent
   of `step_delay_ms`):
   ```json
   { "id": "deadline", "action": "set_variable", "name": "deadline", "value": "${unix}" },
   { "id": "plus30", "action": "increment", "name": "deadline", "by": 30 },
   { "id": "wait_file", "action": "while", "max_iterations": 200,
     "condition": { "left": "${unix}", "op": "less_than", "right": "${deadline}" },
     "steps": [
       { "id": "exists", "action": "file_exists", "path": "${out_path}", "output_var": "done" },
       { "id": "done_if", "action": "if", "condition": { "left": "${done}", "op": "is_true" },
         "then": [ { "id": "brk", "action": "break" } ],
         "else": [ { "id": "nap", "action": "wait", "ms": 500 } ] } ] }
   ```
3. **Verify the result and fail loudly:**
   ```json
   { "id": "verify", "action": "if",
     "condition": { "left": "${out_path}", "op": "path_exists" },
     "then": [ { "id": "info", "action": "file_info", "path": "${out_path}", "output_var": "f" },
               { "id": "ok", "action": "log", "message": "Saved ${f.name} (${f.size} bytes)" } ],
     "else": [ { "id": "fail", "action": "stop", "success": false,
                 "message": "${out_path} was not created" } ] }
   ```
4. **Confirm the app closed** after quitting: `window_wait_close` with `on_error: "continue"`.

Know where the app saves files (Tally exports into its own install folder by default) so the
verification checks the right path; ask the user if unsure.

### Worked example (Tally Balance Sheet export)

Request: *"1. Open D:\TallyPrime\tally.exe 2. Wait 20 s 3. Press T 4. Wait 2 s 5. Enter 6. Wait 3 s
7. Down ×6 8. Enter 9. Wait 3 s 10. Ctrl+E 11. C 12. Down ×16 13. Enter 14. Type
BalanceSheet_<yyyyMMdd_hhmmss>.xml 15. Enter 16. Ctrl+A 17. E 18. Wait 10 s 19. Alt+F4 20. Esc
21. Esc 22. Y — wait 1 s between all steps."* Result (with upgrades 1–4; the plain translation
simply keeps `wait` 20000 for step 2 and `wait` 10000 for step 18):

```json
{
  "name": "Tally: export Balance Sheet",
  "version": 1,
  "settings": { "key_delay_ms": 150, "input_pause_ms": 100, "step_delay_ms": 1000,
                "mouse_move_ms": 0, "failsafe": true, "abort_hotkey": "ctrl+alt+q" },
  "variables": {
    "tally_exe": "D:\\TallyPrime\\tally.exe",
    "export_dir": "D:\\TallyPrime",
    "export_file": "BalanceSheet_${now:yyyyMMdd_HHmmss}.xml",
    "export_path": "${export_dir}\\${export_file}"
  },
  "steps": [
    { "id": "s01_open", "action": "run_program", "program": "${tally_exe}", "working_dir": "${export_dir}" },
    { "id": "s02_wait_ready", "action": "image_wait", "image": "images/tally_ready.png", "confidence": 0.9, "timeout_ms": 60000 },
    { "id": "s02b_focus", "action": "window_activate", "title": "TallyPrime", "timeout_ms": 10000 },
    { "id": "s03_t", "action": "key_press", "keys": "t" },
    { "id": "s04_wait", "action": "wait", "ms": 2000 },
    { "id": "s05_enter", "action": "key_press", "keys": "enter" },
    { "id": "s06_wait", "action": "wait", "ms": 3000 },
    { "id": "s07_down6", "action": "key_press", "keys": "down", "times": 6 },
    { "id": "s08_enter", "action": "key_press", "keys": "enter" },
    { "id": "s09_wait", "action": "wait", "ms": 3000 },
    { "id": "s10_export", "action": "key_press", "keys": "ctrl+e" },
    { "id": "s11_c", "action": "key_press", "keys": "c" },
    { "id": "s12_down16", "action": "key_press", "keys": "down", "times": 16 },
    { "id": "s13_enter", "action": "key_press", "keys": "enter" },
    { "id": "s14_filename", "action": "type_text", "text": "${export_file}" },
    { "id": "s15_enter", "action": "key_press", "keys": "enter" },
    { "id": "s16_ctrl_a", "action": "key_press", "keys": "ctrl+a" },
    { "id": "s17_e", "action": "key_press", "keys": "e" },
    { "id": "s18_deadline", "action": "set_variable", "name": "deadline", "value": "${unix}" },
    { "id": "s18_plus30", "action": "increment", "name": "deadline", "by": 30 },
    { "id": "s18_wait_file", "action": "while", "max_iterations": 200,
      "condition": { "left": "${unix}", "op": "less_than", "right": "${deadline}" },
      "steps": [
        { "id": "s18a_exists", "action": "file_exists", "path": "${export_path}", "output_var": "exported" },
        { "id": "s18b_done", "action": "if", "condition": { "left": "${exported}", "op": "is_true" },
          "then": [ { "id": "s18c_break", "action": "break" } ],
          "else": [ { "id": "s18d_wait", "action": "wait", "ms": 500 } ] } ] },
    { "id": "s18e_verify", "action": "if", "condition": { "left": "${export_path}", "op": "path_exists" },
      "then": [ { "id": "s18f_info", "action": "file_info", "path": "${export_path}", "output_var": "xml" },
                { "id": "s18g_log", "action": "log", "message": "Exported ${xml.name} (${xml.size} bytes) to ${xml.parent}" } ],
      "else": [ { "id": "s18h_fail", "action": "stop", "success": false,
                  "message": "Export failed: ${export_path} was not created within 30 s. Tally was left open so you can see where it stopped." } ] },
    { "id": "s19_close", "action": "key_press", "keys": "alt+f4" },
    { "id": "s20_esc", "action": "key_press", "keys": "esc" },
    { "id": "s21_esc", "action": "key_press", "keys": "esc" },
    { "id": "s22_yes", "action": "key_press", "keys": "y" },
    { "id": "s23_closed", "action": "window_wait_close", "title": "TallyPrime", "timeout_ms": 15000, "on_error": "continue" }
  ]
}
```

This is the complete, tested flow (also `examples/tally_balance_sheet_export.json`). The image of
Tally's ready screen must come from the user's own screen (display scaling and licence screen
differ per machine); without it, keep `{ "id": "s02_wait", "action": "wait", "ms": 20000 }` and
tell the user how to upgrade it. Tally's quit sequence is Alt+F4 → "Quit? Yes/No" → Y; the extra
Esc presses come from the user's list — keep the user's order even when a step looks redundant.

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
   *Reading text on screen* below.
6. **Mouse coordinates** (`mouse_click` x/y, `mouse_move` with `duration_ms`/`tween`,
   `mouse_drag`, `mouse_down`/`mouse_up`) — last resort; breaks when windows move or DPI changes.
   Use `window_set_state` maximized first to stabilise layouts. `screen_size` gives the resolution.

### Data files (Excel, CSV, PDF, text)

Use these instead of opening Excel/PDF in their apps whenever the task is only to *read* data:

| Action | Gives `output_var` |
|---|---|
| `read_excel` (`path`, `sheet` = name or empty for the first, optional `range` like `A1:D50`) | rows of the sheet |
| `read_csv` (`path`, `delimiter` empty = auto-detect `,` `;` tab `|`) | rows of the file |
| `read_pdf` (`path`, `pages` like `1-3,5`, empty = all) | the text |
| `file_read` + `for_each` (items = the text) | text files, one item per line |

For tables: with `has_header: true` (default) each row is a record — loop with
`for_each items: "${rows}"` and use `${row.Company}`, `${row.Legal Name}`. Set **`column`**
(`"GSTIN"`, a letter `"B"`, or a number `"2"`) to get just that column as a plain list, the usual
input for a `for_each`. `skip_empty` drops blank rows/cells; `count_var` receives the count.
Excel numbers stay numbers, dates become `YYYY-MM-DD`; CSV values are text (leading zeros and
codes are kept). In Studio, Sheet/Column are dropdowns read from the file and **👁 Preview
data** shows the result.

```json
{ "id": "list", "action": "read_excel", "path": "${flow_dir}\\gstins.xlsx", "sheet": "GSTINs",
  "column": "GSTIN", "output_var": "gstins", "count_var": "n" },
{ "id": "each", "action": "for_each", "items": "${gstins}", "item_var": "gstin", "steps": [ … ] }
```

### Writing CSV and Excel files

Collect results in a list with **`list_append`** (`name` = list variable, `value` = a record such
as `{"Comment": "${row.Comment}", "Label": "${label}"}`; the list is created on first use), then
write it out:

- **`write_csv`** (`path`, `data: "${results}"`, optional `columns` order, `delimiter` `,` `;`
  `\t`, `append`, `excel_bom` true so Excel shows non-English text). Quoting is handled.
- **`write_excel`** (`path` .xlsx, `sheet`, `data`, `columns`): creates/replaces the workbook
  with a bold, frozen header row and fitted columns. Numbers stay numbers; text starting with
  `=` becomes a formula. It cannot add to an existing workbook: write a new file each run
  (e.g. with `${now:yyyyMMdd}` in the name) or use `write_csv` with `append: true`.

`data` can be a list of records (one column per key), a list of lists, or a list of values.

### Text, dates and maths

| Action | Use |
|---|---|
| `text_transform` | `operation` trim / upper / lower / title / length / url_encode / remove_spaces |
| `text_replace` | `find` → `replace_with`; `regex: true` for patterns (`$1` inserts a group), `ignore_case` |
| `text_extract` | regular expression `pattern`; `group` 1 = first `( )`; `all: true` → list; `found_var` |
| `text_split` / `text_join` | text ↔ list (`separator` `,`, `\n` lines, `\t`; `regex` splits on a pattern) |
| `date_calc` | `operation` format / add / diff / parts. `date` empty = now, `today`, Excel serials; `input_format` like `dd/MM/yyyy` (empty = common formats, **day first**); `unit` days, months, years…; `format` output pattern |
| `calculate` | `expression` such as `"${amount} * 18 / 100"`, `decimals` rounding. `+ - * / % ^ ( )`, comparisons give 1/0, functions `round`, `int`, `floor`, `ceil`, `abs`, `min`, `max`, `sqrt`. Division is decimal (7/2 = 3.5) |

Remove thousands separators before calculating (`text_replace` `,` → ``). Regex patterns in JSON
need doubled backslashes: `"\\d+"`. Example: pull an invoice number from PDF text with
`"pattern": "Invoice\\s*No[.:]?\\s*(\\S+)", "group": 1`.

### Error handling: try / catch / finally

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

### AI models (ai_prompt)

`ai_prompt` sends one prompt and stores the reply. Providers:

| `provider` | Needs | Notes |
|---|---|---|
| `lmstudio` (default) | LM Studio's local server running with a model loaded (Developer tab → Start Server, port 1234) | Data stays on the PC; `model` empty = the loaded model |
| `ollama` | Ollama running (port 11434) and `model` | local |
| `openai_compatible` | `base_url` (e.g. `http://localhost:8000/v1`) | vLLM, LocalAI, Groq, OpenRouter… |
| `openai` | `api_key_secret` | default model `gpt-4.1-mini` |
| `gemini` | `api_key_secret` | default model `gemini-2.5-flash` |
| `anthropic` | `api_key_secret` | Claude; default model `claude-opus-5-5` |

- `system` = instructions, `prompt` = the text with `${variables}`.
- **`labels`** (e.g. `["Fav", "UnFav", "Neutral"]`) tells the model to answer with one label and
  guarantees the stored value is exactly one of them; an answer matching none fails the step
  (add `retries`, and `on_error: continue` for batches).
- `json: true` asks for JSON and stores the parsed object (`${reply.sentiment}`).
- `temperature` (0 = predictable; not sent to Claude), `max_tokens` (default 4096; thinking models
  need room), `timeout_ms`.
- API keys: the user runs `autoflow secret set openai` once (or 🔑 Set key… in Studio) and the flow
  says `"api_key_secret": "openai"`. Never put a key in the flow.
- **Privacy:** cloud providers receive the text. Before sending customer or financial data to
  OpenAI/Gemini/Claude, confirm with the user; suggest `lmstudio` to keep data on the PC.
- For a batch, check once that a local server is up (see `classify_comments.json`: an
  `http_request` to `http://localhost:1234/v1/models` with `status_var`, then `stop` with a clear
  message) instead of failing every row.

### Web APIs (http_request)

`http_request`: `method`, `url`, `headers` (object), `body` (a string is sent as-is; an object is
sent as JSON), `bearer_secret` (token from Credential Manager → `Authorization: Bearer …`),
`save_to` (download a file), `output_var` (JSON responses become objects: `${response.items.0.name}`),
`status_var`, `fail_on_error_status` (default true), `timeout_ms`. In a dry run only GET/HEAD
requests that don't save a file are made.

### Reading e-mail

- **`read_email`** (IMAP): `provider` gmail / outlook365 / yahoo / zoho / zoho.in, or `imap_host`
  + `imap_port`; `username`; `password_secret` (Gmail: enable IMAP and use an App Password).
  Microsoft 365 mailboxes usually block password logins: use `outlook_read` for them.
- **`outlook_read`**: the classic Outlook desktop app, no password; `account` picks a mailbox;
  `folder` like `Inbox` or `Inbox/Invoices`.

Both filter with `unread_only` (default true), `from`, `subject_contains`, `since` (a date),
`max`, `newest_first`; afterwards `mark_read` and/or `move_to` a folder. `save_attachments_to` a
folder (existing files are not overwritten: `name (2).pdf`) with `attachment_filter` like `*.pdf`.
`output_var` gets a list of records `{from, from_name, to, subject, date, body, attachments
(saved paths), attachment_names}`; loop over them, then over `${mail.attachments}`. Reading
doesn't mark mail as read unless `mark_read` is set.

### Databases (db_query, db_execute)

`connection` is a SQLite file (`sqlite:output/app.db` or a path ending in `.db`; created when
missing) or an ODBC connection string: `DSN=MyDsn`,
`Driver={ODBC Driver 18 for SQL Server};Server=.;Database=Sales;Trusted_Connection=yes;TrustServerCertificate=yes`,
`Driver={Microsoft Excel Driver (*.xls, *.xlsx, *.xlsm, *.xlsb)};DBQ=C:\\data\\book.xlsx;ReadOnly=1`
(sheets are tables: ``[Sheet1$]``), MS Access, MySQL, PostgreSQL, or **TallyPrime's ODBC server**
(enable ODBC in Tally, then `DSN=TallyODBC64_9000`). The user lists drivers with
`Get-OdbcDriver` in PowerShell. A password goes in as `PWD={password}` with `password_secret`.

- **Always pass values with `?` and `params`** (`"sql": "INSERT INTO t VALUES (?, ?)"`,
  `"params": ["${row.Name}", "${amount}"]`), never `${…}` inside the SQL — validate warns, and
  checks that the number of `?` matches `params`.
- `db_query` → `output_var` list of records (numbers typed), `count_var`, `max_rows`.
- `db_execute` → `affected_var`. Without params, SQLite runs a whole script (`CREATE …; INSERT …;`).
- The connection is opened once and reused by later steps with the same `connection`.

### Reading text on screen (OCR)

`ocr_read`, `ocr_find`, `ocr_click` use the OCR engine built into Windows (offline, no install).
Source: `window` (title part), `region` {x, y, width, height}, `monitor`, or `image` (a picture file;
not for `ocr_click`). `scale` 2 (default) helps with small fonts; `language` like `en-US` (the PC's
Windows languages; `autoflow ocr --languages` lists them).

- `ocr_read` → `output_var` text (lines), `lines_var`, `words_var` (each word with screen x/y).
- `ocr_find` → `found_var` / `x_var` / `y_var` for some `text` (may be several words); waits up to
  `timeout_ms`; never fails.
- `ocr_click` clicks the text (`offset_x`/`offset_y` to click the box beside a label), e.g. menu
  items in Tally or a remote desktop where UI Automation sees nothing.
- Test first: `autoflow ocr --window "Tally" --find "Balance Sheet"`. OCR confuses look-alikes
  (`1`/`I`, `0`/`O`): match on words, and don't use OCR output as exact codes without checking.

### Sending e-mail

Two actions. **Never** put a password in a flow; never ask the user to paste one into chat.

- **`outlook_send`**: uses the classic Outlook desktop app and the account already signed in,
  so no password is needed. `send_now: false` (default) opens the mail as a draft for the user
  to check and send; `send_now: true` sends at once (use this for scheduled flows). `account`
  picks a sender account by address. The new Outlook app can't be automated.
- **`send_email`**: SMTP with `provider` `gmail` / `outlook365` / `outlook.com` / `yahoo` /
  `zoho` (server and port filled in) or `custom` (+ `smtp_host`, `smtp_port`, `security`
  starttls|tls|none). `username` is the login (the e-mail address). `password_secret` is the
  *name* of a password stored in Windows Credential Manager: the user runs
  `autoflow secret set gmail` once (or uses 🔑 Set password… in Studio). **Gmail needs an App
  Password** (Google Account → Security → 2-Step Verification → App passwords). Microsoft has
  been turning off password logins for SMTP, so for Outlook/Microsoft 365 accounts prefer
  `outlook_send`.

Both take `to` / `cc` / `bcc` (separated by `;` or `,`), `subject`, `body` (`html: true` for
HTML) and `attachments`, a list of paths that can contain variables. A missing attachment fails
the step before anything is sent.

```json
{ "id": "mail", "action": "outlook_send", "to": "accounts@company.com",
  "subject": "Balance Sheet ${now:dd-MM-yyyy}", "body": "Please find attached.",
  "attachments": ["${export_path}"], "send_now": true }
```

Sending mail on the user's behalf is an outward action. Confirm recipients with the user, and
use `send_now: false` or a test address the first time.

### Web pages (browser_*)

For web applications use the **Web** actions — they drive Edge (default) or Chrome through the
DevTools protocol, address elements by their HTML **`element_id`** (or a CSS `selector`), work
without moving the mouse, and are far more reliable than clicking screen coordinates:

```json
{ "id": "open",  "action": "browser_open", "url": "https://example.com/contact", "browser": "edge" },
{ "id": "name",  "action": "browser_type", "element_id": "name", "text": "${person}" },
{ "id": "send",  "action": "browser_click", "element_id": "submit" },
{ "id": "wait",  "action": "browser_wait", "element_id": "confirmation", "timeout_ms": 15000 },
{ "id": "read",  "action": "browser_get_text", "element_id": "confirmation", "output_var": "msg" },
{ "id": "shot",  "action": "browser_screenshot", "path": "${flow_dir}\\output\\result.png" },
{ "id": "close", "action": "browser_close" }
```

- The field is `element_id` (bare id, no `#`) — every step already has its own `id`. Use
  `selector` for anything else (`form button[type=submit]`, `.result`, `table tr:nth-child(2) td`).
- `browser_open` accepts a URL or a local `.html` path; `headless: true` hides the window,
  `keep_open: true` leaves it open after the run. Only one browser per flow; the browser is
  closed automatically when the flow ends.
- `browser_type` clears the field first (`clear`) and can `press_enter`. `browser_get_text`
  reads visible text, `attribute: "value"` (inputs), `"html"`, or any attribute (`href`).
- `browser_wait` for the element that proves the next page/state arrived before reading it;
  for results that load in place (AJAX), `browser_wait_text` waits until some text (e.g.
  "Legal Name") appears, optionally inside one element, and can return the text.
- `browser_type` fires key/change events after inserting text, so Angular/React pages react
  (e.g. a CAPTCHA that only appears after a value is typed).
- `browser_screenshot` with `full_page: true` saves the whole page, not just the visible part
  (also for apps that scroll inside a container).
- **Finding element ids:** run `autoflow web-pick <url>` (or Studio's **🎯 Pick on page** on a
  web step): a browser opens, the user hovers (outline shows) and **Ctrl+clicks** the element;
  you get its `element_id`, or a `selector` when it has no id. No developer tools needed.
- **CAPTCHAs are always solved by the user, never by the flow or you.** Pattern: screenshot the
  CAPTCHA element, show it in `input_box` with `image`, type the user's answer:
  ```json
  { "id": "shot", "action": "browser_screenshot", "element_id": "imgCaptcha", "path": "${flow_dir}\\captcha.png" },
  { "id": "ask",  "action": "input_box", "title": "CAPTCHA", "prompt": "Type the characters shown", "image": "${flow_dir}\\captcha.png", "output_var": "captcha" },
  { "id": "fill", "action": "browser_type", "element_id": "fo-captcha", "text": "${captcha}" }
  ```
  Such flows need a person present: don't schedule them.
- `<select>` and anything else unusual: `browser_eval` runs JavaScript and returns its value,
  e.g. `"document.getElementById('topic').value = 'Support'"` or `"document.title"`.
- `browser_screenshot` saves the visible page, or one element with `element_id`/`selector`.
- The browser uses a private temporary profile (not the user's logged-in profile): sites that
  need a login must be logged into within the flow — ask the user for credentials via
  `input_box` (`password: true`) at run time, never hard-code them.
- To find ids, the user can right-click the element → Inspect in Edge/Chrome.

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

## Scheduling a flow

When the user wants a flow to run by itself ("every morning", "every Friday", "each hour"), use
Windows Task Scheduler through `autoflow schedule` — don't build polling loops or ask them to
keep a window open:

```powershell
autoflow schedule add C:\flows\tally_export.json --when "weekdays 18:30" --name "Tally export"
```

- `--when`: `daily HH:MM`, `weekdays HH:MM`, `weekly mon,thu HH:MM`, `monthly D HH:MM`,
  `once YYYY-MM-DD HH:MM`, `every 30m` / `every 2h`, `logon`. 24-hour times.
- Optional: `--var name=value` (passed to every run), `--time-limit-minutes 120`,
  `--print-xml` (show the task without creating it).
- The task runs `autoflow.exe run "<flow>" --scheduled`: hidden console, log appended to
  `<flow folder>\logs\<flow name>.log`, exit code shown as the task's last result
  (0 success, 1 flow failed, 2 invalid). Check with `autoflow schedule list`; test right away
  with `autoflow schedule run "<name>"` and read the log.
- Tasks run only while the user is logged on, never overlap, catch up after a missed start
  (PC off) and may run on battery. Studio has the same in **⏰ Schedule…**.
- **Desktop flows need an unlocked screen.** Windows blocks synthetic keyboard/mouse input on a
  locked or disconnected session, so tell the user to keep the PC logged in and unlocked at
  that time (or schedule pure file/program flows, which work regardless).
- Validate first — `schedule add` refuses an invalid flow. Make scheduled flows self-checking
  (verify outputs, `stop` with `success: false`) because nobody is watching; avoid
  `message_box`/`input_box`, which would wait forever.
- Paths in the flow should be absolute or `${flow_dir}`-relative; the task's working directory
  is the flow's folder.

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

## Examples

See the repo's `examples/` folder:
`file_organizer.json` (files, loops, conditions, report), `calculator_uia.json` (UI Automation,
read-back verification), `notepad_keyboard.json` (keyboard, Save-As dialog, file check),
`pyautogui_tour.json` (screen size, eased mouse moves, pixel colour, region screenshot, image
search, message box), `tally_balance_sheet_export.json` (keyboard-driven Tally export with
wait-for-screen, wait-for-file and verification), `excel_sales_sheet.json` (Excel: click "Blank
workbook" via UI Automation, type a table with `\t`/`\n`, formulas, check a result by copying the
cell to the clipboard, Save As/OneDrive "Move" dialog via its `File name` box, quit only if the
flow started Excel).

Excel tips: in a typed row `\t` moves right and `\n` returns to the row's first column; F5 + a
cell address + Enter jumps to a cell; Ctrl+C then `clipboard_get` reads a value; F12 opens Save
As — or "This file was created in the cloud" when Excel auto-creates files in OneDrive — so find
the file-name box with `window: "Excel"`, `automation_id: "1001"`, `name: "File name"`
(`name_contains: true`) instead of relying on the dialog title.

`web_form_demo.json` (with `web/contact_form.html`): open a page, type into fields by
`element_id`, set a `<select>` via `browser_eval`, submit, wait for and read the confirmation,
verify it, screenshot the page and one element.

`tally_export_and_mail.json`: run the Tally export as a sub-flow (`run_flow` with
`share_variables`, so `${export_path}` is available), then mail the XML via `outlook_send`
(draft by default) or Gmail `send_email`.

`data_import_demo.json`: one Excel column as a list, a CSV as records in a loop, a second sheet
by name, and PDF text checked with `contains` (files in `examples/data/`).

`classify_comments.json`: read customer comments from `data/customer_comments.xlsx`, label each
Fav/UnFav/Neutral with `ai_prompt` (LM Studio by default, `labels`), collect with `list_append`
and save as CSV and Excel; checks first that the local AI server is running.

`gst_invoice_register.json` (runs offline): GSTINs from a PDF via `text_extract`, GST maths with
`calculate` (CGST+SGST or IGST by state code), due dates with `date_calc`, SQLite with
`db_execute`/`db_query`, an Excel register, all inside `try`/`finally`.

`invoice_inbox.json`: unread "Invoice" mails from Gmail (`read_email`) or Outlook
(`outlook_read`), PDF attachments saved, invoice number and amount extracted from each PDF,
stored in SQLite and an Excel sheet; each PDF in its own `try`.

`gst_search.json`: read the GSTINs from `data/gstins.xlsx`, then for each GSTIN open the GST "Search Taxpayer" page, type the
GSTIN, show the CAPTCHA picture to the user in an `input_box` (up to 3 attempts), search, wait
for "Legal Name" with `browser_wait_text`, and save a full-page screenshot named after the GSTIN.
