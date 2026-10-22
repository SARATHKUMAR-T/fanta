use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;

/// All timestamps are stored as local time in this format, so they sort and compare as text.
pub const TS: &str = "%Y-%m-%d %H:%M:%S";

pub fn now_ts() -> String {
    chrono::Local::now().format(TS).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub due_at: Option<String>,
    pub remind_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusSession {
    pub id: i64,
    /// "focus" or "break"
    pub kind: String,
    pub task_id: Option<i64>,
    pub started_at: String,
    pub ends_at: String,
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
        );
        CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            done INTEGER NOT NULL DEFAULT 0,
            due_at TEXT,
            remind_at TEXT,
            reminded INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            done_at TEXT
        );
        CREATE TABLE IF NOT EXISTS focus_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            task_id INTEGER,
            started_at TEXT NOT NULL,
            ends_at TEXT NOT NULL,
            ended_at TEXT,
            completed INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(conn)
}

// ---- messages ----

pub fn insert_message(conn: &Connection, role: &str, content: &str) -> anyhow::Result<()> {
    conn.execute("INSERT INTO messages (role, content) VALUES (?1, ?2)", params![role, content])?;
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
        Ok(StoredMessage { id: r.get(0)?, role: r.get(1)?, content: r.get(2)?, created_at: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---- tasks ----

const TASK_COLS: &str = "id, title, done, due_at, remind_at, created_at";

fn task_row(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        done: r.get::<_, i64>(2)? != 0,
        due_at: r.get(3)?,
        remind_at: r.get(4)?,
        created_at: r.get(5)?,
    })
}

pub fn add_task(conn: &Connection, title: &str, due_at: Option<&str>, remind_at: Option<&str>) -> anyhow::Result<Task> {
    conn.execute(
        "INSERT INTO tasks (title, due_at, remind_at) VALUES (?1, ?2, ?3)",
        params![title, due_at, remind_at],
    )?;
    get_task(conn, conn.last_insert_rowid())?.ok_or_else(|| anyhow::anyhow!("task vanished"))
}

pub fn get_task(conn: &Connection, id: i64) -> anyhow::Result<Option<Task>> {
    Ok(conn
        .query_row(&format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"), params![id], task_row)
        .optional()?)
}

/// Open tasks first (soonest due first), then done tasks (most recent first).
pub fn list_tasks(conn: &Connection, include_done: bool) -> anyhow::Result<Vec<Task>> {
    let filter = if include_done { "" } else { "WHERE done = 0" };
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks {filter}
         ORDER BY done ASC, COALESCE(due_at, remind_at) IS NULL, COALESCE(due_at, remind_at), done_at DESC, id"
    ))?;
    let rows = stmt.query_map([], task_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_task_done(conn: &Connection, id: i64, done: bool) -> anyhow::Result<bool> {
    let n = conn.execute(
        "UPDATE tasks SET done = ?2, done_at = CASE WHEN ?2 THEN datetime('now', 'localtime') END WHERE id = ?1",
        params![id, done],
    )?;
    Ok(n > 0)
}

pub fn delete_task(conn: &Connection, id: i64) -> anyhow::Result<bool> {
    Ok(conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])? > 0)
}

/// Open tasks whose reminder time has passed and that haven't fired yet.
pub fn due_reminders(conn: &Connection) -> anyhow::Result<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE done = 0 AND reminded = 0 AND remind_at IS NOT NULL AND remind_at <= ?1"
    ))?;
    let rows = stmt.query_map(params![now_ts()], task_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn mark_reminded(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("UPDATE tasks SET reminded = 1 WHERE id = ?1", params![id])?;
    Ok(())
}

// ---- focus sessions ----

pub fn active_session(conn: &Connection) -> anyhow::Result<Option<FocusSession>> {
    Ok(conn
        .query_row(
            "SELECT id, kind, task_id, started_at, ends_at FROM focus_sessions
             WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok(FocusSession {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    task_id: r.get(2)?,
                    started_at: r.get(3)?,
                    ends_at: r.get(4)?,
                })
            },
        )
        .optional()?)
}

pub fn start_session(conn: &Connection, kind: &str, minutes: u32, task_id: Option<i64>) -> anyhow::Result<FocusSession> {
    let now = chrono::Local::now();
    let ends = now + chrono::Duration::minutes(minutes as i64);
    conn.execute(
        "INSERT INTO focus_sessions (kind, task_id, started_at, ends_at) VALUES (?1, ?2, ?3, ?4)",
        params![kind, task_id, now.format(TS).to_string(), ends.format(TS).to_string()],
    )?;
    active_session(conn)?.ok_or_else(|| anyhow::anyhow!("session vanished"))
}

pub fn end_session(conn: &Connection, id: i64, completed: bool) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE focus_sessions SET ended_at = ?2, completed = ?3 WHERE id = ?1",
        params![id, now_ts(), completed],
    )?;
    Ok(())
}

/// Number of completed focus blocks today.
pub fn focus_blocks_today(conn: &Connection) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM focus_sessions
         WHERE kind = 'focus' AND completed = 1 AND date(started_at) = date('now', 'localtime')",
        [],
        |r| r.get(0),
    )?)
}

/// When the most recent focus block ended (or started, if still running).
pub fn last_focus_activity(conn: &Connection) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(ended_at, started_at) FROM focus_sessions WHERE kind = 'focus' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

// ---- settings (small key/value state) ----

pub fn get_setting(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete_setting(conn: &Connection, key: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}
