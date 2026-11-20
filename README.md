<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.png">
    <img src="assets/wordmark.png" alt="AutoFlowMate" width="560">
  </picture>
</p>
<p align="center"><b>RPA for a more productive you.</b> Automate your flow, with a friendly mate.</p>


A Robotic Process Automation (RPA) tool for Windows, written in Rust. It automates desktop
applications, web pages, files, Excel, CSV and PDF data, databases, e-mail, web APIs and AI
models, reads text on screen with OCR, and runs on a schedule.
Flows are plain **JSON**, so you can build them in a visual editor, write them by hand, or have
an **AI agent** generate them.

- **AutoFlowMate Studio** (`autoflow-studio.exe`): a visual editor with a **flowchart view**
  (drag actions from the palette onto any connector, drag nodes to reorder, if/else branches and
  loop frames), a list and a JSON view, a property form, Run/Dry-run/Stop, **breakpoints and
  step-by-step running** with the variables shown at each pause, **undo/redo**, a live log, an
  element picker, image snipping and a live mouse-position bar.
- **`autoflow` CLI**: run, validate, inspect, and generate templates/schema/docs. Every command has
  `--json` output for automation.
- **AI skill** ([skills/autoflowmate/SKILL.md](skills/autoflowmate/SKILL.md)): teaches agents such
  as Claude Code to author, validate and run flows.

## What it can automate (100 actions)

