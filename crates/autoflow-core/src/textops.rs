//! Helpers behind the text, date and calculate actions.

use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, Timelike};
use serde_json::{json, Value};

// ---------------------------------------------------------------------- text

/// "hello WORLD" -> "Hello World".
pub fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = !(c == '\'' || c == '’');
        }
    }
    out
}

/// Percent-encode for use inside a URL query value.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------- dates

/// Formats tried when no input format is given. Day-first, as in India and Europe.
const AUTO_FORMATS: &[&str] = &[
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%dT%H:%M",
    "%d-%m-%Y %H:%M:%S",
    "%d/%m/%Y %H:%M:%S",
    "%d-%m-%Y %H:%M",
    "%d/%m/%Y %H:%M",
    "%d.%m.%Y %H:%M",
    "%Y/%m/%d %H:%M:%S",
];
const AUTO_DATES: &[&str] = &[
    "%Y-%m-%d", "%d-%m-%Y", "%d/%m/%Y", "%d.%m.%Y", "%Y/%m/%d", "%Y%m%d", "%d %b %Y", "%d-%b-%Y", "%d %B %Y", "%b %d, %Y", "%B %d, %Y",
    "%d-%b-%y", "%d/%m/%y",
];

/// Parse a date/time. Empty = now. `format` uses the same tokens as `${now:...}`
/// (dd-MM-yyyy) or strftime; empty = try common formats (day first) and Excel serial numbers.
pub fn parse_date(input: &str, format: &str) -> Result<NaiveDateTime> {
    let s = input.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("now") {
        return Ok(Local::now().naive_local());
    }
    if s.eq_ignore_ascii_case("today") {
        return Ok(Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap());
    }
    if !format.trim().is_empty() {
        let f = crate::vars::date_format(format.trim());
        if let Ok(d) = NaiveDateTime::parse_from_str(s, &f) {
            return Ok(d);
        }
        if let Ok(d) = NaiveDate::parse_from_str(s, &f) {
            return Ok(d.and_hms_opt(0, 0, 0).unwrap());
        }
        bail!("`{s}` does not match the date format `{format}`");
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Local).naive_local());
    }
    for f in AUTO_FORMATS {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Ok(d);
        }
    }
    for f in AUTO_DATES {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return Ok(d.and_hms_opt(0, 0, 0).unwrap());
        }
    }
    // Excel stores dates as days since 1899-12-30.
    if let Ok(serial) = s.parse::<f64>() {
        if (1.0..2_958_466.0).contains(&serial) {
            let base = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap().and_hms_opt(0, 0, 0).unwrap();
            return Ok(base + Duration::milliseconds((serial * 86_400_000.0).round() as i64));
        }
    }
    bail!("cannot read `{s}` as a date; set input_format (e.g. dd/MM/yyyy)")
}

pub fn format_date(d: &NaiveDateTime, format: &str) -> String {
    let f = if format.trim().is_empty() { "%Y-%m-%d %H:%M:%S".to_string() } else { crate::vars::date_format(format.trim()) };
    d.format(&f).to_string()
}

fn unit_of(unit: &str) -> Result<&'static str> {
    Ok(match unit.trim().to_ascii_lowercase().trim_end_matches('s') {
        "second" | "sec" => "seconds",
        "minute" | "min" => "minutes",
        "hour" => "hours",
        "" | "day" => "days",
        "week" => "weeks",
        "month" => "months",
        "year" => "years",
        other => bail!("unknown unit `{other}` (seconds, minutes, hours, days, weeks, months, years)"),
    })
}

pub fn add(d: NaiveDateTime, amount: i64, unit: &str) -> Result<NaiveDateTime> {
    let months = |m: i64| -> Result<NaiveDateTime> {
        let r = if m >= 0 { d.checked_add_months(Months::new(m as u32)) } else { d.checked_sub_months(Months::new((-m) as u32)) };
        r.ok_or_else(|| anyhow!("date out of range"))
    };
    Ok(match unit_of(unit)? {
        "seconds" => d + Duration::seconds(amount),
        "minutes" => d + Duration::minutes(amount),
        "hours" => d + Duration::hours(amount),
        "days" => d + Duration::days(amount),
        "weeks" => d + Duration::weeks(amount),
        "months" => months(amount)?,
        _ => months(amount * 12)?,
    })
}

