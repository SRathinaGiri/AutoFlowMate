//! Databases: SQLite files (built in) and anything with an ODBC driver:
//! SQL Server, MySQL, PostgreSQL, Oracle, MS Access, TallyPrime's ODBC server...
//!
//! `connection` is either
//! * a SQLite file: `sqlite:data/app.db`, or any path ending in .db / .sqlite / .sqlite3; or
//! * an ODBC connection string: `DSN=TallyODBC64_9000`,
//!   `Driver={ODBC Driver 18 for SQL Server};Server=.;Database=Sales;Trusted_Connection=yes;TrustServerCertificate=yes`.
//!   `{password}` in it is replaced by the secret named in `password_secret`.
//!
//! Values are passed with `?` placeholders and `params`, never pasted into the SQL.

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Map, Value};
use std::path::Path;

pub enum Conn {
    Sqlite(rusqlite::Connection),
    Odbc(odbc_api::Connection<'static>),
}

/// The SQLite file a connection string names, if it is one.
pub fn sqlite_path(conn: &str) -> Option<String> {
    let c = conn.trim();
    if let Some(p) = c.strip_prefix("sqlite:") {
        return Some(p.trim_start_matches("//").to_string());
    }
    let lower = c.to_ascii_lowercase();
    if !c.contains('=') && (lower.ends_with(".db") || lower.ends_with(".sqlite") || lower.ends_with(".sqlite3") || lower == ":memory:") {
        return Some(c.to_string());
    }
    None
}

/// Connection strings are logged without their password.
pub fn redact(conn: &str) -> String {
    conn.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, _)) if matches!(k.trim().to_ascii_lowercase().as_str(), "pwd" | "password") => format!("{k}=***"),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The file a SQLite connection string points at (relative to the flow folder).
pub fn sqlite_file(conn: &str, flow_dir: &Path) -> Option<String> {
    sqlite_path(conn).map(|p| if p == ":memory:" || Path::new(&p).is_absolute() { p } else { flow_dir.join(&p).display().to_string() })
}

/// Open a database. `read_only` (dry runs) opens SQLite without write access and never creates it.
pub fn open(conn: &str, flow_dir: &Path, read_only: bool) -> Result<Conn> {
    if let Some(path) = sqlite_file(conn, flow_dir) {
        if read_only {
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
            let c = rusqlite::Connection::open_with_flags(&path, flags).with_context(|| format!("cannot open SQLite database {path}"))?;
            return Ok(Conn::Sqlite(c));
        }
        if path != ":memory:" {
            if let Some(dir) = Path::new(&path).parent() {
                std::fs::create_dir_all(dir).ok();
            }
        }
        let c = rusqlite::Connection::open(&path).with_context(|| format!("cannot open SQLite database {path}"))?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        return Ok(Conn::Sqlite(c));
    }
    let env = odbc_api::environment().map_err(|e| anyhow!("ODBC is not available: {e}"))?;
    let c = env.connect_with_connection_string(conn, odbc_api::ConnectionOptions::default()).map_err(|e| anyhow!("cannot connect to {}: {e}", redact(conn)))?;
    Ok(Conn::Odbc(c))
}

fn to_sqlite(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::Null => S::Null,
        Value::Bool(b) => S::Integer(*b as i64),
        Value::Number(n) => n.as_i64().map(S::Integer).unwrap_or_else(|| S::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => S::Text(s.clone()),
        other => S::Text(other.to_string()),
    }
}

fn from_sqlite(v: rusqlite::types::ValueRef) -> Value {
    use rusqlite::types::ValueRef as R;
    match v {
        R::Null => Value::Null,
        R::Integer(i) => json!(i),
        R::Real(f) => crate::textops::number_value(f, None),
        R::Text(t) => json!(String::from_utf8_lossy(t)),
        R::Blob(b) => json!(format!("<{} bytes>", b.len())),
    }
}

fn odbc_params(params: &[Value]) -> Vec<Option<String>> {
    params
        .iter()
        .map(|v| match v {
            Value::Null => None,
            Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
            other => Some(crate::vars::to_text(other)),
        })
        .collect()
}

