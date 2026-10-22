import { Fragment, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { api, ApprovalRequest } from "../api";
import Character from "../companion/Character";
import { SendIcon, ShieldIcon } from "../ui/Icons";

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
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, toolNote, busy]);

  async function send(textArg?: string) {
    const text = (textArg ?? draft).trim();
    if (!text || busy) return;
    setDraft("");
    setBusy(true);
    setMessages((prev) => [...prev, { role: "user", content: text }]);
    emit("ui://thinking");
    try {
      await api.send(text);
    } catch (err) {
      setMessages((prev) => [...prev, { role: "error", content: String(err) }]);
    } finally {
      setBusy(false);
      setToolNote(null);
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
        <div ref={bottomRef} />
      </div>

      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          ref={inputRef}
          rows={1}
          autoFocus
          value={draft}
          placeholder={`Message ${name}…`}
          onChange={(e) => {
            setDraft(e.target.value);
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
        <button className="send" disabled={busy || !draft.trim()} aria-label="Send">
          <SendIcon size={18} />
        </button>
      </form>
    </div>
  );
}
