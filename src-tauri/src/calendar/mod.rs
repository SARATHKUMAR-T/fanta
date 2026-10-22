//! Outlook calendar: Microsoft Graph (read/write) or a published ICS feed
//! (read-only), cached locally and refreshed in the background.

pub mod graph;
pub mod ics;

use chrono::{Duration, Local, NaiveDateTime, Utc};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{db, desktop, AppState};

const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const HEADS_UP_MINUTES: i64 = 5;
const KEYRING_SERVICE: &str = "fanta";
const KEYRING_USER: &str = "outlook-refresh-token";

// settings keys
const PROVIDER: &str = "cal_provider";
const CLIENT_ID: &str = "cal_client_id";
const TENANT: &str = "cal_tenant";
const ICS_URL: &str = "cal_ics_url";
const ACCOUNT: &str = "cal_account";
const LAST_SYNC: &str = "cal_last_sync";
const LAST_ERROR: &str = "cal_error";

#[derive(Default)]
pub struct Calendar {
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
    sync_now: Notify,
}

#[derive(Serialize)]
pub struct Status {
    /// "none", "graph", or "ics"
    pub provider: String,
    pub account: Option<String>,
    pub last_sync: Option<String>,
    pub error: Option<String>,
    pub can_write: bool,
    pub client_id: Option<String>,
    pub tenant: Option<String>,
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    db::get_setting(&app.state::<AppState>().db.lock().unwrap(), key).ok().flatten()
}

fn set(app: &AppHandle, key: &str, value: &str) -> anyhow::Result<()> {
    db::set_setting(&app.state::<AppState>().db.lock().unwrap(), key, value)
}

/// Same as `provider`, for callers already holding the db lock.
pub fn provider_conn(conn: &rusqlite::Connection) -> String {
    db::get_setting(conn, PROVIDER).ok().flatten().unwrap_or_else(|| "none".into())
}

pub fn provider(app: &AppHandle) -> String {
    setting(app, PROVIDER).unwrap_or_else(|| "none".into())
}

fn keyring() -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?)
}

fn graph_app(app: &AppHandle) -> anyhow::Result<graph::App> {
    Ok(graph::App {
        client_id: setting(app, CLIENT_ID).ok_or_else(|| anyhow::anyhow!("Outlook isn't set up"))?,
        tenant: setting(app, TENANT).unwrap_or_else(|| "organizations".into()),
    })
}

pub fn status(app: &AppHandle) -> Status {
    let provider = provider(app);
    Status {
        can_write: provider == "graph",
        account: setting(app, ACCOUNT),
        last_sync: setting(app, LAST_SYNC),
        error: setting(app, LAST_ERROR),
        client_id: setting(app, CLIENT_ID),
        tenant: setting(app, TENANT),
        provider,
    }
}

pub fn connected(app: &AppHandle) -> bool {
    provider(app) != "none"
}

pub fn request_sync(app: &AppHandle) {
    app.state::<AppState>().calendar.sync_now.notify_one();
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if connected(&app) {
                match sync(&app).await {
                    Ok(()) => {
                        let state = app.state::<AppState>();
                        let conn = state.db.lock().unwrap();
                        let _ = db::set_setting(&conn, LAST_SYNC, &db::now_ts());
                        let _ = db::delete_setting(&conn, LAST_ERROR);
                    }
                    Err(e) => {
                        eprintln!("calendar sync: {e}");
                        let _ = set(&app, LAST_ERROR, &e.to_string());
                    }
                }
                let _ = app.emit("calendar://changed", ());
            }
            let cal = &app.state::<AppState>().calendar;
            let _ = tokio::time::timeout(SYNC_EVERY, cal.sync_now.notified()).await;
        }
    });
}

async fn sync(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let from = Utc::now() - Duration::days(1);
    let to = Utc::now() + Duration::days(14);
    let events = match provider(app).as_str() {
        "graph" => {
            let token = access_token(app).await?;
            graph::calendar_view(&state.calendar.http, &token, from, to).await?
        }
        "ics" => {
            let url = setting(app, ICS_URL).ok_or_else(|| anyhow::anyhow!("no ICS link set"))?;
            let text = state.calendar.http.get(&url).send().await?.error_for_status()?.text().await?;
            ics::parse(&text, from, to)
        }
        _ => return Ok(()),
    };
    let mut conn = state.db.lock().unwrap();
    db::replace_calendar(&mut conn, &events)
}

