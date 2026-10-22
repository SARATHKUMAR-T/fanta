//! The floating companion (a small transparent always-on-top window with the
//! character) and the chat panel that opens beside it.

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::{db, AppState};

pub const COMPANION: &str = "companion";
pub const PANEL: &str = "panel";

/// Logical sizes. The companion grows upward when it shows a speech bubble.
const COMPANION_SIZE: (f64, f64) = (170.0, 200.0);
const COMPANION_BUBBLE_SIZE: (f64, f64) = (300.0, 330.0);
const PANEL_SIZE: (f64, f64) = (420.0, 660.0);
const MARGIN: i32 = 24;
const POS_KEY: &str = "companion_pos";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let companion = WebviewWindowBuilder::new(app, COMPANION, WebviewUrl::App("index.html?view=companion".into()))
        .title("Fanta")
        .inner_size(COMPANION_SIZE.0, COMPANION_SIZE.1)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible_on_all_workspaces(true)
        .visible(false)
        .build()?;
    place_companion(app, &companion);
    companion.show()?;

    let panel = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html?view=panel".into()))
        .title("Fanta")
        .inner_size(PANEL_SIZE.0, PANEL_SIZE.1)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()?;

    let app2 = app.clone();
    companion.on_window_event(move |e| match e {
        WindowEvent::Moved(pos) => {
            // Save where the compact window would be, so a bubble resize doesn't shift Fanta on restart.
            if let Some(c) = app2.get_webview_window(COMPANION) {
                if let (Ok(size), Ok(scale)) = (c.outer_size(), c.scale_factor()) {
                    let (cw, ch) = ((COMPANION_SIZE.0 * scale) as i32, (COMPANION_SIZE.1 * scale) as i32);
                    let x = pos.x + (size.width as i32 - cw) / 2;
                    let y = pos.y + size.height as i32 - ch;
                    let _ = db::set_setting(&app2.state::<AppState>().db.lock().unwrap(), POS_KEY, &format!("{x},{y}"));
                }
            }
            if panel_visible(&app2) {
                let _ = position_panel(&app2);
            }
        }
        WindowEvent::CloseRequested { api, .. } => api.prevent_close(),
        _ => {}
    });
    let app3 = app.clone();
    panel.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            let _ = hide_panel(&app3);
        }
    });
    Ok(())
}

/// Restores the saved spot, or defaults to the bottom-right of the screen.
fn place_companion(app: &AppHandle, w: &WebviewWindow) {
    let saved = db::get_setting(&app.state::<AppState>().db.lock().unwrap(), POS_KEY).ok().flatten();
    let pos = saved.and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some(PhysicalPosition::new(x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))
    });
    let monitor = w.primary_monitor().ok().flatten();
    let pos = match (pos, &monitor) {
        // Ignore a saved spot that's now off-screen (e.g. a monitor was unplugged).
        (Some(p), _) if w.available_monitors().unwrap_or_default().iter().any(|m| {
            let (mp, ms) = (m.position(), m.size());
            p.x >= mp.x && p.y >= mp.y && p.x < mp.x + ms.width as i32 - 40 && p.y < mp.y + ms.height as i32 - 40
        }) => p,
        (_, Some(m)) => {
            let scale = m.scale_factor();
            let (mp, ms) = (m.position(), m.size());
            PhysicalPosition::new(
                mp.x + ms.width as i32 - (COMPANION_SIZE.0 * scale) as i32 - MARGIN,
                mp.y + ms.height as i32 - (COMPANION_SIZE.1 * scale) as i32 - MARGIN * 3,
            )
        }
        _ => PhysicalPosition::new(100, 100),
    };
    let _ = w.set_position(pos);
}

fn panel_visible(app: &AppHandle) -> bool {
    app.get_webview_window(PANEL).and_then(|p| p.is_visible().ok()).unwrap_or(false)
}

/// Puts the panel beside the companion, on whichever side has room.
fn position_panel(app: &AppHandle) -> tauri::Result<()> {
    let (Some(c), Some(p)) = (app.get_webview_window(COMPANION), app.get_webview_window(PANEL)) else {
        return Ok(());
    };
    let cpos = c.outer_position()?;
    let csize = c.outer_size()?;
    let psize = p.outer_size()?;
    let Some(m) = c.current_monitor()?.or(c.primary_monitor()?) else { return Ok(()) };
    let (mp, ms) = (m.position(), m.size());
    let (left, right) = (mp.x, mp.x + ms.width as i32);
    let (top, bottom) = (mp.y, mp.y + ms.height as i32);

    let room_left = cpos.x - left;
    let x = if room_left >= psize.width as i32 {
        cpos.x - psize.width as i32 + 10
    } else {
        cpos.x + csize.width as i32 - 10
    };
    // Bottom edges line up, so the panel looks like it grows out of Fanta.
    let y = cpos.y + csize.height as i32 - psize.height as i32;
    let x = x.clamp(left, (right - psize.width as i32).max(left));
    let y = y.clamp(top, (bottom - psize.height as i32).max(top));
    p.set_position(PhysicalPosition::new(x, y))
}

pub fn show_panel(app: &AppHandle) -> tauri::Result<()> {
    let Some(p) = app.get_webview_window(PANEL) else { return Ok(()) };
    if let Some(c) = app.get_webview_window(COMPANION) {
        c.show()?;
    }
    position_panel(app)?;
    p.show()?;
    p.unminimize()?;
    p.set_focus()?;
    let _ = app.emit("panel://visible", true);
    Ok(())
}

pub fn hide_panel(app: &AppHandle) -> tauri::Result<()> {
    if let Some(p) = app.get_webview_window(PANEL) {
        p.hide()?;
    }
    let _ = app.emit("panel://visible", false);
    Ok(())
}

pub fn toggle_panel(app: &AppHandle) -> tauri::Result<()> {
    if panel_visible(app) {
        hide_panel(app)
    } else {
        show_panel(app)
    }
}

pub fn toggle_companion(app: &AppHandle) -> tauri::Result<()> {
    let Some(c) = app.get_webview_window(COMPANION) else { return Ok(()) };
    if c.is_visible()? {
        hide_panel(app)?;
        c.hide()
    } else {
        c.show()
    }
}

/// Grows the companion window (upward, keeping Fanta's feet in place) to fit a
/// speech bubble, and shrinks it back so it doesn't block clicks when idle.
pub fn set_companion_bubble(app: &AppHandle, bubble: bool) -> tauri::Result<()> {
    let Some(c) = app.get_webview_window(COMPANION) else { return Ok(()) };
    let scale = c.scale_factor()?;
    let (w, h) = if bubble { COMPANION_BUBBLE_SIZE } else { COMPANION_SIZE };
    let (w, h) = ((w * scale) as i32, (h * scale) as i32);
    let pos = c.outer_position()?;
    let size = c.outer_size()?;
    if size.width as i32 == w && size.height as i32 == h {
        return Ok(());
    }
    let x = pos.x + (size.width as i32 - w) / 2;
    let y = pos.y + size.height as i32 - h;
    c.set_size(PhysicalSize::new(w as u32, h as u32))?;
    c.set_position(PhysicalPosition::new(x, y))
}
