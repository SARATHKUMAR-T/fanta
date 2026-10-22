//! Read-only fallback: an Outlook "publish calendar" ICS feed, with recurring
//! events expanded locally.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::collections::HashMap;

use crate::db::{self, NewCalEvent};

#[derive(Debug, Clone)]
struct Prop {
    name: String,
    params: HashMap<String, String>,
    value: String,
}

/// Joins folded lines (continuations start with a space or tab).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    lines
}

fn parse_prop(line: &str) -> Option<Prop> {
    // NAME;PARAM=a;PARAM2="b:c":VALUE - the first ':' outside quotes splits.
    let mut in_quotes = false;
    let split = line.char_indices().find(|&(_, c)| {
        if c == '"' {
            in_quotes = !in_quotes;
        }
        c == ':' && !in_quotes
    })?;
    let (head, value) = (&line[..split.0], &line[split.0 + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    Some(Prop { name, params, value: value.to_string() })
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Outlook writes Windows zone names; map the common ones to IANA.
fn iana_zone(tzid: &str) -> Option<chrono_tz::Tz> {
    if let Ok(tz) = tzid.parse::<chrono_tz::Tz>() {
        return Some(tz);
    }
    let iana = match tzid {
        "India Standard Time" => "Asia/Kolkata",
        "Sri Lanka Standard Time" => "Asia/Colombo",
        "Pacific Standard Time" => "America/Los_Angeles",
        "Mountain Standard Time" => "America/Denver",
        "Central Standard Time" => "America/Chicago",
        "Eastern Standard Time" => "America/New_York",
        "GMT Standard Time" => "Europe/London",
        "Greenwich Standard Time" => "Atlantic/Reykjavik",
        "W. Europe Standard Time" => "Europe/Berlin",
        "Central Europe Standard Time" => "Europe/Budapest",
        "Central European Standard Time" => "Europe/Warsaw",
        "Romance Standard Time" => "Europe/Paris",
        "E. Europe Standard Time" => "Europe/Chisinau",
        "FLE Standard Time" => "Europe/Kiev",
        "Arabian Standard Time" => "Asia/Dubai",
        "Singapore Standard Time" => "Asia/Singapore",
        "China Standard Time" => "Asia/Shanghai",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "New Zealand Standard Time" => "Pacific/Auckland",
        "UTC" | "Coordinated Universal Time" => "UTC",
        _ => return None,
    };
    iana.parse().ok()
}

/// The machine's zone, used when an event's TZID is unknown.
fn local_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone().ok().and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC)
}

/// A DTSTART/DTEND-style value. Returns the instant, its zone, and whether it's a date.
fn parse_dt(p: &Prop) -> Option<(DateTime<Utc>, chrono_tz::Tz, bool)> {
    let v = p.value.trim();
    if p.params.get("VALUE").map(String::as_str) == Some("DATE") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        let tz = local_zone();
        let dt = tz.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest()?;
        return Some((dt.with_timezone(&Utc), tz, true));
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((n.and_utc(), chrono_tz::UTC, false));
    }
    let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let tz = p.params.get("TZID").and_then(|z| iana_zone(z)).unwrap_or_else(local_zone);
    let dt = tz.from_local_datetime(&n).earliest()?;
    Some((dt.with_timezone(&Utc), tz, false))
}

fn local_ts(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&Local).format(db::TS).to_string()
}

/// Rewrites a DTSTART/EXDATE value for the rrule crate (IANA zone or UTC).
fn rrule_line(name: &str, dt: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    if tz == chrono_tz::UTC {
        format!("{name}:{}", dt.format("%Y%m%dT%H%M%SZ"))
    } else {
        format!("{name};TZID={}:{}", tz.name(), dt.with_timezone(&tz).format("%Y%m%dT%H%M%S"))
    }
}

