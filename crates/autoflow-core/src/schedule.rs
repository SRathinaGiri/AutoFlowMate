//! Scheduling flows with Windows Task Scheduler.
//!
//! Tasks live in the `\AutoFlowMate\` folder of Task Scheduler and run
//! `autoflow.exe run "<flow>" --scheduled`. They are registered from Task XML
//! (not `schtasks` option strings) so dates are locale-independent and there is
//! no 261-character command limit. Every task:
//! * runs only while the user is logged on (desktop automation needs an
//!   interactive, unlocked session: Windows blocks synthetic input otherwise),
//! * never starts a second copy while one is running,
//! * starts as soon as possible after a missed start (PC was off/asleep),
//! * may run on battery, and is stopped after `time_limit_minutes`.

use anyhow::{anyhow, bail, Context as _, Result};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const FOLDER: &str = "\\AutoFlowMate\\";

/// When a scheduled flow runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// One time at a date and time.
    Once { at: NaiveDateTime },
    /// Every day at a time.
    Daily { time: NaiveTime },
    /// Monday to Friday at a time.
    Weekdays { time: NaiveTime },
    /// On the given weekdays (0 = Monday … 6 = Sunday) at a time.
    Weekly { days: Vec<u8>, time: NaiveTime },
    /// On a day of every month (1-31) at a time.
    Monthly { day: u8, time: NaiveTime },
    /// Every N minutes, starting now (or at `start` today).
    Every { minutes: u32 },
    /// When the user logs on (after `delay_seconds`).
    Logon { delay_seconds: u32 },
}

const DAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

impl Trigger {
    /// Parse a short text form, as used by the CLI and Studio:
    /// `once 2026-11-20 09:00`, `daily 09:00`, `weekdays 18:30`, `weekly mon,thu 09:00`,
    /// `monthly 1 09:00`, `every 30m` / `every 2h`, `logon`.
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        let time = |t: &str| -> Result<NaiveTime> {
            NaiveTime::parse_from_str(t, "%H:%M").map_err(|_| anyhow!("time must be HH:MM (24 h), got `{t}`"))
        };
        Ok(match parts.as_slice() {
            ["once", d, t] => {
                let date = NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| anyhow!("date must be YYYY-MM-DD, got `{d}`"))?;
                Trigger::Once { at: date.and_time(time(t)?) }
            }
            ["daily", t] => Trigger::Daily { time: time(t)? },
            ["weekdays", t] => Trigger::Weekdays { time: time(t)? },
            ["weekly", days, t] => {
                let mut v = vec![];
                for d in days.split(',') {
                    let d = d.trim().to_ascii_lowercase();
                    let i = DAY_NAMES
                        .iter()
                        .position(|n| n.to_ascii_lowercase().starts_with(&d) && d.len() >= 2)
                        .ok_or_else(|| anyhow!("unknown weekday `{d}` (use mon,tue,wed,thu,fri,sat,sun)"))?;
                    if !v.contains(&(i as u8)) {
                        v.push(i as u8);
                    }
                }
                v.sort();
                Trigger::Weekly { days: v, time: time(t)? }
            }
            ["monthly", day, t] => {
                let day: u8 = day.parse().ok().filter(|d| (1..=31).contains(d)).ok_or_else(|| anyhow!("day of month must be 1-31"))?;
                Trigger::Monthly { day, time: time(t)? }
            }
            ["every", n] => {
                let n = n.to_ascii_lowercase();
                let minutes = if let Some(h) = n.strip_suffix('h') {
                    h.parse::<u32>().map(|h| h * 60)
                } else {
                    n.trim_end_matches('m').parse::<u32>()
                }
                .map_err(|_| anyhow!("interval must look like 30m or 2h"))?;
                if minutes == 0 {
                    bail!("interval must be at least 1 minute");
                }
                Trigger::Every { minutes }
            }
            ["logon"] => Trigger::Logon { delay_seconds: 30 },
            _ => bail!(
                "unknown schedule `{s}`. Examples: \"daily 09:00\", \"weekdays 18:30\", \"weekly mon,thu 09:00\", \
                 \"monthly 1 09:00\", \"once 2026-11-20 09:00\", \"every 30m\", \"logon\""
            ),
        })
    }

    /// Human-readable description, e.g. "every Mon, Thu at 09:00".
    pub fn describe(&self) -> String {
        match self {
            Trigger::Once { at } => format!("once on {}", at.format("%Y-%m-%d at %H:%M")),
            Trigger::Daily { time } => format!("daily at {}", time.format("%H:%M")),
            Trigger::Weekdays { time } => format!("Monday-Friday at {}", time.format("%H:%M")),
            Trigger::Weekly { days, time } => format!(
                "every {} at {}",
                days.iter().map(|d| &DAY_NAMES[*d as usize][..3]).collect::<Vec<_>>().join(", "),
                time.format("%H:%M")
            ),
            Trigger::Monthly { day, time } => format!("monthly on day {day} at {}", time.format("%H:%M")),
            Trigger::Every { minutes } if minutes % 60 == 0 => format!("every {} h", minutes / 60),
            Trigger::Every { minutes } => format!("every {minutes} min"),
            Trigger::Logon { .. } => "at log-on".into(),
        }
    }

    fn xml(&self, user: &str) -> String {
        let today = Local::now().date_naive();
        let at = |t: &NaiveTime| format!("{}T{}", today.format("%Y-%m-%d"), t.format("%H:%M:00"));
        let cal = |t: &NaiveTime, body: String| {
            format!("<CalendarTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled>{body}</CalendarTrigger>", at(t))
        };
        let days = |d: &[u8]| d.iter().map(|i| format!("<{}/>", DAY_NAMES[*i as usize])).collect::<String>();
        match self {
            Trigger::Once { at } => format!(
                "<TimeTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></TimeTrigger>",
                at.format("%Y-%m-%dT%H:%M:00")
            ),
            Trigger::Daily { time } => cal(time, "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>".into()),
            Trigger::Weekdays { time } => cal(
                time,
                format!("<ScheduleByWeek><DaysOfWeek>{}</DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek>", days(&[0, 1, 2, 3, 4])),
            ),
            Trigger::Weekly { days: d, time } => cal(
                time,
                format!("<ScheduleByWeek><DaysOfWeek>{}</DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek>", days(d)),
            ),
            Trigger::Monthly { day, time } => cal(
                time,
                format!(
                    "<ScheduleByMonth><DaysOfMonth><Day>{day}</Day></DaysOfMonth><Months>{}</Months></ScheduleByMonth>",
                    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
                        .iter()
                        .map(|m| format!("<{m}/>"))
                        .collect::<String>()
                ),
            ),
            Trigger::Every { minutes } => {
                let start = Local::now().naive_local() + chrono::Duration::minutes(1);
                format!(
                    "<TimeTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled>\
                     <Repetition><Interval>PT{minutes}M</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition></TimeTrigger>",
                    start.with_second(0).unwrap_or(start).format("%Y-%m-%dT%H:%M:%S")
                )
            }
            Trigger::Logon { delay_seconds } => format!(
                "<LogonTrigger><Enabled>true</Enabled><UserId>{}</UserId><Delay>PT{delay_seconds}S</Delay></LogonTrigger>",
                esc(user)
            ),
        }
    }
}

