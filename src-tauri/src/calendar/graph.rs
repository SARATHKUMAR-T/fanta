//! Microsoft Graph: device-code sign-in and calendar read/write.
//! Uses a public-client app registration, so no secret is shipped.

use chrono::{DateTime, Local, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{self, NewCalEvent};

/// Calendar plus Teams chats (find chats, send in them). None of these need admin
/// consent by default, though a tenant can still require it.
const SCOPES: &str = "offline_access User.Read Calendars.ReadWrite Chat.ReadBasic ChatMessage.Send";
pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";

#[derive(Debug, Clone)]
pub struct App {
    pub client_id: String,
    /// "organizations", "common", or a tenant id/domain.
    pub tenant: String,
}

impl App {
    fn authority(&self) -> String {
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0", self.tenant)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

/// Plain-language fixes for the sign-in errors people actually hit.
fn hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "AADSTS50059" | "AADSTS50194" => "Paste the Directory (tenant) ID from the app's Overview page into the Tenant field.",
        "AADSTS700016" => "That client ID wasn't found in this tenant - check both IDs.",
        "AADSTS7000218" => "In the app registration, turn on Authentication → Allow public client flows.",
        "AADSTS65001" | "AADSTS90094" | "AADSTS90095" => {
            "Your organization requires an admin to approve Fanta. Ask IT to grant consent, or use the ICS link instead."
        }
        "AADSTS53003" => "Blocked by your organization's Conditional Access policy - ask IT, or use the ICS link.",
        _ => return None,
    })
}

fn oauth_error(body: &str) -> anyhow::Error {
    match serde_json::from_str::<OAuthError>(body) {
        Ok(e) => {
            // First line of Microsoft's description carries the AADSTS code and reason.
            let desc = e.error_description.unwrap_or_default();
            let first = desc.lines().next().unwrap_or("");
            let reason = first.split(" Trace ID").next().unwrap_or(first);
            let code = reason.split(':').next().unwrap_or("");
            match hint(code) {
                Some(h) => anyhow::anyhow!("{h} ({code})"),
                None => anyhow::anyhow!("{}: {reason}", e.error),
            }
        }
        Err(_) => anyhow::anyhow!("sign-in failed: {body}"),
    }
}

pub async fn begin_device_code(http: &reqwest::Client, app: &App) -> anyhow::Result<DeviceCode> {
    let resp = http
        .post(format!("{}/devicecode", app.authority()))
        .form(&[("client_id", app.client_id.as_str()), ("scope", SCOPES)])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(oauth_error(&body));
    }
    Ok(serde_json::from_str(&body)?)
}

/// Polls until the user finishes signing in, the code expires, or they decline.
pub async fn poll_device_code(http: &reqwest::Client, app: &App, code: &DeviceCode) -> anyhow::Result<Tokens> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        let resp = http
            .post(format!("{}/token", app.authority()))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", app.client_id.as_str()),
                ("device_code", code.device_code.as_str()),
            ])
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if status.is_success() {
            return Ok(serde_json::from_str(&body)?);
        }
        match serde_json::from_str::<OAuthError>(&body).map(|e| e.error).as_deref() {
            Ok("authorization_pending") => {}
            Ok("slow_down") => interval += 5,
            _ => return Err(oauth_error(&body)),
        }
    }
    anyhow::bail!("the sign-in code expired - try again")
}

pub async fn refresh(http: &reqwest::Client, app: &App, refresh_token: &str) -> anyhow::Result<Tokens> {
    let resp = http
        .post(format!("{}/token", app.authority()))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", app.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(oauth_error(&body));
    }
    Ok(serde_json::from_str(&body)?)
}

pub async fn get_json(http: &reqwest::Client, token: &str, url: &str) -> anyhow::Result<Value> {
    let resp = http.get(url).bearer_auth(token).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Graph {status}: {}", body["error"]["message"].as_str().unwrap_or("request failed"));
    }
    Ok(body)
}

/// Display name and email of the signed-in user.
pub async fn me(http: &reqwest::Client, token: &str) -> anyhow::Result<String> {
    let me = get_json(http, token, &format!("{GRAPH}/me?$select=displayName,mail,userPrincipalName")).await?;
    let email = me["mail"].as_str().or(me["userPrincipalName"].as_str()).unwrap_or("");
    Ok(format!("{} <{}>", me["displayName"].as_str().unwrap_or("Outlook"), email))
}

/// Graph returns UTC wall times like "2026-10-22T09:30:00.0000000" when no
/// timezone preference is sent.
fn utc_to_local_ts(s: &str) -> Option<String> {
    let trimmed = s.split('.').next()?;
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some(naive.and_utc().with_timezone(&Local).format(db::TS).to_string())
}

