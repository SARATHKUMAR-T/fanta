"""Fanta's desktop plugin: an MCP server (stdio, no dependencies beyond the
system's python3-gi) that reads and operates GUI apps through the Linux
accessibility tree (AT-SPI) and takes screenshots through xdg-desktop-portal.
Works on Wayland, where synthetic mouse input isn't allowed.

Element ids look like "11.0.3.2": the app's index on the desktop, then child
indexes. They stay valid until the window's layout changes, so re-read the
tree after acting.
"""
import base64
import json
import os
import sys
import time
import urllib.parse

import gi

gi.require_version("Atspi", "2.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import Atspi, GdkPixbuf, Gio, GLib  # noqa: E402

Atspi.set_timeout(1500, 15000)  # an unresponsive app shouldn't hang Fanta
S = Atspi.StateType
MAX_LINES = 400
SKIP_APPS = {"gnome-shell", "mutter-x11-frames", "xdg-desktop-portal-gtk", "ibus-extension-gtk3", "ibus-x11"}
INTERACTIVE = {"push button", "toggle button", "check box", "radio button", "menu item", "check menu item", "radio menu item",
               "link", "combo box", "text", "entry", "password text", "spin button", "slider", "page tab", "list item",
               "tree item", "table cell", "menu", "toggle", "switch", "icon"}

TOOLS = [
    {"name": "list_windows", "description": "List open apps and their windows (only apps that expose accessibility).",
     "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": True}},
    {"name": "window_tree",
     "description": "Show the visible controls of an app's window as an indented list with element ids, roles, names, states and text. Use the ids with click/set_text.",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string", "description": "App name or window title from list_windows"},
         "depth": {"type": "integer", "description": "How deep to go. Default 25."}}, "required": ["app"]},
     "annotations": {"readOnlyHint": True}},
    {"name": "find", "description": "Find controls in an app whose name or text contains some words (and optionally a role like 'push button').",
     "inputSchema": {"type": "object", "properties": {
         "app": {"type": "string"}, "text": {"type": "string"}, "role": {"type": "string"}}, "required": ["app"]},
     "annotations": {"readOnlyHint": True}},
    {"name": "read_text", "description": "Read the full text of an element (a document, text field, or label).",
     "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]},
     "annotations": {"readOnlyHint": True}},
    {"name": "screenshot", "description": "Take a screenshot of the whole screen to see what's on it. GNOME may ask the user for permission the first time.",
     "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": True}},
    {"name": "click", "description": "Click/activate a control (button, menu item, link, checkbox, tab...) by element id.",
     "inputSchema": {"type": "object", "properties": {
         "id": {"type": "string"},
         "element": {"type": "string", "description": "Human description shown to the user, e.g. 'Save button'"}}, "required": ["id", "element"]}},
    {"name": "set_text", "description": "Replace the text in an editable field or document by element id.",
     "inputSchema": {"type": "object", "properties": {
         "id": {"type": "string"}, "text": {"type": "string"},
         "element": {"type": "string", "description": "Human description, e.g. 'search box'"}}, "required": ["id", "text", "element"]}},
    {"name": "focus", "description": "Move keyboard focus to an element (and raise its window) by id.",
     "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "element": {"type": "string"}}, "required": ["id", "element"]}},
    {"name": "press_keys", "description": "Press a key or shortcut in the focused window, e.g. 'Return', 'ctrl+s', 'alt+F4', or type literal text with text=. May not work on every Wayland setup.",
     "inputSchema": {"type": "object", "properties": {"keys": {"type": "string"}, "text": {"type": "string"}}}},
]


# ---- accessibility tree ----

def desktop():
    return Atspi.get_desktop(0)


def apps():
    d = desktop()
    for i in range(d.get_child_count()):
        a = d.get_child_at_index(i)
        if a is not None and a.get_name() not in SKIP_APPS and a.get_child_count() > 0:
            yield i, a


def states(node):
    try:
        s = node.get_state_set()
    except GLib.Error:
        return set()
    return {t for t in (S.FOCUSED, S.CHECKED, S.PRESSED, S.SELECTED, S.ACTIVE, S.EDITABLE, S.ENABLED, S.SENSITIVE, S.SHOWING, S.EXPANDED) if s.contains(t)}


