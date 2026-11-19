//! Reading tables and text from Excel, CSV and PDF files.
//!
//! Every table source is turned into raw rows of JSON values, then `shape`
//! applies the common options: header row, a single column, skipping blanks.

use anyhow::{anyhow, bail, Context as _, Result};
use calamine::{open_workbook_auto, Data, Reader};
use serde_json::{Map, Value};
use std::path::Path;

/// Sheet names of a workbook (.xlsx, .xlsm, .xls, .xlsb, .ods), in workbook order.
pub fn sheet_names(path: &Path) -> Result<Vec<String>> {
    let wb = open_workbook_auto(path).with_context(|| format!("cannot open workbook {}", path.display()))?;
    Ok(wb.sheet_names().to_vec())
}

/// Raw rows of a sheet (empty name = first sheet), optionally limited to a range like `A1:D50`.
/// Returns the sheet name actually used.
pub fn read_excel_raw(path: &Path, sheet: &str, range: &str) -> Result<(String, Vec<Vec<Value>>)> {
    let mut wb = open_workbook_auto(path).with_context(|| format!("cannot open workbook {}", path.display()))?;
    let names = wb.sheet_names().to_vec();
    let name = if sheet.trim().is_empty() {
        names.first().cloned().ok_or_else(|| anyhow!("the workbook has no sheets"))?
    } else {
        names
            .iter()
            .find(|n| n.eq_ignore_ascii_case(sheet.trim()))
            .cloned()
            .ok_or_else(|| anyhow!("sheet \"{sheet}\" not found; sheets are: {}", names.join(", ")))?
    };
    let mut r = wb.worksheet_range(&name).map_err(|e| anyhow!("cannot read sheet \"{name}\": {e}"))?;
    if !range.trim().is_empty() {
        let (a, b) = parse_range(range)?;
        r = r.range(a, b);
    }
    let rows = r.rows().map(|row| row.iter().map(cell_value).collect()).collect();
    Ok((name, rows))
}

fn cell_value(c: &Data) -> Value {
    match c {
        Data::Empty => Value::String(String::new()),
        Data::String(s) => Value::String(s.clone()),
        Data::Int(i) => Value::from(*i),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(*f as i64),
        Data::Float(f) => Value::from(*f),
        Data::Bool(b) => Value::Bool(*b),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(d) if d.time() == chrono::NaiveTime::MIN => Value::String(d.format("%Y-%m-%d").to_string()),
            Some(d) => Value::String(d.format("%Y-%m-%d %H:%M:%S").to_string()),
            None => Value::from(dt.as_f64()),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => Value::String(s.clone()),
        Data::Error(e) => Value::String(format!("#{e:?}")),
    }
}

/// "B2:D50" -> ((row, col), (row, col)), zero-based.
fn parse_range(s: &str) -> Result<((u32, u32), (u32, u32))> {
    let (a, b) = s.trim().split_once(':').ok_or_else(|| anyhow!("range must look like A1:D50, got `{s}`"))?;
    Ok((parse_cell(a)?, parse_cell(b)?))
}

fn parse_cell(s: &str) -> Result<(u32, u32)> {
    let s = s.trim().to_ascii_uppercase();
    let letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &s[letters.len()..];
    let col = column_index(&letters).ok_or_else(|| anyhow!("bad cell `{s}`"))?;
    let row: u32 = digits.parse().map_err(|_| anyhow!("bad cell `{s}`"))?;
    if row == 0 {
        bail!("bad cell `{s}`");
    }
    Ok((row - 1, col as u32))
}

/// "A" -> 0, "Z" -> 25, "AA" -> 26.
fn column_index(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut n = 0usize;
    for c in letters.to_ascii_uppercase().chars() {
        n = n * 26 + (c as usize - 'A' as usize + 1);
    }
    Some(n - 1)
}

fn column_letter(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

/// Raw rows of a CSV/TSV file. `delimiter` empty = detect (comma, semicolon, tab, pipe).
pub fn read_csv_raw(path: &Path, delimiter: &str) -> Result<Vec<Vec<Value>>> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    // Excel saves "CSV UTF-8" with a BOM and plain "CSV" in the ANSI code page: accept both.
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
    let text = text.trim_start_matches('\u{feff}');
    let delim = match delimiter {
        "" => detect_delimiter(text),
        "\\t" | "tab" => b'\t',
        d if d.len() == 1 => d.as_bytes()[0],
        d => bail!("delimiter must be one character (or empty to detect), got `{d}`"),
    };
    let mut rdr = csv::ReaderBuilder::new().has_headers(false).flexible(true).delimiter(delim).from_reader(text.as_bytes());
    let mut rows = vec![];
    for rec in rdr.records() {
        let rec = rec.with_context(|| format!("bad CSV in {}", path.display()))?;
        rows.push(rec.iter().map(|f| Value::String(f.trim().to_string())).collect());
    }
    Ok(rows)
}

