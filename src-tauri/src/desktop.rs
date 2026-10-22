//! Linux/GNOME desktop integration: notifications, Do Not Disturb, idle time.
//! Everything here degrades to a no-op if the desktop doesn't support it.

use rusqlite::Connection;
use std::process::Command;

use crate::db;

const DND_SCHEMA: &str = "org.gnome.desktop.notifications";
const DND_KEY: &str = "show-banners";
/// Remembers the user's own banner setting while Fanta has DND switched on.
const DND_PREV: &str = "dnd_prev_show_banners";

/// App name on Fanta's own notifications; the notification monitor skips these.
pub const OWN_APP_NAME: &str = "Fanta";

/// Shows a desktop notification. `urgent` uses critical urgency, which GNOME
/// still displays while Do Not Disturb is on.
pub fn notify(title: &str, body: &str, urgent: bool) {
    let (title, body) = (title.to_string(), body.to_string());
    // The D-Bus call blocks briefly; keep it off the async runtime.
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname(OWN_APP_NAME).summary(&title).body(&body);
        if urgent {
            n.urgency(notify_rust::Urgency::Critical);
        }
        if let Err(e) = n.show() {
            eprintln!("notification failed: {e}");
        }
    });
}

fn gsettings(args: &[&str]) -> Option<String> {
    let out = Command::new("gsettings").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Turns on GNOME Do Not Disturb, remembering the previous setting.
pub fn dnd_on(conn: &Connection) {
    if db::get_setting(conn, DND_PREV).ok().flatten().is_none() {
        if let Some(cur) = gsettings(&["get", DND_SCHEMA, DND_KEY]) {
            let _ = db::set_setting(conn, DND_PREV, &cur);
        }
    }
    gsettings(&["set", DND_SCHEMA, DND_KEY, "false"]);
}

/// Restores the user's banner setting if Fanta changed it.
pub fn dnd_off(conn: &Connection) {
    if let Ok(Some(prev)) = db::get_setting(conn, DND_PREV) {
        gsettings(&["set", DND_SCHEMA, DND_KEY, &prev]);
        let _ = db::delete_setting(conn, DND_PREV);
    }
}

/// Milliseconds since the last keyboard/mouse input, via GNOME's Mutter idle monitor.
pub fn idle_ms() -> Option<u64> {
    let out = Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.gnome.Mutter.IdleMonitor",
            "--object-path", "/org/gnome/Mutter/IdleMonitor/Core",
            "--method", "org.gnome.Mutter.IdleMonitor.GetIdletime",
        ])
        .output()
        .ok()?;
    // Output looks like: (uint64 12345,)
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1)?.trim_end_matches(",)").parse().ok()
}