/// `to - from` in whole units (months/years count whole calendar months).
pub fn diff(from: NaiveDateTime, to: NaiveDateTime, unit: &str) -> Result<i64> {
    let secs = (to - from).num_seconds();
    Ok(match unit_of(unit)? {
        "seconds" => secs,
        "minutes" => secs / 60,
        "hours" => secs / 3600,
        "days" => (to - from).num_days(),
        "weeks" => (to - from).num_days() / 7,
        u => {
            let mut m = (to.year() - from.year()) as i64 * 12 + to.month() as i64 - from.month() as i64;
            // Not yet a whole month if the day/time has not been reached.
            let (fd, td) = ((from.day(), from.time()), (to.day(), to.time()));
            if m > 0 && td < fd {
                m -= 1;
            } else if m < 0 && td > fd {
                m += 1;
            }
            if u == "years" { m / 12 } else { m }
        }
    })
}

/// Year, month, day, weekday... of a date, as an object.
pub fn parts(d: &NaiveDateTime) -> Value {
    json!({
        "year": d.year(), "month": d.month(), "day": d.day(),
        "hour": d.hour(), "minute": d.minute(), "second": d.second(),
        "weekday": d.weekday().number_from_monday(), "weekday_name": d.format("%A").to_string(),
        "month_name": d.format("%B").to_string(), "day_of_year": d.ordinal(),
        "date": d.format("%Y-%m-%d").to_string(), "time": d.format("%H:%M:%S").to_string(),
    })
}

// ---------------------------------------------------------------- calculate

/// Evaluate arithmetic: + - * / % ^, parentheses, comparisons (== != < <= > >=),
/// && || !, and functions round, int, floor, ceil, abs, min, max, sqrt, log.
/// Thousands separators in numbers like 1,23,456.50 are not allowed: remove them first.
pub fn calculate(expr: &str) -> Result<f64> {
    let e = expr.trim();
    if e.is_empty() {
        bail!("expression is empty");
    }
    let mut ns = |name: &str, args: Vec<f64>| -> Option<f64> {
        match (name, args.as_slice()) {
            ("sqrt", [x]) => Some(x.sqrt()),
            ("pow", [x, y]) => Some(x.powf(*y)),
            _ => None,
        }
    };
    fasteval::ez_eval(e, &mut ns).map_err(|err| anyhow!("cannot calculate `{e}`: {err:?}"))
}

/// A number as JSON: whole numbers become integers.
pub fn number_value(n: f64, decimals: Option<u32>) -> Value {
    let n = match decimals {
        Some(d) => {
            let f = 10f64.powi(d as i32);
            (n * f).round() / f
        }
        None => n,
    };
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDateTime {
        parse_date(s, "").unwrap()
    }

    #[test]
    fn text_helpers() {
        assert_eq!(title_case("hello WORLD, it's ok"), "Hello World, It's Ok");
        assert_eq!(url_encode("a b&c=ü"), "a%20b%26c%3D%C3%BC");
    }

    #[test]
    fn dates() {
        assert_eq!(format_date(&d("31/01/2026"), "yyyy-MM-dd"), "2026-01-31");
        assert_eq!(format_date(&d("2026-01-31 14:05:00"), "dd-MM-yyyy hh:mm tt"), "31-01-2026 02:05 PM");
        assert_eq!(format_date(&parse_date("01-31-2026", "MM-dd-yyyy").unwrap(), "dd/MM/yyyy"), "31/01/2026");
        assert_eq!(format_date(&d("45658"), "yyyy-MM-dd"), "2025-01-01", "Excel serial");
        assert_eq!(format_date(&add(d("31/01/2026"), 1, "month").unwrap(), "yyyy-MM-dd"), "2026-02-28");
        assert_eq!(format_date(&add(d("2026-03-01"), -1, "days").unwrap(), "yyyy-MM-dd"), "2026-02-28");
        assert_eq!(diff(d("2026-01-01"), d("2026-03-15"), "days").unwrap(), 73);
        assert_eq!(diff(d("2025-04-01"), d("2026-03-31"), "months").unwrap(), 11);
        assert_eq!(diff(d("2025-04-01"), d("2026-04-01"), "years").unwrap(), 1);
        assert_eq!(parts(&d("2026-11-19"))["weekday_name"], "Thursday");
        assert!(parse_date("not a date", "").is_err());
    }

    #[test]
    fn calc() {
        assert_eq!(calculate("7/2").unwrap(), 3.5);
        assert_eq!(calculate("(100 + 18) * 2").unwrap(), 236.0);
        assert_eq!(number_value(calculate("1000 * 0.18").unwrap(), Some(2)), json!(180));
        assert_eq!(number_value(calculate("10/3").unwrap(), Some(2)), json!(3.33));
        assert_eq!(calculate("max(3, 9, 4)").unwrap(), 9.0);
        assert_eq!(calculate("sqrt(16)").unwrap(), 4.0);
        assert_eq!(calculate("5 > 3").unwrap(), 1.0);
        assert!(calculate("2 +").is_err());
    }
}