fn detect_delimiter(text: &str) -> u8 {
    let first = text.lines().next().unwrap_or("");
    [b',', b';', b'\t', b'|']
        .into_iter()
        .max_by_key(|d| first.bytes().filter(|b| b == d).count())
        .filter(|d| first.as_bytes().contains(d))
        .unwrap_or(b',')
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

/// Turn raw rows into the variable value:
/// * `column` set: a list with that column's values (header name, letter like `B`, or 1-based number);
/// * else with a header row: a list of records {header: value};
/// * else: a list of rows (lists).
pub fn shape(mut rows: Vec<Vec<Value>>, has_header: bool, column: &str, skip_empty: bool) -> Result<Value> {
    if skip_empty {
        rows.retain(|r| !r.iter().all(is_blank));
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let headers: Vec<String> = if has_header && !rows.is_empty() {
        let mut seen = std::collections::HashSet::new();
        rows.remove(0)
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let base = match crate::vars::to_text(v).trim() {
                    "" => format!("Column{}", i + 1),
                    t => t.to_string(),
                };
                let mut name = base.clone();
                let mut n = 2;
                while !seen.insert(name.clone()) {
                    name = format!("{base}_{n}");
                    n += 1;
                }
                name
            })
            .chain((0..width).map(column_letter))
            .take(width)
            .collect()
    } else {
        (0..width).map(column_letter).collect()
    };

    let col = column.trim();
    if !col.is_empty() {
        let idx = headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(col))
            .or_else(|| col.parse::<usize>().ok().filter(|n| *n >= 1).map(|n| n - 1))
            .or_else(|| column_index(col))
            .filter(|i| *i < width.max(1))
            .ok_or_else(|| anyhow!("column \"{col}\" not found; columns are: {}", headers.join(", ")))?;
        let mut out: Vec<Value> = rows.iter().map(|r| r.get(idx).cloned().unwrap_or(Value::String(String::new()))).collect();
        if skip_empty {
            out.retain(|v| !is_blank(v));
        }
        return Ok(Value::Array(out));
    }
    if has_header {
        Ok(Value::Array(
            rows.into_iter()
                .map(|r| {
                    let mut m = Map::new();
                    for (i, h) in headers.iter().enumerate() {
                        m.insert(h.clone(), r.get(i).cloned().unwrap_or(Value::String(String::new())));
                    }
                    Value::Object(m)
                })
                .collect(),
        ))
    } else {
        Ok(Value::Array(rows.into_iter().map(Value::Array).collect()))
    }
}

/// Header names of a table (first row), for editors.
pub fn headers(rows: &[Vec<Value>]) -> Vec<String> {
    rows.iter()
        .find(|r| !r.iter().all(is_blank))
        .map(|r| r.iter().map(|v| crate::vars::to_text(v).trim().to_string()).collect())
        .unwrap_or_default()
}

/// Text of a PDF; `pages` like "1-3,5" (empty = all). Returns (text, page count).
pub fn read_pdf(path: &Path, pages: &str) -> Result<(String, usize)> {
    let all = pdf_extract::extract_text_by_pages(path).map_err(|e| anyhow!("cannot read PDF {}: {e}", path.display()))?;
    let n = all.len();
    let wanted: Vec<usize> = if pages.trim().is_empty() {
        (1..=n).collect()
    } else {
        let mut v = vec![];
        for part in pages.split(',') {
            let part = part.trim();
            let (a, b) = match part.split_once('-') {
                Some((a, b)) => (a.trim().parse::<usize>(), if b.trim().is_empty() { Ok(n) } else { b.trim().parse::<usize>() }),
                None => (part.parse::<usize>(), part.parse::<usize>()),
            };
            let (a, b) = (a.map_err(|_| anyhow!("bad page range `{part}`"))?, b.map_err(|_| anyhow!("bad page range `{part}`"))?);
            if a == 0 || b < a {
                bail!("bad page range `{part}`");
            }
            v.extend(a..=b.min(n));
        }
        v
    };
    let text = wanted.iter().filter_map(|p| all.get(p - 1)).map(|t| t.trim_end()).collect::<Vec<_>>().join("\n\n");
    Ok((text, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw() -> Vec<Vec<Value>> {
        vec![
            vec![json!("GSTIN"), json!("Name"), json!("")],
            vec![json!("09AAACH7409R1ZZ"), json!("HCL UP"), json!("")],
            vec![json!(""), json!(""), json!("")],
            vec![json!("33AAACH7409R1Z8"), json!("HCL TN"), json!("x")],
        ]
    }

    #[test]
    fn shapes() {
        assert_eq!(shape(raw(), true, "GSTIN", true).unwrap(), json!(["09AAACH7409R1ZZ", "33AAACH7409R1Z8"]));
        assert_eq!(shape(raw(), true, "b", true).unwrap(), json!(["HCL UP", "HCL TN"]));
        assert_eq!(shape(raw(), true, "2", true).unwrap(), json!(["HCL UP", "HCL TN"]));
        let recs = shape(raw(), true, "", true).unwrap();
        assert_eq!(recs[1]["Name"], json!("HCL TN"));
        assert_eq!(recs[1]["Column3"], json!("x"));
        assert_eq!(shape(raw(), false, "", true).unwrap().as_array().unwrap().len(), 3);
        assert!(shape(raw(), true, "Nope", true).is_err());
    }

    #[test]
    fn cells_and_columns() {
        assert_eq!(parse_cell("B2").unwrap(), (1, 1));
        assert_eq!(parse_cell("aa10").unwrap(), (9, 26));
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(27), "AB");
        assert_eq!(column_index("AB"), Some(27));
    }

    #[test]
    fn csv_detects_delimiters() {
        let dir = std::env::temp_dir().join(format!("afm_csv_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.csv");
        std::fs::write(&p, "\u{feff}a;b;c\n1;\"x;y\";3\n").unwrap();
        let rows = read_csv_raw(&p, "").unwrap();
        assert_eq!(rows[0][0], json!("a"));
        assert_eq!(rows[1][1], json!("x;y"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
