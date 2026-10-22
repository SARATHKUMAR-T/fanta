//! Pomodoro analytics, computed from the focus_sessions history.

use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db;

/// Minutes actually spent in a session: wall time minus pauses.
const ACTUAL_MINUTES: &str = "MAX(0, (julianday(COALESCE(ended_at, ?1)) - julianday(started_at)) * 1440 - paused_secs / 60.0)";

#[derive(Serialize, Default)]
pub struct Today {
    pub pomodoros: i64,
    pub focus_minutes: i64,
    pub break_minutes: i64,
    /// Focus blocks stopped before the timer ran out.
    pub interrupted: i64,
    pub goal: i64,
}

#[derive(Serialize)]
pub struct Day {
    pub date: String,
    /// "Mon", "Tue", ...
    pub label: String,
    pub pomodoros: i64,
    pub minutes: i64,
    pub is_today: bool,
}

#[derive(Serialize, Default)]
pub struct Period {
    pub pomodoros: i64,
    pub focus_minutes: i64,
    /// Completed / (completed + stopped early), 0..1.
    pub completion_rate: f64,
    pub active_days: i64,
    pub avg_per_active_day: f64,
}

#[derive(Serialize)]
pub struct TaskStat {
    pub title: String,
    pub pomodoros: i64,
    pub minutes: i64,
}

#[derive(Serialize)]
pub struct HistoryItem {
    pub id: i64,
    pub kind: String,
    pub task_title: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub planned_minutes: i64,
    pub actual_minutes: i64,
    pub completed: bool,
}

#[derive(Serialize)]
pub struct Stats {
    pub today: Today,
    /// Monday to Sunday of the current week.
    pub week: Vec<Day>,
    pub week_pomodoros: i64,
    pub week_minutes: i64,
    /// Days in a row (up to today) with at least one pomodoro.
    pub streak: i64,
    pub best_streak: i64,
    pub last30: Period,
    pub by_task: Vec<TaskStat>,
    /// Completed pomodoros by starting hour (0-23), last 30 days.
    pub by_hour: Vec<i64>,
    pub history: Vec<HistoryItem>,
}