| Category | Actions |
|---|---|
| Flow | set_variable, increment, log, wait, if, repeat, while, for_each, break, stop, run_flow, **try** (catch / finally) |
| File | file_copy, file_move, file_delete, file_write, file_read, file_exists, file_info, dir_create, dir_delete, dir_list |
| Data | read_excel (sheet or one column), read_csv, read_pdf, list_append, write_csv, write_excel |
| Documents | read_pdf_table (invoice lines, statements), word_read, word_fill (templates with {{placeholders}}), word_write, zip_create, zip_extract |
| Text | text_transform, text_replace, text_extract (regular expressions), text_split, text_join |
| Date & Math | date_calc (format, add, difference, parts), calculate (expressions with rounding) |
| Database | db_query, db_execute (SQLite built in; SQL Server, MySQL, Access, Excel, TallyPrime and others through ODBC) |
| Internet | http_request (REST APIs, downloads) |
| AI | ai_prompt (LM Studio, Ollama, OpenAI, Gemini, Claude; force one of given labels, or JSON) |
| Program | run_program (with output capture/timeout), kill_program, open (file/folder/URL) |
| Window | window_activate, window_wait, window_wait_close, window_close, window_set_state, window_exists |
| Mouse | mouse_move (duration + easing), mouse_click, mouse_down, mouse_up, mouse_scroll, mouse_drag, mouse_position |
| Keyboard | type_text (keys / unicode / paste modes), key_press (`ctrl+shift+s`, `win+r`, ...), key_down, key_up |
| UI Element | ui_click, ui_set_text, ui_get_text, ui_wait, ui_exists (Windows UI Automation selectors) |
| Image | image_find, image_find_all, image_click, image_wait, image_wait_vanish (find a picture on screen) |
| OCR | ocr_read, ocr_find, ocr_click (Windows' built-in OCR, offline) |
| Screen | screenshot (monitor, window or region), screen_size, pixel_get, pixel_matches |
| Web | browser_open, browser_navigate, browser_type, browser_click, browser_wait, browser_wait_text, browser_get_text, browser_screenshot (page, full page or element), browser_eval, browser_close (Edge/Chrome; elements by **element id** or CSS selector) |
| Email | send_email (SMTP: Gmail, Microsoft 365, any server; attachments), outlook_send (Outlook desktop app; draft or send), read_email (IMAP), outlook_read (save attachments, mark read, move) |
| Dialog | message_box (alert/confirm), input_box (prompt/password, optional picture) |
| Clipboard | clipboard_set, clipboard_get |

Each step can also set `retries`, `retry_delay_ms`, `on_error: continue`, `delay_before_ms`,
`delay_after_ms`, `enabled` and `comment`.

### PyAutoGUI features, ported to Rust

| PyAutoGUI | AutoFlowMate |
|---|---|
| `FAILSAFE` | `settings.failsafe`: mouse into **any corner** aborts, even mid-typing, plus `settings.abort_hotkey` (default **Ctrl+Alt+Q**) |
| `PAUSE` | `settings.input_pause_ms` (after every mouse/keyboard action), `settings.step_delay_ms`, per-step `delay_before_ms` / `delay_after_ms` |
| `write(text, interval)` | `type_text` + `settings.key_delay_ms` (default 100 ms; 150–250 for slow apps like Tally) |
| `press(keys, presses, interval)`, `hotkey`, `keyDown`/`keyUp` | `key_press` (`times`, `interval_ms`), `key_down`, `key_up` |
| `moveTo/moveRel(duration, tween)` | `mouse_move` (`duration_ms`, `relative`, 10 `tween` curves) |
| `click(clicks, interval, button)`, `mouseDown/Up`, `dragTo/dragRel`, `scroll/hscroll` | `mouse_click`, `mouse_down`, `mouse_up`, `mouse_drag`, `mouse_scroll` |
| `size()`, `position()`, `pixel()`, `pixelMatchesColor()` | `screen_size`, `mouse_position`, `pixel_get`, `pixel_matches` |
| `locateOnScreen / locateCenterOnScreen / locateAllOnScreen (confidence, region)`, `click(image)` | `image_find`, `image_find_all`, `image_click`, `image_wait`, `image_wait_vanish` (zero-mean NCC, same score as OpenCV `TM_CCOEFF_NORMED`) |
| `screenshot(region=...)` | `screenshot` with `region` |
| `alert`, `confirm`, `prompt`, `password` | `message_box`, `input_box` |
| `displayMousePosition()` | `autoflow mouse`, Studio status bar |

### Run settings

```json
"settings": { "key_delay_ms": 100, "input_pause_ms": 100, "step_delay_ms": 0,
              "mouse_move_ms": 0, "failsafe": true, "abort_hotkey": "ctrl+alt+q" }
```

Edit them in Studio (toolbar and Flow settings panel) or override for one run with
`--key-delay`, `--input-pause`, `--step-delay` and `--no-failsafe`.

Full reference: [skills/autoflowmate/references/actions.md](skills/autoflowmate/references/actions.md)
(generated by `autoflow docs`). The JSON Schema is in [schema/flow.schema.json](schema/flow.schema.json).

## Example

```json
{
  "name": "Calculator via UI Automation",
  "variables": { "a": "7", "b": "8" },
  "steps": [
    { "id": "launch", "action": "open", "target": "calculator:" },
    { "id": "activate", "action": "window_activate", "title": "Calculator", "exact": true },
    { "id": "digit_a", "action": "ui_click", "window": "Calculator", "automation_id": "num${a}Button", "invoke": true },
    { "id": "plus", "action": "ui_click", "window": "Calculator", "automation_id": "plusButton", "invoke": true },
    { "id": "digit_b", "action": "ui_click", "window": "Calculator", "automation_id": "num${b}Button", "invoke": true },
    { "id": "equals", "action": "ui_click", "window": "Calculator", "automation_id": "equalButton", "invoke": true },
    { "id": "read", "action": "ui_get_text", "window": "Calculator", "automation_id": "CalculatorResults", "output_var": "display" },
    { "id": "show", "action": "log", "message": "Calculator says: ${display}" },
    { "id": "close", "action": "window_close", "title": "Calculator", "exact": true }
  ]
}
```

More in [examples/](examples): `file_organizer.json`, `calculator_uia.json`, `notepad_keyboard.json`,
`pyautogui_tour.json`, `tally_balance_sheet_export.json` (keyboard-driven TallyPrime export),
`excel_sales_sheet.json` (Excel: type a table with formulas, check the total, save) and
`web_form_demo.json` (fill a web form by element id, read the result, take screenshots) and
`gst_search.json` (GST taxpayer search with a CAPTCHA you type; saves each result page).

## Build

Requirements: Windows 10/11, Rust (stable, MSVC toolchain) and the Visual Studio C++ build tools.

```powershell
cargo build --release
```

Binaries: `target\release\autoflow.exe` (CLI) and `target\release\autoflow-studio.exe` (GUI).

### Download a release

Ready-made Windows builds are on the [Releases page](https://github.com/SRathinaGiri/AutoFlowMate/releases).
Unzip and run `autoflow-studio.exe`.

### Releasing (maintainers)

1. Bump `version` in the workspace `Cargo.toml`.
2. Add release notes in `.github/release-notes/vX.Y.Z.md`.
3. Commit, then tag and push:

```powershell
git tag v0.2.0
```

```powershell
git push origin v0.2.0
```

The **Release** workflow (`.github/workflows/release.yml`) runs on Windows. It checks that the tag
matches `Cargo.toml`, runs the tests, builds the portable zip with `scripts/package.ps1`, and
publishes a GitHub release with the zip and its SHA-256 checksum. **CI** builds and tests every
push to `main`. To build the same zip locally, run `scripts\package.ps1`.

## Use

```powershell
.\target\release\autoflow-studio.exe examples\file_organizer.json
```

```powershell
.\target\release\autoflow.exe validate examples\file_organizer.json
```

```powershell
.\target\release\autoflow.exe run examples\file_organizer.json --dry-run
```

```powershell
.\target\release\autoflow.exe run examples\calculator_uia.json --var a=9 --var b=4 --json
```

```powershell
.\target\release\autoflow.exe inspect --delay 3
```

```powershell
.\target\release\autoflow.exe run examples\notepad_keyboard.json --key-delay 200
```

Other commands: `actions`, `template <action>`, `windows`, `mouse`, `locate <image>`, `ocr`, `secret`, `schedule`, `schema`, `docs`, `new`.

The right-hand panel has three tabs:

- **Step**: the selected step's fields, each with a suitable editor. File and folder fields have
  📄/💾/📁 Browse buttons; a choice inside the flow's folder is stored relative to it. Numeric
  fields have a **Value | Variable** switch with a list of existing variables. Text fields have a
  **𝑥** menu that inserts `${variable}` (yours, step outputs, or built-ins such as
  `${timestamp}`). Output fields suggest existing names. Every variable list includes
  **➕ New variable…**.
- **Variables**: declare the flow's variables with a type (Text, Integer, Number, Yes/No, Date
  with a calendar, Time, Date + time, URL, File, Folder, List) and edit each value with the
  matching editor. Renaming a variable updates every `${old}` reference. The tab also lists the
  variables that steps create and the built-ins.
- **Flow**: name, description and run settings.

In Studio, the **Flowchart** view shows the flow as a diagram. **Drag** an action from the palette
onto any connector to insert it there, drag a node onto another connector to move it, and
right-click a node to enable/disable, duplicate or delete it. Clicking a connector's ⊕ makes it the
insertion point for palette clicks. Ctrl+mouse wheel zooms. The **List** and **JSON** views show
the same flow. Edit fields on the right. **⌖ Pick element** captures the control under the mouse
after 3 seconds and fills the selected `ui_*` or mouse step, or adds a new `ui_click`. For
`image_*` steps, **📷 Snip image** opens the Windows snipping tool and saves the picture next to the
flow. F5 runs, Ctrl+S saves, Ctrl+Z / Ctrl+Y undo and redo.

**Debugging:** press **F9** on a step (or click the dot in the List view, or right-click a node)
to set a breakpoint. **▶ Run** stops before that step; **⏭ Step (F10)** runs one step at a time
from the start. While paused, a window shows every variable's current value (typed passwords are
masked), and you can **Continue (F5)**, **Step (F10)** or **Stop**. Studio asks before closing or
opening another flow when there are unsaved changes.

