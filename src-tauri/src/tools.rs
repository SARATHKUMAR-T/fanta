//! Tool registry. Each tool is a JSON-schema description plus a handler.
//! Phase 3 bridges MCP plugins in alongside these built-ins.

use chrono::{Duration, Local, NaiveDateTime, NaiveTime};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::{db, focus, AppState};

type Handler = fn(&AppHandle, &Value) -> anyhow::Result<String>;

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub handler: Handler,
}

pub struct Registry {
    tools: Vec<Tool>,
}

fn tool(name: &'static str, description: &'static str, parameters: Value, handler: Handler) -> Tool {
    Tool { name, description, parameters, handler }
}

impl Registry {
    pub fn with_builtins() -> Self {
        let task_ref = json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "Task id from list_tasks" },
                "title": { "type": "string", "description": "Part of the task title, if the id is unknown" }
            }
        });
        Self {
            tools: vec![
                tool(
                    "get_current_time",
                    "Get the current local date and time.",
                    json!({ "type": "object", "properties": {} }),
                    |_, _| Ok(Local::now().format("%A %Y-%m-%d %H:%M:%S").to_string()),
                ),
                tool(
                    "add_task",
                    "Add a task to the user's to-do list.",
                    json!({
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "due": { "type": "string", "description": "Optional deadline, 'YYYY-MM-DD HH:MM' or 'HH:MM'" }
                        },
                        "required": ["title"]
                    }),
                    add_task,
                ),
                tool(
                    "set_reminder",
                    "Remind the user about something at a time. Use in_minutes for relative times ('in 20 minutes'), or at for clock times.",
                    json!({
                        "type": "object",
                        "properties": {
                            "text": { "type": "string", "description": "What to remind them about" },
                            "in_minutes": { "type": "integer" },
                            "at": { "type": "string", "description": "'YYYY-MM-DD HH:MM' or 'HH:MM' (24h, local time)" }
                        },
                        "required": ["text"]
                    }),
                    set_reminder,
                ),
                tool(
                    "list_tasks",
                    "List the user's tasks with ids, deadlines, and reminders.",
                    json!({
                        "type": "object",
                        "properties": { "include_done": { "type": "boolean" } }
                    }),
                    list_tasks,
                ),
                tool("complete_task", "Mark a task as done.", task_ref.clone(), complete_task),
                tool("delete_task", "Delete a task permanently.", task_ref, delete_task),
                tool(
                    "start_focus",
                    "Start a focus (pomodoro) block. Turns on Do Not Disturb until it ends.",
                    json!({
                        "type": "object",
                        "properties": {
                            "minutes": { "type": "integer", "description": "Defaults to the user's pomodoro length" },
                            "task": { "type": "string", "description": "Optional title of the task to focus on" }
                        }
                    }),
                    start_focus,
                ),
                tool(
                    "stop_focus",
                    "Stop the current focus block or break.",
                    json!({ "type": "object", "properties": {} }),
                    |app, _| Ok(if focus::stop(app)? { "Stopped." } else { "Nothing was running." }.into()),
                ),
                tool(
                    "list_notifications",
                    "List desktop notifications Fanta captured (Teams, Slack, mail...), newest first, with Fanta's triage. Use for 'what did I miss?'.",
                    json!({
                        "type": "object",
                        "properties": {
                            "category": { "type": "string", "enum": ["urgent", "fyi", "ignore", "all"], "description": "Default: urgent and fyi" },
                            "since_minutes": { "type": "integer", "description": "How far back to look. Default 240." }
                        }
                    }),
                    list_notifications,
                ),
                tool(
                    "focus_status",
                    "Check whether a focus block or break is running, and how many blocks were done today.",
                    json!({ "type": "object", "properties": {} }),
                    focus_status,
                ),
            ],
        }
    }

    /// Tool definitions in Ollama/OpenAI function-calling format.
    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect()
    }

    pub fn call(&self, app: &AppHandle, name: &str, args: &Value) -> String {
        match self.tools.iter().find(|t| t.name == name) {
            Some(t) => (t.handler)(app, args).unwrap_or_else(|e| format!("error: {e}")),
            None => format!("error: unknown tool '{name}'"),
        }
    }
}

// ---- argument helpers (small models are loose with types) ----

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    match args.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Parses 'YYYY-MM-DD HH:MM', ISO 'YYYY-MM-DDTHH:MM[:SS]', or 'HH:MM' (next occurrence).
fn parse_when(s: &str) -> Option<String> {
    let s = s.trim().replace('T', " ");
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Some(dt.format(db::TS).to_string());
        }
    }
    let t = NaiveTime::parse_from_str(&s, "%H:%M").ok()?;
    let now = Local::now().naive_local();
    let mut dt = now.date().and_time(t);
    if dt <= now {
        dt += Duration::days(1);
    }
    Some(dt.format(db::TS).to_string())
}

fn friendly(ts: &str) -> String {
    NaiveDateTime::parse_from_str(ts, db::TS)
        .map(|dt| {
            if dt.date() == Local::now().date_naive() {
                dt.format("today %H:%M").to_string()
            } else {
                dt.format("%a %d %b %H:%M").to_string()
            }
        })
        .unwrap_or_else(|_| ts.to_string())
}