/// A task to create.
#[derive(Debug, Clone)]
pub struct TaskSpec {
    /// Task name inside the AutoFlowMate folder.
    pub name: String,
    /// Absolute path of the flow JSON.
    pub flow: PathBuf,
    pub trigger: Trigger,
    /// Absolute path of autoflow.exe (the CLI).
    pub exe: PathBuf,
    /// Extra arguments for `autoflow run` (e.g. `--var x=1`).
    pub extra_args: Vec<String>,
    /// Stop the run after this many minutes.
    pub time_limit_minutes: u32,
}

/// Information about an existing task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskInfo {
    pub name: String,
    pub state: String,
    pub description: String,
    pub next_run: String,
    pub last_run: String,
    /// 0 = success, 1 = flow failed, 2 = invalid flow, 267011 = never run, 267009 = running.
    pub last_result: i64,
}

impl TaskInfo {
    pub fn last_result_text(&self) -> String {
        match self.last_result {
            0 => "success".into(),
            1 => "flow failed (see log)".into(),
            2 => "flow invalid".into(),
            267011 => "never run".into(),
            267009 => "running".into(),
            267014 => "stopped (time limit)".into(),
            n => format!("code {n} (0x{n:X})"),
        }
    }
}

/// Make a valid task name from free text.
pub fn sanitize_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "flow".into() } else { t }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn current_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    }
}

fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

/// The Task Scheduler XML for a spec.
pub fn task_xml(spec: &TaskSpec) -> String {
    let user = current_user();
    let mut args = vec!["run".to_string(), quote(&spec.flow.display().to_string()), "--scheduled".into()];
    args.extend(spec.extra_args.iter().map(|a| quote(a)));
    let workdir = spec.flow.parent().map(|p| p.display().to_string()).unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>AutoFlowMate</Author>
    <Description>{desc}</Description>
  </RegistrationInfo>
  <Triggers>{trigger}</Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <AllowHardTerminate>true</AllowHardTerminate>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT{limit}M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{args}</Arguments>
      <WorkingDirectory>{workdir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        desc = esc(&format!("AutoFlowMate | {} | {}", spec.trigger.describe(), spec.flow.display())),
        trigger = spec.trigger.xml(&user),
        user = esc(&user),
        limit = spec.time_limit_minutes.max(1),
        exe = esc(&spec.exe.display().to_string()),
        args = esc(&args.join(" ")),
        workdir = esc(&workdir),
    )
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn schtasks(args: &[&str]) -> Result<String> {
    let out = hidden(Command::new("schtasks").args(args)).output().context("cannot run schtasks.exe")?;
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        bail!("schtasks failed: {}", text.trim());
    }
    Ok(text)
}

