# Scheduling a flow, and running it when a file or e-mail arrives

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

## Triggers: when a file or e-mail arrives

Instead of a clock, a flow can be started by events. List them in the flow's `triggers`; the
watcher runs the flow once per event with the file path or the mail in a variable:

```json
"triggers": [
  { "kind": "file", "folder": "inbox/invoices", "pattern": "*.pdf", "variable": "file",
    "move_to": "done", "move_failed_to": "failed" },
  { "kind": "email", "source": "imap", "provider": "gmail", "username": "you@gmail.com",
    "password_secret": "gmail", "subject_contains": "Invoice", "save_attachments_to": "inbox/mail",
    "attachment_filter": "*.pdf", "every_minutes": 5, "variable": "mail" }
]
```

- File trigger: waits until a new file has stopped changing (`settle_ms`, default 2 s) and is not
  open in another program, then runs the flow with `${file}` = its full path. `move_to` /
  `move_failed_to` (relative to the watched folder) move it afterwards by result, so nothing is
  processed twice. Files already there when watching starts are skipped unless
  `include_existing: true`. A missing folder is created.
- E-mail trigger: checks unread mail every `every_minutes`; each matching mail is marked read
  (and moved with `move_to`) so it triggers once; `${mail.subject}`, `${mail.from}`,
  `${mail.attachments}` (saved paths). `source: "outlook"` uses the Outlook desktop app (`account`).
- Give the trigger variable a sample default in `variables` (e.g. a test PDF) so the flow can
  also be run and tested by hand.
- Start watching: `autoflow watch flow.json` (Ctrl+C stops), or once: `--once` (files already
  there count as new). At log-on, all day: `autoflow schedule add flow.json --when logon --watch`
  (Studio: ⏰ Schedule… → "Watch the flow's triggers"). With `--when "every 15m" --watch` it checks
  once per start instead. The log is `logs\<flow>.log`.
- Moving files and marking mail read are changes: confirm with the user (see the confirmation gate).

