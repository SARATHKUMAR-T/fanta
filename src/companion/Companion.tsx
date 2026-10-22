import { useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, ApprovalRequest, FocusStatus, InboxItem, parseTs, Persona, ReplyTarget } from "../api";
import Character from "./Character";
import { UI_EVENTS, useMood } from "./useMood";
import "./character.css";
import "./companion.css";

type Bubble = {
  kind: "info" | "alert" | "message";
  title?: string;
  meta?: string;
  text: string;
  more?: number;
  /** A Teams message that can be replied to. */
  reply?: ReplyTarget;
  /** An approval waiting for an answer. */
  approvalId?: number;
};

const BUBBLE_MS = 12_000;
const MESSAGE_BUBBLE_MS = 16_000;
const DRAG_THRESHOLD = 5;
const CHAT_APPS = ["Teams", "Slack", "WhatsApp", "Discord"];

function mmss(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function clip(text: string, max = 220) {
  const t = text.replace(/https:\/\/\S+/g, "").replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max - 1) + "…" : t;
}

export default function Companion() {
  const mood = useMood();
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
  const bubbleTimer = useRef<number | undefined>(undefined);
  const pendingApprovals = useRef(new Set<number>());
  const lastAnnounced = useRef<number | null>(null);
  const persona = useRef<Persona | null>(null);
  const focusRef = useRef<FocusStatus | null>(null);
  const panelOpenRef = useRef(false);
  const press = useRef<{ x: number; y: number; dragging: boolean } | null>(null);
  const [voice, setVoice] = useState<"idle" | "listening" | "transcribing">("idle");
  const [speaking, setSpeaking] = useState(false);

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

  /** Reads out new messages once they're sorted: exact text, who sent it, which chat. */
  async function announceMessages() {
    const items = (await api.listNotifications()).slice(0, 15).reverse(); // oldest first
    if (lastAnnounced.current === null) {
      lastAnnounced.current = items.length ? items[items.length - 1].id : 0; // don't replay old ones at startup
      return;
    }
    const fresh: InboxItem[] = [];
    for (const n of items) {
      if (n.id <= lastAnnounced.current) continue;
      if (n.category === "pending") break; // wait until it's sorted, keep order
      lastAnnounced.current = n.id;
      fresh.push(n);
    }
    const mode = persona.current?.notifications.bubbles ?? "messages";
    const focusing = focusRef.current?.active?.kind === "focus";
    const worth = fresh.filter(
      (n) => n.category === "urgent" || (mode === "messages" && !focusing && n.category === "fyi" && CHAT_APPS.includes(n.source)),
    );
    if (mode === "off" || worth.length === 0 || panelOpenRef.current) return;
    const n = worth[worth.length - 1];
    const urgent = n.category === "urgent";
    emit(UI_EVENTS.newMessage, urgent ? "urgent" : "message");
    say(
      {
        kind: urgent ? "alert" : "message",
        title: `${urgent ? "🔴 " : "💬 "}${n.sender ?? n.title}`,
        meta: [n.chat, n.source].filter(Boolean).join(" · "),
        text: clip(n.text ?? n.body),
        more: worth.length - 1,
        reply: n.source === "Teams" ? { id: n.id, sender: n.sender ?? n.title, chat: n.chat, text: n.text ?? n.body } : undefined,
      },
      MESSAGE_BUBBLE_MS,
    );
  }

  useEffect(() => {
    api.persona().then((p) => (persona.current = p));
    const refreshFocus = () =>
      api.focusStatus().then((f) => {
        focusRef.current = f;
        setFocus(f);
      });
    const refreshInbox = async () => {
      setUnseen(await api.inboxUnseen());
      await announceMessages();
    };
    refreshFocus();
    refreshInbox();
    if (import.meta.env.DEV && location.search.includes("demoBubble")) {
      say(
        {
          kind: "message",
          title: "💬 Akilesh",
          meta: "Aarthi Selvakumari, +3 · Teams",
          text: "did you push the creator studio changes?",
          reply: { id: 2, sender: "Akilesh", chat: "Aarthi Selvakumari, +3", text: "did you push the creator studio changes?" },
        },
        0,
      );
    }

    const subs = [
      listen("focus://changed", refreshFocus),
      listen("inbox://changed", refreshInbox),
      listen<string>("chat://proactive", (e) =>
        say({ kind: e.payload.startsWith("⏰") || e.payload.startsWith("📅") ? "alert" : "info", text: clip(e.payload) }),
      ),
      listen<ApprovalRequest>("approval://request", (e) => {
        pendingApprovals.current.add(e.payload.id);
        say({ kind: "alert", title: "Can I go ahead?", text: clip(e.payload.text, 180), approvalId: e.payload.id }, 0);
      }),
      listen<{ id: number }>("approval://resolved", (e) => {
        pendingApprovals.current.delete(e.payload.id);
        if (pendingApprovals.current.size === 0) say(null);
      }),
      listen<"idle" | "listening" | "transcribing">("voice://state", (e) => setVoice(e.payload)),
      listen<boolean>("voice://speaking", (e) => setSpeaking(e.payload)),
      listen<string>("voice://heard", (e) => {
        if (!panelOpenRef.current) say({ kind: "info", title: "🎙️ You said", text: clip(e.payload) }, 0);
      }),
      // Spoken conversations happen with the chat closed, so show the answer here too.
      listen<string>("ui://voice-reply", (e) => {
        if (!panelOpenRef.current) say({ kind: "info", title: "Fanta", text: clip(e.payload, 260) }, 20_000);
      }),
      listen<string>("voice://error", (e) => {
        if (!panelOpenRef.current) say({ kind: "info", text: e.payload }, 4000);
      }),
      listen<boolean>("panel://visible", (e) => {
        panelOpenRef.current = e.payload;
        setPanelOpen(e.payload);
        if (e.payload) say(null);
      }),
    ];
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      subs.forEach((p) => p.then((f) => f()));
      clearInterval(tick);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const focusing = focus?.active?.kind === "focus";
  const onBreak = !!focus?.active && !focusing;
  const paused = !!focus?.active?.paused_at;
  // A paused timer shows the time that was left when it was paused.
  const remaining = focus?.active
    ? parseTs(focus.active.ends_at).getTime() - (paused ? parseTs(focus.active.paused_at!).getTime() : now)
    : 0;

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

  // Right-click = talk (or stop her talking).
  function onContextMenu(e: React.MouseEvent) {
    e.preventDefault();
    dismissHint();
    if (speaking) api.voiceStop();
    else
      api.voiceListen().catch(() => {
        // Voice isn't set up yet: open the chat, where the mic button offers the download.
        api.togglePanel();
      });
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
          {bubble.title && <div className="speech-title">{bubble.title}</div>}
          {bubble.meta && <div className="speech-meta">{bubble.meta}</div>}
          <div className="speech-text">{bubble.text}</div>
          {!!bubble.more && <div className="speech-more">+{bubble.more} more · open chat</div>}
          {(bubble.reply || bubble.approvalId) && (
            <div className="speech-actions" onClick={(e) => e.stopPropagation()}>
              {bubble.reply && (
                <button
                  className="speech-btn primary"
                  onClick={async () => {
                    say(null);
                    await emit("ui://reply-to", bubble.reply);
                    if (!panelOpenRef.current) api.togglePanel();
                  }}
                >
                  ↩ Reply
                </button>
              )}
              {bubble.approvalId && (
                <>
                  <button className="speech-btn primary" onClick={() => api.answerApproval(bubble.approvalId!, true)}>
                    Approve
                  </button>
                  <button className="speech-btn" onClick={() => api.answerApproval(bubble.approvalId!, false)}>
                    Not now
                  </button>
                </>
              )}
            </div>
          )}
        </div>
      )}

      <div
        className={`buddy ${panelOpen ? "open" : ""}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onContextMenu={onContextMenu}
        title="Click to chat · right-click to talk · drag to move"
      >
        <div className="halo" />
        <Character mood={mood} size={132} />
        {unseen > 0 && <span className="badge">{unseen > 9 ? "9+" : unseen}</span>}
        {(focusing || onBreak) && (
          <span className={`timer ${onBreak ? "break" : ""}`}>
            {paused ? "⏸" : onBreak ? "☕" : "🎧"} {mmss(remaining)}
          </span>
        )}
        {voice !== "idle" && <span className={`mic-badge ${voice}`}>🎙️</span>}
        {hint && !bubble && <span className="hint">Click me!</span>}
      </div>
    </div>
  );
}
