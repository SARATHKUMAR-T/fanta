mod approvals;
mod calendar;
mod db;
mod desktop;
mod focus;
mod llm;
mod notifications;
mod persona;
mod scheduler;
mod tools;

use llm::{Message, Ollama};
use persona::Persona;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

const HISTORY_LIMIT: usize = 20;
const MAX_TOOL_ROUNDS: usize = 5;

pub struct AppState {
    pub persona: Persona,
    pub db: Mutex<rusqlite::Connection>,
    pub ollama: Ollama,
    pub tools: tools::Registry,
    pub calendar: calendar::Calendar,
    pub approvals: approvals::Approvals,
}

#[tauri::command]
fn get_persona(state: State<'_, AppState>) -> Persona {
    state.persona.clone()
}

#[tauri::command]
fn get_history(state: State<'_, AppState>) -> Result<Vec<db::StoredMessage>, String> {
    db::recent_messages(&state.db.lock().unwrap(), 100).map_err(|e| e.to_string())
}

/// Runs the agent loop for one user message, streaming tokens as `chat://token`.
#[tauri::command]
async fn send_message(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<String, String> {
    let mut messages = vec![];
    // Takes the db lock itself, so it must run before we hold it below.
    let meetings = calendar::today_summary(&app);
    {
        let conn = state.db.lock().unwrap();
        // Open tasks go last in the system prompt so the stable prefix stays cacheable.
        let open: Vec<String> = db::list_tasks(&conn, false)
            .map_err(|e| e.to_string())?
            .iter()
            .take(30)
            .map(|t| format!("#{} {}", t.id, t.title))
            .collect();
        let tasks = if open.is_empty() { "(none)".to_string() } else { open.join("\n") };
        messages.push(Message::new(
            "system",
            format!("{}\nOpen tasks:\n{tasks}\n{meetings}", state.persona.full_system_prompt()),
        ));
        let history = db::recent_messages(&conn, HISTORY_LIMIT).map_err(|e| e.to_string())?;
        messages.extend(history.into_iter().map(|m| Message::new(&m.role, m.content)));
        db::insert_message(&conn, "user", &text).map_err(|e| e.to_string())?;
    }
    messages.push(Message::new("user", text));

    let schemas = state.tools.schemas();
    for _ in 0..MAX_TOOL_ROUNDS {
        let reply = state
            .ollama
            .chat(&state.persona.model, &messages, &schemas, |tok| {
                let _ = app.emit("chat://token", tok);
            })
            .await
            .map_err(|e| e.to_string())?;

        if reply.tool_calls.is_empty() {
            let content = reply.content.trim().to_string();
            db::insert_message(&state.db.lock().unwrap(), "assistant", &content).map_err(|e| e.to_string())?;
            return Ok(content);
        }

        let calls = reply.tool_calls.clone();
        messages.push(reply);
        for call in calls {
            let (name, args) = (&call.function.name, &call.function.arguments);
            let result = match state.tools.approval(&app, name, args) {
                Some(Err(e)) => format!("error: {e}"),
                Some(Ok(description)) => {
                    let _ = app.emit("chat://tool", "waiting for your approval");
                    if approvals::ask(&app, name, &description).await {
                        let _ = app.emit("chat://tool", name);
                        state.tools.call(&app, name, args).await
                    } else {
                        "The user declined this action. Acknowledge briefly; don't retry unless they ask.".into()
                    }
                }
                None => {
                    let _ = app.emit("chat://tool", name);
                    state.tools.call(&app, name, args).await
                }
            };
            let mut msg = Message::new("tool", result);
            msg.tool_name = Some(call.function.name);
            messages.push(msg);
        }
    }
    Err("Too many tool rounds; stopping.".into())
}

fn err(e: anyhow::Error) -> String {
    e.to_string()
}

#[tauri::command]
fn list_tasks(state: State<'_, AppState>, include_done: bool) -> Result<Vec<db::Task>, String> {
    db::list_tasks(&state.db.lock().unwrap(), include_done).map_err(err)
}

#[tauri::command]
fn add_task(app: AppHandle, state: State<'_, AppState>, title: String) -> Result<db::Task, String> {
    let task = db::add_task(&state.db.lock().unwrap(), title.trim(), None, None).map_err(err)?;
    let _ = app.emit("tasks://changed", ());
    Ok(task)
}

#[tauri::command]
fn set_task_done(app: AppHandle, state: State<'_, AppState>, id: i64, done: bool) -> Result<(), String> {
    db::set_task_done(&state.db.lock().unwrap(), id, done).map_err(err)?;
    let _ = app.emit("tasks://changed", ());
    Ok(())
}

#[tauri::command]
fn delete_task(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::delete_task(&state.db.lock().unwrap(), id).map_err(err)?;
    let _ = app.emit("tasks://changed", ());
    Ok(())
}

#[tauri::command]
fn focus_status(app: AppHandle) -> Result<focus::FocusStatus, String> {
    focus::status(&app).map_err(err)
}

#[tauri::command]
/// Returns a note if the block was shortened for a meeting.
fn start_focus(app: AppHandle, minutes: Option<u32>, task_id: Option<i64>) -> Result<Option<String>, String> {
    focus::start(&app, minutes, task_id).map(|(_, note)| note).map_err(err)
}

#[tauri::command]
fn stop_focus(app: AppHandle) -> Result<(), String> {
    focus::stop(&app).map(|_| ()).map_err(err)
}

#[tauri::command]
fn list_notifications(state: State<'_, AppState>, category: Option<String>) -> Result<Vec<db::Notification>, String> {
    db::list_notifications(&state.db.lock().unwrap(), category.as_deref(), None, 200).map_err(err)
}

#[tauri::command]
fn inbox_unseen(state: State<'_, AppState>) -> Result<i64, String> {
    db::unseen_notification_count(&state.db.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn mark_inbox_seen(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    db::mark_notifications_seen(&state.db.lock().unwrap()).map_err(err)?;
    let _ = app.emit("inbox://changed", ());
    Ok(())
}

/// Lets the user correct a triage decision. Kept for learning preferences later.
#[tauri::command]
fn set_notification_category(app: AppHandle, state: State<'_, AppState>, id: i64, category: String) -> Result<(), String> {
    if !["urgent", "fyi", "ignore"].contains(&category.as_str()) {
        return Err(format!("unknown category '{category}'"));
    }
    db::set_triage(&state.db.lock().unwrap(), id, &category, None, Some("set by you")).map_err(err)?;
    let _ = app.emit("inbox://changed", ());
    Ok(())
}

#[tauri::command]
fn answer_approval(app: AppHandle, id: u64, approved: bool) {
    approvals::answer(&app, id, approved);
}

#[tauri::command]
fn calendar_status(app: AppHandle) -> calendar::Status {
    calendar::status(&app)
}

#[tauri::command]
async fn calendar_sign_in(app: AppHandle, client_id: String, tenant: String) -> Result<calendar::graph::DeviceCode, String> {
    if client_id.trim().is_empty() {
        return Err("Enter the Application (client) ID first.".into());
    }
    calendar::begin_graph_sign_in(&app, &client_id, &tenant).await.map_err(err)
}

#[tauri::command]
async fn calendar_connect_ics(app: AppHandle, url: String) -> Result<usize, String> {
    calendar::connect_ics(&app, &url).await.map_err(err)
}

#[tauri::command]
fn calendar_disconnect(app: AppHandle) -> Result<(), String> {
    calendar::disconnect(&app).map_err(err)
}

#[tauri::command]
fn calendar_sync(app: AppHandle) {
    calendar::request_sync(&app);
}

/// Events overlapping the local-time range [from, to).
#[tauri::command]
fn calendar_events(state: State<'_, AppState>, from: String, to: String) -> Result<Vec<db::CalEvent>, String> {
    db::events_between(&state.db.lock().unwrap(), &from, &to).map_err(err)
}

#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links can be opened".into());
    }
    desktop::open_url(&url);
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let persona = persona::load().expect("failed to load persona.yaml");
    let state = AppState {
        ollama: Ollama::new(&persona.ollama_url),
        db: Mutex::new(db::open().expect("failed to open database")),
        tools: tools::Registry::with_builtins(),
        calendar: calendar::Calendar::default(),
        approvals: approvals::Approvals::default(),
        persona,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            // If Fanta quit mid-focus, don't leave the user stuck in Do Not Disturb.
            {
                let state = app.state::<AppState>();
                let conn = state.db.lock().unwrap();
                if db::active_session(&conn)?.is_none() {
                    desktop::dnd_off(&conn);
                }
                db::prune_notifications(&conn, state.persona.notifications.keep_days)?;
            }
            scheduler::spawn(app.handle().clone());
            notifications::spawn(app.handle().clone());
            calendar::spawn(app.handle().clone());

            let show = MenuItem::with_id(app, "show", "Open Fanta", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Fanta")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => {
                        desktop::dnd_off(&app.state::<AppState>().db.lock().unwrap());
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        // Closing the window hides Fanta to the tray instead of quitting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_persona,
            get_history,
            send_message,
            list_tasks,
            add_task,
            set_task_done,
            delete_task,
            focus_status,
            start_focus,
            stop_focus,
            list_notifications,
            inbox_unseen,
            mark_inbox_seen,
            set_notification_category,
            answer_approval,
            calendar_status,
            calendar_sign_in,
            calendar_connect_ics,
            calendar_disconnect,
            calendar_sync,
            calendar_events,
            open_link
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
