//! Pomodoro / focus sessions. Shared by UI commands, chat tools, and the scheduler.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{calendar, db, desktop, notifications, AppState};

#[derive(Serialize)]
pub struct FocusStatus {
    pub active: Option<db::FocusSession>,
    pub task_title: Option<String>,
    pub blocks_today: i64,
    pub focus_minutes: u32,
    pub break_minutes: u32,
}

pub fn status(app: &AppHandle) -> anyhow::Result<FocusStatus> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let active = db::active_session(&conn)?;
    let task_title = match active.as_ref().and_then(|s| s.task_id) {
        Some(id) => db::get_task(&conn, id)?.map(|t| t.title),
        None => None,
    };
    Ok(FocusStatus {
        active,
        task_title,
        blocks_today: db::focus_blocks_today(&conn)?,
        focus_minutes: state.persona.pomodoro.focus_minutes,
        break_minutes: state.persona.pomodoro.break_minutes,
    })
}

/// Least focus time worth starting before a meeting.
const MIN_BLOCK_BEFORE_MEETING: i64 = 10;
/// Ends the block this long before a meeting, to get ready.
const MEETING_BUFFER: i64 = 2;

/// Starts a focus block, replacing any running session. If a meeting would
/// interrupt it, the block is shortened; the returned note says so.
pub fn start(app: &AppHandle, minutes: Option<u32>, task_id: Option<i64>) -> anyhow::Result<(db::FocusSession, Option<String>)> {
    let state = app.state::<AppState>();
    let mut minutes = minutes.filter(|m| (1..=240).contains(m)).unwrap_or(state.persona.pomodoro.focus_minutes);
    let mut note = None;
    if let Some(m) = calendar::next_meeting_within(app, minutes as i64 + MEETING_BUFFER) {
        let room = calendar::minutes_until(&m.start_at) - MEETING_BUFFER;
        if room < MIN_BLOCK_BEFORE_MEETING {
            anyhow::bail!("“{}” starts at {} - too close for a focus block. Start one after it?", m.subject, calendar::hm(&m.start_at));
        }
        minutes = room as u32;
        note = Some(format!("Shortened to {minutes} min so you're free for “{}” at {}.", m.subject, calendar::hm(&m.start_at)));
    }
    let session = {
        let conn = state.db.lock().unwrap();
        if let Some(prev) = db::active_session(&conn)? {
            db::end_session(&conn, prev.id, false)?;
        }
        let session = db::start_session(&conn, "focus", minutes, task_id)?;
        if state.persona.focus_dnd {
            desktop::dnd_on(&conn);
        }
        session
    };
    let _ = app.emit("focus://changed", ());
    Ok((session, note))
}

/// Stops whatever session is running. Returns false if nothing was running.
pub fn stop(app: &AppHandle) -> anyhow::Result<bool> {
    let state = app.state::<AppState>();
    let stopped = {
        let conn = state.db.lock().unwrap();
        desktop::dnd_off(&conn);
        match db::active_session(&conn)? {
            Some(s) => {
                db::end_session(&conn, s.id, false)?;
                if s.kind == "focus" {
                    notifications::spawn_digest(app.clone(), s.started_at);
                }
                true
            }
            None => false,
        }
    };
    let _ = app.emit("focus://changed", ());
    Ok(stopped)
}

/// Called by the scheduler: finishes an expired session and rolls focus -> break.
pub fn tick(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let p = &state.persona;
    let finished = {
        let conn = state.db.lock().unwrap();
        match db::active_session(&conn)? {
            Some(s) if s.ends_at <= db::now_ts() => {
                db::end_session(&conn, s.id, true)?;
                if s.kind == "focus" {
                    desktop::dnd_off(&conn);
                    db::start_session(&conn, "break", p.pomodoro.break_minutes, s.task_id)?;
                    notifications::spawn_digest(app.clone(), s.started_at.clone());
                }
                Some(s.kind)
            }
            _ => None,
        }
    };
    match finished.as_deref() {
        Some("focus") => desktop::notify(
            &p.name,
            &format!("Focus block done - nice. Take {} minutes, stretch, get some water.", p.pomodoro.break_minutes),
            false,
        ),
        Some(_) => desktop::notify(&p.name, "Break's over. Ready for another round when you are.", false),
        None => return Ok(()),
    }
    let _ = app.emit("focus://changed", ());
    Ok(())
}
