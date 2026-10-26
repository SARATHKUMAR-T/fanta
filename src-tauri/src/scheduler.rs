//! Background loop: fires reminders, advances pomodoro sessions, and lets Fanta
//! speak up on its own (morning brief, gentle nudges) during the workday.

use chrono::{Local, NaiveDateTime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::llm::Message;
use crate::{brain, calendar, db, desktop, focus, AppState};

const TICK: Duration = Duration::from_secs(15);
/// Nudge after this long without a focus block (and no more often than this).
const NUDGE_AFTER_MIN: i64 = 90;
/// Don't nudge someone who's away from the keyboard.
const MAX_IDLE_MS: u64 = 5 * 60 * 1000;

const LAST_BRIEF: &str = "last_brief_date";
const LAST_NUDGE: &str = "last_nudge_at";

static SPEAKING: AtomicBool = AtomicBool::new(false);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = fire_reminders(&app) {
                eprintln!("reminders: {e}");
            }
            if let Err(e) = focus::tick(&app) {
                eprintln!("focus: {e}");
            }
            if let Err(e) = calendar::heads_up(&app) {
                eprintln!("calendar heads-up: {e}");
            }
            if let Err(e) = proactive(&app).await {
                eprintln!("proactive: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Posts a message from Fanta into the chat and as a desktop notification.
/// `urgent` notifications show even during Do Not Disturb.
pub fn post(app: &AppHandle, content: &str, urgent: bool) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    db::insert_message(&state.db.lock().unwrap(), "assistant", content)?;
    let _ = app.emit("chat://proactive", content);
    desktop::notify(&state.persona.name, content, urgent);
    Ok(())
}

fn fire_reminders(app: &AppHandle) -> anyhow::Result<()> {
    let due = db::due_reminders(&app.state::<AppState>().db.lock().unwrap())?;
    for task in due {
        db::mark_reminded(&app.state::<AppState>().db.lock().unwrap(), task.id)?;
        // The user asked for this one, so it breaks through focus mode.
        post(app, &format!("⏰ Reminder: {}", task.title), true)?;
    }
    Ok(())
}

fn minutes_since(ts: &str) -> Option<i64> {
    let then = NaiveDateTime::parse_from_str(ts, db::TS).ok()?;
    Some((Local::now().naive_local() - then).num_minutes())
}

async fn proactive(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let now = Local::now();
    if !state.persona.proactive || !state.persona.in_workday(now) || SPEAKING.load(Ordering::SeqCst) {
        return Ok(());
    }
    let today = now.format("%Y-%m-%d").to_string();
    // These take the db lock, so gather them first.
    let meetings = calendar::today_summary(app);
    let meeting_soon = calendar::in_meeting_now(app).is_some() || calendar::next_meeting_within(app, 20).is_some();

    let instruction = {
        let conn = state.db.lock().unwrap();
        if db::get_setting(&conn, LAST_BRIEF)?.as_deref() != Some(&today) {
            db::set_setting(&conn, LAST_BRIEF, &today)?;
            db::set_setting(&conn, LAST_NUDGE, &db::now_ts())?;
            Some(
                "This is your first check-in with the user today. Give a short, warm brief: \
                  greet them in a way that fits the current time of day, mention their meetings \
                  (if any), the most important open tasks and any deadlines or reminders today, \
                  and suggest what to focus on first, fitting around meetings. 2-4 sentences.",
            )
        } else {
            let idle_ok = desktop::idle_ms().is_none_or(|ms| ms < MAX_IDLE_MS);
            let running = db::active_session(&conn)?.is_some();
            let has_tasks = !db::list_tasks(&conn, false)?.is_empty();
            let since_focus = db::last_focus_activity(&conn)?.and_then(|t| minutes_since(&t)).unwrap_or(i64::MAX);
            let since_nudge = db::get_setting(&conn, LAST_NUDGE)?.and_then(|t| minutes_since(&t)).unwrap_or(i64::MAX);
            if idle_ok
                && !running
                && !meeting_soon
                && has_tasks
                && since_focus >= NUDGE_AFTER_MIN
                && since_nudge >= NUDGE_AFTER_MIN
            {
                db::set_setting(&conn, LAST_NUDGE, &db::now_ts())?;
                Some(
                    "The user hasn't done a focus block in a while and has open tasks. \
                      Gently nudge them back on track: suggest one specific task and offer \
                      to start a focus block. One or two sentences, no guilt-tripping.",
                )
            } else {
                None
            }
        }
    };

    if let Some(instruction) = instruction {
        SPEAKING.store(true, Ordering::SeqCst);
        let result = speak(app, &meetings, instruction).await;
        SPEAKING.store(false, Ordering::SeqCst);
        post(app, &result?, false)?;
    }
    Ok(())
}

/// Asks the model for a short proactive message in Fanta's voice.
/// `extra` is added to the task context the model sees.
pub async fn speak(app: &AppHandle, extra: &str, instruction: &str) -> anyhow::Result<String> {
    let state = app.state::<AppState>();
    let context = {
        let conn = state.db.lock().unwrap();
        let tasks = db::list_tasks(&conn, false)?;
        let list = if tasks.is_empty() {
            "(none)".to_string()
        } else {
            tasks
                .iter()
                .take(15)
                .map(|t| {
                    let when = t.due_at.as_ref().map(|d| format!(" (due {d})")).unwrap_or_default();
                    format!("- {}{when}", t.title)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!("Open tasks:\n{list}\nFocus blocks done today: {}\n{extra}", db::focus_blocks_today(&conn)?)
    };
    let messages = vec![
        Message::new("system", state.persona.full_system_prompt()),
        Message::new("user", format!("[Internal trigger, not from the user]\n{context}\n\n{instruction}")),
    ];
    brain::background(app, &messages).await
}
