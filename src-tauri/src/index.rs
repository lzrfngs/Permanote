use std::fs;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::vault;

pub struct Index {
    conn: Mutex<Connection>,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub date: String,
    pub snippet: String,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS days (
  date TEXT PRIMARY KEY,
  content TEXT NOT NULL,
  modified TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS days_fts USING fts5(
  date UNINDEXED,
  content,
  tokenize='porter unicode61'
);
";

impl Index {
    pub fn open() -> Result<Self, String> {
        let path = vault::vault_root()?
            .join(".permanote")
            .join("index.sqlite");
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Wipe and rebuild from every day file on disk. Called on startup so the
    /// index is always recoverable from the source of truth.
    pub fn rebuild(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM days; DELETE FROM days_fts;")
            .map_err(|e| e.to_string())?;
        let days_dir = vault::vault_root()?.join("days");
        let entries = match fs::read_dir(&days_dir) {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };
        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if s.len() == 10 => s.to_string(),
                _ => continue,
            };
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(format_systime)
                .unwrap_or_default();
            insert_day(&conn, &stem, &content, &modified)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn index_day(&self, date: &str, content: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let modified = format_systime(std::time::SystemTime::now());
        insert_day(&conn, date, content, &modified)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let q = sanitize_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT date, snippet(days_fts, 1, '<<', '>>', '…', 12) \
                 FROM days_fts WHERE days_fts MATCH ?1 \
                 ORDER BY rank LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![q, limit as i64], |row| {
                Ok(SearchHit {
                    date: row.get(0)?,
                    snippet: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    }
}

fn insert_day(conn: &Connection, date: &str, content: &str, modified: &str) -> Result<(), String> {
    conn.execute("DELETE FROM days WHERE date = ?1", params![date])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM days_fts WHERE date = ?1", params![date])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO days(date, content, modified) VALUES(?1, ?2, ?3)",
        params![date, content, modified],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO days_fts(date, content) VALUES(?1, ?2)",
        params![date, content],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Strip FTS5 metacharacters; tokenize on whitespace; append `*` to each
/// token for prefix-match. Returns empty for empty/whitespace input.
fn sanitize_query(q: &str) -> String {
    let cleaned: String = q
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("{t}*"))
        .collect();
    tokens.join(" ")
}

fn format_systime(t: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}", dur.as_secs())
}