def find_app(query):
    q = (query or "").strip().lower()
    best = None
    for i, a in apps():
        names = [(a.get_name() or "").lower()]
        for w in range(a.get_child_count()):
            win = a.get_child_at_index(w)
            if win is not None:
                names.append((win.get_name() or "").lower())
        if q in names:
            return i, a
        if best is None and any(q in n for n in names):
            best = (i, a)
    if best:
        return best
    raise ValueError(f"no app matching '{query}'. Open ones: " + ", ".join(a.get_name() for _, a in apps()))


def resolve(eid):
    try:
        parts = [int(p) for p in str(eid).split(".")]
    except ValueError:
        raise ValueError(f"'{eid}' isn't an element id (they look like 11.0.3)")
    node = desktop()
    for p in parts:
        node = node.get_child_at_index(p) if node is not None else None
        if node is None:
            raise ValueError(f"element {eid} is gone - read the window again")
    return node


def text_of(node, limit=120):
    t = node.get_text_iface()
    if t is None:
        return ""
    try:
        n = Atspi.Text.get_character_count(t)
        s = Atspi.Text.get_text(t, 0, min(n, limit))
        return s + ("…" if n > limit else "")
    except GLib.Error:
        return ""


def describe(node, eid):
    role = node.get_role_name() or "?"
    name = (node.get_name() or "").strip()
    st = states(node)
    flags = []
    if S.FOCUSED in st:
        flags.append("focused")
    if S.CHECKED in st or S.PRESSED in st:
        flags.append("on")
    if S.SELECTED in st:
        flags.append("selected")
    if S.EDITABLE in st:
        flags.append("editable")
    if S.ENABLED not in st and S.SENSITIVE not in st:
        flags.append("disabled")
    line = f"[{eid}] {role}"
    if name:
        line += f' "{name[:100]}"'
    text = text_of(node).replace("\n", " ").strip() if role not in ("label",) else ""
    if text and text != name:
        line += f" text={json.dumps(text, ensure_ascii=False)}"
    if flags:
        line += f" ({', '.join(flags)})"
    return line


def walk(node, eid, depth, max_depth, out, indent=0):
    if len(out) >= MAX_LINES or depth > max_depth:
        return
    st = states(node)
    if depth > 1 and S.SHOWING not in st:
        return  # hidden subtree (closed menus, other tabs)
    role = node.get_role_name() or ""
    name = (node.get_name() or "").strip()
    shown = depth <= 1 or name or role in INTERACTIVE or node.get_text_iface() is not None and role not in ("panel", "filler", "section")
    if shown:
        out.append("  " * indent + describe(node, eid))
    n = node.get_child_count()
    for c in range(min(n, 200)):
        child = node.get_child_at_index(c)
        if child is not None:
            walk(child, f"{eid}.{c}", depth + 1, max_depth, out, indent + (1 if shown else 0))


# ---- tools ----

def list_windows(_):
    lines = []
    for i, a in apps():
        wins = []
        for w in range(a.get_child_count()):
            win = a.get_child_at_index(w)
            if win is None:
                continue
            st = states(win)
            title = win.get_name() or "(untitled)"
            if S.SHOWING not in st and not win.get_name():
                continue  # Chrome keeps hidden helper frames around
            wins.append(f"  [{i}.{w}] {win.get_role_name()} \"{title}\"" + (" (active)" if S.ACTIVE in st else ""))
        lines.append(f"{a.get_name()}\n" + "\n".join(wins))
    note = ("\nApps started before accessibility was switched on may be missing; restarting them helps. "
            "Chrome/Electron apps need --force-renderer-accessibility.")
    return ("\n".join(lines) or "No accessible apps found.") + note


def window_tree(args):
    i, a = find_app(args.get("app"))
    max_depth = int(args.get("depth") or 25)
    out = []
    for w in range(a.get_child_count()):
        win = a.get_child_at_index(w)
        if win is None:
            continue
        # Several windows: only the visible ones.
        if a.get_child_count() > 1 and S.SHOWING not in states(win):
            continue
        walk(win, f"{i}.{w}", 1, max_depth, out)
    if len(out) >= MAX_LINES:
        out.append(f"…(cut at {MAX_LINES} lines; use find, or a smaller depth)")
    return "\n".join(out) or "That app shows no controls."


