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
        CREATE TABLE IF NOT EXISTS notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            source TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            category TEXT NOT NULL DEFAULT 'pending',
            summary TEXT,
            reason TEXT,
            during_focus INTEGER NOT NULL DEFAULT 0,
            seen INTEGER NOT NULL DEFAULT 0,
            received_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS notifications_received ON notifications (received_at);
        CREATE TABLE IF NOT EXISTS calendar_events (
            num INTEGER PRIMARY KEY AUTOINCREMENT,
            remote_id TEXT NOT NULL UNIQUE,
            subject TEXT NOT NULL,
            start_at TEXT NOT NULL,
            end_at TEXT NOT NULL,
            all_day INTEGER NOT NULL DEFAULT 0,
            location TEXT,
            join_url TEXT,
            organizer TEXT,
            response TEXT,
            show_as TEXT,
            cancelled INTEGER NOT NULL DEFAULT 0,
            notified INTEGER NOT NULL DEFAULT 0,
            sync_id TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS calendar_events_start ON calendar_events (start_at);
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

// ---- captured notifications ----

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: i64,
    pub app: String,
    pub source: String,
    pub title: String,
    pub body: String,
    /// "pending" until triaged, then "urgent", "fyi", or "ignore".
    pub category: String,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub during_focus: bool,
    pub seen: bool,
    pub received_at: String,
}

const NOTIF_COLS: &str = "id, app, source, title, body, category, summary, reason, during_focus, seen, received_at";

fn notif_row(r: &rusqlite::Row) -> rusqlite::Result<Notification> {
    Ok(Notification {
        id: r.get(0)?,
        app: r.get(1)?,
        source: r.get(2)?,
        title: r.get(3)?,
        body: r.get(4)?,
        category: r.get(5)?,
        summary: r.get(6)?,
        reason: r.get(7)?,
        during_focus: r.get::<_, i64>(8)? != 0,
        seen: r.get::<_, i64>(9)? != 0,
        received_at: r.get(10)?,
    })
}

pub fn insert_notification(
    conn: &Connection,
    app: &str,
    source: &str,
    title: &str,
    body: &str,
    during_focus: bool,
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO notifications (app, source, title, body, during_focus, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![app, source, title, body, during_focus, now_ts()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_notification(conn: &Connection, id: i64) -> anyhow::Result<Option<Notification>> {
    Ok(conn
        .query_row(&format!("SELECT {NOTIF_COLS} FROM notifications WHERE id = ?1"), params![id], notif_row)
        .optional()?)
}

pub fn set_triage(conn: &Connection, id: i64, category: &str, summary: Option<&str>, reason: Option<&str>) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE notifications SET category = ?2, summary = COALESCE(?3, summary), reason = COALESCE(?4, reason) WHERE id = ?1",
        params![id, category, summary, reason],
    )?;
    Ok(())
}

/// Newest first. `category` None means all; `since` filters by received time.
pub fn list_notifications(
    conn: &Connection,
    category: Option<&str>,
    since: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NOTIF_COLS} FROM notifications
         WHERE (?1 IS NULL OR category = ?1) AND (?2 IS NULL OR received_at >= ?2)
         ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![category, since, limit as i64], notif_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn pending_notifications(conn: &Connection) -> anyhow::Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM notifications WHERE category = 'pending' ORDER BY id")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn unseen_notification_count(conn: &Connection) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE seen = 0 AND category IN ('urgent', 'fyi')",
        [],
        |r| r.get(0),
    )?)
}

pub fn mark_notifications_seen(conn: &Connection) -> anyhow::Result<()> {
    conn.execute("UPDATE notifications SET seen = 1 WHERE seen = 0", [])?;
    Ok(())
}

/// Keeps the inbox from growing forever; notification text is personal data.
pub fn prune_notifications(conn: &Connection, keep_days: u32) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM notifications WHERE received_at < datetime('now', 'localtime', ?1)",
        params![format!("-{keep_days} days")],
    )?;
    Ok(())
}

// ---- calendar cache ----

