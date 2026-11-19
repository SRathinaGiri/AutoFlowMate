# Scheduling a flow

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