/// Create (or replace) a scheduled task.
pub fn create(spec: &TaskSpec) -> Result<()> {
    if !spec.flow.is_absolute() || !spec.flow.exists() {
        bail!("flow file not found: {}", spec.flow.display());
    }
    if !spec.exe.exists() {
        bail!("autoflow.exe not found at {}", spec.exe.display());
    }
    if let Trigger::Once { at } = &spec.trigger {
        if *at <= Local::now().naive_local() {
            bail!("the time {} is in the past", at.format("%Y-%m-%d %H:%M"));
        }
    }
    let xml = task_xml(spec);
    let file = std::env::temp_dir().join(format!("autoflow_task_{}.xml", std::process::id()));
    // Task Scheduler expects UTF-16 LE with BOM for XML files.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&file, bytes)?;
    let full = format!("{FOLDER}{}", sanitize_name(&spec.name));
    let r = schtasks(&["/Create", "/TN", &full, "/XML", &file.display().to_string(), "/F"]);
    let _ = std::fs::remove_file(&file);
    r.map(|_| ())
}

/// Start a task now.
pub fn run_now(name: &str) -> Result<()> {
    schtasks(&["/Run", "/TN", &format!("{FOLDER}{}", sanitize_name(name))]).map(|_| ())
}

/// Delete a task.
pub fn remove(name: &str) -> Result<()> {
    schtasks(&["/Delete", "/TN", &format!("{FOLDER}{}", sanitize_name(name)), "/F"]).map(|_| ())
}

/// All AutoFlowMate tasks with their next/last run and result.
pub fn list() -> Result<Vec<TaskInfo>> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$f = { param($d) if ($d -and $d.Year -gt 1999) { $d.ToString('yyyy-MM-dd HH:mm') } else { '' } }
$items = @(Get-ScheduledTask -TaskPath '\AutoFlowMate\' | ForEach-Object {
  $i = $_ | Get-ScheduledTaskInfo
  [pscustomobject]@{ name = $_.TaskName; state = "$($_.State)"; description = "$($_.Description)";
    next_run = (& $f $i.NextRunTime); last_run = (& $f $i.LastRunTime); last_result = [int64]$i.LastTaskResult }
})
ConvertTo-Json -InputObject $items -Compress
"#;
    let out = hidden(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]))
        .output()
        .context("cannot run PowerShell")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() || text == "null" {
        return Ok(vec![]);
    }
    serde_json::from_str(text).map_err(|e| anyhow!("cannot read task list: {e}: {text}"))
}

/// Tasks whose description points at this flow file.
pub fn for_flow(flow: &Path) -> Result<Vec<TaskInfo>> {
    let want = flow.display().to_string().to_lowercase();
    Ok(list()?.into_iter().filter(|t| t.description.to_lowercase().ends_with(&want)).collect())
}

/// Where `autoflow run --scheduled` writes its log for a flow: `<flow folder>\logs\<flow name>.log`.
pub fn log_path(flow: &Path) -> PathBuf {
    let stem = flow.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "flow".into());
    flow.parent().unwrap_or(Path::new(".")).join("logs").join(format!("{stem}.log"))
}

/// The weekday index (0 = Monday) of today; handy for UIs.
pub fn today_weekday() -> u8 {
    Local::now().weekday().num_days_from_monday() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_triggers() {
        assert_eq!(Trigger::parse("daily 09:00").unwrap().describe(), "daily at 09:00");
        assert_eq!(Trigger::parse("weekly thu,mon 18:30").unwrap().describe(), "every Mon, Thu at 18:30");
        assert_eq!(Trigger::parse("every 2h").unwrap(), Trigger::Every { minutes: 120 });
        assert_eq!(Trigger::parse("every 15m").unwrap(), Trigger::Every { minutes: 15 });
        assert!(matches!(Trigger::parse("once 2030-01-02 07:05").unwrap(), Trigger::Once { .. }));
        assert!(Trigger::parse("monthly 32 09:00").is_err());
        assert!(Trigger::parse("daily 9am").is_err());
        assert!(Trigger::parse("sometimes").is_err());
    }

    #[test]
    fn xml_is_well_formed_enough() {
        let spec = TaskSpec {
            name: "Tally <export> & more".into(),
            flow: PathBuf::from(r"D:\flows\tally & co.json"),
            trigger: Trigger::parse("weekdays 18:30").unwrap(),
            exe: PathBuf::from(r"C:\Program Files\AutoFlowMate\autoflow.exe"),
            extra_args: vec!["--var".into(), "who=A B".into()],
            time_limit_minutes: 60,
        };
        let x = task_xml(&spec);
        assert!(x.contains("<Monday/><Tuesday/><Wednesday/><Thursday/><Friday/>"));
        assert!(x.contains("tally &amp; co.json"));
        assert!(x.contains("--scheduled"));
        assert!(x.contains("&quot;who=A B&quot;"));
        assert!(x.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(x.contains("PT60M"));
        assert_eq!(sanitize_name("a/b:c"), "a_b_c");
    }
}
