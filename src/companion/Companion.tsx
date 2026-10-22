import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, ApprovalRequest, FocusStatus, parseTs } from "../api";
import Character, { Mood } from "./Character";
import "./character.css";
import "./companion.css";

type Bubble = { text: string; kind: "info" | "alert" };

const BUBBLE_MS = 12_000;
const DRAG_THRESHOLD = 5;

function mmss(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function clip(text: string, max = 160) {
  const t = text.replace(/https:\/\/\S+/g, "").replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max - 1) + "…" : t;
}

export default function Companion() {
  const [transient, setTransient] = useState<Mood | null>(null);
  const [bubble, setBubble] = useState<Bubble | null>(null);
  const [focus, setFocus] = useState<FocusStatus | null>(null);
  const [unseen, setUnseen] = useState(0);
  const [panelOpen, setPanelOpen] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [hint, setHint] = useState(() => {
    try {
      return !localStorage.getItem("fanta.hint-seen");
    } catch {
      return false;
    }
  });
  const moodTimer = useRef<number | undefined>(undefined);
  const bubbleTimer = useRef<number | undefined>(undefined);
  const pendingApprovals = useRef(new Set<number>());
  const lastUrgent = useRef<number | null>(null);
  const press = useRef<{ x: number; y: number; dragging: boolean } | null>(null);

  function flash(mood: Mood, ms: number) {
    setTransient(mood);
    clearTimeout(moodTimer.current);
    moodTimer.current = window.setTimeout(() => setTransient(null), ms);
  }

  async function say(b: Bubble | null, ms = BUBBLE_MS) {
    clearTimeout(bubbleTimer.current);
    if (b) {
      // Grow the window first so the bubble has room.
      await api.companionBubble(true);
      setBubble(b);
      if (ms > 0) bubbleTimer.current = window.setTimeout(() => say(null), ms);
    } else {
      setBubble(null);
      await api.companionBubble(false);
    }
  }

  useEffect(() => {
    const refreshFocus = () => api.focusStatus().then(setFocus);
    const refreshInbox = async () => {
      setUnseen(await api.inboxUnseen());
      const urgent = await api.listNotifications("urgent");
      const newest = urgent[0];
      if (lastUrgent.current === null) {
        lastUrgent.current = newest?.id ?? 0; // don't announce old ones on startup
      } else if (newest && newest.id > lastUrgent.current) {
        lastUrgent.current = newest.id;
        if (!newest.seen) {
          flash("alert", 5000);
          say({ text: `Urgent · ${newest.source}: ${newest.summary ?? newest.title}`, kind: "alert" });
        }
      }
    };
    refreshFocus();
    refreshInbox();

    const subs = [
      listen("focus://changed", refreshFocus),
      listen("inbox://changed", refreshInbox),
      listen<string>("chat://token", () => flash("talking", 1400)),
      listen<string>("chat://tool", () => flash("thinking", 20_000)),
      listen("ui://thinking", () => flash("thinking", 20_000)),
      listen<string>("chat://proactive", (e) => {
        flash("happy", 3000);
        say({ text: clip(e.payload), kind: e.payload.startsWith("⏰") || e.payload.startsWith("📅") ? "alert" : "info" });
      }),
      listen<ApprovalRequest>("approval://request", (e) => {
        pendingApprovals.current.add(e.payload.id);
        flash("alert", 60_000);
        say({ text: `Can I go ahead? ${clip(e.payload.text, 120)}`, kind: "alert" }, 0);
      }),
      listen<{ id: number }>("approval://resolved", (e) => {
        pendingApprovals.current.delete(e.payload.id);
        if (pendingApprovals.current.size === 0) {
          flash("happy", 1500);
          say(null);
        }
      }),
      listen<boolean>("panel://visible", (e) => {
        setPanelOpen(e.payload);
        if (e.payload) {
          say(null);
          flash("happy", 1500);
        }
      }),
    ];
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      subs.forEach((p) => p.then((f) => f()));
      clearInterval(tick);
    };
  }, []);

  const focusing = focus?.active?.kind === "focus";
  const onBreak = focus?.active?.kind === "break";
  const hour = new Date(now).getHours();
  const base: Mood = focusing ? "focus" : hour >= 23 || hour < 6 ? "sleepy" : "idle";
  const mood = transient ?? base;
  const remaining = focus?.active ? parseTs(focus.active.ends_at).getTime() - now : 0;

  function dismissHint() {
    if (!hint) return;
    setHint(false);
    try {
      localStorage.setItem("fanta.hint-seen", "1");
    } catch {
      /* private mode: the hint just shows again next time */
    }
  }

  // Press and release without moving = click (toggle chat); press and move = drag.
  function onPointerDown(e: React.PointerEvent) {
    if (e.button !== 0) return;
    press.current = { x: e.screenX, y: e.screenY, dragging: false };
  }
  function onPointerMove(e: React.PointerEvent) {
    const p = press.current;
    if (!p || p.dragging) return;
    if (Math.hypot(e.screenX - p.x, e.screenY - p.y) > DRAG_THRESHOLD) {
      p.dragging = true;
      getCurrentWindow().startDragging();
    }
  }
  function onPointerUp() {
    const p = press.current;
    press.current = null;
    if (p && !p.dragging) {
      dismissHint();
      api.togglePanel();
    }
  }

  return (
    <div className="companion">
      {bubble && (
        <div className={`speech ${bubble.kind}`} onClick={() => api.togglePanel()}>
          <button
            className="speech-close"
            aria-label="Dismiss"
            onClick={(e) => {
              e.stopPropagation();
              say(null);
            }}
          >
            ×
          </button>
          {bubble.text}
        </div>
      )}

      <div
        className={`buddy ${panelOpen ? "open" : ""}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        title="Click to chat · drag to move"
      >
        <div className="halo" />
        <Character mood={mood} size={132} />
        {unseen > 0 && <span className="badge">{unseen > 9 ? "9+" : unseen}</span>}
        {(focusing || onBreak) && (
          <span className={`timer ${onBreak ? "break" : ""}`}>
            {onBreak ? "☕" : "🎧"} {mmss(remaining)}
          </span>
        )}
        {hint && !bubble && <span className="hint">Click me!</span>}
      </div>
    </div>
  );
}
