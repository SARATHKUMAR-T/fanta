//! Pomodoro timer: focus blocks, short and long breaks, pause/extend/skip.
//! Shared by UI commands, chat tools, and the scheduler.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{calendar, db, desktop, notifications, stats, AppState};

/// Timer settings. Stored in the settings table; defaults come from persona.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub focus_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    /// A long break after this many pomodoros.
    pub long_break_every: u32,
    pub daily_goal: u32,
    /// Start the break automatically when a focus block ends.
    pub auto_start_breaks: bool,
    /// Start the next focus block automatically when a break ends.
    pub auto_start_focus: bool,
}

const CONFIG_KEY: &str = "pomodoro_config";

pub fn config(app: &AppHandle) -> Config {
    let state = app.state::<AppState>();
    let p = &state.persona.pomodoro;
    let defaults = Config {
        focus_minutes: p.focus_minutes,
        short_break_minutes: p.break_minutes,
        long_break_minutes: 15,
        long_break_every: 4,
        daily_goal: 8,
        auto_start_breaks: true,
        auto_start_focus: false,
    };
    let saved = db::get_setting(&state.db.lock().unwrap(), CONFIG_KEY).ok().flatten();
    saved.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(defaults)
}

pub fn set_config(app: &AppHandle, c: &Config) -> anyhow::Result<()> {
    let ok = |v: u32, lo: u32, hi: u32| (lo..=hi).contains(&v);
    if !(ok(c.focus_minutes, 1, 180)
        && ok(c.short_break_minutes, 1, 60)
        && ok(c.long_break_minutes, 1, 120)
        && ok(c.long_break_every, 2, 12)
        && ok(c.daily_goal, 1, 30))
    {
        anyhow::bail!("those numbers are out of range");
    }
    db::set_setting(&app.state::<AppState>().db.lock().unwrap(), CONFIG_KEY, &serde_json::to_string(c)?)?;
    let _ = app.emit("focus://changed", ());
    Ok(())
}

#[derive(Serialize)]
pub struct FocusStatus {
    pub active: Option<db::FocusSession>,
    pub task_title: Option<String>,
    pub blocks_today: i64,
    pub focus_minutes: u32,
    pub break_minutes: u32,
    pub config: Config,
    /// Pomodoros done in the current set (resets after a long break).
    pub cycle_done: i64,
    /// What the next break will be: "short_break" or "long_break".
    pub next_break: String,
}

fn next_break(cfg: &Config, cycle_done: i64) -> &'static str {
    if cycle_done > 0 && cycle_done % cfg.long_break_every as i64 == 0 {
        "long_break"
    } else {
        "short_break"
    }
}

pub fn status(app: &AppHandle) -> anyhow::Result<FocusStatus> {
    let cfg = config(app);
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let active = db::active_session(&conn)?;
    let task_title = match active.as_ref().and_then(|s| s.task_id) {
        Some(id) => db::get_task(&conn, id)?.map(|t| t.title),
        None => None,
    };
    let cycle_done = db::focus_since_long_break(&conn)?;
    Ok(FocusStatus {
        active,
        task_title,
        blocks_today: db::focus_blocks_today(&conn)?,
        focus_minutes: cfg.focus_minutes,
        break_minutes: cfg.short_break_minutes,
        next_break: next_break(&cfg, cycle_done).into(),
        cycle_done,
        config: cfg,
    })
}

pub fn analytics(app: &AppHandle) -> anyhow::Result<stats::Stats> {
    let goal = config(app).daily_goal as i64;
    stats::compute(&app.state::<AppState>().db.lock().unwrap(), goal)
}

/// Least focus time worth starting before a meeting.
const MIN_BLOCK_BEFORE_MEETING: i64 = 10;
/// Ends the block this long before a meeting, to get ready.
const MEETING_BUFFER: i64 = 2;

fn changed(app: &AppHandle) {
    let _ = app.emit("focus://changed", ());
}

/// Starts a focus block, replacing any running session. If a meeting would
/// interrupt it, the block is shortened; the returned note says so.
pub fn start(app: &AppHandle, minutes: Option<u32>, task_id: Option<i64>) -> anyhow::Result<(db::FocusSession, Option<String>)> {
    let cfg = config(app);
    let state = app.state::<AppState>();
    let mut minutes = minutes.filter(|m| (1..=240).contains(m)).unwrap_or(cfg.focus_minutes);
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
    changed(app);
    Ok((session, note))
}

