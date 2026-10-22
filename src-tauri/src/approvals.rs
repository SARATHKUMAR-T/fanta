//! Asks the user before Fanta takes an action with side effects outside the app
//! (sending invites, declining meetings, and later plugin actions).

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::AppState;

const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct Approvals {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
}

#[derive(Clone, Serialize)]
struct Request<'a> {
    id: u64,
    tool: &'a str,
    text: &'a str,
}

/// Shows an Approve/Deny card in chat and waits. No answer within 5 minutes counts as no.
pub async fn ask(app: &AppHandle, tool: &str, text: &str) -> bool {
    let approvals = &app.state::<AppState>().approvals;
    let id = approvals.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let (tx, rx) = oneshot::channel();
    approvals.pending.lock().unwrap().insert(id, tx);
    let _ = app.emit("approval://request", Request { id, tool, text });
    let approved = matches!(tokio::time::timeout(TIMEOUT, rx).await, Ok(Ok(true)));
    approvals.pending.lock().unwrap().remove(&id);
    let _ = app.emit("approval://resolved", serde_json::json!({ "id": id, "approved": approved }));
    approved
}

pub fn answer(app: &AppHandle, id: u64, approved: bool) {
    if let Some(tx) = app.state::<AppState>().approvals.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(approved);
    }
}
