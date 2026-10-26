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
                let _ = db::set_triage(
                    &worker_app.state::<AppState>().db.lock().unwrap(),
                    id,
                    "fyi",
                    None,
                    Some("triage failed"),
                );
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

        let desktop_entry =
            hints.get("desktop-entry").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
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

    let (incoming, triaged) = {
        let conn = state.db.lock().unwrap();
        let during_focus = db::active_session(&conn)?.is_some_and(|s| s.kind == "focus");
        let parsed = parse_message(&source, &title, &body);
        let id = db::insert_notification(&conn, app_name, &source, &title, &body, &parsed, during_focus)?;
        let who = parsed.sender.clone().unwrap_or_else(|| title.clone());
        let rule = rule_triage(cfg, app_name, &source, &who, &parsed.text);
        if let Some((category, reason)) = &rule {
            db::set_triage(&conn, id, category, None, Some(reason))?;
        }
        let n = db::get_notification(&conn, id)?.ok_or_else(|| anyhow::anyhow!("notification {id} vanished"))?;
        let context = db::earlier_in_chat(&conn, &n, 3)?;
        let watched = watched_chat(&cfg.watch, &n);
        (Incoming { notification: n, watched, context }, rule.is_some())
    };
    let id = incoming.notification.id;
    // Straight to the companion, before the (slow) model triage, so she can react at once.
    let _ = app.emit("inbox://new", &incoming);
    let _ = app.emit("inbox://changed", ());
    if triaged {
        after_triage(app, id)?;
    } else if let Some(q) = QUEUE.get() {
        let _ = q.send(id);
    }
    Ok(())
}

/// A message as soon as it arrives, for the companion's instant bubble.
#[derive(serde::Serialize)]
struct Incoming {
    #[serde(flatten)]
    notification: db::Notification,
    /// Named (or posted to) a chat in `notifications.watch`: `Some(that name)`.
    watched: Option<String>,
    /// Earlier messages in the same chat, oldest first.
    context: Vec<db::Notification>,
}