def find(args):
    i, a = find_app(args.get("app"))
    words = (args.get("text") or "").lower().split()
    role = (args.get("role") or "").lower().strip()
    out = []

    def visit(node, eid, depth):
        if len(out) >= 50 or depth > 40:
            return
        if depth > 1 and S.SHOWING not in states(node):
            return
        hay = ((node.get_name() or "") + " " + text_of(node, 300)).lower()
        if (not words or all(w in hay for w in words)) and (not role or (node.get_role_name() or "") == role):
            if depth > 1:
                out.append(describe(node, eid))
        for c in range(min(node.get_child_count(), 300)):
            child = node.get_child_at_index(c)
            if child is not None:
                visit(child, f"{eid}.{c}", depth + 1)

    visit(a, str(i), 0)
    return "\n".join(out) or "Nothing matched."


def read_text(args):
    node = resolve(args["id"])
    t = node.get_text_iface()
    if t is None:
        return node.get_name() or "(no text)"
    n = Atspi.Text.get_character_count(t)
    s = Atspi.Text.get_text(t, 0, min(n, 50000))
    return s + (f"\n…({n - 50000} more characters)" if n > 50000 else "")


def click(args):
    node = resolve(args["id"])
    act = node.get_action_iface()
    if act is not None and Atspi.Action.get_n_actions(act) > 0:
        names = [Atspi.Action.get_action_name(act, k) for k in range(Atspi.Action.get_n_actions(act))]
        k = next((names.index(n) for n in ("click", "press", "activate", "toggle", "jump", "open") if n in names), 0)
        Atspi.Action.do_action(act, k)
        return f"Done ({names[k] or 'action'}): {describe(node, args['id'])}"
    sel_parent = node.get_parent()
    if sel_parent is not None and sel_parent.get_selection_iface() is not None:
        Atspi.Selection.select_child(sel_parent.get_selection_iface(), node.get_index_in_parent())
        return f"Selected {describe(node, args['id'])}"
    raise ValueError("that element can't be clicked - try its parent or a button near it")


def set_text(args):
    node = resolve(args["id"])
    et = node.get_editable_text_iface()
    if et is None:
        raise ValueError("that element isn't editable - find the text field inside it")
    if not Atspi.EditableText.set_text_contents(et, args.get("text", "")):
        raise ValueError("the app refused the text")
    return f"Text set: {describe(node, args['id'])}"


def focus(args):
    node = resolve(args["id"])
    comp = node.get_component_iface()
    if comp is None or not Atspi.Component.grab_focus(comp):
        raise ValueError("couldn't focus that element")
    return f"Focused {describe(node, args['id'])}"


KEYSYMS = {"return": 0xFF0D, "enter": 0xFF0D, "tab": 0xFF09, "escape": 0xFF1B, "esc": 0xFF1B, "backspace": 0xFF08,
           "delete": 0xFFFF, "up": 0xFF52, "down": 0xFF54, "left": 0xFF51, "right": 0xFF53, "home": 0xFF50,
           "end": 0xFF57, "page_up": 0xFF55, "pageup": 0xFF55, "page_down": 0xFF56, "pagedown": 0xFF56, "space": 0x20}
KEYSYMS.update({f"f{n}": 0xFFBE + n - 1 for n in range(1, 13)})
MODS = {"shift": 1, "ctrl": 4, "control": 4, "alt": 8, "super": 64, "meta": 64}


def press_keys(args):
    if args.get("text"):
        Atspi.generate_keyboard_event(0, args["text"], Atspi.KeySynthType.STRING)
        return f"Typed {len(args['text'])} characters."
    combo = [k.strip().lower() for k in (args.get("keys") or "").replace("-", "+").split("+") if k.strip()]
    if not combo:
        raise ValueError("give keys like 'ctrl+s' or text")
    *mods, key = combo
    mask = 0
    for m in mods:
        if m not in MODS:
            raise ValueError(f"unknown modifier {m}")
        mask |= MODS[m]
    sym = KEYSYMS.get(key) or (ord(key) if len(key) == 1 else None)
    if sym is None:
        raise ValueError(f"unknown key {key}")
    if mask:
        Atspi.generate_keyboard_event(mask, None, Atspi.KeySynthType.LOCKMODIFIERS)
    Atspi.generate_keyboard_event(sym, None, Atspi.KeySynthType.SYM)
    if mask:
        Atspi.generate_keyboard_event(mask, None, Atspi.KeySynthType.UNLOCKMODIFIERS)
    return f"Pressed {args['keys']}."


