mod db;
mod llm;
mod persona;
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

struct AppState {
    persona: Persona,
    db: Mutex<rusqlite::Connection>,
    ollama: Ollama,
    tools: tools::Registry,
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
    let mut messages = vec![Message::new("system", state.persona.full_system_prompt())];
    {
        let conn = state.db.lock().unwrap();
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
            let _ = app.emit("chat://tool", &call.function.name);
            let result = state.tools.call(&call.function.name, &call.function.arguments);
            let mut msg = Message::new("tool", result);
            msg.tool_name = Some(call.function.name);
            messages.push(msg);
        }
    }
    Err("Too many tool rounds; stopping.".into())
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
        persona,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
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
                    "quit" => app.exit(0),
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
        .invoke_handler(tauri::generate_handler![get_persona, get_history, send_message])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
