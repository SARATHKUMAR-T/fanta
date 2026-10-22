use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("fanta")
}

pub fn open() -> anyhow::Result<Connection> {
    std::fs::create_dir_all(data_dir())?;
    let conn = Connection::open(data_dir().join("fanta.db"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );",
    )?;
    Ok(conn)
}

pub fn insert_message(conn: &Connection, role: &str, content: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO messages (role, content) VALUES (?1, ?2)",
        params![role, content],
    )?;
    Ok(())
}

/// Most recent `limit` messages, oldest first.
pub fn recent_messages(conn: &Connection, limit: usize) -> anyhow::Result<Vec<StoredMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, created_at FROM
           (SELECT * FROM messages ORDER BY id DESC LIMIT ?1)
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(StoredMessage {
            id: r.get(0)?,
            role: r.get(1)?,
            content: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
