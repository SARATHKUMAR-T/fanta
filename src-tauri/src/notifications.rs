//! Notification hub: watches every desktop notification on the session bus
//! (Teams/Slack/mail in the browser all go through org.freedesktop.Notifications),
//! triages each one locally, and holds non-urgent ones back during focus.

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use zbus::zvariant::OwnedValue;

use crate::llm::Message;
use crate::persona::NotificationConfig;
use crate::{db, desktop, scheduler, AppState};

const BODY_LIMIT: usize = 2000;
/// The same notification can be seen twice when a relay forwards it to GNOME Shell.
const DEDUP_WINDOW: Duration = Duration::from_secs(3);

static QUEUE: OnceLock<UnboundedSender<i64>> = OnceLock::new();

pub fn spawn(app: AppHandle) {
    let (tx, mut rx) = unbounded_channel::<i64>();
    let _ = QUEUE.set(tx.clone());

    // Anything left untriaged from a previous run.
    if let Ok(pending) = db::pending_notifications(&app.state::<AppState>().db.lock().unwrap()) {
        for id in pending {
            let _ = tx.send(id);
        }
    }

    // Triage worker: one at a time, so a burst of messages doesn't swamp the CPU.
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(id) = rx.recv().await {
            if let Err(e) = triage(&worker_app, id).await {
                eprintln!("triage {id}: {e}");
                let _ = db::set_triage(&worker_app.state::<AppState>().db.lock().unwrap(), id, "fyi", None, Some("triage failed"));
                let _ = worker_app.emit("inbox://changed", ());
            }
        }
    });

    // Monitor: reconnects if the bus connection drops.
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = monitor(&app).await {
                eprintln!("notification monitor: {e}");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

async fn monitor(app: &AppHandle) -> anyhow::Result<()> {
    // A monitoring connection can't be used for anything else, so it gets its own.
    let conn = zbus::Connection::session().await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::MethodCall)
        .interface("org.freedesktop.Notifications")?
        .member("Notify")?
        .build();
    zbus::fdo::MonitoringProxy::new(&conn).await?.become_monitor(&[rule], 0).await?;

    let mut recent: VecDeque<(Instant, String)> = VecDeque::new();
    let mut stream = zbus::MessageStream::from(&conn);
    while let Some(msg) = stream.next().await {
        let msg = msg?;
        if msg.header().member().map(|m| m.as_str()) != Some("Notify") {
            continue;
        }
        type Notify = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
        let Ok((app_name, _, _, title, body, _, hints, _)) = msg.body().deserialize::<Notify>() else {
            continue;
        };
        if app_name == desktop::OWN_APP_NAME {
            continue;
        }

        let key = format!("{app_name}\u{1f}{title}\u{1f}{body}");
        let now = Instant::now();
        recent.retain(|(t, _)| now.duration_since(*t) < DEDUP_WINDOW);
        if recent.iter().any(|(_, k)| *k == key) {
            continue;
        }
        recent.push_back((now, key));

        let desktop_entry = hints
            .get("desktop-entry")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        if let Err(e) = capture(app, &app_name, &desktop_entry, &title, &body) {
            eprintln!("capture: {e}");
        }
    }
    Ok(())
}

fn capture(app: &AppHandle, app_name: &str, desktop_entry: &str, title: &str, body: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let cfg = &state.persona.notifications;
    if !cfg.enabled {
        return Ok(());
    }
    let body: String = strip_markup(body).chars().take(BODY_LIMIT).collect();
    let title = strip_markup(title);
    let source = detect_source(app_name, desktop_entry, &title, &body);
    if title.is_empty() && body.is_empty() {
        return Ok(());
    }

    let id = {
        let conn = state.db.lock().unwrap();
        let during_focus = db::active_session(&conn)?.is_some_and(|s| s.kind == "focus");
        let id = db::insert_notification(&conn, app_name, &source, &title, &body, during_focus)?;
        if let Some((category, reason)) = rule_triage(cfg, app_name, &source, &title, &body) {
            db::set_triage(&conn, id, category, None, Some(&reason))?;
            drop(conn);
            let _ = app.emit("inbox://changed", ());
            after_triage(app, id)?;
            return Ok(());
        }
        id
    };
    let _ = app.emit("inbox://changed", ());
    if let Some(q) = QUEUE.get() {
        let _ = q.send(id);
    }
    Ok(())
}

/// Removes HTML that some apps (and Chrome) put in notification bodies.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