/// Turn an ODBC text cell into a number/bool where the column type says so.
fn odbc_value(text: Option<String>, dt: &odbc_api::DataType) -> Value {
    use odbc_api::DataType as D;
    let Some(t) = text else { return Value::Null };
    match dt {
        D::Integer | D::SmallInt | D::BigInt | D::TinyInt => t.trim().parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
        D::Numeric { scale: 0, .. } | D::Decimal { scale: 0, .. } => t.trim().parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
        D::Numeric { .. } | D::Decimal { .. } | D::Float { .. } | D::Real | D::Double => {
            t.trim().parse::<f64>().map(|n| crate::textops::number_value(n, None)).unwrap_or(json!(t))
        }
        D::Bit => json!(matches!(t.trim(), "1" | "true" | "TRUE")),
        _ => json!(t),
    }
}

/// Run a SELECT and return its rows as records {column: value}. `max_rows` 0 = all.
pub fn query(c: &mut Conn, sql: &str, params: &[Value], max_rows: usize) -> Result<Vec<Value>> {
    let limit = if max_rows == 0 { usize::MAX } else { max_rows };
    match c {
        Conn::Sqlite(db) => {
            let mut stmt = db.prepare(sql).map_err(|e| anyhow!("SQL error: {e}"))?;
            let names: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter().map(to_sqlite))).map_err(|e| anyhow!("SQL error: {e}"))?;
            let mut out = vec![];
            while let Some(r) = rows.next()? {
                if out.len() >= limit {
                    break;
                }
                let mut m = Map::new();
                for (i, n) in names.iter().enumerate() {
                    m.insert(n.clone(), from_sqlite(r.get_ref(i)?));
                }
                out.push(Value::Object(m));
            }
            Ok(out)
        }
        Conn::Odbc(db) => {
            use odbc_api::{Cursor, IntoParameter, ResultSetMetadata};
            let p: Vec<_> = odbc_params(params).into_iter().map(|v| v.into_parameter()).collect();
            let Some(mut cursor) = db.execute(sql, p.as_slice(), None).map_err(|e| anyhow!("SQL error: {e}"))? else {
                bail!("the statement returned no rows (use db_execute for INSERT / UPDATE / DELETE)");
            };
            let names: Vec<String> = cursor.column_names()?.collect::<std::result::Result<_, _>>()?;
            let types: Vec<odbc_api::DataType> = (1..=names.len() as u16).map(|i| cursor.col_data_type(i)).collect::<std::result::Result<_, _>>()?;
            let mut out = vec![];
            let mut buf = Vec::new();
            while let Some(mut row) = cursor.next_row()? {
                if out.len() >= limit {
                    break;
                }
                let mut m = Map::new();
                for (i, n) in names.iter().enumerate() {
                    buf.clear();
                    let present = row.get_text(i as u16 + 1, &mut buf)?;
                    let text = present.then(|| String::from_utf8_lossy(&buf).into_owned());
                    m.insert(n.clone(), odbc_value(text, &types[i]));
                }
                out.push(Value::Object(m));
            }
            Ok(out)
        }
    }
}

/// Run INSERT / UPDATE / DELETE / CREATE... Returns the number of rows changed.
/// Without params, SQLite runs every statement of a script separated by `;`.
pub fn execute(c: &mut Conn, sql: &str, params: &[Value]) -> Result<usize> {
    match c {
        Conn::Sqlite(db) => {
            if params.is_empty() {
                let before = db.total_changes();
                db.execute_batch(sql).map_err(|e| anyhow!("SQL error: {e}"))?;
                Ok((db.total_changes() - before) as usize)
            } else {
                db.execute(sql, rusqlite::params_from_iter(params.iter().map(to_sqlite))).map_err(|e| anyhow!("SQL error: {e}"))
            }
        }
        Conn::Odbc(db) => {
            use odbc_api::IntoParameter;
            let p: Vec<_> = odbc_params(params).into_iter().map(|v| v.into_parameter()).collect();
            let mut stmt = db.preallocate()?;
            stmt.execute(sql, p.as_slice()).map_err(|e| anyhow!("SQL error: {e}"))?;
            Ok(stmt.row_count()?.unwrap_or(0))
        }
    }
}