/// Finds a task by id, or by a case-insensitive title fragment among open tasks.
fn resolve_task(app: &AppHandle, args: &Value) -> anyhow::Result<db::Task> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    if let Some(id) = arg_i64(args, "id") {
        return db::get_task(&conn, id)?.ok_or_else(|| anyhow::anyhow!("no task with id {id}"));
    }
    let needle = arg_str(args, "title").ok_or_else(|| anyhow::anyhow!("give an id or title"))?.to_lowercase();
    let matches: Vec<_> = db::list_tasks(&conn, false)?
        .into_iter()
        .filter(|t| t.title.to_lowercase().contains(&needle))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => anyhow::bail!("no open task matching '{needle}'"),
        _ => anyhow::bail!(
            "several tasks match: {}",
            matches.iter().map(|t| format!("#{} {}", t.id, t.title)).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn tasks_changed(app: &AppHandle) {
    let _ = app.emit("tasks://changed", ());
}

// ---- handlers ----

fn add_task(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let title = arg_str(args, "title").ok_or_else(|| anyhow::anyhow!("title is required"))?;
    let due = arg_str(args, "due").and_then(parse_when);
    let task = db::add_task(&app.state::<AppState>().db.lock().unwrap(), title, due.as_deref(), None)?;
    tasks_changed(app);
    Ok(match &task.due_at {
        Some(d) => format!("Added task #{} '{}', due {}.", task.id, task.title, friendly(d)),
        None => format!("Added task #{} '{}'.", task.id, task.title),
    })
}

fn set_reminder(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let text = arg_str(args, "text").ok_or_else(|| anyhow::anyhow!("text is required"))?;
    let when = match (arg_i64(args, "in_minutes"), arg_str(args, "at")) {
        (Some(m), _) if m > 0 => (Local::now() + Duration::minutes(m)).format(db::TS).to_string(),
        (_, Some(at)) => parse_when(at).ok_or_else(|| anyhow::anyhow!("couldn't understand time '{at}'"))?,
        _ => anyhow::bail!("give in_minutes or at"),
    };
    let task = db::add_task(&app.state::<AppState>().db.lock().unwrap(), text, None, Some(&when))?;
    tasks_changed(app);
    Ok(format!("Reminder set for {}: '{}' (task #{}).", friendly(&when), task.title, task.id))
}

fn list_tasks(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let include_done = args.get("include_done").and_then(Value::as_bool).unwrap_or(false);
    let tasks = db::list_tasks(&app.state::<AppState>().db.lock().unwrap(), include_done)?;
    if tasks.is_empty() {
        return Ok("No tasks.".into());
    }
    Ok(tasks
        .iter()
        .map(|t| {
            let mut line = format!("#{} [{}] {}", t.id, if t.done { "x" } else { " " }, t.title);
            if let Some(d) = &t.due_at {
                line += &format!(" (due {})", friendly(d));
            }
            if let Some(r) = &t.remind_at {
                line += &format!(" (reminder {})", friendly(r));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn complete_task(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let task = resolve_task(app, args)?;
    db::set_task_done(&app.state::<AppState>().db.lock().unwrap(), task.id, true)?;
    tasks_changed(app);
    Ok(format!("Marked #{} '{}' done.", task.id, task.title))
}

fn delete_task(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let task = resolve_task(app, args)?;
    db::delete_task(&app.state::<AppState>().db.lock().unwrap(), task.id)?;
    tasks_changed(app);
    Ok(format!("Deleted #{} '{}'.", task.id, task.title))
}

fn start_focus(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let task = match arg_str(args, "task") {
        Some(t) => resolve_task(app, &json!({ "title": t })).ok(),
        None => None,
    };
    let minutes = arg_i64(args, "minutes").map(|m| m as u32);
    let s = focus::start(app, minutes, task.as_ref().map(|t| t.id))?;
    let on = task.map(|t| format!(" on '{}'", t.title)).unwrap_or_default();
    Ok(format!("Focus block started{on}, ends at {}. Do Not Disturb is on.", friendly(&s.ends_at)))
}

fn list_notifications(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let minutes = arg_i64(args, "since_minutes").filter(|m| *m > 0).unwrap_or(240);
    let since = (Local::now() - Duration::minutes(minutes)).format(db::TS).to_string();
    let category = arg_str(args, "category").unwrap_or("default");
    let items = db::list_notifications(
        &app.state::<AppState>().db.lock().unwrap(),
        matches!(category, "urgent" | "fyi" | "ignore").then_some(category),
        Some(&since),
        40,
    )?;
    let items: Vec<_> = items
        .into_iter()
        .filter(|n| category != "default" || n.category == "urgent" || n.category == "fyi")
        .collect();
    if items.is_empty() {
        return Ok(format!("No matching notifications in the last {minutes} minutes."));
    }
    Ok(items
        .iter()
        .map(|n| {
            let what = n.summary.clone().unwrap_or_else(|| format!("{}: {}", n.title, n.body.chars().take(140).collect::<String>()));
            format!("[{}] {} {} · {}", n.category, friendly(&n.received_at), n.source, what)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn focus_status(app: &AppHandle, _: &Value) -> anyhow::Result<String> {
    let st = focus::status(app)?;
    Ok(match st.active {
        Some(s) => format!(
            "A {} is running until {}{}. Focus blocks done today: {}.",
            if s.kind == "focus" { "focus block" } else { "break" },
            friendly(&s.ends_at),
            st.task_title.map(|t| format!(" (task: {t})")).unwrap_or_default(),
            st.blocks_today
        ),
        None => format!("Nothing running. Focus blocks done today: {}.", st.blocks_today),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_and_iso_times() {
        assert_eq!(parse_when("2026-10-22 16:00").unwrap(), "2026-10-22 16:00:00");
        assert_eq!(parse_when("2026-10-22T16:30:15").unwrap(), "2026-10-22 16:30:15");
    }

    #[test]
    fn clock_time_is_in_the_future() {
        let ts = parse_when("00:00").unwrap();
        assert!(ts > db::now_ts());
    }

    #[test]
    fn loose_integer_args() {
        assert_eq!(arg_i64(&json!({"id": "7"}), "id"), Some(7));
        assert_eq!(arg_i64(&json!({"id": 7.0}), "id"), Some(7));
    }
}
