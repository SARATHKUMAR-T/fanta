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
    notify_with_link(title, body, urgent, None);
}

/// Like `notify`, with a "Join" button (and click) that opens `link`.
pub fn notify_with_link(title: &str, body: &str, urgent: bool, link: Option<&str>) {
    let (title, body, link) = (title.to_string(), body.to_string(), link.map(str::to_string));
    // The D-Bus calls block (waiting for a click blocks until dismissal); keep them off the async runtime.
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname(OWN_APP_NAME).summary(&title).body(&body);
        if urgent {
            n.urgency(notify_rust::Urgency::Critical);
        }
        if link.is_some() {
            n.action("default", "Join");
            n.action("join", "Join");
        }
        match n.show() {
            Ok(handle) => {
                if let Some(url) = link {
                    handle.wait_for_action(|action| {
                        if action == "join" || action == "default" {
                            open_url(&url);
                        }
                    });
                }
            }
            Err(e) => eprintln!("notification failed: {e}"),
        }
    });
}

pub fn open_url(url: &str) {
    if let Err(e) = Command::new("xdg-open").arg(url).spawn() {
        eprintln!("couldn't open {url}: {e}");
    }
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

/// Short chimes Fanta plays herself, so they sound the same on every desktop.
#[derive(Clone, Copy)]
pub enum Chime {
    /// Rising three notes: a focus block is done, time for a break.
    FocusDone,
    /// Two notes: the break is over.
    BreakDone,
}

const CHIME_RATE: u32 = 44_100;

/// Bell-like tone: a few harmonics with a soft attack and exponential decay.
fn synth_chime(chime: Chime) -> Vec<u8> {
    // (start seconds, frequency Hz)
    let notes: &[(f32, f32)] = match chime {
        Chime::FocusDone => &[(0.0, 659.25), (0.16, 830.61), (0.32, 987.77)], // E5 G#5 B5
        Chime::BreakDone => &[(0.0, 783.99), (0.2, 587.33)],                 // G5 D5
    };
    let length = notes.last().map(|n| n.0).unwrap_or(0.0) + 1.4;
    let n = (length * CHIME_RATE as f32) as usize;
    let mut buf = vec![0f32; n];
    for &(start, freq) in notes {
        let s0 = (start * CHIME_RATE as f32) as usize;
        for i in s0..n {
            let t = (i - s0) as f32 / CHIME_RATE as f32;
            let attack = (t / 0.008).min(1.0);
            let decay = (-t * 3.2).exp();
            let tone = (2.0 * std::f32::consts::PI * freq * t).sin()
                + 0.35 * (2.0 * std::f32::consts::PI * freq * 2.0 * t).sin() * (-t * 6.0).exp()
                + 0.12 * (2.0 * std::f32::consts::PI * freq * 3.01 * t).sin() * (-t * 9.0).exp();
            buf[i] += tone * attack * decay;
        }
    }
    let peak = buf.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let gain = 0.32 / peak; // gentle, not startling
    buf.iter().flat_map(|v| ((v * gain * 32767.0) as i16).to_le_bytes()).collect()
}

/// Plays a chime without blocking.
pub fn chime(chime: Chime) {
    std::thread::spawn(move || {
        let pcm = synth_chime(chime);
        let child = Command::new("aplay")
            .args(["-q", "-f", "S16_LE", "-r", &CHIME_RATE.to_string(), "-c", "1", "-t", "raw", "-"])
            .stdin(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut c) => {
                if let Some(mut stdin) = c.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(&pcm);
                }
                let _ = c.wait();
            }
            Err(e) => eprintln!("chime: {e}"),
        }
    });
}

/// Whether chimes are on (Settings → Sounds; on by default).
pub fn sounds_on(conn: &Connection) -> bool {
    db::get_setting(conn, "sounds_enabled").ok().flatten().as_deref() != Some("0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimes_are_short_and_gentle() {
        for c in [Chime::FocusDone, Chime::BreakDone] {
            let pcm = synth_chime(c);
            let samples: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            let secs = samples.len() as f32 / CHIME_RATE as f32;
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!((1.4..2.0).contains(&secs), "{secs}s");
            assert!((9_000..11_500).contains(&peak), "peak {peak}");
            assert!(samples.last().unwrap().unsigned_abs() < 400, "ends quietly");
        }
    }
}
