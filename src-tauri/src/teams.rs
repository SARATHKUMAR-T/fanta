//! Sending Teams chat messages through Microsoft Graph, reusing the Microsoft
//! sign-in from the calendar. Finds the right chat from how Teams names it in
//! notifications ("Ravi Kumar", "Aarthi Selvakumari, +3", or a chat topic).

use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::calendar::{self, graph};
use crate::{db, AppState};

const CACHE_FOR: Duration = Duration::from_secs(10 * 60);
const MAX_CHATS: usize = 300;

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub id: String,
    /// "oneOnOne", "group", or "meeting"
    pub kind: String,
    pub topic: Option<String>,
    /// Everyone except the signed-in user.
    pub others: Vec<String>,
}

impl Chat {
    /// What Teams shows for this chat in notifications and the chat list.
    pub fn label(&self) -> String {
        if let Some(t) = self.topic.as_deref().filter(|t| !t.trim().is_empty()) {
            return t.to_string();
        }
        match self.others.len() {
            0 => "(just you)".into(),
            1 => self.others[0].clone(),
            n => format!("{}, +{}", self.others[0], n - 1),
        }
    }
}

#[derive(Default)]
pub struct Teams {
    cache: Mutex<Option<(Instant, Vec<Chat>)>>,
}

fn require_microsoft(app: &AppHandle) -> anyhow::Result<()> {
    if calendar::provider(app) != "graph" {
        anyhow::bail!(
            "Teams isn't connected - sign in with Microsoft in Settings → Outlook calendar (it covers Teams too)"
        );
    }
    Ok(())
}

/// Graph says 403 when the sign-in predates the Teams permissions.
fn explain(e: anyhow::Error) -> anyhow::Error {
    let s = e.to_string();
    if s.contains("403") || s.contains("Forbidden") || s.contains("AADSTS65001") {
        anyhow::anyhow!(
            "Teams needs permission - in Settings, Disconnect and sign in with Microsoft again to allow chats ({s})"
        )
    } else {
        e
    }
}

pub async fn chats(app: &AppHandle, refresh: bool) -> anyhow::Result<Vec<Chat>> {
    require_microsoft(app)?;
    let state = app.state::<AppState>();
    if !refresh {
        if let Some((at, list)) = state.teams.cache.lock().unwrap().as_ref() {
            if at.elapsed() < CACHE_FOR {
                return Ok(list.clone());
            }
        }
    }
    let token = calendar::access_token(app).await?;
    let me = graph::get_json(&state.http, &token, &format!("{}/me?$select=id", graph::GRAPH)).await.map_err(explain)?;
    let my_id = me["id"].as_str().unwrap_or_default().to_string();

    let mut out = vec![];
    let mut url = format!("{}/me/chats?$expand=members&$top=50", graph::GRAPH);
    while out.len() < MAX_CHATS {
        let page = graph::get_json(&state.http, &token, &url).await.map_err(explain)?;
        for c in page["value"].as_array().into_iter().flatten() {
            let others = c["members"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| m["userId"].as_str() != Some(my_id.as_str()))
                .filter_map(|m| m["displayName"].as_str().map(String::from))
                .collect();
            out.push(Chat {
                id: c["id"].as_str().unwrap_or_default().to_string(),
                kind: c["chatType"].as_str().unwrap_or("group").to_string(),
                topic: c["topic"].as_str().map(String::from),
                others,
            });
        }
        match page["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    *state.teams.cache.lock().unwrap() = Some((Instant::now(), out.clone()));
    Ok(out)
}

fn norm(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Does `name` ("Ravi", "ravi kumar") refer to this person ("Ravi Kumar")?
fn name_matches(person: &str, name: &str) -> bool {
    let (p, n) = (norm(person), norm(name));
    !n.is_empty() && (p == n || p.split(' ').any(|w| w == n) || p.starts_with(&format!("{n} ")))
}

/// Finds the chat a name refers to: a chat label/topic, or a person's 1:1 chat.
/// Errors list the candidates when it's ambiguous, so the user (or model) can pick.
pub fn resolve<'a>(chats: &'a [Chat], to: &str) -> anyhow::Result<&'a Chat> {
    let want = norm(to);
    // 1. Exactly how Teams names the chat ("Aarthi Selvakumari, +3", or its topic).
    let by_label: Vec<&Chat> = chats.iter().filter(|c| norm(&c.label()) == want).collect();
    if by_label.len() == 1 {
        return Ok(by_label[0]);
    }
    // 2. A person: their 1:1 chat.
    let direct: Vec<&Chat> =
        chats.iter().filter(|c| c.kind == "oneOnOne" && c.others.iter().any(|o| name_matches(o, to))).collect();
    match direct.len() {
        1 => return Ok(direct[0]),
        n if n > 1 => anyhow::bail!(
            "more than one person matches '{to}': {} - use their full name",
            direct.iter().map(|c| c.label()).collect::<Vec<_>>().join(", ")
        ),
        _ => {}
    }
    // 3. A group chat whose topic contains the words.
    let by_topic: Vec<&Chat> =
        chats.iter().filter(|c| c.topic.as_deref().is_some_and(|t| norm(t).contains(&want))).collect();
    if by_topic.len() == 1 {
        return Ok(by_topic[0]);
    }
    let groups_with: Vec<String> = chats
        .iter()
        .filter(|c| c.kind != "oneOnOne" && c.others.iter().any(|o| name_matches(o, to)))
        .take(5)
        .map(|c| c.label())
        .collect();
    if groups_with.is_empty() {
        anyhow::bail!("no Teams chat found for '{to}' (Fanta can only message chats that already exist)");
    }
    anyhow::bail!("no 1:1 chat with '{to}'. Group chats with them: {} - say which one", groups_with.join("; "))
}

/// The chat a captured notification came from. Group chats show up as
/// "First Person, +N" (or their topic); direct messages as the sender's name.
pub fn chat_for_notification<'a>(chats: &'a [Chat], n: &db::Notification) -> anyhow::Result<&'a Chat> {
    match n.chat.as_deref() {
        Some(chat_label) => {
            if let Ok(c) = resolve(chats, chat_label) {
                return Ok(c);
            }
            // "Aarthi Selvakumari, +3": group chat with that first person and 3 more,
            // that also includes the sender.
            if let Some((first, more)) = chat_label.rsplit_once(", +") {
                let count: usize = more.trim().parse().unwrap_or(0);
                let sender = n.sender.as_deref().unwrap_or("");
                let found: Vec<&Chat> = chats
                    .iter()
                    .filter(|c| {
                        c.kind != "oneOnOne"
                            && c.others.len() == count + 1
                            && c.others.iter().any(|o| name_matches(o, first.trim()))
                            && (sender.is_empty() || c.others.iter().any(|o| name_matches(o, sender)))
                    })
                    .collect();
                if found.len() == 1 {
                    return Ok(found[0]);
                }
            }
            anyhow::bail!("couldn't find the Teams chat “{chat_label}”")
        }
        None => resolve(chats, n.sender.as_deref().unwrap_or(&n.title)),
    }
}

