mod approvals;
mod apps;
mod brain;
mod calendar;
mod cloud;
mod db;
mod desktop;
mod focus;
mod llm;
mod notifications;
mod persona;
mod plugins;
mod scheduler;
mod stats;
mod system;
mod teams;
mod tools;
mod voice;
mod windows;

use llm::{Message, Ollama};
use persona::Persona;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};

const HISTORY_LIMIT: usize = 20;
/// Talk to Fanta from anywhere.
const VOICE_SHORTCUT: &str = "ctrl+alt+space";
const MAX_TOOL_ROUNDS: usize = 5;
/// Cloud models can drive a browser or app, which takes many steps.
const MAX_CLOUD_TOOL_ROUNDS: usize = 40;
/// Screenshots kept in a turn's context at once.
const MAX_IMAGES: usize = 3;

pub struct AppState {
    pub persona: Persona,
    pub db: Mutex<rusqlite::Connection>,
    pub ollama: Ollama,
    /// Shared client for cloud model APIs.
    pub http: reqwest::Client,
    pub tools: tools::Registry,
    pub calendar: calendar::Calendar,
    pub approvals: approvals::Approvals,
    pub plugins: plugins::Host,
    pub voice: voice::Voice,
    pub teams: teams::Teams,
    /// Set by the Stop button; the running chat turn checks it between steps.
    pub chat_cancel: std::sync::atomic::AtomicBool,
}

#[tauri::command]
fn get_persona(state: State<'_, AppState>) -> Persona {
    state.persona.clone()
}

#[tauri::command]
fn get_history(state: State<'_, AppState>) -> Result<Vec<db::StoredMessage>, String> {
    db::recent_messages(&state.db.lock().unwrap(), 100).map_err(|e| e.to_string())
}

/// What Fanta knows about right now beyond the chat: recent desktop messages
/// (exact text) and installed plugins. Goes at the end of the system prompt.
fn live_context(state: &AppState, conn: &rusqlite::Connection) -> String {
    let since = (chrono::Local::now() - chrono::Duration::hours(3)).format(db::TS).to_string();
    let items: Vec<db::Notification> = db::list_notifications(conn, None, Some(&since), 40)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.category != "ignore")
        .take(15)
        .collect();
    let mut recent: Vec<String> = items.iter().map(|n| format!("- {}", notifications::describe(n))).collect();
    recent.reverse();
    // Spelled out because small models don't reliably infer it from the list.
    let needs_reply: Vec<String> = items
        .iter()
        .filter(|n| {
            let t = n.text.as_deref().unwrap_or(&n.body).trim();
            t.starts_with("(mentioned you)") || t.ends_with('?') || n.category == "urgent"
        })
        .map(|n| format!("- {}", notifications::describe(n)))
        .collect();
    let mut out = String::new();
    if !recent.is_empty() {
        out += &format!(
            "Recent messages on the user's desktop (last 3 hours, exact text, oldest first):\n{}\n",
            recent.join("\n")
        );
    }
    if !needs_reply.is_empty() {
        out += &format!("Likely need a reply from the user (mentions, questions, urgent):\n{}\n", needs_reply.join("\n"));
    }
    out += if calendar::provider_conn(conn) == "graph" {
        "Teams sending: connected - use teams_reply / teams_send_message.\n"
    } else {
        "Teams sending: NOT connected. If the user asks you to send a Teams message, don't call the Teams tools; \
         tell them to connect Microsoft once in Settings → Microsoft 365 (then you can send for them), and offer to \
         draft the message so they can paste it.\n"
    };
    let plugins = state.plugins.summary();
    if !plugins.is_empty() {
        out += &format!("Installed plugins:\n{plugins}\n");
    }
    out
}