/// Starts a break. `long` None picks short or long by where you are in the cycle.
pub fn start_break(app: &AppHandle, long: Option<bool>, minutes: Option<u32>) -> anyhow::Result<db::FocusSession> {
    let cfg = config(app);
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let current = db::active_session(&conn)?;
    let task_id = current.as_ref().and_then(|s| s.task_id);
    if let Some(prev) = current {
        // Ending a focus block early to take a break: it counts as stopped, not done.
        db::end_session(&conn, prev.id, false)?;
    }
    desktop::dnd_off(&conn);
    let kind = match long {
        Some(true) => "long_break",
        Some(false) => "short_break",
        None => next_break(&cfg, db::focus_since_long_break(&conn)?),
    };
    let default = if kind == "long_break" { cfg.long_break_minutes } else { cfg.short_break_minutes };
    let minutes = minutes.filter(|m| (1..=120).contains(m)).unwrap_or(default);
    let session = db::start_session(&conn, kind, minutes, task_id)?;
    drop(conn);
    changed(app);
    Ok(session)
}

pub fn pause(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    if let Some(s) = db::active_session(&conn)? {
        db::pause_session(&conn, s.id)?;
    }
    drop(conn);
    changed(app);
    Ok(())
}

pub fn resume(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    if let Some(s) = db::active_session(&conn)? {
        db::resume_session(&conn, s.id)?;
    }
    drop(conn);
    changed(app);
    Ok(())
}

/// Adds time to the running focus block or break.
pub fn extend(app: &AppHandle, minutes: i64) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    if let Some(s) = db::active_session(&conn)? {
        db::extend_session(&conn, s.id, minutes.clamp(1, 60))?;
    }
    drop(conn);
    changed(app);
    Ok(())
}

/// Ends the current phase now and moves on: focus -> break, break -> idle (or focus).
pub fn skip(app: &AppHandle) -> anyhow::Result<()> {
    let current = db::active_session(&app.state::<AppState>().db.lock().unwrap())?;
    match current {
        Some(s) if s.is_focus() => {
            start_break(app, None, None)?;
        }
        Some(s) => {
            db::end_session(&app.state::<AppState>().db.lock().unwrap(), s.id, false)?;
            if config(app).auto_start_focus {
                start(app, None, s.task_id)?;
            }
            changed(app);
        }
        None => {}
    }
    Ok(())
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
                if s.is_focus() {
                    notifications::spawn_digest(app.clone(), s.started_at);
                }
                true
            }
            None => false,
        }
    };
    changed(app);
    Ok(stopped)
}

/// Called by the scheduler: finishes an expired session and moves through the cycle.
pub fn tick(app: &AppHandle) -> anyhow::Result<()> {
    let cfg = config(app);
    let state = app.state::<AppState>();
    let name = state.persona.name.clone();
    let (finished, sounds) = {
        let conn = state.db.lock().unwrap();
        let sounds = desktop::sounds_on(&conn);
        let finished = match db::active_session(&conn)? {
            // A paused timer never runs out.
            Some(s) if s.paused_at.is_none() && s.ends_at <= db::now_ts() => {
                db::end_session(&conn, s.id, true)?;
                if s.is_focus() {
                    desktop::dnd_off(&conn);
                    notifications::spawn_digest(app.clone(), s.started_at.clone());
                }
                Some(s)
            }
            _ => None,
        };
        (finished, sounds)
    };
    let Some(done) = finished else { return Ok(()) };

    if sounds {
        desktop::chime(if done.is_focus() { desktop::Chime::FocusDone } else { desktop::Chime::BreakDone });
    }
    if done.is_focus() {
        let (cycle, today) = {
            let conn = state.db.lock().unwrap();
            (db::focus_since_long_break(&conn)?, db::focus_blocks_today(&conn)?)
        };
        let long = next_break(&cfg, cycle) == "long_break";
        let mins = if long { cfg.long_break_minutes } else { cfg.short_break_minutes };
        let progress = format!("{today}/{} today", cfg.daily_goal);
        if cfg.auto_start_breaks {
            start_break(app, Some(long), None)?;
            let what = if long {
                format!("You earned a long break - {mins} minutes.")
            } else {
                format!("Take {mins} minutes, stretch, get some water.")
            };
            desktop::notify(&name, &format!("Pomodoro done ({progress}). {what}"), false);
        } else {
            desktop::notify(&name, &format!("Pomodoro done ({progress}). Start a {mins}-minute break when you're ready."), false);
        }
    } else if cfg.auto_start_focus {
        let note = match start(app, None, done.task_id) {
            Ok(_) => "Next pomodoro started.".to_string(),
            Err(e) => e.to_string(),
        };
        desktop::notify(&name, &format!("Break's over. {note}"), false);
    } else {
        desktop::notify(&name, "Break's over. Ready for another round when you are.", false);
    }
    changed(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_break_every_n() {
        let cfg = Config {
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            long_break_every: 4,
            daily_goal: 8,
            auto_start_breaks: true,
            auto_start_focus: false,
        };
        let kinds: Vec<&str> = (1..=8).map(|n| next_break(&cfg, n)).collect();
        assert_eq!(kinds, ["short_break", "short_break", "short_break", "long_break", "short_break", "short_break", "short_break", "long_break"]);
    }
}
