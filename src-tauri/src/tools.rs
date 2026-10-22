//! Tool registry. Each tool is a JSON-schema description plus a handler.
//! Phase 3 bridges MCP plugins in alongside these built-ins.

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::calendar::{self, graph::NewMeeting};
use crate::{db, focus, AppState};

type SyncHandler = fn(&AppHandle, &Value) -> anyhow::Result<String>;
type AsyncHandler = fn(AppHandle, Value) -> BoxFuture<'static, anyhow::Result<String>>;
/// Validates the arguments and describes the action for the approval card.
type Approval = fn(&AppHandle, &Value) -> anyhow::Result<String>;

pub enum Handler {
    Sync(SyncHandler),
    Async(AsyncHandler),
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub handler: Handler,
    /// Set for actions with side effects outside Fanta; the user must approve each call.
    pub approval: Option<Approval>,
}

pub struct Registry {
    tools: Vec<Tool>,
}

fn tool(name: &'static str, description: &'static str, parameters: Value, handler: SyncHandler) -> Tool {
    Tool { name, description, parameters, handler: Handler::Sync(handler), approval: None }
}

fn async_tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
    handler: AsyncHandler,
    approval: Option<Approval>,
) -> Tool {
    Tool { name, description, parameters, handler: Handler::Async(handler), approval }
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
                            "due": { "type": "string", "description": "Optional deadline, local 'YYYY-MM-DD HH:MM'. Include the date unless it's today." }
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
                            "at": { "type": "string", "description": "Local 'YYYY-MM-DD HH:MM' (24h). Include the date unless it's today." }
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
                    "get_schedule",
                    "Get the user's Outlook calendar events for a day (or several days).",
                    json!({
                        "type": "object",
                        "properties": {
                            "date": { "type": "string", "description": "'today', 'tomorrow', a weekday name, or YYYY-MM-DD. Default today." },
                            "days": { "type": "integer", "description": "Number of days to include. Default 1." }
                        }
                    }),
                    get_schedule,
                ),
                tool(
                    "find_free_time",
                    "Find free slots in the user's workday on a date, between meetings.",
                    json!({
                        "type": "object",
                        "properties": {
                            "date": { "type": "string", "description": "'today', 'tomorrow', a weekday name, or YYYY-MM-DD. Default today." },
                            "min_minutes": { "type": "integer", "description": "Shortest useful slot. Default 30." }
                        }
                    }),
                    find_free_time,
                ),
                async_tool(
                    "create_event",
                    "Create an Outlook calendar event, optionally inviting people and adding a Teams link. The user is asked to approve.",
                    json!({
                        "type": "object",
                        "properties": {
                            "subject": { "type": "string" },
                            "start": { "type": "string", "description": "Full local date and time 'YYYY-MM-DD HH:MM' (24h). Work out the date from the current time - 'tomorrow 3pm' needs tomorrow's date." },
                            "duration_minutes": { "type": "integer", "description": "Default 30" },
                            "attendees": { "type": "array", "items": { "type": "string" }, "description": "Email addresses" },
                            "online_meeting": { "type": "boolean", "description": "Add a Teams meeting link" }
                        },
                        "required": ["subject", "start"]
                    }),
                    |app, args| Box::pin(async move { create_event(&app, &args).await }),
                    Some(describe_create_event),
                ),
                async_tool(
                    "respond_to_event",
                    "Accept, decline, or tentatively accept a meeting invite. The user is asked to approve.",
                    json!({
                        "type": "object",
                        "properties": {
                            "id": { "type": "integer", "description": "Event number from get_schedule" },
                            "subject": { "type": "string", "description": "Part of the event title, if the id is unknown" },
                            "response": { "type": "string", "enum": ["accept", "decline", "tentative"] },
                            "comment": { "type": "string", "description": "Optional note to the organizer" }
                        },
                        "required": ["response"]
                    }),
                    |app, args| Box::pin(async move { respond_to_event(&app, &args).await }),
                    Some(describe_response),
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

    /// `None` if the tool runs without asking; otherwise the approval text, or
    /// an error if the arguments are invalid before even asking.
    pub fn approval(&self, app: &AppHandle, name: &str, args: &Value) -> Option<anyhow::Result<String>> {
        let tool = self.tools.iter().find(|t| t.name == name)?;
        tool.approval.map(|describe| describe(app, args))
    }

    pub async fn call(&self, app: &AppHandle, name: &str, args: &Value) -> String {
        let result = match self.tools.iter().find(|t| t.name == name).map(|t| &t.handler) {
            Some(Handler::Sync(f)) => f(app, args),
            Some(Handler::Async(f)) => f(app.clone(), args.clone()).await,
            None => return format!("error: unknown tool '{name}'"),
        };
        result.unwrap_or_else(|e| format!("error: {e}"))
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
    let needle = arg_str(args, "title").map(str::to_lowercase);
    if let Some(id) = arg_i64(args, "id") {
        // Small models sometimes invent ids; when a title is also given, it must agree.
        match (db::get_task(&conn, id)?, &needle) {
            (Some(t), None) => return Ok(t),
            (Some(t), Some(n)) if t.title.to_lowercase().contains(n.as_str()) => return Ok(t),
            (None, None) => anyhow::bail!("no task with id {id}"),
            _ => {}
        }
    }
    let needle = needle.ok_or_else(|| anyhow::anyhow!("give an id or title"))?;
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
    let (s, note) = focus::start(app, minutes, task.as_ref().map(|t| t.id))?;
    let on = task.map(|t| format!(" on '{}'", t.title)).unwrap_or_default();
    let note = note.map(|n| format!(" {n}")).unwrap_or_default();
    Ok(format!("Focus block started{on}, ends at {}. Do Not Disturb is on.{note}", friendly(&s.ends_at)))
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
        .map(|n| format!("[{}] {}", n.category, crate::notifications::describe(n)))
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

// ---- calendar ----

fn require_calendar(app: &AppHandle) -> anyhow::Result<()> {
    if !calendar::connected(app) {
        anyhow::bail!("no calendar connected - the user can connect Outlook from the ⚙ Connections panel");
    }
    Ok(())
}

/// 'today', 'tomorrow', a weekday name (next occurrence, today included), or YYYY-MM-DD.
fn parse_date(s: Option<&str>) -> anyhow::Result<NaiveDate> {
    let today = Local::now().date_naive();
    let Some(s) = s.map(|s| s.trim().to_lowercase()) else { return Ok(today) };
    match s.as_str() {
        "" | "today" => return Ok(today),
        "tomorrow" => return Ok(today + Duration::days(1)),
        _ => {}
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Ok(d);
    }
    let (next, day) = match s.strip_prefix("next ") {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix("this ").unwrap_or(&s)),
    };
    let weekdays = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    if let Some(i) = weekdays.iter().position(|w| day.starts_with(&w[..3])) {
        let mut ahead = (i as i64 - today.weekday().num_days_from_monday() as i64).rem_euclid(7);
        if next && ahead == 0 {
            ahead = 7;
        }
        return Ok(today + Duration::days(ahead));
    }
    anyhow::bail!("couldn't understand date '{s}'")
}

fn day_label(d: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if d == today {
        "Today".into()
    } else if d == today + Duration::days(1) {
        "Tomorrow".into()
    } else {
        d.format("%A %d %b").to_string()
    }
}

fn get_schedule(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    require_calendar(app)?;
    let first = parse_date(arg_str(args, "date"))?;
    let days = arg_i64(args, "days").unwrap_or(1).clamp(1, 14);
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let mut out = vec![];
    for i in 0..days {
        let d = first + Duration::days(i);
        let events = db::events_between(&conn, &format!("{d} 00:00:00"), &format!("{} 00:00:00", d + Duration::days(1)))?;
        let lines = if events.is_empty() {
            "  (no meetings)".to_string()
        } else {
            events.iter().map(|e| format!("  {}", calendar::describe(e))).collect::<Vec<_>>().join("\n")
        };
        out.push(format!("{}:\n{lines}", day_label(d)));
    }
    Ok(out.join("\n"))
}

fn find_free_time(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    require_calendar(app)?;
    let d = parse_date(arg_str(args, "date"))?;
    let min = arg_i64(args, "min_minutes").filter(|m| *m > 0).unwrap_or(30);
    let state = app.state::<AppState>();
    let wd = &state.persona.workday;
    let mut cursor = format!("{d} {}:00", wd.start);
    let day_end = format!("{d} {}:00", wd.end);
    let now = db::now_ts();
    if cursor < now {
        cursor = now;
    }
    if cursor >= day_end {
        return Ok(format!("{}'s workday ({}–{}) is already over.", day_label(d), wd.start, wd.end));
    }
    let busy = db::busy_between(&state.db.lock().unwrap(), &cursor, &day_end)?;
    let mut slots = vec![];
    let mut push = |from: &str, to: &str| {
        let len = NaiveDateTime::parse_from_str(to, db::TS)
            .ok()
            .zip(NaiveDateTime::parse_from_str(from, db::TS).ok())
            .map(|(t, f)| (t - f).num_minutes())
            .unwrap_or(0);
        if len >= min {
            slots.push(format!("{}–{} ({len} min)", calendar::hm(from), calendar::hm(to)));
        }
    };
    for e in &busy {
        if e.start_at > cursor {
            push(&cursor, &e.start_at);
        }
        if e.end_at > cursor {
            cursor = e.end_at.clone();
        }
    }
    if cursor < day_end {
        push(&cursor, &day_end);
    }
    Ok(if slots.is_empty() {
        format!("No free slots of {min}+ minutes {} within {}–{}.", day_label(d).to_lowercase(), wd.start, wd.end)
    } else {
        format!("Free {}: {}", day_label(d).to_lowercase(), slots.join(", "))
    })
}

fn meeting_from_args(args: &Value) -> anyhow::Result<NewMeeting> {
    let subject = arg_str(args, "subject").ok_or_else(|| anyhow::anyhow!("subject is required"))?;
    let start_raw = arg_str(args, "start").ok_or_else(|| anyhow::anyhow!("start is required"))?;
    let start_at = parse_when(start_raw).ok_or_else(|| anyhow::anyhow!("couldn't understand start '{start_raw}'"))?;
    let minutes = arg_i64(args, "duration_minutes").filter(|m| (5..=600).contains(m)).unwrap_or(30);
    let end_at = (NaiveDateTime::parse_from_str(&start_at, db::TS)? + Duration::minutes(minutes)).format(db::TS).to_string();
    let attendees = args
        .get("attendees")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(bad) = attendees.iter().find(|a| !a.contains('@')) {
        anyhow::bail!("'{bad}' isn't an email address - ask the user for it");
    }
    Ok(NewMeeting {
        subject: subject.to_string(),
        start_at,
        end_at,
        attendees,
        online: args.get("online_meeting").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn describe_create_event(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    if !calendar::status(app).can_write {
        anyhow::bail!("creating events needs Outlook sign-in (not connected, or connected read-only via ICS)");
    }
    let m = meeting_from_args(args)?;
    let mut text = format!("Create “{}” · {} {}–{}", m.subject, friendly(&m.start_at), calendar::hm(&m.start_at), calendar::hm(&m.end_at));
    if !m.attendees.is_empty() {
        text += &format!(" · invite {}", m.attendees.join(", "));
    }
    if m.online {
        text += " · Teams link";
    }
    Ok(text)
}

async fn create_event(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let e = calendar::create_event(app, &meeting_from_args(args)?).await?;
    Ok(format!("Created: {}", calendar::describe(&e)))
}

fn resolve_event(app: &AppHandle, args: &Value) -> anyhow::Result<db::CalEvent> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let needle = arg_str(args, "subject").map(str::to_lowercase);
    if let Some(num) = arg_i64(args, "id") {
        // Small models sometimes invent ids; when a subject is also given, it must agree.
        match (db::get_event(&conn, num)?, &needle) {
            (Some(e), None) => return Ok(e),
            (Some(e), Some(n)) if e.subject.to_lowercase().contains(n.as_str()) => return Ok(e),
            (None, None) => anyhow::bail!("no event #{num}"),
            _ => {}
        }
    }
    let needle = needle.ok_or_else(|| anyhow::anyhow!("give an event id or subject"))?;
    let upcoming = db::events_between(&conn, &db::now_ts(), "9999")?;
    let matches: Vec<_> = upcoming.into_iter().filter(|e| e.subject.to_lowercase().contains(&needle)).collect();
    match matches.len() {
        0 => anyhow::bail!("no upcoming event matching '{needle}'"),
        1 => Ok(matches.into_iter().next().unwrap()),
        // Recurring meetings match many times; the soonest is almost always the one meant.
        _ if matches.iter().all(|e| e.subject == matches[0].subject) => Ok(matches.into_iter().next().unwrap()),
        _ => anyhow::bail!(
            "several events match: {}",
            matches.iter().take(6).map(|e| format!("#{} {} {}", e.num, friendly(&e.start_at), e.subject)).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// (Graph action, verb for the approval card, past tense for the result)
fn graph_response(args: &Value) -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    match arg_str(args, "response").map(str::to_lowercase).as_deref() {
        Some("accept") => Ok(("accept", "Accept", "Accepted")),
        Some("decline") => Ok(("decline", "Decline", "Declined")),
        Some("tentative") | Some("tentativelyaccept") => Ok(("tentativelyAccept", "Tentatively accept", "Tentatively accepted")),
        _ => anyhow::bail!("response must be accept, decline, or tentative"),
    }
}

fn describe_response(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    if !calendar::status(app).can_write {
        anyhow::bail!("responding to invites needs Outlook sign-in (not connected, or connected read-only via ICS)");
    }
    let (_, verb, _) = graph_response(args)?;
    let e = resolve_event(app, args)?;
    let mut text = format!("{verb} “{}” · {} {}", e.subject, friendly(&e.start_at), calendar::hm(&e.start_at));
    if let Some(org) = &e.organizer {
        text += &format!(" · organizer {org}");
    }
    if let Some(c) = arg_str(args, "comment") {
        text += &format!(" · note: “{c}”");
    }
    Ok(text)
}

async fn respond_to_event(app: &AppHandle, args: &Value) -> anyhow::Result<String> {
    let (response, _, done) = graph_response(args)?;
    let e = resolve_event(app, args)?;
    calendar::respond(app, &e, response, arg_str(args, "comment").unwrap_or("")).await?;
    Ok(format!("{done} “{}” ({}).", e.subject, friendly(&e.start_at)))
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
    fn dates() {
        let today = Local::now().date_naive();
        assert_eq!(parse_date(None).unwrap(), today);
        assert_eq!(parse_date(Some("tomorrow")).unwrap(), today + Duration::days(1));
        assert_eq!(parse_date(Some("2026-12-01")).unwrap(), NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        let fri = parse_date(Some("Friday")).unwrap();
        assert_eq!(fri.weekday(), chrono::Weekday::Fri);
        assert!((0..7).contains(&(fri - today).num_days()));
        let next_mon = parse_date(Some("next monday")).unwrap();
        assert_eq!(next_mon.weekday(), chrono::Weekday::Mon);
        assert!((1..=7).contains(&(next_mon - today).num_days()));
    }

    #[test]
    fn loose_integer_args() {
        assert_eq!(arg_i64(&json!({"id": "7"}), "id"), Some(7));
        assert_eq!(arg_i64(&json!({"id": 7.0}), "id"), Some(7));
    }
}