## Reading Excel, CSV and PDF files

| Action | What you get |
|---|---|
| `read_excel` | A sheet (chosen by name, or the first one), or just one **column**, from .xlsx, .xlsm, .xls, .xlsb or .ods |
| `read_csv` | A CSV or TSV table; the delimiter is detected automatically |
| `read_pdf` | The text of all pages, or of selected pages such as `1-3,5` |

With a header row, every row becomes a record that you use in a loop as `${row.Company}`. Set
`column` to "GSTIN", "B" or "2" to get one column as a plain list for `for_each`. In Studio,
**Sheet** and **Column** are dropdowns filled from the file, and **👁 Preview data** shows the
rows the flow will get. Text files work with `file_read`, and `for_each` loops over the lines.
Example: `examples/data_import_demo.json`. The GST example reads its GSTINs from
`examples/data/gstins.xlsx`.

## Settings: provider lists

**File > Settings** in Studio holds the lists that the `provider` fields offer: AI providers
(server, default model, model list, key name) and e-mail providers (SMTP and IMAP servers). Change
them, or add your own, such as Groq, OpenRouter, a company LLM server or your mail server. They're
saved in `%APPDATA%\AutoFlowMate\presets.json` and used by Studio, the command line and
scheduled tasks on that PC. Fields left empty in a step show, and use, the provider's defaults.