/// Runs the agent loop for one user message, streaming tokens as `chat://token`.
#[tauri::command]
async fn send_message(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<String, String> {
    let mut messages = vec![];
    let mut routing_context = String::new();
    // These take the db lock themselves, so they must run before we hold it below.
    let meetings = calendar::today_summary(&app);
    let cloud = brain::is_cloud(&app);
    {
        let conn = state.db.lock().unwrap();
        // Volatile context goes last in the system prompt so the stable prefix stays cacheable.
        let open: Vec<String> = db::list_tasks(&conn, false)
            .map_err(|e| e.to_string())?
            .iter()
            .take(30)
            .map(|t| format!("#{} {}", t.id, t.title))
            .collect();
        let tasks = if open.is_empty() { "(none)".to_string() } else { open.join("\n") };
        messages.push(Message::new(
            "system",
            format!(
                "{}\nOpen tasks:\n{tasks}\n{meetings}\n{}",
                state.persona.full_system_prompt(),
                live_context(&state, &conn)
            ),
        ));
        let history = db::recent_messages(&conn, HISTORY_LIMIT).map_err(|e| e.to_string())?;
        if let Some(prev) = history.iter().rev().find(|m| m.role == "user") {
            routing_context = prev.content.clone();
        }
        messages.extend(history.into_iter().map(|m| Message::new(&m.role, m.content)));
        db::insert_message(&conn, "user", &text).map_err(|e| e.to_string())?;
    }
    messages.push(Message::new("user", text.clone()));

    routing_context.push(' ');
    routing_context.push_str(&text);
    let mut schemas = state.tools.schemas();
    // Without Microsoft sign-in the Teams tools can only fail; small models call them anyway.
    if calendar::provider(&app) != "graph" {
        schemas.retain(|t| !t["function"]["name"].as_str().unwrap_or("").starts_with("teams_"));
    }
    // Shell and file writes need a model that can plan multi-step work.
    if !cloud {
        schemas.retain(|t| !matches!(t["function"]["name"].as_str(), Some("run_command" | "write_file" | "run_app_cli")));
    }
    // Cloud models handle many tools well; the small local model gets only the plugins mentioned.
    schemas.extend(if cloud { state.plugins.all_schemas() } else { state.plugins.schemas_for(&routing_context) });
    let rounds = if cloud { MAX_CLOUD_TOOL_ROUNDS } else { MAX_TOOL_ROUNDS };
    state.chat_cancel.store(false, Ordering::SeqCst);
    // What tools did this turn, so a failure afterwards doesn't hide that they ran.
    let mut done: Vec<String> = vec![];
    for _ in 0..rounds {
        if state.chat_cancel.load(Ordering::SeqCst) {
            return Ok(stopped(&app, &state, &done));
        }
        let reply = match brain::chat(&app, &messages, &schemas, |tok| {
            let _ = app.emit("chat://token", tok);
        })
        .await
        {
            Ok(r) => r,
            Err(e) if !done.is_empty() => {
                let content = format!("Done: {}\n\n(I couldn't finish my reply: {e})", done.join(" "));
                let _ = app.emit("chat://token", &content);
                db::insert_message(&state.db.lock().unwrap(), "assistant", &content).map_err(|e| e.to_string())?;
                return Ok(content);
            }
            Err(e) => return Err(e.to_string()),
        };

        if reply.tool_calls.is_empty() {
            let mut content = reply.content.trim().to_string();
            // The local model can't be trusted to say this, so say it for it.
            if teams_not_connected_request(&app, &text) {
                let note = "\n\n(To actually send Teams messages for you, I need Microsoft access once: \
                            Settings → Microsoft 365. Until then, copy this into Teams, or use ↩ Reply on the message.)";
                let _ = app.emit("chat://token", note);
                content.push_str(note);
            }
            db::insert_message(&state.db.lock().unwrap(), "assistant", &content).map_err(|e| e.to_string())?;
            return Ok(content);
        }

        let calls = reply.tool_calls.clone();
        messages.push(reply);
        for call in calls {
            let (name, args) = (&call.function.name, &call.function.arguments);
            let (result, images) = if state.chat_cancel.load(Ordering::SeqCst) {
                ("error: the user pressed Stop".to_string(), vec![])
            } else if cloud::invalid_args(args) {
                ("error: the tool arguments weren't valid JSON - try the call again".to_string(), vec![])
            } else {
                dispatch(&app, &state, name, args, cloud).await
            };
            if !result.starts_with("error") && !result.starts_with("The user") {
                done.push(result.lines().next().unwrap_or("").to_string());
            }
            let mut msg = Message::new("tool", result);
            msg.tool_name = Some(call.function.name);
            msg.tool_call_id = call.id;
            if cloud && !images.is_empty() {
                msg.images = images;
                keep_recent_images(&mut messages, MAX_IMAGES - 1);
            }
            messages.push(msg);
        }
    }
    Err("Too many tool rounds; stopping.".into())
}

/// Drops screenshots from all but the `keep` most recent tool results: old
/// screens are stale and each one costs over a thousand tokens per round.
fn keep_recent_images(messages: &mut [Message], keep: usize) {
    let mut seen = 0;
    for m in messages.iter_mut().rev().filter(|m| !m.images.is_empty()) {
        seen += 1;
        if seen > keep {
            m.images.clear();
            m.content.push_str("\n[screenshot removed - take a new one if needed]");
        }
    }
}

/// Ends a turn the user stopped, keeping a record of what already ran.
fn stopped(app: &AppHandle, state: &AppState, done: &[String]) -> String {
    let content = if done.is_empty() { "Stopped.".to_string() } else { format!("Stopped. Done so far: {}", done.join(" ")) };
    let _ = app.emit("chat://token", &content);
    let _ = db::insert_message(&state.db.lock().unwrap(), "assistant", &content);
    content
}

/// The Stop button: ends the running turn after the current step and declines any pending approval.
#[tauri::command]
fn chat_stop(app: AppHandle, state: State<'_, AppState>) {
    state.chat_cancel.store(true, Ordering::SeqCst);
    approvals::deny_all(&app);
}

/// The user asked to message someone on Teams, but Microsoft sign-in isn't set up.
fn teams_not_connected_request(app: &AppHandle, text: &str) -> bool {
    if calendar::provider(app) == "graph" {
        return false;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let asks_to_send = words.iter().any(|w| ["message", "tell", "reply", "text", "ping", "send", "dm", "msg"].contains(w))
        || lower.contains("let ") && lower.contains(" know")
        || lower.contains("on my behalf");
    if !asks_to_send {
        return false;
    }
    if lower.contains("teams") || lower.contains("on my behalf") {
        return true;
    }
    // Names of people who've messaged recently ("tell akilesh ...").
    let state = app.state::<AppState>();
    let senders: Vec<String> = db::list_notifications(&state.db.lock().unwrap(), None, None, 200)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.source == "Teams")
        .filter_map(|n| n.sender)
        .flat_map(|s| s.to_lowercase().split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|w| w.len() > 2)
        .collect();
    words.iter().any(|w| senders.iter().any(|s| s == w))
}

/// Runs one tool call (built-in or plugin), asking the user first when required.
async fn dispatch(app: &AppHandle, state: &AppState, name: &str, args: &serde_json::Value, cloud: bool) -> (String, Vec<llm::Image>) {
    let is_plugin = state.plugins.is_plugin_tool(name);
    let approval = if is_plugin {
        state.plugins.approval_text(name, args).map(Ok)
    } else {
        state.tools.approval(app, name, args)
    };
    match approval {
        Some(Err(e)) => return (format!("error: {e}"), vec![]),
        Some(Ok(description)) => {
            let _ = app.emit("chat://tool", "waiting for your approval");
            if !approvals::ask(app, name, &description).await {
                return ("The user declined this action. Acknowledge briefly; don't retry unless they ask.".into(), vec![]);
            }
        }
        None => {}
    }
    let _ = app.emit("chat://tool", name);
    if is_plugin {
        state.plugins.call(name, args, plugins::result_limit(cloud)).await
    } else {
        (state.tools.call(app, name, args).await, vec![])
    }
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

/// `long` None picks short/long by the cycle; `minutes` None uses the configured length.
#[tauri::command]
fn start_break(app: AppHandle, long: Option<bool>, minutes: Option<u32>) -> Result<(), String> {
    focus::start_break(&app, long, minutes).map(|_| ()).map_err(err)
}

#[tauri::command]
fn pause_focus(app: AppHandle) -> Result<(), String> {
    focus::pause(&app).map_err(err)
}

#[tauri::command]
fn resume_focus(app: AppHandle) -> Result<(), String> {
    focus::resume(&app).map_err(err)
}

#[tauri::command]
fn extend_focus(app: AppHandle, minutes: i64) -> Result<(), String> {
    focus::extend(&app, minutes).map_err(err)
}

#[tauri::command]
fn skip_focus(app: AppHandle) -> Result<(), String> {
    focus::skip(&app).map_err(err)
}

#[tauri::command]
fn focus_stats(app: AppHandle) -> Result<stats::Stats, String> {
    focus::analytics(&app).map_err(err)
}

#[tauri::command]
fn pomodoro_config_set(app: AppHandle, config: focus::Config) -> Result<(), String> {
    focus::set_config(&app, &config).map_err(err)
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

#[tauri::command]
async fn app_clis() -> Vec<apps::AppCli> {
    tauri::async_runtime::spawn_blocking(apps::discover).await.unwrap_or_default()
}

#[tauri::command]
fn plugins_info(state: State<'_, AppState>) -> Vec<plugins::PluginInfo> {
    state.plugins.info()
}

#[tauri::command]
async fn plugins_reload(app: AppHandle) -> Result<(), String> {
    plugins::reload(&app).await.map_err(err)
}

#[tauri::command]
async fn plugin_add(app: AppHandle, name: String, command: String, triggers: String) -> Result<(), String> {
    plugins::add(&name, &command, &triggers).map_err(err)?;
    plugins::reload(&app).await.map_err(err)
}

#[tauri::command]
async fn plugin_add_desktop(app: AppHandle) -> Result<(), String> {
    plugins::add_desktop().map_err(err)?;
    plugins::reload(&app).await.map_err(err)
}

#[tauri::command]
async fn plugin_remove(app: AppHandle, name: String) -> Result<(), String> {
    plugins::remove(&name).map_err(err)?;
    plugins::reload(&app).await.map_err(err)
}

#[tauri::command]
async fn plugin_set_enabled(app: AppHandle, name: String, enabled: bool) -> Result<(), String> {
    plugins::set_enabled(&name, enabled).map_err(err)?;
    plugins::reload(&app).await.map_err(err)
}

/// Doesn't restart the plugin; takes effect on the next call.
#[tauri::command]
async fn plugin_set_auto_approve(app: AppHandle, name: String, tool: String, auto: bool) -> Result<(), String> {
    plugins::set_auto_approve(&name, &tool, auto).map_err(err)?;
    plugins::reload(&app).await.map_err(err)
}

#[tauri::command]
fn brain_status(app: AppHandle) -> brain::Status {
    brain::status(&app)
}

#[tauri::command]
fn brain_save_key(provider: String, key: String) -> Result<(), String> {
    brain::save_key(&provider, &key).map_err(err)
}

#[tauri::command]
fn brain_configure(app: AppHandle, provider: String, model: Option<String>, background: bool) -> Result<(), String> {
    brain::configure(&app, &provider, model.as_deref(), background).map_err(err)
}

/// Lists models the key can use; `key` lets the UI check a key before saving it.
#[tauri::command]
async fn brain_models(app: AppHandle, provider: String, key: Option<String>) -> Result<Vec<String>, String> {
    brain::list_models(&app, &provider, key.as_deref()).await.map_err(err)
}

/// Sends text the user wrote in a reply box; clicking Send is their approval.
/// Returns the chat's name.
#[tauri::command]
async fn teams_reply(app: AppHandle, notification_id: i64, text: String) -> Result<String, String> {
    teams::reply_to_notification(&app, notification_id, &text).await.map_err(err)
}

#[tauri::command]
fn sounds_get(state: State<'_, AppState>) -> bool {
    desktop::sounds_on(&state.db.lock().unwrap())
}

/// Turns chimes on/off; plays one when turning on so you hear what it sounds like.
#[tauri::command]
fn sounds_set(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    db::set_setting(&state.db.lock().unwrap(), "sounds_enabled", if on { "1" } else { "0" }).map_err(err)?;
    if on {
        desktop::chime(desktop::Chime::FocusDone);
    }
    Ok(())
}

#[tauri::command]
fn sounds_preview(which: String) {
    desktop::chime(if which == "break" { desktop::Chime::BreakDone } else { desktop::Chime::FocusDone });
}

#[tauri::command]
fn voice_status(app: AppHandle) -> voice::Status {
    voice::status(&app)
}

#[tauri::command]
async fn voice_install(app: AppHandle) -> Result<(), String> {
    voice::install(&app).await.map_err(err)
}

/// Starts listening; called again while listening, it ends the recording.
#[tauri::command]
fn voice_listen(app: AppHandle) -> Result<(), String> {
    voice::start_listening(&app).map_err(err)
}

#[tauri::command]
fn voice_speak(app: AppHandle, text: String) -> Result<(), String> {
    voice::speak(&app, &text).map_err(err)
}

#[tauri::command]
fn voice_stop(app: AppHandle) {
    voice::stop_speaking(&app);
}

#[tauri::command]
fn voice_set_mode(app: AppHandle, mode: String) -> Result<(), String> {
    voice::set_speak_mode(&app, &mode).map_err(err)
}

#[tauri::command]
fn voice_should_speak(app: AppHandle, by_voice: bool) -> bool {
    voice::should_speak(&app, by_voice)
}

#[tauri::command]
fn toggle_panel(app: AppHandle) -> Result<(), String> {
    windows::toggle_panel(&app).map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_panel(app: AppHandle) -> Result<(), String> {
    windows::hide_panel(&app).map_err(|e| e.to_string())
}

#[tauri::command]
fn companion_bubble(app: AppHandle, visible: bool) -> Result<(), String> {
    windows::set_companion_bubble(&app, visible).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_config_dir() {
    let _ = std::process::Command::new("xdg-open").arg(persona::config_dir()).spawn();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let persona = persona::load().expect("failed to load persona.yaml");
    let state = AppState {
        ollama: Ollama::new(&persona.ollama_url),
        http: reqwest::Client::new(),
        db: Mutex::new(db::open().expect("failed to open database")),
        tools: tools::Registry::with_builtins(),
        calendar: calendar::Calendar::default(),
        approvals: approvals::Approvals::default(),
        chat_cancel: Default::default(),
        plugins: plugins::Host::default(),
        voice: voice::Voice::default(),
        teams: teams::Teams::default(),
        persona,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(VOICE_SHORTCUT)
                .expect("valid shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Err(e) = voice::start_listening(app) {
                            let _ = app.emit("voice://error", e.to_string());
                        }
                    }
                })
                .build(),
        )
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
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = plugins::reload(&handle).await {
                    eprintln!("plugins: {e}");
                }
            });

            windows::create(app.handle())?;

            let chat = MenuItem::with_id(app, "chat", "Open chat", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show / hide Fanta", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&chat, &show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Fanta")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "chat" => {
                        let _ = windows::show_panel(app);
                    }
                    "show" => {
                        let _ = windows::toggle_companion(app);
                    }
                    "quit" => {
                        desktop::dnd_off(&app.state::<AppState>().db.lock().unwrap());
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        let _ = windows::toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
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
            start_break,
            pause_focus,
            resume_focus,
            extend_focus,
            skip_focus,
            focus_stats,
            pomodoro_config_set,
            list_notifications,
            inbox_unseen,
            mark_inbox_seen,
            set_notification_category,
            answer_approval,
            chat_stop,
            app_clis,
            plugin_add_desktop,
            calendar_status,
            calendar_sign_in,
            calendar_connect_ics,
            calendar_disconnect,
            calendar_sync,
            calendar_events,
            open_link,
            plugins_info,
            plugins_reload,
            plugin_add,
            plugin_remove,
            plugin_set_enabled,
            plugin_set_auto_approve,
            open_config_dir,
            toggle_panel,
            hide_panel,
            companion_bubble,
            brain_status,
            brain_save_key,
            brain_configure,
            brain_models,
            voice_status,
            voice_install,
            voice_listen,
            voice_speak,
            voice_stop,
            voice_set_mode,
            voice_should_speak,
            teams_reply,
            sounds_get,
            sounds_set,
            sounds_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