/// rrule requires UNTIL in UTC when DTSTART has a zone; Outlook sometimes sends a bare date.
fn normalize_rrule(rule: &str) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some(("UNTIL", v)) if v.len() == 8 => format!("UNTIL={v}T235959Z"),
            Some(("UNTIL", v)) if !v.ends_with('Z') => format!("UNTIL={v}Z"),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn busy_status(v: &str) -> &'static str {
    match v.to_ascii_uppercase().as_str() {
        "FREE" => "free",
        "TENTATIVE" => "tentative",
        "OOF" => "oof",
        "WORKINGELSEWHERE" => "workingElsewhere",
        _ => "busy",
    }
}

fn find_join_url(text: &str) -> Option<String> {
    let start = text.find("https://teams.microsoft.com/l/meetup-join/")?;
    let rest = &text[start..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '"' || c == '<').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

struct VEvent {
    props: Vec<Prop>,
}

impl VEvent {
    fn get(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }
    fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(|p| unescape(&p.value)).filter(|s| !s.trim().is_empty())
    }
}

fn base_event(ev: &VEvent, uid: &str, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) -> NewCalEvent {
    let description = ev.text("DESCRIPTION").unwrap_or_default();
    NewCalEvent {
        remote_id: format!("{uid}|{}", start.timestamp()),
        subject: ev.text("SUMMARY").unwrap_or_else(|| "(no title)".into()),
        start_at: local_ts(start),
        end_at: local_ts(end),
        all_day,
        location: ev.text("LOCATION"),
        join_url: ev
            .text("X-MICROSOFT-SKYPETEAMSMEETINGURL")
            .or_else(|| find_join_url(&description)),
        organizer: ev
            .get("ORGANIZER")
            .and_then(|p| p.params.get("CN").cloned().or_else(|| Some(p.value.trim_start_matches("mailto:").to_string()))),
        response: None,
        show_as: Some(busy_status(&ev.text("X-MICROSOFT-CDO-BUSYSTATUS").unwrap_or_default()).to_string()),
        cancelled: ev.text("STATUS").is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED")),
    }
}