fn local_ts_to_graph(ts: &str) -> anyhow::Result<Value> {
    let naive = NaiveDateTime::parse_from_str(ts, db::TS)?;
    let local = naive
        .and_local_timezone(Local)
        .earliest()
        .ok_or_else(|| anyhow::anyhow!("invalid local time {ts}"))?;
    Ok(json!({ "dateTime": local.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC" }))
}

fn to_event(v: &Value) -> Option<NewCalEvent> {
    let all_day = v["isAllDay"].as_bool().unwrap_or(false);
    let (start_at, end_at) = if all_day {
        // All-day events are midnight-to-midnight in the event's own zone; keep the dates.
        let d = |k: &str| v[k]["dateTime"].as_str().map(|s| format!("{} 00:00:00", &s[..10]));
        (d("start")?, d("end")?)
    } else {
        (utc_to_local_ts(v["start"]["dateTime"].as_str()?)?, utc_to_local_ts(v["end"]["dateTime"].as_str()?)?)
    };
    let text = |p: &Value| p.as_str().map(str::to_string).filter(|s| !s.is_empty());
    Some(NewCalEvent {
        remote_id: v["id"].as_str()?.to_string(),
        subject: text(&v["subject"]).unwrap_or_else(|| "(no title)".into()),
        start_at,
        end_at,
        all_day,
        location: text(&v["location"]["displayName"]),
        join_url: text(&v["onlineMeeting"]["joinUrl"]),
        organizer: text(&v["organizer"]["emailAddress"]["name"]),
        response: text(&v["responseStatus"]["response"]),
        show_as: text(&v["showAs"]),
        cancelled: v["isCancelled"].as_bool().unwrap_or(false),
    })
}

/// All event instances (recurrences expanded) overlapping the window.
pub async fn calendar_view(
    http: &reqwest::Client,
    token: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<NewCalEvent>> {
    let fmt = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut url = format!(
        "{GRAPH}/me/calendarView?startDateTime={}&endDateTime={}&$top=100&$orderby=start/dateTime\
         &$select=id,subject,start,end,isAllDay,location,onlineMeeting,organizer,responseStatus,showAs,isCancelled",
        fmt(from),
        fmt(to)
    );
    let mut out = vec![];
    for _ in 0..20 {
        let page = get_json(http, token, &url).await?;
        out.extend(page["value"].as_array().into_iter().flatten().filter_map(to_event));
        match page["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

pub struct NewMeeting {
    pub subject: String,
    pub start_at: String,
    pub end_at: String,
    pub attendees: Vec<String>,
    pub online: bool,
}

pub async fn create_event(http: &reqwest::Client, token: &str, m: &NewMeeting) -> anyhow::Result<NewCalEvent> {
    let mut body = json!({
        "subject": m.subject,
        "start": local_ts_to_graph(&m.start_at)?,
        "end": local_ts_to_graph(&m.end_at)?,
        "attendees": m.attendees.iter().map(|a| json!({
            "emailAddress": { "address": a }, "type": "required"
        })).collect::<Vec<_>>(),
    });
    if m.online {
        body["isOnlineMeeting"] = json!(true);
        body["onlineMeetingProvider"] = json!("teamsForBusiness");
    }
    let resp = http.post(format!("{GRAPH}/me/events")).bearer_auth(token).json(&body).send().await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Graph {status}: {}", v["error"]["message"].as_str().unwrap_or("couldn't create event"));
    }
    to_event(&v).ok_or_else(|| anyhow::anyhow!("unexpected response creating event"))
}

/// `response` is "accept", "decline", or "tentativelyAccept".
pub async fn respond(http: &reqwest::Client, token: &str, remote_id: &str, response: &str, comment: &str) -> anyhow::Result<()> {
    let resp = http
        .post(format!("{GRAPH}/me/events/{}/{response}", urlencoding::encode(remote_id)))
        .bearer_auth(token)
        .json(&json!({ "comment": comment, "sendResponse": true }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        anyhow::bail!("Graph {status}: {}", v["error"]["message"].as_str().unwrap_or("couldn't respond"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_tenant_error() {
        let body = r#"{"error":"invalid_request","error_description":"AADSTS50059: No tenant-identifying information found. Trace ID: x"}"#;
        assert!(oauth_error(body).to_string().starts_with("Paste the Directory (tenant) ID"));
    }

    #[test]
    fn parses_graph_event() {
        let v = json!({
            "id": "AAMk1", "subject": "Standup", "isAllDay": false, "isCancelled": false,
            "start": { "dateTime": "2026-10-22T04:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-10-22T04:15:00.0000000", "timeZone": "UTC" },
            "onlineMeeting": { "joinUrl": "https://teams.microsoft.com/l/meetup-join/x" },
            "responseStatus": { "response": "accepted" }, "showAs": "busy",
            "location": { "displayName": "" }, "organizer": { "emailAddress": { "name": "Ravi" } }
        });
        let e = to_event(&v).unwrap();
        assert_eq!(e.subject, "Standup");
        assert_eq!(e.location, None);
        assert_eq!(e.join_url.as_deref(), Some("https://teams.microsoft.com/l/meetup-join/x"));
        let expected = NaiveDateTime::parse_from_str("2026-10-22 04:00:00", db::TS)
            .unwrap()
            .and_utc()
            .with_timezone(&Local)
            .format(db::TS)
            .to_string();
        assert_eq!(e.start_at, expected);
    }
}