/// Names the real source when a browser sends notifications for a web app.
fn detect_source(app_name: &str, desktop_entry: &str, title: &str, body: &str) -> String {
    let hay = format!("{app_name} {desktop_entry} {title} {body}").to_lowercase();
    for (needle, name) in [
        ("teams", "Teams"),
        ("slack", "Slack"),
        ("outlook", "Outlook"),
        ("mail.google", "Gmail"),
        ("gmail", "Gmail"),
        ("whatsapp", "WhatsApp"),
        ("discord", "Discord"),
        ("jira", "Jira"),
        ("github", "GitHub"),
    ] {
        if hay.contains(needle) {
            return name.into();
        }
    }
    if app_name.is_empty() { "Unknown".into() } else { app_name.into() }
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Matches a keyword or phrase on word boundaries ("down" won't match "download").
fn contains_phrase(text_words: &[String], phrase: &str) -> bool {
    let p = words(phrase);
    !p.is_empty() && text_words.windows(p.len()).any(|w| w == p.as_slice())
}

/// Cheap, predictable rules that run before the model.
fn rule_triage(cfg: &NotificationConfig, app_name: &str, source: &str, title: &str, body: &str) -> Option<(&'static str, String)> {
    let muted = |name: &str| cfg.muted_apps.iter().any(|m| m.eq_ignore_ascii_case(name));
    if muted(app_name) || muted(source) {
        return Some(("ignore", "muted app".into()));
    }
    let title_words = words(title);
    if let Some(v) = cfg.vip.iter().find(|v| contains_phrase(&title_words, v)) {
        return Some(("urgent", format!("from VIP {v}")));
    }
    let all_words = words(&format!("{title} {body}"));
    if let Some(k) = cfg.urgent_keywords.iter().find(|k| contains_phrase(&all_words, k)) {
        return Some(("urgent", format!("mentions '{k}'")));
    }
    None
}

const TRIAGE_PROMPT: &str = "You triage desktop notifications for a busy professional.
Categories:
- urgent: needs attention within minutes - a direct question or request with time pressure, \
an incident or outage, a meeting starting now, their manager asking something.
- fyi: worth reading later - normal chat messages, mentions, updates, emails.
- ignore: noise - promotions, reactions/likes, 'X is typing', app or system chatter, \
download complete, music.
Reply with JSON: category, summary (plain, max 12 words, who + what), reason (max 8 words).";

async fn triage(app: &AppHandle, id: i64) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let Some(n) = db::get_notification(&state.db.lock().unwrap(), id)? else {
        return Ok(());
    };
    if n.category != "pending" {
        return Ok(());
    }
    let schema = json!({
        "type": "object",
        "properties": {
            "category": { "type": "string", "enum": ["urgent", "fyi", "ignore"] },
            "summary": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["category", "summary", "reason"]
    });
    let messages = [
        Message::new("system", TRIAGE_PROMPT),
        Message::new("user", format!("App: {}\nTitle: {}\nBody: {}", n.source, n.title, n.body)),
    ];
    let out: Value = state.ollama.complete_json(&state.persona.model, &messages, &schema).await?;
    let category = match out["category"].as_str() {
        Some(c @ ("urgent" | "fyi" | "ignore")) => c,
        _ => "fyi",
    };
    db::set_triage(&state.db.lock().unwrap(), id, category, out["summary"].as_str(), out["reason"].as_str())?;
    let _ = app.emit("inbox://changed", ());
    after_triage(app, id)
}

/// Urgent messages that arrive during focus break through Do Not Disturb.
fn after_triage(app: &AppHandle, id: i64) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let Some(n) = db::get_notification(&conn, id)? else { return Ok(()) };
    let still_focusing = db::active_session(&conn)?.is_some_and(|s| s.kind == "focus");
    if n.category == "urgent" && n.during_focus && still_focusing {
        let text = n.summary.clone().unwrap_or_else(|| format!("{}: {}", n.title, n.body));
        desktop::notify(&format!("Urgent · {}", n.source), &text, true);
    }
    Ok(())
}

/// After a focus block: waits for triage to catch up, then posts what came in.
pub fn spawn_digest(app: AppHandle, since: String) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = digest(&app, &since).await {
            eprintln!("digest: {e}");
        }
    });
}

async fn digest(app: &AppHandle, since: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    for _ in 0..30 {
        let waiting = db::list_notifications(&state.db.lock().unwrap(), Some("pending"), Some(since), 1)?;
        if waiting.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let items: Vec<db::Notification> = db::list_notifications(&state.db.lock().unwrap(), None, Some(since), 50)?
        .into_iter()
        .filter(|n| n.category == "urgent" || n.category == "fyi")
        .collect();
    if items.is_empty() {
        return Ok(());
    }
    let list = items
        .iter()
        .rev()
        .map(|n| format!("- [{}] {} · {}", n.category, n.source, n.summary.clone().unwrap_or_else(|| n.title.clone())))
        .collect::<Vec<_>>()
        .join("\n");
    let message = scheduler::speak(
        app,
        &format!("Messages that arrived during the focus block:\n{list}"),
        "The user just finished a focus block and is now on a short break; the next block only \
         starts when they choose. Briefly tell them what came in while they focused: lead with \
         anything urgent, group the rest, skip trivia. Don't promise anything else. 2-4 sentences.",
    )
    .await?;
    scheduler::post(app, &message, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_chrome_markup() {
        assert_eq!(
            strip_markup("<a href=\"https://teams.microsoft.com\">teams.microsoft.com</a>\n\nHi &amp; bye"),
            "teams.microsoft.com\n\nHi & bye"
        );
    }

    #[test]
    fn detects_teams_in_browser() {
        assert_eq!(detect_source("Google Chrome", "google-chrome", "Alice Smith", "teams.microsoft.com\n\nping"), "Teams");
        assert_eq!(detect_source("Thunderbird", "", "Invoice", "hello"), "Thunderbird");
    }

    #[test]
    fn rules() {
        let mut cfg = NotificationConfig::default();
        cfg.vip = vec!["Priya Raman".into()];
        cfg.muted_apps = vec!["Spotify".into()];
        assert_eq!(rule_triage(&cfg, "Spotify", "Spotify", "Song", "x").unwrap().0, "ignore");
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Priya Raman", "hi").unwrap().0, "urgent");
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "is prod down?").unwrap().0, "urgent");
        assert!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "download finished, calm down").is_none());
    }
}