/// ODBC drivers installed on this PC (for Studio and error messages).
pub fn odbc_drivers() -> Vec<String> {
    odbc_api::environment().ok().and_then(|e| e.drivers().ok()).map(|d| d.into_iter().map(|d| d.description).collect()).unwrap_or_default()
}

/// ODBC data sources (DSNs) set up on this PC: (name, driver).
pub fn odbc_data_sources() -> Vec<(String, String)> {
    odbc_api::environment().ok().and_then(|e| e.data_sources().ok()).map(|d| d.into_iter().map(|s| (s.server_name, s.driver)).collect()).unwrap_or_default()
}

/// A connection string to start from for a driver, with the parts the user fills in.
pub fn connection_template(driver: &str) -> String {
    let d = driver.to_lowercase();
    let rest = if d.contains("excel") {
        "DBQ=C:\\data\\book.xlsx;ReadOnly=1"
    } else if d.contains("access") && d.contains("text") {
        "DefaultDir=C:\\data;Extensions=csv,txt"
    } else if d.contains("access") {
        "DBQ=C:\\data\\database.accdb"
    } else if d.contains("dbase") {
        "DefaultDir=C:\\data"
    } else if d.contains("sql server") {
        "Server=.;Database=MyDatabase;Trusted_Connection=yes;TrustServerCertificate=yes"
    } else if d.contains("mysql") || d.contains("mariadb") {
        "Server=localhost;Database=mydb;UID=user;PWD={password}"
    } else if d.contains("postgres") {
        "Server=localhost;Port=5432;Database=mydb;UID=user;PWD={password}"
    } else if d.contains("oracle") {
        "DBQ=//host:1521/service;UID=user;PWD={password}"
    } else if d.contains("tally") {
        "Server=localhost;Port=9000"
    } else {
        "Server=;Database=;UID=;PWD={password}"
    };
    format!("Driver={{{driver}}};{rest}")
}

/// Open the connection once to check it works (Studio's Test button).
pub fn test_connection(conn: &str, flow_dir: &Path) -> Result<String> {
    if let Some(f) = sqlite_file(conn, flow_dir) {
        if f != ":memory:" && !Path::new(&f).exists() {
            return Ok(format!("{f} does not exist yet; it will be created on the first db_execute"));
        }
        let mut c = open(conn, flow_dir, true)?;
        let tables = query(&mut c, "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name", &[], 0)?;
        let names: Vec<String> = tables.iter().filter_map(|t| t.get("name").and_then(Value::as_str)).map(str::to_string).collect();
        return Ok(format!("connected; tables: {}", if names.is_empty() { "(none)".into() } else { names.join(", ") }));
    }
    open(conn, flow_dir, true)?;
    Ok("connected".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_strings() {
        assert_eq!(sqlite_path("sqlite:data/app.db").as_deref(), Some("data/app.db"));
        assert_eq!(sqlite_path("C:\\x\\sales.sqlite").as_deref(), Some("C:\\x\\sales.sqlite"));
        assert_eq!(sqlite_path("DSN=TallyODBC64_9000"), None);
        assert_eq!(redact("Driver={X};Server=s;UID=sa;PWD=hunter2"), "Driver={X};Server=s;UID=sa;PWD=***");
    }

    #[test]
    fn sqlite_round_trip() {
        let mut c = open(":memory:", Path::new("."), false).unwrap();
        let n = execute(&mut c, "CREATE TABLE t (id INTEGER, name TEXT, amount REAL); INSERT INTO t VALUES (1, 'a', 1.5);", &[]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(execute(&mut c, "INSERT INTO t VALUES (?, ?, ?)", &[json!(2), json!("O'Brien"), json!(99.25)]).unwrap(), 1);
        let rows = query(&mut c, "SELECT * FROM t WHERE amount > ? ORDER BY id", &[json!(1)], 0).unwrap();
        assert_eq!(rows, vec![json!({"id": 1, "name": "a", "amount": 1.5}), json!({"id": 2, "name": "O'Brien", "amount": 99.25})]);
        assert_eq!(query(&mut c, "SELECT * FROM t", &[], 1).unwrap().len(), 1);
        assert!(query(&mut c, "SELECT * FROM missing", &[], 0).unwrap_err().to_string().contains("no such table"));
    }
}