/// Letters and digits only, so "webapp-team", "WebApp Team" and "webapp_team" all match.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Which watched chat (if any) a message belongs to. Teams puts a channel or chat
/// name in the title ("Akilesh posted in webapp-team") or as the group chat label.
fn watched_chat(watch: &[String], n: &db::Notification) -> Option<String> {
    let hay = squash(&format!("{} {} {}", n.title, n.chat.as_deref().unwrap_or(""), n.body));
    watch.iter().find(|w| !squash(w).is_empty() && hay.contains(&squash(w))).cloned()
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

/// A line like "teams.cloud.microsoft" that Chrome puts above web-app notification text.
fn is_origin_line(line: &str) -> bool {
    let l = line.trim();
    !l.is_empty()
        && l.len() < 64
        && !l.contains(' ')
        && l.contains('.')
        && l.chars().all(|c| c.is_ascii_alphanumeric() || ".-:/".contains(c))
}

/// Splits a chat notification into sender, chat and the exact message text.
/// Teams in Chrome sends title "Aarthi S, +3" (the chat) and body
/// "teams.cloud.microsoft\n\nAkilesh: customer grievance agent?".
pub fn parse_message(source: &str, title: &str, body: &str) -> db::ParsedMessage {
    let mut lines: Vec<&str> = body.lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty() || is_origin_line(l)) {
        lines.remove(0);
    }
    let text = lines.join("\n").trim().to_string();
    let title = title.trim();
    let is_chat_app = matches!(source, "Teams" | "Slack" | "WhatsApp" | "Discord");
    if is_chat_app {
        if let Some((who, rest)) = text.split_once(": ") {
            let who = who.trim();
            if !who.is_empty() && who.chars().count() <= 40 && !who.contains('\n') && !who.contains("http") {
                return db::ParsedMessage {
                    sender: Some(who.to_string()),
                    chat: (title != who && !title.is_empty()).then(|| title.to_string()),
                    text: rest.trim().to_string(),
                };
            }
        }
    }
    // "Akilesh Murali @mentioned you" -> sender "Akilesh Murali", the action kept with the text.
    let mut sender = title.to_string();
    let mut action = None;
    for suffix in [
        " @mentioned you",
        " mentioned you",
        " reacted to your message",
        " replied to your message",
        " replied to a message",
        " liked your message",
    ] {
        if let Some(name) = title.strip_suffix(suffix) {
            sender = name.trim().to_string();
            action = Some(suffix.trim().trim_start_matches('@'));
            break;
        }
    }
    let text = if text.is_empty() { title.to_string() } else { text };
    db::ParsedMessage {
        sender: (!sender.is_empty()).then_some(sender),
        chat: None,
        // Keep what kind of message it was - a mention means they want the user's attention.
        text: match action {
            Some(a) => format!("({a}) {text}"),
            None => text,
        },
    }
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
    if app_name.is_empty() {
        "Unknown".into()
    } else {
        app_name.into()
    }
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// Matches a keyword or phrase on word boundaries ("down" won't match "download").
fn contains_phrase(text_words: &[String], phrase: &str) -> bool {
    let p = words(phrase);
    !p.is_empty() && text_words.windows(p.len()).any(|w| w == p.as_slice())
}

/// Cheap, predictable rules that run before the model.
fn rule_triage(
    cfg: &NotificationConfig,
    app_name: &str,
    source: &str,
    title: &str,
    body: &str,
) -> Option<(&'static str, String)> {
    let muted = |name: &str| cfg.muted_apps.iter().any(|m| m.eq_ignore_ascii_case(name));
    if muted(app_name) || muted(source) {
        return Some(("ignore", "muted app".into()));
    }
    // Reactions and typing indicators: nothing to read, and the model is slow to say so.
    let lower = body.to_lowercase();
    if lower.starts_with("(reacted to your message)")
        || lower.starts_with("(liked your message)")
        || lower.ends_with(" is typing")
    {
        return Some(("ignore", "reaction".into()));
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
an incident or outage, a meeting starting now, their manager asking for something now.
- fyi: worth reading - any real chat message a person wrote (questions, replies, updates, \
files shared, mentions), emails, calendar changes.
- ignore: noise only - reactions/likes, 'X is typing', a lone emoji or GIF, promotions, \
app or system chatter, download complete, music.
A message a person typed is never ignore unless it is only an emoji, sticker, GIF or reaction.
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
        Message::new(
            "user",
            format!(
                "App: {}\nChat: {}\nFrom: {}\nMessage: {}",
                n.source,
                n.chat.as_deref().unwrap_or("-"),
                n.sender.as_deref().unwrap_or(&n.title),
                n.text.as_deref().unwrap_or(&n.body)
            ),
        ),
    ];
    let out: Value = state.ollama.complete_json(&state.persona.model, &messages, &schema).await?;
    let category = out["category"].as_str().unwrap_or("fyi");
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
        let who = n.sender.clone().unwrap_or_else(|| n.title.clone());
        let text = n.text.clone().unwrap_or_else(|| n.body.clone());
        desktop::notify(&format!("Urgent · {who} ({})", n.source), &text, true);
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
    let list = items.iter().rev().map(|n| format!("- [{}] {}", n.category, describe(n))).collect::<Vec<_>>().join("\n");
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

/// One line with who said what, for prompts: "15:42 Teams · Akilesh in “Team chat”: hi".
pub fn describe(n: &db::Notification) -> String {
    let who = n.sender.clone().unwrap_or_else(|| n.title.clone());
    let chat = n.chat.as_ref().map(|c| format!(" in “{c}”")).unwrap_or_default();
    let text: String = n.text.clone().unwrap_or_else(|| n.body.clone()).chars().take(300).collect();
    format!("{} {} · {who}{chat}: {text}", n.received_at.get(11..16).unwrap_or(""), n.source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_teams_group_message() {
        let m = parse_message(
            "Teams",
            "Aarthi Selvakumari, +3",
            "teams.cloud.microsoft\n\nAkilesh: customer grievance agent?",
        );
        assert_eq!(m.sender.as_deref(), Some("Akilesh"));
        assert_eq!(m.chat.as_deref(), Some("Aarthi Selvakumari, +3"));
        assert_eq!(m.text, "customer grievance agent?");

        let dm = parse_message("Teams", "Ravi Kumar", "teams.microsoft.com\n\nare you free at 3?");
        assert_eq!((dm.sender.as_deref(), dm.chat, dm.text.as_str()), (Some("Ravi Kumar"), None, "are you free at 3?"));

        let mention = parse_message(
            "Teams",
            "Akilesh Murali @mentioned you",
            "teams.cloud.microsoft\n\nSarathkumar come on lets finish",
        );
        assert_eq!(mention.sender.as_deref(), Some("Akilesh Murali"));
        assert_eq!(mention.text, "(mentioned you) Sarathkumar come on lets finish");

        let mail = parse_message("Thunderbird", "Invoice #881", "Your invoice: due 12:30");
        assert_eq!((mail.sender.as_deref(), mail.text.as_str()), (Some("Invoice #881"), "Your invoice: due 12:30"));
    }

    #[test]
    fn strips_chrome_markup() {
        assert_eq!(
            strip_markup("<a href=\"https://teams.microsoft.com\">teams.microsoft.com</a>\n\nHi &amp; bye"),
            "teams.microsoft.com\n\nHi & bye"
        );
    }

    #[test]
    fn detects_teams_in_browser() {
        assert_eq!(
            detect_source("Google Chrome", "google-chrome", "Alice Smith", "teams.microsoft.com\n\nping"),
            "Teams"
        );
        assert_eq!(detect_source("Thunderbird", "", "Invoice", "hello"), "Thunderbird");
    }

    #[test]
    fn rules() {
        let cfg = NotificationConfig {
            vip: vec!["Priya Raman".into()],
            muted_apps: vec!["Spotify".into()],
            ..Default::default()
        };
        assert_eq!(rule_triage(&cfg, "Spotify", "Spotify", "Song", "x").unwrap().0, "ignore");
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Priya Raman", "hi").unwrap().0, "urgent");
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "is prod down?").unwrap().0, "urgent");
        assert!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "download finished, calm down").is_none());
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "(reacted to your message) 👍").unwrap().0, "ignore");
        assert_eq!(rule_triage(&cfg, "Chrome", "Teams", "Bob", "Bob is typing").unwrap().0, "ignore");
    }

    #[test]
    fn watched_chats() {
        let watch = vec!["webapp-team".to_string()];
        let mut n = db::Notification {
            id: 1,
            app: "Google Chrome".into(),
            source: "Teams".into(),
            title: "Akilesh posted in WebApp Team".into(),
            body: String::new(),
            sender: None,
            chat: None,
            text: None,
            category: "pending".into(),
            summary: None,
            reason: None,
            during_focus: false,
            seen: false,
            received_at: String::new(),
        };
        assert_eq!(watched_chat(&watch, &n).as_deref(), Some("webapp-team"));
        n.title = "Ravi Kumar".into();
        n.chat = Some("webapp_team".into());
        assert!(watched_chat(&watch, &n).is_some());
        n.chat = Some("Creator Studio".into());
        assert!(watched_chat(&watch, &n).is_none());
        assert!(watched_chat(&[], &n).is_none());
    }
}