/// A valid Graph access token, refreshing (and rotating the stored refresh token) as needed.
pub async fn access_token(app: &AppHandle) -> anyhow::Result<String> {
    let cal = &app.state::<AppState>().calendar;
    if let Some((token, expires)) = cal.token.lock().unwrap().clone() {
        if expires > Instant::now() + std::time::Duration::from_secs(60) {
            return Ok(token);
        }
    }
    let refresh_token = keyring()?
        .get_password()
        .map_err(|_| anyhow::anyhow!("Outlook sign-in expired - sign in again from Connections"))?;
    let tokens = graph::refresh(&cal.http, &graph_app(app)?, &refresh_token).await?;
    if let Some(rt) = &tokens.refresh_token {
        keyring()?.set_password(rt)?;
    }
    let expires = Instant::now() + std::time::Duration::from_secs(tokens.expires_in.max(0) as u64);
    *cal.token.lock().unwrap() = Some((tokens.access_token.clone(), expires));
    Ok(tokens.access_token)
}

/// Starts device-code sign-in. The UI shows the code; completion arrives as `calendar://auth`.
pub async fn begin_graph_sign_in(app: &AppHandle, client_id: &str, tenant: &str) -> anyhow::Result<graph::DeviceCode> {
    let gapp = graph::App {
        client_id: client_id.trim().to_string(),
        tenant: if tenant.trim().is_empty() { "organizations".into() } else { tenant.trim().to_string() },
    };
    let http = app.state::<AppState>().calendar.http.clone();
    let code = graph::begin_device_code(&http, &gapp).await?;

    let app = app.clone();
    let pending = code.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let tokens = graph::poll_device_code(&http, &gapp, &pending).await?;
            let refresh_token = tokens
                .refresh_token
                .ok_or_else(|| anyhow::anyhow!("no refresh token - is offline_access granted?"))?;
            keyring()?.set_password(&refresh_token)?;
            let account = graph::me(&http, &tokens.access_token).await.unwrap_or_else(|_| "Outlook".into());
            {
                let state = app.state::<AppState>();
                let conn = state.db.lock().unwrap();
                db::set_setting(&conn, CLIENT_ID, &gapp.client_id)?;
                db::set_setting(&conn, TENANT, &gapp.tenant)?;
                db::set_setting(&conn, PROVIDER, "graph")?;
                db::set_setting(&conn, ACCOUNT, &account)?;
                db::delete_setting(&conn, LAST_ERROR)?;
            }
            let expires = Instant::now() + std::time::Duration::from_secs(tokens.expires_in.max(0) as u64);
            *app.state::<AppState>().calendar.token.lock().unwrap() = Some((tokens.access_token, expires));
            anyhow::Ok(account)
        }
        .await;
        let payload = match result {
            Ok(account) => {
                request_sync(&app);
                serde_json::json!({ "ok": true, "message": format!("Connected as {account}") })
            }
            Err(e) => serde_json::json!({ "ok": false, "message": e.to_string() }),
        };
        let _ = app.emit("calendar://auth", payload);
    });
    Ok(code)
}

/// Checks the feed parses before saving it.
pub async fn connect_ics(app: &AppHandle, url: &str) -> anyhow::Result<usize> {
    let url = url.trim().replacen("webcal://", "https://", 1);
    let http = &app.state::<AppState>().calendar.http;
    let text = http.get(&url).send().await?.error_for_status()?.text().await?;
    if !text.contains("BEGIN:VCALENDAR") {
        anyhow::bail!("that link didn't return a calendar (ICS) feed");
    }
    let count = ics::parse(&text, Utc::now() - Duration::days(1), Utc::now() + Duration::days(14)).len();
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::set_setting(&conn, ICS_URL, &url)?;
        db::set_setting(&conn, PROVIDER, "ics")?;
        db::set_setting(&conn, ACCOUNT, "Published calendar (read-only)")?;
        db::delete_setting(&conn, LAST_ERROR)?;
    }
    request_sync(app);
    Ok(count)
}

pub fn disconnect(app: &AppHandle) -> anyhow::Result<()> {
    if let Ok(entry) = keyring() {
        let _ = entry.delete_credential();
    }
    *app.state::<AppState>().calendar.token.lock().unwrap() = None;
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    for key in [PROVIDER, ICS_URL, ACCOUNT, LAST_SYNC, LAST_ERROR] {
        db::delete_setting(&conn, key)?;
    }
    db::clear_calendar(&conn)?;
    drop(conn);
    let _ = app.emit("calendar://changed", ());
    Ok(())
}

// ---- helpers used by focus, scheduler, and chat tools ----

fn ts_in(minutes: i64) -> String {
    (Local::now() + Duration::minutes(minutes)).format(db::TS).to_string()
}