## Writing results, text, dates and maths

Collect results with `list_append` (a record per row) and save them with `write_csv` (Excel
friendly UTF-8, appending without a second header, optional `escape_formulas`) or `write_excel`
(bold frozen header, fitted columns, numbers stay numbers; text that starts with `=` stays text
unless `allow_formulas` is set, so data from mail, web or AI can't run as a formula). `text_extract` pulls values out of text with a regular
expression, for example an invoice number from a PDF, and `text_replace`, `text_split`,
`text_join` and `text_transform` do the rest. `date_calc` formats, adds to and compares dates
(day-first input such as 31/01/2026 and Excel date numbers are understood), and `calculate`
evaluates arithmetic like `${amount} * 18 / 100` with rounding.

Wrap risky steps in **`try`**: when a step fails, `catch` runs with the message in `${error}` and
the flow carries on; `finally` always runs. See `examples/gst_invoice_register.json`, which runs
offline. It reads GSTINs from a PDF, works out CGST/SGST or IGST, adds due dates, stores the
invoices in SQLite and writes an Excel register.

## AI: classify, summarise, extract

`ai_prompt` sends a prompt (with `${variables}`) to a language model and stores the reply:

| `provider` | Where it runs |
|---|---|
| `lmstudio` (default) | On your PC: LM Studio's local server (Developer tab → Start Server) with any chat model loaded |
| `ollama` | On your PC: Ollama |
| `openai_compatible` | Any OpenAI-style server (`base_url`) |
| `openai`, `gemini`, `anthropic` | Cloud: OpenAI, Google Gemini, Anthropic Claude (API key stored with `autoflow secret set NAME`) |

Give `labels` such as `["Fav", "UnFav", "Neutral"]` and the stored answer is always exactly one of
them. That is all a classification needs. `json: true` returns structured data instead.
`examples/classify_comments.json` reads customer comments from Excel, labels each one, and saves
a CSV and an Excel file:

```powershell
.\target\release\autoflow.exe run examples\classify_comments.json
```

```powershell
.\target\release\autoflow.exe run examples\classify_comments.json --var provider=openai --var api_key_secret=openai
```

With a local model the comments never leave your PC; cloud providers receive the text you send.
`http_request` calls any other web API (JSON in and out, bearer tokens from Credential Manager,
file downloads).

## Databases

`db_query` returns rows as records for `for_each`, `write_csv` or `write_excel`; `db_execute` runs
INSERT/UPDATE/DELETE/CREATE. Values go in through `?` placeholders and `params`, never pasted into
the SQL.

| `connection` | Database |
|---|---|
| `sqlite:output/app.db` | A SQLite file (built in, created if missing) |
| `Driver={ODBC Driver 18 for SQL Server};Server=.;Database=Sales;Trusted_Connection=yes;TrustServerCertificate=yes` | SQL Server |
| `Driver={Microsoft Excel Driver (*.xls, *.xlsx, *.xlsm, *.xlsb)};DBQ=C:\data\book.xlsx;ReadOnly=1` | An Excel workbook as tables (`[Sheet1$]`) |
| `DSN=TallyODBC64_9000` | TallyPrime's ODBC server |

Any database with an ODBC driver works (MySQL, PostgreSQL, Oracle, Access). A password is written
as `PWD={password}` and taken from Credential Manager (`password_secret`).

## Reading text on screen (OCR)

`ocr_read` reads the text of a window, a screen region or a picture; `ocr_find` and `ocr_click`
locate and click a label such as a menu item or button. This helps with apps that show nothing to
UI Automation, like some Tally screens or remote desktops. It uses the OCR engine built into
Windows 10/11, so it is offline and adds almost nothing to the download. See what it reads with:

```powershell
.\target\release\autoflow.exe ocr --window "Notepad" --find "File"
```

## Documents: PDF tables, Word, zip

`read_pdf_table` reads a table from a PDF, such as invoice lines or a bank statement, as records
(`${item.Amount}`). `word_fill` turns a Word template with `{{customer}}`, `{{total}}` placeholders
into a finished letter: you design the letter in Word, and the flow fills it for every row of an
Excel sheet. `word_read` gets the text and tables out of a .docx; `word_write` makes a simple
report with a table. `zip_create` / `zip_extract` pack and unpack archives. See
`examples/documents_demo.json`, which runs offline.

## Triggers: run when a file or e-mail arrives

Add `triggers` to a flow and run `autoflow watch flow.json`. Each PDF dropped into a folder, or
each new e-mail that matches, starts the flow once, with the file path or the mail in a variable.
Handled files are moved to a `done` or `failed` folder. To watch all day from log-on:

```powershell
.\target\release\autoflow.exe schedule add examples\invoice_folder_watch.json --when logon --watch
```

## Recording

`autoflow record new.json`, or **⏺ Record** in Studio, turns your clicks and typing into steps.
A click on a button or menu that Windows can identify becomes a `ui_click` on that control, so the
recording still works when the window moves; other clicks keep their screen position. Typing in
a password box is never recorded: the flow asks for the password when it runs. Press
**Ctrl+Alt+R** to stop.

## Sending and reading e-mail

| Action | How it sends |
|---|---|
| `outlook_send` | Through the **Outlook desktop app**, with the account already signed in, so no password is needed. It can open the mail as a **draft for you to check**, or send it straight away (`send_now`). |
| `send_email` | Over **SMTP**. Gmail, Microsoft 365, Outlook.com, Yahoo and Zoho are preset, or use any server. Supports STARTTLS, TLS or none. |

Both support To/Cc/Bcc, HTML or text bodies, and **attachments** (`${variables}` allowed).
Passwords are never stored in flows. Store them once in **Windows Credential Manager**:

```powershell
.\target\release\autoflow.exe secret set gmail
```

Or click **🔑 Set password…** on a `send_email` step in Studio. Then reference the name with
`"password_secret": "gmail"`. Gmail requires an **App Password** (Google Account → Security →
2-Step Verification → App passwords). For Outlook and Microsoft 365 accounts, `outlook_send`
is the dependable choice, because Microsoft is disabling password logins over SMTP. See
`examples/tally_export_and_mail.json`: it exports the Tally Balance Sheet and mails it as an
attachment.

To **read** mail, use `read_email` (IMAP: Gmail, Yahoo, Zoho or any server) or `outlook_read`
(the Outlook desktop app). Filter by unread, sender, subject words and date. Attachments are
saved to a folder (`*.pdf` filters, nothing is overwritten), and messages can be marked read or
moved afterwards. `examples/invoice_inbox.json` saves PDF invoices from the inbox, extracts the
invoice number and amount, and records them in SQLite and Excel.

## Web automation

The **Web** actions drive Microsoft Edge (or Chrome) through the DevTools protocol. Address any
element by its HTML `element_id` (or a CSS `selector`): type into it, click it, wait for it, read
its text/value/attributes, run JavaScript, and screenshot the page or a single element. No
mouse movement is involved, so web steps are not affected by screen layout.

```json
{ "id": "open",  "action": "browser_open", "url": "https://example.com/contact" },
{ "id": "name",  "action": "browser_type", "element_id": "name", "text": "Rathinagiri" },
{ "id": "send",  "action": "browser_click", "element_id": "submit" },
{ "id": "wait",  "action": "browser_wait", "element_id": "confirmation" },
{ "id": "read",  "action": "browser_get_text", "element_id": "confirmation", "output_var": "msg" },
{ "id": "shot",  "action": "browser_screenshot", "path": "result.png" }
```

- **Finding element ids without developer tools:** run `autoflow web-pick https://…`, or use
  **🎯 Pick on page** on a web step in Studio. A browser opens and outlines whatever you hover;
  **Ctrl+click** the field or button and you get its `element_id`, or a CSS `selector` if it has
  no id.
- `browser_wait_text` waits for text that a page loads in place, such as search results.
  `browser_screenshot` with `full_page: true` captures the entire page.
- **CAPTCHAs** are always typed by you. The flow can show the CAPTCHA picture inside an
  `input_box` (`image` field) and type your answer. See `examples/gst_search.json`, which
  looks up GSTINs on the GST portal and saves each result page.

Try it offline with `examples/web_form_demo.json` and the bundled `examples/web/contact_form.html`.
The browser runs with a private temporary profile, which is deleted afterwards, and it closes
when the flow ends unless `keep_open` is set.

## Scheduling

Flows can run by themselves through **Windows Task Scheduler**: it is built into Windows,
survives reboots, needs no AutoFlowMate process running in the background and records every
run's result. In Studio use **⏰ Schedule…**, or from the command line:

```powershell
.\target\release\autoflow.exe schedule add examples\tally_balance_sheet_export.json --when "weekdays 18:30"
```

```powershell
.\target\release\autoflow.exe schedule list
```

| `--when` | Runs |
|---|---|
| `daily 09:00` | every day at 09:00 |
| `weekdays 18:30` | Monday–Friday |
| `weekly mon,thu 09:00` | on the listed days |
| `monthly 1 09:00` | on day 1 of each month |
| `once 2026-11-20 09:00` | one time |
| `every 30m`, `every 2h` | repeatedly, starting in a minute |
| `logon` | when you log on |

`schedule run NAME` starts a task now (to test it) and `schedule remove NAME` deletes it. Tasks
live in the *AutoFlowMate* folder of Task Scheduler. Each one:

- runs `autoflow.exe run "<flow>" --scheduled`, which hides the console window and appends a
  timestamped log to `<flow folder>\logs\<flow name>.log`;
- reports the exit code as the task's last result (0 = success, 1 = flow failed, 2 = invalid flow);
- runs only while you are logged on, never starts a second copy, catches up after a missed start
  (PC off) and may run on battery.

**Desktop flows (keyboard, mouse, windows) need your session logged in and the screen unlocked.**
Windows blocks synthetic input on a locked screen. Flows that only use files and programs work
regardless. You can also log any manual run with `autoflow run flow.json --log run.log`.

## Using with AI agents

`SKILL.md` holds the workflow, safety rules and action-selection order; topic guides (data, AI,
e-mail, databases, OCR, web, scheduling, examples) are in `skills/autoflowmate/references/`. Copy or
link `skills/autoflowmate` into your agent's skill folder, for Claude Code:

```powershell
Copy-Item -Recurse skills\autoflowmate "$env:USERPROFILE\.claude\skills\autoflowmate"
```

The agent then follows the loop *inspect → write JSON → `validate --json` → `run --dry-run` →
`run --json`*, reading structured events to fix problems.

## Project layout

```
crates/autoflow-core   flow model, actions, engine, validation, input, UI Automation
crates/autoflow-cli    `autoflow` command-line tool
crates/autoflow-gui    `autoflow-studio` visual editor (egui)
examples/              sample flows
schema/                generated JSON Schema
skills/autoflowmate/   AI agent skill + generated action reference
docs/                  architecture and roadmap
assets/                icon, splash and logo images (assets/brand/ holds the source art;
                       rebuild with `python scripts/brand_assets.py`)
```

Built on [enigo](https://crates.io/crates/enigo) (mouse/keyboard),
[uiautomation](https://crates.io/crates/uiautomation) (Windows UI Automation),
[egui/eframe](https://crates.io/crates/eframe) (GUI), [xcap](https://crates.io/crates/xcap)
(screenshots), [arboard](https://crates.io/crates/arboard) (clipboard),
[headless_chrome](https://crates.io/crates/headless_chrome) (web),
[calamine](https://crates.io/crates/calamine) and
[rust_xlsxwriter](https://crates.io/crates/rust_xlsxwriter) (Excel),
[lettre](https://crates.io/crates/lettre), [imap](https://crates.io/crates/imap) and
[mail-parser](https://crates.io/crates/mail-parser) (e-mail),
[ureq](https://crates.io/crates/ureq) (HTTP), [rusqlite](https://crates.io/crates/rusqlite) and
[odbc-api](https://crates.io/crates/odbc-api) (databases), [fasteval](https://crates.io/crates/fasteval)
(maths) and the [windows](https://crates.io/crates/windows) crate (Windows OCR).

## License

MIT: free to use, change and share, including commercially, with no warranty. See [LICENSE](LICENSE).