fn per_day(conn: &Connection, now: &str, from: &str, to: &str) -> anyhow::Result<Vec<(String, i64, i64)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT date(started_at) d,
                SUM(CASE WHEN completed = 1 THEN 1 ELSE 0 END),
                CAST(ROUND(SUM({ACTUAL_MINUTES})) AS INTEGER)
         FROM focus_sessions
         WHERE kind = 'focus' AND date(started_at) BETWEEN ?2 AND ?3
         GROUP BY d ORDER BY d"
    ))?;
    let rows = stmt.query_map(params![now, from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// (current streak, best streak) from the dates that had a completed pomodoro.
fn streaks(dates: &[NaiveDate], today: NaiveDate) -> (i64, i64) {
    let mut best = 0;
    let mut run = 0;
    let mut prev: Option<NaiveDate> = None;
    for &d in dates {
        run = if prev.is_some_and(|p| d - p == Duration::days(1)) { run + 1 } else { 1 };
        best = best.max(run);
        prev = Some(d);
    }
    // The current streak survives a day that hasn't had a pomodoro *yet*.
    let current = match dates.last() {
        Some(&last) if last == today || last == today - Duration::days(1) => run,
        _ => 0,
    };
    (current, best)
}

pub fn compute(conn: &Connection, goal: i64) -> anyhow::Result<Stats> {
    let now = db::now_ts();
    let today = Local::now().date_naive();
    let d = |x: NaiveDate| x.format("%Y-%m-%d").to_string();

    // Today
    let mut t = conn.query_row(
        &format!(
            "SELECT
               COALESCE(SUM(CASE WHEN kind = 'focus' AND completed = 1 THEN 1 ELSE 0 END), 0),
               CAST(ROUND(COALESCE(SUM(CASE WHEN kind = 'focus' THEN {ACTUAL_MINUTES} ELSE 0 END), 0)) AS INTEGER),
               CAST(ROUND(COALESCE(SUM(CASE WHEN kind != 'focus' THEN {ACTUAL_MINUTES} ELSE 0 END), 0)) AS INTEGER),
               COALESCE(SUM(CASE WHEN kind = 'focus' AND completed = 0 AND ended_at IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM focus_sessions WHERE date(started_at) = ?2"
        ),
        params![now, d(today)],
        |r| Ok(Today { pomodoros: r.get(0)?, focus_minutes: r.get(1)?, break_minutes: r.get(2)?, interrupted: r.get(3)?, goal: 0 }),
    )?;
    t.goal = goal;

    // This week (Mon..Sun)
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let rows = per_day(conn, &now, &d(monday), &d(monday + Duration::days(6)))?;
    let week: Vec<Day> = (0..7)
        .map(|i| {
            let date = monday + Duration::days(i);
            let (p, m) = rows.iter().find(|r| r.0 == d(date)).map(|r| (r.1, r.2)).unwrap_or((0, 0));
            Day { date: d(date), label: date.format("%a").to_string(), pomodoros: p, minutes: m, is_today: date == today }
        })
        .collect();

    // Last 30 days
    let from30 = d(today - Duration::days(29));
    let last30 = conn.query_row(
        &format!(
            "SELECT
               COALESCE(SUM(CASE WHEN completed = 1 THEN 1 ELSE 0 END), 0),
               CAST(ROUND(COALESCE(SUM({ACTUAL_MINUTES}), 0)) AS INTEGER),
               COALESCE(SUM(CASE WHEN completed = 0 AND ended_at IS NOT NULL THEN 1 ELSE 0 END), 0),
               COUNT(DISTINCT CASE WHEN completed = 1 THEN date(started_at) END)
             FROM focus_sessions WHERE kind = 'focus' AND date(started_at) >= ?2"
        ),
        params![now, from30],
        |r| {
            let (done, minutes, stopped, days): (i64, i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
            Ok(Period {
                pomodoros: done,
                focus_minutes: minutes,
                completion_rate: if done + stopped > 0 { done as f64 / (done + stopped) as f64 } else { 0.0 },
                active_days: days,
                avg_per_active_day: if days > 0 { done as f64 / days as f64 } else { 0.0 },
            })
        },
    )?;

    // Streaks (all time)
    let dates: Vec<NaiveDate> = conn
        .prepare("SELECT DISTINCT date(started_at) FROM focus_sessions WHERE kind = 'focus' AND completed = 1 ORDER BY 1")?
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok().and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
        .collect();
    let (streak, best_streak) = streaks(&dates, today);

    // By task, last 30 days
    let by_task = conn
        .prepare(&format!(
            "SELECT COALESCE(t.title, 'No task'),
                    SUM(CASE WHEN f.completed = 1 THEN 1 ELSE 0 END),
                    CAST(ROUND(SUM({})) AS INTEGER)
             FROM focus_sessions f LEFT JOIN tasks t ON t.id = f.task_id
             WHERE f.kind = 'focus' AND date(f.started_at) >= ?2
             GROUP BY COALESCE(t.title, 'No task') ORDER BY 2 DESC, 3 DESC LIMIT 6",
            ACTUAL_MINUTES.replace("ended_at", "f.ended_at").replace("started_at", "f.started_at").replace("paused_secs", "f.paused_secs")
        ))?
        .query_map(params![now, from30], |r| Ok(TaskStat { title: r.get(0)?, pomodoros: r.get(1)?, minutes: r.get(2)? }))?
        .collect::<Result<Vec<_>, _>>()?;

    // Best hours, last 30 days
    let mut by_hour = vec![0i64; 24];
    let mut stmt = conn.prepare(
        "SELECT CAST(strftime('%H', started_at) AS INTEGER), COUNT(*) FROM focus_sessions
         WHERE kind = 'focus' AND completed = 1 AND date(started_at) >= ?1 GROUP BY 1",
    )?;
    for row in stmt.query_map(params![from30], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (h, n) = row?;
        if (0..24).contains(&h) {
            by_hour[h as usize] = n;
        }
    }

    // Session log
    let history = conn
        .prepare(&format!(
            "SELECT f.id, f.kind, t.title, f.started_at, f.ended_at, COALESCE(f.planned_minutes, 0),
                    CAST(ROUND({}) AS INTEGER), f.completed
             FROM focus_sessions f LEFT JOIN tasks t ON t.id = f.task_id
             WHERE f.ended_at IS NOT NULL ORDER BY f.id DESC LIMIT 80",
            ACTUAL_MINUTES.replace("ended_at", "f.ended_at").replace("started_at", "f.started_at").replace("paused_secs", "f.paused_secs")
        ))?
        .query_map(params![now], |r| {
            Ok(HistoryItem {
                id: r.get(0)?,
                kind: r.get(1)?,
                task_title: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
                planned_minutes: r.get(5)?,
                actual_minutes: r.get(6)?,
                completed: r.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Stats {
        week_pomodoros: week.iter().map(|d| d.pomodoros).sum(),
        week_minutes: week.iter().map(|d| d.minutes).sum(),
        today: t,
        week,
        streak,
        best_streak,
        last30,
        by_task,
        by_hour,
        history,
    })
}

/// A few sentences for the chat model ("how did I do this week?").
pub fn summary(s: &Stats) -> String {
    let best_hour = s.by_hour.iter().enumerate().max_by_key(|(_, n)| **n).filter(|(_, n)| **n > 0).map(|(h, _)| h);
    format!(
        "Today: {} of {} pomodoros ({} min focused, {} stopped early, {} min of breaks).\n\
         This week: {} pomodoros, {} min. Per day: {}.\n\
         Streak: {} day(s) in a row (best {}).\n\
         Last 30 days: {} pomodoros, {:.1} h, {:.0}% completed, {:.1} per active day.{}\n\
         Top tasks (30 days): {}",
        s.today.pomodoros,
        s.today.goal,
        s.today.focus_minutes,
        s.today.interrupted,
        s.today.break_minutes,
        s.week_pomodoros,
        s.week_minutes,
        s.week.iter().map(|d| format!("{} {}", d.label, d.pomodoros)).collect::<Vec<_>>().join(", "),
        s.streak,
        s.best_streak,
        s.last30.pomodoros,
        s.last30.focus_minutes as f64 / 60.0,
        s.last30.completion_rate * 100.0,
        s.last30.avg_per_active_day,
        best_hour.map(|h| format!(" Most productive hour: {h}:00.")).unwrap_or_default(),
        if s.by_task.is_empty() {
            "none yet".into()
        } else {
            s.by_task.iter().map(|t| format!("{} ({})", t.title, t.pomodoros)).collect::<Vec<_>>().join(", ")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT);
             INSERT INTO tasks VALUES (1, 'Q4 report');
             CREATE TABLE focus_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT, task_id INTEGER,
               started_at TEXT, ends_at TEXT, ended_at TEXT, completed INTEGER DEFAULT 0,
               planned_minutes INTEGER, paused_at TEXT, paused_secs INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        c
    }

    fn add(c: &Connection, kind: &str, day_offset: i64, hour: u32, minutes: i64, completed: bool, paused_secs: i64, task: Option<i64>) {
        let start = (Local::now().date_naive() - Duration::days(day_offset)).and_hms_opt(hour, 0, 0).unwrap();
        let end = start + Duration::minutes(minutes) + Duration::seconds(paused_secs);
        c.execute(
            "INSERT INTO focus_sessions (kind, task_id, started_at, ends_at, ended_at, completed, planned_minutes, paused_secs)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, 25, ?6)",
            params![kind, task, start.format(db::TS).to_string(), end.format(db::TS).to_string(), completed, paused_secs],
        )
        .unwrap();
    }

    #[test]
    fn counts_and_minutes() {
        let c = conn();
        add(&c, "focus", 0, 9, 25, true, 0, Some(1));
        add(&c, "short_break", 0, 9, 5, true, 0, None);
        add(&c, "focus", 0, 10, 25, true, 300, Some(1)); // paused 5 min: still 25 focused
        add(&c, "focus", 0, 11, 10, false, 0, None); // stopped early
        add(&c, "focus", 1, 9, 25, true, 0, None);
        add(&c, "focus", 2, 9, 25, true, 0, None);
        add(&c, "focus", 5, 9, 25, true, 0, None); // breaks the streak
        let s = compute(&c, 8).unwrap();
        assert_eq!(s.today.pomodoros, 2);
        assert_eq!(s.today.focus_minutes, 60);
        assert_eq!(s.today.break_minutes, 5);
        assert_eq!(s.today.interrupted, 1);
        assert_eq!(s.streak, 3);
        assert_eq!(s.best_streak, 3);
        assert_eq!(s.last30.pomodoros, 5);
        assert!((s.last30.completion_rate - 5.0 / 6.0).abs() < 1e-9);
        let q4 = s.by_task.iter().find(|t| t.title == "Q4 report").unwrap();
        assert_eq!((q4.pomodoros, q4.minutes), (2, 50));
        assert_eq!(s.by_task[0].title, "No task"); // 3 pomodoros beats 2
        assert_eq!(s.by_hour[9], 4);
        assert_eq!(s.week.len(), 7);
        assert_eq!(s.week.iter().filter(|d| d.is_today).count(), 1);
        assert_eq!(s.history.len(), 7);
        assert!(summary(&s).contains("Today: 2 of 8"));
    }

    #[test]
    fn streak_survives_a_day_not_started_yet() {
        let t = Local::now().date_naive();
        let days = [t - Duration::days(3), t - Duration::days(2), t - Duration::days(1)];
        assert_eq!(streaks(&days, t), (3, 3));
        assert_eq!(streaks(&days[..2], t), (0, 2));
    }
}
