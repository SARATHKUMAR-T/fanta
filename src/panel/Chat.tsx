import { Fragment, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { api, ApprovalRequest, ReplyTarget } from "../api";
import Character from "../companion/Character";
import { UI_EVENTS } from "../companion/useMood";
import { MicIcon, SendIcon, ShieldIcon, StopIcon } from "../ui/Icons";

type VoiceState = "idle" | "listening" | "transcribing";

type Msg =
  | { role: "user" | "assistant" | "error"; content: string }
  | { role: "approval"; id: number; content: string; state: "pending" | "approved" | "declined" };

const SUGGESTIONS = [
  { icon: "🗓️", text: "Plan my day" },
  { icon: "🎯", text: "Start a focus block" },
  { icon: "📥", text: "What did I miss?" },
  { icon: "📅", text: "What's on tomorrow?" },
];

function greeting() {
  const h = new Date().getHours();
  if (h < 5) return "Up late?";
  if (h < 12) return "Good morning!";
  if (h < 17) return "Good afternoon!";
  return "Good evening!";
}

/** Renders https links as tappable chips that open in the browser. */
function Linkified({ text }: { text: string }) {
  const parts = text.split(/(https:\/\/\S+)/g);
  return (
    <>
      {parts.map((p, i) =>
        p.startsWith("https://") ? (
          <a key={i} href="#" className="link-chip" onClick={(e) => (e.preventDefault(), api.openLink(p))}>
            {p.includes("teams.microsoft.com") ? "Join in Teams ↗" : new URL(p).hostname + " ↗"}
          </a>
        ) : (
          <Fragment key={i}>{p}</Fragment>
        ),
      )}
    </>
  );
}

export default function Chat({ name }: { name: string }) {
  const [messages, setMessages] = useState<Msg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastTyping = useRef(0);
  const [voice, setVoice] = useState<VoiceState>("idle");
  const [level, setLevel] = useState(0);
  const [voiceNote, setVoiceNote] = useState<string | null>(null);
  const [voiceSetup, setVoiceSetup] = useState<{ installing: boolean; progress?: string; error?: string } | null>(null);
  const [speaking, setSpeaking] = useState(false);
  // Until Microsoft sign-in is set up, replies are copied and Teams is opened instead.
  const [teamsConnected, setTeamsConnected] = useState(() => !(import.meta.env.DEV && location.search.includes("nograph")));
  const [replyTo, setReplyTo] = useState<ReplyTarget | null>(() =>
    import.meta.env.DEV && location.search.includes("replyDemo")
      ? { id: 2, sender: "Akilesh", chat: "Aarthi Selvakumari, +3", text: "did you push the creator studio changes?" }
      : null,
  );
  // Event listeners are registered once, so they reach the latest send() through a ref.
  const sendRef = useRef<(text: string, byVoice: boolean) => void>(() => {});

  useEffect(() => {
    api.history().then((h) => setMessages(h.map((m) => ({ role: m.role as "user" | "assistant", content: m.content }))));
    const subs = [
      listen<string>("chat://token", (e) => {
        setToolNote(null);
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last?.role === "assistant") return [...prev.slice(0, -1), { ...last, content: last.content + e.payload }];
          return [...prev, { role: "assistant", content: e.payload }];
        });
      }),
      listen<string>("chat://tool", (e) =>
        setToolNote(e.payload.includes(" ") ? e.payload : `Using ${e.payload.replace(/^[a-z0-9-]+__/i, "").replace(/_/g, " ")}`),
      ),
      listen<string>("chat://proactive", (e) => setMessages((prev) => [...prev, { role: "assistant", content: e.payload }])),
      listen<ApprovalRequest>("approval://request", (e) =>
        setMessages((prev) => [...prev, { role: "approval", id: e.payload.id, content: e.payload.text, state: "pending" }]),
      ),
      listen<{ id: number; approved: boolean }>("approval://resolved", (e) =>
        setMessages((prev) =>
          prev.map((m) => (m.role === "approval" && m.id === e.payload.id ? { ...m, state: e.payload.approved ? "approved" : "declined" } : m)),
        ),
      ),
      listen<boolean>("panel://visible", (e) => e.payload && setTimeout(() => inputRef.current?.focus(), 50)),
      listen<ReplyTarget>("ui://reply-to", async (e) => {
        setTeamsConnected((await api.calendarStatus()).provider === "graph");
        setReplyTo(e.payload);
        setTimeout(() => inputRef.current?.focus(), 80);
      }),
      listen<string>("chat://note", (e) => setMessages((prev) => [...prev, { role: "assistant", content: e.payload }])),
      listen<VoiceState>("voice://state", (e) => {
        setVoice(e.payload);
        if (e.payload === "listening") setVoiceNote(null);
      }),
      listen<number>("voice://level", (e) => setLevel(e.payload)),
      listen<string>("voice://heard", (e) => sendRef.current(e.payload, true)),
      listen<string>("voice://error", (e) => {
        setVoiceNote(e.payload);
        setTimeout(() => setVoiceNote(null), 5000);
      }),
      listen<boolean>("voice://speaking", (e) => setSpeaking(e.payload)),
      listen<{ label: string; done: number; total: number }>("voice://progress", (e) =>
        setVoiceSetup((v) => ({
          installing: true,
          progress: `Downloading ${e.payload.label}… ${Math.round(e.payload.done / 1e6)} / ${Math.round(e.payload.total / 1e6)} MB`,
          error: v?.error,
        })),
      ),
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, toolNote, busy]);

  /** Reply box mode: the user's exact words go straight to that Teams chat. */
  async function sendReply(target: ReplyTarget, text: string) {
    if (!teamsConnected) {
      // No permission to send yet: hand the text over to Teams in the browser.
      await navigator.clipboard.writeText(text).catch(() => {});
      await api.openLink("https://teams.cloud.microsoft/");
      setMessages((prev) => [
        ...prev,
        {
          role: "assistant",
          content: `📋 Copied your reply and opened Teams - paste it into ${target.chat ? `“${target.chat}”` : `your chat with ${target.sender}`}. To let me send it for you next time, connect Microsoft in Settings.`,
        },
      ]);
      setDraft("");
      setReplyTo(null);
      return;
    }
    setBusy(true);
    setToolNote("Sending on Teams");
    try {
      await api.teamsReply(target.id, text);
      setDraft("");
      setReplyTo(null);
      emit(UI_EVENTS.userMessage, text);
    } catch (err) {
      setMessages((prev) => [...prev, { role: "error", content: String(err) }]);
      emit(UI_EVENTS.error);
    } finally {
      setBusy(false);
      setToolNote(null);
    }
  }

  async function send(textArg?: string, byVoice = false) {
    const text = (textArg ?? draft).trim();
    if (!text || busy) return;
    if (replyTo && !byVoice) return sendReply(replyTo, text);
    setDraft("");
    setBusy(true);
    setMessages((prev) => [...prev, { role: "user", content: text }]);
    emit(UI_EVENTS.userMessage, text);
    emit(UI_EVENTS.thinking);
    try {
      const reply = await api.send(text);
      if (byVoice) emit("ui://voice-reply", reply); // the companion shows it if the panel is closed
      if (await api.voiceShouldSpeak(byVoice)) await api.voiceSpeak(reply);
    } catch (err) {
      setMessages((prev) => [...prev, { role: "error", content: String(err) }]);
      emit(UI_EVENTS.error);
    } finally {
      setBusy(false);
      setToolNote(null);
    }
  }

  sendRef.current = (text, byVoice) => send(text, byVoice);

  async function mic() {
    if (speaking) {
      await api.voiceStop();
      return;
    }
    const st = await api.voiceStatus();
    if (!st.installed) {
      setVoiceSetup({ installing: st.installing });
      return;
    }
    try {
      await api.voiceListen(); // a second click while listening means "done talking"
    } catch (e) {
      setVoiceNote(String(e));
    }
  }

  async function installVoice() {
    setVoiceSetup({ installing: true, progress: "Starting download…" });
    try {
      await api.voiceInstall();
      setVoiceSetup(null);
      setVoiceNote("Voice is ready - click the mic and talk to me!");
      api.voiceSpeak("Hi! I can hear you now. Click the mic, or press Control Alt Space, and just talk.");
    } catch (e) {
      setVoiceSetup({ installing: false, error: String(e) });
    }
  }

  const last = messages[messages.length - 1];
  const waiting = busy && (toolNote || last?.role === "user" || last?.role === "approval");

  return (
    <div className="chat-view">
      <div className="messages">
        {messages.length === 0 && (
          <div className="welcome">
            <Character mood="happy" size={112} />
            <h2>{greeting()}</h2>
            <p>I'm {name}. I'll keep your tasks, focus and messages in check. What should we tackle?</p>
            <div className="suggestions">
              {SUGGESTIONS.map((s) => (
                <button key={s.text} onClick={() => send(s.text)}>
                  <span>{s.icon}</span>
                  {s.text}
                </button>
              ))}
            </div>
          </div>
        )}

        {messages.map((m, i) => {
          if (m.role === "approval") {
            return (
              <div key={i} className={`approval ${m.state}`}>
                <div className="approval-head">
                  <ShieldIcon size={16} /> {name} needs your OK
                </div>
                <div className="approval-text">{m.content}</div>
                {m.state === "pending" ? (
                  <div className="approval-actions">
                    <button className="btn primary" onClick={() => api.answerApproval(m.id, true)}>
                      Approve
                    </button>
                    <button className="btn ghost" onClick={() => api.answerApproval(m.id, false)}>
                      Not now
                    </button>
                  </div>
                ) : (
                  <div className="approval-result">{m.state === "approved" ? "✓ Approved" : "Skipped"}</div>
                )}
              </div>
            );
          }
          if (m.role === "error") {
            return (
              <div key={i} className="msg-error">
                {m.content}
              </div>
            );
          }
          const mine = m.role === "user";
          const prevSame = messages[i - 1]?.role === m.role;
          return (
            <div key={i} className={`row ${mine ? "mine" : "theirs"} ${prevSame ? "grouped" : ""}`}>
              {!mine && <div className="avatar-sm">{!prevSame && <Character mood="idle" size={30} face />}</div>}
              <div className="bubble">
                <Linkified text={m.content} />
              </div>
            </div>
          );
        })}

        {waiting && (
          <div className="row theirs">
            <div className="avatar-sm">
              <Character mood="thinking" size={30} face />
            </div>
            <div className="bubble typing">
              {toolNote ? <span className="tool-note">{toolNote}…</span> : <span className="dots"><i /><i /><i /></span>}
            </div>
          </div>
        )}
        {voiceSetup && (
          <div className="voice-setup">
            <div className="voice-setup-title">🎙️ Talk to {name}</div>
            <p>
              Speech runs fully on this computer - nothing is sent anywhere. It needs a one-time download of about 230 MB (speech recognition and{" "}
              {name}'s voice).
            </p>
            {voiceSetup.progress && <div className="muted small">{voiceSetup.progress}</div>}
            {voiceSetup.error && <div className="error-text">{voiceSetup.error}</div>}
            <div className="approval-actions">
              <button className="btn primary" disabled={voiceSetup.installing} onClick={installVoice}>
                {voiceSetup.installing ? "Downloading…" : "Download voice"}
              </button>
              {!voiceSetup.installing && (
                <button className="btn ghost" onClick={() => setVoiceSetup(null)}>
                  Not now
                </button>
              )}
            </div>
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      {replyTo && (
        <div className="reply-chip">
          <div className="reply-chip-body">
            <div className="reply-chip-to">
              ↩ Reply on Teams to <b>{replyTo.sender}</b>
              {replyTo.chat && <span className="muted"> in {replyTo.chat}</span>}
            </div>
            <div className="reply-chip-quote">“{replyTo.text}”</div>
            {!teamsConnected && (
              <div className="reply-chip-setup">
                Not connected yet - Send will copy your reply and open Teams.{" "}
                <button className="btn link small" onClick={() => emit("ui://open-settings")}>
                  Connect Microsoft to send directly →
                </button>
              </div>
            )}
          </div>
          <button className="icon-btn" title="Cancel reply" onClick={() => setReplyTo(null)}>
            ×
          </button>
        </div>
      )}
      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        {voice !== "idle" ? (
          <div className="listening" onClick={() => voice === "listening" && api.voiceListen()}>
            <span className="rec-dot" />
            <span className="levels">
              {[0.5, 0.8, 1, 0.8, 0.5].map((k, i) => (
                <i key={i} style={{ height: `${6 + Math.min(1, level * 2.2) * 18 * k}px` }} />
              ))}
            </span>
            <span>{voice === "listening" ? "Listening… (click when done)" : "Got it, thinking…"}</span>
          </div>
        ) : (
        <textarea
          ref={inputRef}
          rows={1}
          autoFocus
          value={draft}
          placeholder={replyTo ? `Reply to ${replyTo.sender}…` : `Message ${name}…`}
          onChange={(e) => {
            setDraft(e.target.value);
            // Let Fanta "listen" while you type (at most once a second).
            const t = Date.now();
            if (t - lastTyping.current > 1000) {
              lastTyping.current = t;
              emit(UI_EVENTS.typing);
            }
            e.target.style.height = "auto";
            e.target.style.height = Math.min(e.target.scrollHeight, 120) + "px";
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        )}
        {voiceNote && <div className="voice-note">{voiceNote}</div>}
        {busy ? (
          <button type="button" className="send stop" onClick={() => { api.chatStop(); if (speaking) api.voiceStop(); }} title="Stop" aria-label="Stop">
            <StopIcon size={18} />
          </button>
        ) : draft.trim() ? (
          <button className="send" aria-label="Send">
            <SendIcon size={18} />
          </button>
        ) : (
          <button
            type="button"
            className={`send mic ${voice} ${speaking ? "speaking" : ""}`}
            onClick={mic}
            title={speaking ? "Stop talking" : voice === "listening" ? "Done talking" : "Talk (Ctrl+Alt+Space)"}
            aria-label="Talk"
          >
            <MicIcon size={18} />
          </button>
        )}
      </form>
    </div>
  );
}