/// Parses a feed into event instances overlapping [from, to).
pub fn parse(text: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<NewCalEvent> {
    let mut events: Vec<VEvent> = vec![];
    let mut current: Option<Vec<Prop>> = None;
    for line in unfold(text) {
        match line.as_str() {
            "BEGIN:VEVENT" => current = Some(vec![]),
            "END:VEVENT" => {
                if let Some(props) = current.take() {
                    events.push(VEvent { props });
                }
            }
            _ => {
                if let (Some(props), Some(p)) = (current.as_mut(), parse_prop(&line)) {
                    props.push(p);
                }
            }
        }
    }

    let mut out: Vec<NewCalEvent> = vec![];
    // Moved/edited single instances of a series: (uid, original start) -> replacement.
    let mut overrides: HashMap<(String, i64), &VEvent> = HashMap::new();
    for ev in &events {
        if let (Some(uid), Some(rid)) = (ev.text("UID"), ev.get("RECURRENCE-ID").and_then(parse_dt)) {
            overrides.insert((uid, rid.0.timestamp()), ev);
        }
    }

    for ev in &events {
        if ev.get("RECURRENCE-ID").is_some() {
            continue;
        }
        let (Some(uid), Some((start, tz, all_day))) = (ev.text("UID"), ev.get("DTSTART").and_then(parse_dt)) else {
            continue;
        };
        let end = ev.get("DTEND").and_then(parse_dt).map(|e| e.0).unwrap_or(start + Duration::hours(if all_day { 24 } else { 1 }));
        let length = end - start;

        let starts: Vec<DateTime<Utc>> = match ev.get("RRULE") {
            None => vec![start],
            Some(rule) => {
                let mut spec = vec![rrule_line("DTSTART", start, tz), format!("RRULE:{}", normalize_rrule(&rule.value))];
                for ex in ev.props.iter().filter(|p| p.name == "EXDATE") {
                    for v in ex.value.split(',') {
                        let p = Prop { value: v.to_string(), ..ex.clone() };
                        if let Some((dt, _, _)) = parse_dt(&p) {
                            spec.push(rrule_line("EXDATE", dt, tz));
                        }
                    }
                }
                match spec.join("\n").parse::<rrule::RRuleSet>() {
                    Ok(set) => set
                        .after((from - length).with_timezone(&rrule::Tz::UTC))
                        .before(to.with_timezone(&rrule::Tz::UTC))
                        .all(500)
                        .dates
                        .into_iter()
                        .map(|d| d.with_timezone(&Utc))
                        .collect(),
                    Err(e) => {
                        eprintln!("ics: skipping recurrence for '{}': {e}", ev.text("SUMMARY").unwrap_or_default());
                        vec![start]
                    }
                }
            }
        };

        for s in starts {
            if let Some(o) = overrides.remove(&(uid.clone(), s.timestamp())) {
                if let Some((os, _, oday)) = o.get("DTSTART").and_then(parse_dt) {
                    let oe = o.get("DTEND").and_then(parse_dt).map(|e| e.0).unwrap_or(os + length);
                    out.push(base_event(o, &uid, os, oe, oday));
                }
            } else {
                out.push(base_event(ev, &uid, s, s + length, all_day));
            }
        }
    }
    // Overrides whose series wasn't in the feed still count as events.
    for ((uid, _), o) in overrides {
        if let Some((os, _, oday)) = o.get("DTSTART").and_then(parse_dt) {
            let oe = o.get("DTEND").and_then(parse_dt).map(|e| e.0).unwrap_or(os + Duration::hours(1));
            out.push(base_event(o, &uid, os, oe, oday));
        }
    }

    let (from_ts, to_ts) = (local_ts(from), local_ts(to));
    out.retain(|e| e.start_at < to_ts && e.end_at > from_ts);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:standup-1\r
SUMMARY:Daily standup\r
DTSTART;TZID=India Standard Time:20261019T100000\r
DTEND;TZID=India Standard Time:20261019T101500\r
RRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;UNTIL=20261231\r
EXDATE;TZID=India Standard Time:20261021T100000\r
DESCRIPTION:Join: https://teams.microsoft.com/l/meetup-join/abc123 \\n see\r
  you\r
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:standup-1\r
RECURRENCE-ID;TZID=India Standard Time:20261022T100000\r
SUMMARY:Daily standup (moved)\r
DTSTART;TZID=India Standard Time:20261022T113000\r
DTEND;TZID=India Standard Time:20261022T114500\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:offsite\r
SUMMARY:Team offsite\r
DTSTART;VALUE=DATE:20261023\r
DTEND;VALUE=DATE:20261024\r
X-MICROSOFT-CDO-BUSYSTATUS:FREE\r
END:VEVENT\r
END:VCALENDAR\r
";

    fn ist(y: i32, m: u32, d: u32, h: u32, min: u32) -> String {
        let tz: chrono_tz::Tz = "Asia/Kolkata".parse().unwrap();
        local_ts(tz.with_ymd_and_hms(y, m, d, h, min, 0).unwrap().with_timezone(&Utc))
    }

    #[test]
    fn expands_recurrence_with_exdate_and_override() {
        let from = Utc.with_ymd_and_hms(2026, 10, 19, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        let mut evs = parse(FEED, from, to);
        evs.sort_by(|a, b| a.start_at.cmp(&b.start_at));
        let standups: Vec<_> = evs.iter().filter(|e| e.subject.starts_with("Daily")).collect();
        // Mon, Tue, (Wed excluded), Thu moved, Fri
        assert_eq!(standups.len(), 4);
        assert_eq!(standups[0].start_at, ist(2026, 10, 19, 10, 0));
        assert!(!standups.iter().any(|e| e.start_at == ist(2026, 10, 21, 10, 0)));
        let moved = standups.iter().find(|e| e.subject.ends_with("(moved)")).unwrap();
        assert_eq!(moved.start_at, ist(2026, 10, 22, 11, 30));
        assert_eq!(standups[0].join_url.as_deref(), Some("https://teams.microsoft.com/l/meetup-join/abc123"));

        let offsite = evs.iter().find(|e| e.subject == "Team offsite").unwrap();
        assert!(offsite.all_day);
        assert_eq!(offsite.show_as.as_deref(), Some("free"));
    }
}