/// An event as fetched from the provider (times already converted to local `TS`).
#[derive(Debug, Clone, Default)]
pub struct NewCalEvent {
    pub remote_id: String,
    pub subject: String,
    pub start_at: String,
    pub end_at: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub join_url: Option<String>,
    pub organizer: Option<String>,
    pub response: Option<String>,
    pub show_as: Option<String>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalEvent {
    /// Short local id the chat can refer to.
    pub num: i64,
    pub remote_id: String,
    pub subject: String,
    pub start_at: String,
    pub end_at: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub join_url: Option<String>,
    pub organizer: Option<String>,
    pub response: Option<String>,
    pub show_as: Option<String>,
    pub cancelled: bool,
}

const CAL_COLS: &str =
    "num, remote_id, subject, start_at, end_at, all_day, location, join_url, organizer, response, show_as, cancelled";

/// Events that actually block the user's time.
const BUSY: &str = "all_day = 0 AND cancelled = 0 AND COALESCE(response, '') != 'declined' AND COALESCE(show_as, '') != 'free'";

fn cal_row(r: &rusqlite::Row) -> rusqlite::Result<CalEvent> {
    Ok(CalEvent {
        num: r.get(0)?,
        remote_id: r.get(1)?,
        subject: r.get(2)?,
        start_at: r.get(3)?,
        end_at: r.get(4)?,
        all_day: r.get::<_, i64>(5)? != 0,
        location: r.get(6)?,
        join_url: r.get(7)?,
        organizer: r.get(8)?,
        response: r.get(9)?,
        show_as: r.get(10)?,
        cancelled: r.get::<_, i64>(11)? != 0,
    })
}

fn upsert_event(conn: &Connection, e: &NewCalEvent, sync_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO calendar_events
           (remote_id, subject, start_at, end_at, all_day, location, join_url, organizer, response, show_as, cancelled, sync_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(remote_id) DO UPDATE SET
           subject = excluded.subject, end_at = excluded.end_at, all_day = excluded.all_day,
           location = excluded.location, join_url = excluded.join_url, organizer = excluded.organizer,
           response = excluded.response, show_as = excluded.show_as, cancelled = excluded.cancelled,
           notified = CASE WHEN start_at = excluded.start_at THEN notified ELSE 0 END,
           start_at = excluded.start_at, sync_id = excluded.sync_id",
        params![
            e.remote_id, e.subject, e.start_at, e.end_at, e.all_day, e.location, e.join_url,
            e.organizer, e.response, e.show_as, e.cancelled, sync_id
        ],
    )?;
    Ok(())
}

/// Replaces the cache with a fresh sync, keeping local ids and heads-up state
/// for events that didn't move.
pub fn replace_calendar(conn: &mut Connection, events: &[NewCalEvent]) -> anyhow::Result<()> {
    let sync_id = now_ts();
    let tx = conn.transaction()?;
    for e in events {
        upsert_event(&tx, e, &sync_id)?;
    }
    tx.execute("DELETE FROM calendar_events WHERE sync_id != ?1", params![sync_id])?;
    tx.commit()?;
    Ok(())
}

/// Adds one event (e.g. one Fanta just created) without waiting for a sync.
pub fn add_event(conn: &Connection, e: &NewCalEvent) -> anyhow::Result<CalEvent> {
    upsert_event(conn, e, &now_ts())?;
    Ok(conn.query_row(
        &format!("SELECT {CAL_COLS} FROM calendar_events WHERE remote_id = ?1"),
        params![e.remote_id],
        cal_row,
    )?)
}

pub fn clear_calendar(conn: &Connection) -> anyhow::Result<()> {
    conn.execute("DELETE FROM calendar_events", [])?;
    Ok(())
}

/// Non-cancelled events overlapping [from, to), in start order.
pub fn events_between(conn: &Connection, from: &str, to: &str) -> anyhow::Result<Vec<CalEvent>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CAL_COLS} FROM calendar_events
         WHERE cancelled = 0 AND start_at < ?2 AND end_at > ?1 ORDER BY all_day DESC, start_at"
    ))?;
    let rows = stmt.query_map(params![from, to], cal_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Busy (non-declined, non-free, timed) events overlapping [from, to).
pub fn busy_between(conn: &Connection, from: &str, to: &str) -> anyhow::Result<Vec<CalEvent>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CAL_COLS} FROM calendar_events WHERE {BUSY} AND start_at < ?2 AND end_at > ?1 ORDER BY start_at"
    ))?;
    let rows = stmt.query_map(params![from, to], cal_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_event(conn: &Connection, num: i64) -> anyhow::Result<Option<CalEvent>> {
    Ok(conn
        .query_row(&format!("SELECT {CAL_COLS} FROM calendar_events WHERE num = ?1"), params![num], cal_row)
        .optional()?)
}

/// Busy events starting in [from, to) that haven't had a heads-up yet.
pub fn due_heads_up(conn: &Connection, from: &str, to: &str) -> anyhow::Result<Vec<CalEvent>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CAL_COLS} FROM calendar_events
         WHERE {BUSY} AND notified = 0 AND start_at >= ?1 AND start_at < ?2 ORDER BY start_at"
    ))?;
    let rows = stmt.query_map(params![from, to], cal_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn mark_event_notified(conn: &Connection, num: i64) -> anyhow::Result<()> {
    conn.execute("UPDATE calendar_events SET notified = 1 WHERE num = ?1", params![num])?;
    Ok(())
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