def screenshot(_):
    # GNOME refuses silent screenshots to unsandboxed apps; then its own
    # screenshot dialog opens and the user confirms the capture.
    try:
        path = portal_screenshot(interactive=False)
    except ValueError:
        path = portal_screenshot(interactive=True)
    pix = GdkPixbuf.Pixbuf.new_from_file(path)
    os.remove(path)  # the portal saves into ~/Pictures; don't leave clutter
    w, h = pix.get_width(), pix.get_height()
    scale = min(1.0, 1568 / max(w, h))
    if scale < 1:
        pix = pix.scale_simple(int(w * scale), int(h * scale), GdkPixbuf.InterpType.BILINEAR)
    ok, data = pix.save_to_bufferv("jpeg", ["quality"], ["80"])
    return {"content": [
        {"type": "text", "text": f"Screenshot ({pix.get_width()}x{pix.get_height()}, scaled from {w}x{h})."},
        {"type": "image", "mimeType": "image/jpeg", "data": base64.b64encode(data).decode()}]}


def portal_screenshot(interactive):
    """Path of a screenshot taken through xdg-desktop-portal."""
    bus = Gio.bus_get_sync(Gio.BusType.SESSION)
    token = f"fanta{int(time.time() * 1000)}"
    sender = bus.get_unique_name()[1:].replace(".", "_")
    handle = f"/org/freedesktop/portal/desktop/request/{sender}/{token}"
    loop = GLib.MainLoop()
    result = {}

    def on_response(_c, _s, _p, _i, _sig, params):
        code, res = params.unpack()
        result["code"], result["uri"] = code, res.get("uri")
        loop.quit()

    sub = bus.signal_subscribe("org.freedesktop.portal.Desktop", "org.freedesktop.portal.Request", "Response",
                               handle, None, Gio.DBusSignalFlags.NO_MATCH_RULE, on_response)
    try:
        bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                      "org.freedesktop.portal.Screenshot", "Screenshot",
                      GLib.Variant("(sa{sv})", ("", {"handle_token": GLib.Variant("s", token),
                                                     "interactive": GLib.Variant("b", interactive)})),
                      None, Gio.DBusCallFlags.NONE, 10000, None)
        GLib.timeout_add_seconds(120 if interactive else 20, loop.quit)
        loop.run()
    finally:
        bus.signal_unsubscribe(sub)
    if result.get("code") != 0 or not result.get("uri"):
        raise ValueError(f"no screenshot (portal response {result.get('code')}) - the user may have declined")
    return urllib.parse.unquote(urllib.parse.urlparse(result["uri"]).path)


HANDLERS = {"list_windows": list_windows, "window_tree": window_tree, "find": find, "read_text": read_text,
            "screenshot": screenshot, "click": click, "set_text": set_text, "focus": focus, "press_keys": press_keys}


def enable_accessibility():
    """Asks toolkits to expose their widgets (apps started after this register on the a11y bus)."""
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION)
        bus.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.freedesktop.DBus.Properties", "Set",
                      GLib.Variant("(ssv)", ("org.a11y.Status", "IsEnabled", GLib.Variant("b", True))),
                      None, Gio.DBusCallFlags.NONE, 2000, None)
    except GLib.Error as e:
        print(f"couldn't enable accessibility: {e}", file=sys.stderr, flush=True)


# ---- MCP over stdio ----

def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    enable_accessibility()
    for line in sys.stdin:
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        method, mid = msg.get("method"), msg.get("id")
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": mid, "result": {
                "protocolVersion": msg["params"]["protocolVersion"],
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fanta-desktop", "version": "1"}}})
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS}})
        elif method == "tools/call":
            name, args = msg["params"]["name"], msg["params"].get("arguments") or {}
            try:
                out = HANDLERS[name](args)
                result = out if isinstance(out, dict) else {"content": [{"type": "text", "text": out}]}
            except Exception as e:  # report to the model instead of crashing
                result = {"content": [{"type": "text", "text": str(e)}], "isError": True}
            send({"jsonrpc": "2.0", "id": mid, "result": result})
        elif method == "ping":
            send({"jsonrpc": "2.0", "id": mid, "result": {}})
        elif mid is not None and method is not None:
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": f"unknown method {method}"}})


if __name__ == "__main__":
    main()