pub fn minutes_until(ts: &str) -> i64 {
    NaiveDateTime::parse_from_str(ts, db::TS)
        // Round up: 4m50s away reads as "in 5 min".
        .map(|t| ((t - Local::now().naive_local()).num_seconds() + 59).div_euclid(60))
        .unwrap_or(i64::MAX)
}

pub fn hm(ts: &str) -> &str {
    ts.get(11..16).unwrap_or(ts)
}

/// The first busy meeting starting within the next `minutes`.
pub fn next_meeting_within(app: &AppHandle, minutes: i64) -> Option<db::CalEvent> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let now = db::now_ts();
    db::busy_between(&conn, &now, &ts_in(minutes))
        .ok()?
        .into_iter()
        .find(|e| e.start_at >= now)
}

pub fn in_meeting_now(app: &AppHandle) -> Option<db::CalEvent> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let now = db::now_ts();
    db::busy_between(&conn, &now, &ts_in(1)).ok()?.into_iter().find(|e| e.start_at <= now)
}

/// One line per event, for the model and for chat answers.
pub fn describe(e: &db::CalEvent) -> String {
    let when = if e.all_day { "all day".to_string() } else { format!("{}–{}", hm(&e.start_at), hm(&e.end_at)) };
    let mut s = format!("#{} {when} {}", e.num, e.subject);
    if e.join_url.is_some() {
        s += " (Teams)";
    } else if let Some(l) = &e.location {
        s += &format!(" @ {l}");
    }
    match e.response.as_deref() {
        Some("tentativelyAccepted") => s += " [tentative]",
        Some("notResponded") | Some("none") => s += " [not responded]",
        Some("declined") => s += " [declined]",
        _ => {}
    }
    if e.show_as.as_deref() == Some("free") {
        s += " [free]";
    }
    s
}

/// Today's remaining meetings, for prompts. Empty if no calendar is connected.
pub fn today_summary(app: &AppHandle) -> String {
    if !connected(app) {
        return String::new();
    }
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let end_of_day = format!("{} 23:59:59", Local::now().format("%Y-%m-%d"));
    let events = db::events_between(&conn, &db::now_ts(), &end_of_day).unwrap_or_default();
    if events.is_empty() {
        "Meetings for the rest of today: none.".into()
    } else {
        format!(
            "Meetings for the rest of today:\n{}",
            events.iter().map(describe).collect::<Vec<_>>().join("\n")
        )
    }
}

/// Called by the scheduler: notifies a few minutes before each meeting.
pub fn heads_up(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let due = {
        let conn = state.db.lock().unwrap();
        let due = db::due_heads_up(&conn, &db::now_ts(), &ts_in(HEADS_UP_MINUTES))?;
        for e in &due {
            db::mark_event_notified(&conn, e.num)?;
        }
        due
    };
    for e in due {
        let mins = minutes_until(&e.start_at).max(0);
        let when = if mins <= 0 { "now".to_string() } else { format!("in {mins} min") };
        let title = format!("{} {when}", e.subject);
        let body = match (&e.join_url, &e.location) {
            (Some(_), _) => "Teams meeting - click Join to open it.".to_string(),
            (None, Some(l)) => format!("Location: {l}"),
            _ => format!("Starts at {}", hm(&e.start_at)),
        };
        desktop::notify_with_link(&title, &body, true, e.join_url.as_deref());
        let chat = match &e.join_url {
            Some(url) => format!("📅 {title}. Join: {url}"),
            None => format!("📅 {title}."),
        };
        db::insert_message(&state.db.lock().unwrap(), "assistant", &chat)?;
        let _ = app.emit("chat://proactive", chat);
    }
    Ok(())
}

// ---- write actions (Graph only) ----

pub async fn create_event(app: &AppHandle, m: &graph::NewMeeting) -> anyhow::Result<db::CalEvent> {
    if provider(app) != "graph" {
        anyhow::bail!("creating events needs Outlook sign-in (the ICS link is read-only)");
    }
    let token = access_token(app).await?;
    let created = graph::create_event(&app.state::<AppState>().calendar.http, &token, m).await?;
    let event = db::add_event(&app.state::<AppState>().db.lock().unwrap(), &created)?;
    let _ = app.emit("calendar://changed", ());
    request_sync(app);
    Ok(event)
}

pub async fn respond(app: &AppHandle, event: &db::CalEvent, response: &str, comment: &str) -> anyhow::Result<()> {
    if provider(app) != "graph" {
        anyhow::bail!("responding to invites needs Outlook sign-in (the ICS link is read-only)");
    }
    let token = access_token(app).await?;
    graph::respond(&app.state::<AppState>().calendar.http, &token, &event.remote_id, response, comment).await?;
    request_sync(app);
    Ok(())
}