pub async fn send(app: &AppHandle, chat: &Chat, text: &str) -> anyhow::Result<()> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("the message is empty");
    }
    let state = app.state::<AppState>();
    let token = calendar::access_token(app).await?;
    let resp = state
        .http
        .post(format!("{}/chats/{}/messages", graph::GRAPH, chat.id))
        .bearer_auth(&token)
        .json(&json!({ "body": { "contentType": "text", "content": text } }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        return Err(explain(anyhow::anyhow!(
            "Teams {status}: {}",
            v["error"]["message"].as_str().unwrap_or("couldn't send")
        )));
    }
    // Keep a note in Fanta's chat so she (and you) know what was sent.
    let note = format!("✓ Sent on Teams to {}: “{text}”", chat.label());
    db::insert_message(&state.db.lock().unwrap(), "assistant", &note)?;
    let _ = app.emit("chat://note", note);
    Ok(())
}

/// Reply to a captured Teams notification with text the user wrote themselves.
pub async fn reply_to_notification(app: &AppHandle, notification_id: i64, text: &str) -> anyhow::Result<String> {
    let n = db::get_notification(&app.state::<AppState>().db.lock().unwrap(), notification_id)?
        .ok_or_else(|| anyhow::anyhow!("that message is gone"))?;
    if n.source != "Teams" {
        anyhow::bail!("replying is only set up for Teams");
    }
    let all = chats(app, false).await?;
    let chat = match chat_for_notification(&all, &n) {
        Ok(c) => c.clone(),
        // A brand-new chat may not be in the cached list yet.
        Err(_) => chat_for_notification(&chats(app, true).await?, &n)?.clone(),
    };
    send(app, &chat, text).await?;
    Ok(chat.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, kind: &str, topic: Option<&str>, others: &[&str]) -> Chat {
        Chat {
            id: id.into(),
            kind: kind.into(),
            topic: topic.map(String::from),
            others: others.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn sample() -> Vec<Chat> {
        vec![
            chat("1", "oneOnOne", None, &["Ravi Kumar"]),
            chat("2", "oneOnOne", None, &["Akilesh Murali"]),
            chat("3", "group", None, &["Aarthi Selvakumari", "Akilesh Murali", "Nanthakumar R", "Priyadharshini S"]),
            chat("4", "group", Some("Creator Studio"), &["Akilesh Murali", "Kiran"]),
            chat("5", "oneOnOne", None, &["Ravi Shankar"]),
        ]
    }

    #[test]
    fn finds_people_and_chats() {
        let c = sample();
        assert_eq!(resolve(&c, "Akilesh").unwrap().id, "2");
        assert_eq!(resolve(&c, "ravi kumar").unwrap().id, "1");
        assert_eq!(resolve(&c, "Creator Studio").unwrap().id, "4");
        assert_eq!(resolve(&c, "Aarthi Selvakumari, +3").unwrap().id, "3");
        let err = resolve(&c, "Ravi").unwrap_err().to_string();
        assert!(err.contains("Ravi Kumar") && err.contains("Ravi Shankar"), "{err}");
        assert!(resolve(&c, "Nobody").is_err());
    }

    #[test]
    fn group_chat_from_notification() {
        let c = sample();
        let mut n = db::Notification {
            id: 1,
            app: "Google Chrome".into(),
            source: "Teams".into(),
            title: "Aarthi Selvakumari, +3".into(),
            body: String::new(),
            sender: Some("Akilesh".into()),
            chat: Some("Aarthi Selvakumari, +3".into()),
            text: Some("did you push?".into()),
            category: "fyi".into(),
            summary: None,
            reason: None,
            during_focus: false,
            seen: false,
            received_at: String::new(),
        };
        assert_eq!(chat_for_notification(&c, &n).unwrap().id, "3");
        n.chat = None;
        n.sender = Some("Ravi Kumar".into());
        assert_eq!(chat_for_notification(&c, &n).unwrap().id, "1");
    }
}
