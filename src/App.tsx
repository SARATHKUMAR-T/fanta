import { Fragment, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, ApprovalRequest, Persona } from "./api";
import CalendarView from "./CalendarView";
import Connections from "./Connections";
import FocusBar from "./FocusBar";
import Inbox from "./Inbox";
import Tasks from "./Tasks";
import "./App.css";

type Msg =
  | { role: "user" | "assistant" | "error"; content: string }
  | { role: "approval"; id: number; content: string; state: "pending" | "approved" | "declined" };
type Tab = "chat" | "tasks" | "inbox" | "calendar";

/** Renders https links as buttons that open in the default browser. */
function Linkified({ text }: { text: string }) {
  const parts = text.split(/(https:\/\/\S+)/g);
  return (
    <>
      {parts.map((p, i) =>
        p.startsWith("https://") ? (
          <a key={i} href="#" onClick={(e) => (e.preventDefault(), api.openLink(p))}>
            {p.includes("teams.microsoft.com") ? "Join in Teams" : p}
          </a>
        ) : (
          <Fragment key={i}>{p}</Fragment>
        ),
      )}
    </>
  );
}

export default function App() {
  const [persona, setPersona] = useState<Persona | null>(null);
  const [messages, setMessages] = useState<Msg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [showConnections, setShowConnections] = useState(false);
  const [openTasks, setOpenTasks] = useState(0);
  const [unseen, setUnseen] = useState(0);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.persona().then(setPersona);
    api.history().then((h) =>
      setMessages(h.map((m) => ({ role: m.role as "user" | "assistant", content: m.content }))),
    );
    const unToken = listen<string>("chat://token", (e) => {
      setToolNote(null);
      setMessages((prev) => {
        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          return [...prev.slice(0, -1), { ...last, content: last.content + e.payload }];
        }
        return [...prev, { role: "assistant", content: e.payload }];
      });
    });
    const unTool = listen<string>("chat://tool", (e) =>
      setToolNote(e.payload.includes(" ") ? `${e.payload}…` : `using ${e.payload.replace(/_/g, " ")}…`),
    );
    const unProactive = listen<string>("chat://proactive", (e) =>
      setMessages((prev) => [...prev, { role: "assistant", content: e.payload }]),
    );
    const unApproval = listen<ApprovalRequest>("approval://request", (e) =>
      setMessages((prev) => [...prev, { role: "approval", id: e.payload.id, content: e.payload.text, state: "pending" }]),
    );
    const unResolved = listen<{ id: number; approved: boolean }>("approval://resolved", (e) =>
      setMessages((prev) =>
        prev.map((m) =>
          m.role === "approval" && m.id === e.payload.id ? { ...m, state: e.payload.approved ? "approved" : "declined" } : m,
        ),
      ),
    );
    const refreshUnseen = () => api.inboxUnseen().then(setUnseen);
    refreshUnseen();
    const unInbox = listen("inbox://changed", refreshUnseen);
    return () => {
      for (const un of [unToken, unTool, unProactive, unApproval, unResolved, unInbox]) un.then((f) => f());
    };
  }, []);

  useEffect(() => bottomRef.current?.scrollIntoView({ behavior: "smooth" }), [messages, toolNote, tab]);

  async function send() {
    const text = draft.trim();
    if (!text || busy) return;
    setDraft("");
    setBusy(true);
    setMessages((prev) => [...prev, { role: "user", content: text }]);
    try {
      await api.send(text);
    } catch (err) {
      setMessages((prev) => [...prev, { role: "error", content: String(err) }]);
    } finally {
      setBusy(false);
      setToolNote(null);
    }
  }

  const name = persona?.name ?? "Fanta";
  const tabs: { id: Tab; label: string; badge?: number }[] = [
    { id: "chat", label: "Chat" },
    { id: "tasks", label: "Tasks", badge: openTasks },
    { id: "inbox", label: "Inbox", badge: unseen },
    { id: "calendar", label: "Calendar" },
  ];

  return (
    <main className="app">
      <header>
        <div className="avatar">{name[0]}</div>
        <div className="header-text">
          <h1>{name}</h1>
          <p>{persona ? `${persona.tagline} · ${persona.model}` : "waking up…"}</p>
        </div>
        <button className="icon-btn" title="Connections" onClick={() => setShowConnections(!showConnections)}>
          ⚙
        </button>
      </header>

      {showConnections ? (
        <div className="pane">
          <Connections onClose={() => setShowConnections(false)} />
        </div>
      ) : (
        <>
          <FocusBar />

          <nav className="tabs">
            {tabs.map((t) => (
              <button key={t.id} className={tab === t.id ? "active" : ""} onClick={() => setTab(t.id)}>
                {t.label}
                {!!t.badge && <span className="badge">{t.badge}</span>}
              </button>
            ))}
          </nav>
        </>
      )}

      <div className={!showConnections && tab === "tasks" ? "pane" : "pane hidden"}>
        <Tasks onCount={setOpenTasks} />
      </div>

      <div className={!showConnections && tab === "inbox" ? "pane" : "pane hidden"}>
        <Inbox visible={!showConnections && tab === "inbox"} />
      </div>

      <div className={!showConnections && tab === "calendar" ? "pane" : "pane hidden"}>
        <CalendarView onConnect={() => setShowConnections(true)} />
      </div>

      <div className={!showConnections && tab === "chat" ? "pane" : "pane hidden"}>
        <section className="chat">
          {messages.length === 0 && <div className="empty">Hey, I'm {name}. What are we focusing on today?</div>}
          {messages.map((m, i) =>
            m.role === "approval" ? (
              <div key={i} className={`approval ${m.state}`}>
                <div className="approval-label">{name} wants to</div>
                <div>{m.content}</div>
                {m.state === "pending" ? (
                  <div className="row">
                    <button className="primary" onClick={() => api.answerApproval(m.id, true)}>
                      Approve
                    </button>
                    <button className="ghost" onClick={() => api.answerApproval(m.id, false)}>
                      Deny
                    </button>
                  </div>
                ) : (
                  <div className="muted small">{m.state === "approved" ? "✓ Approved" : "✕ Declined"}</div>
                )}
              </div>
            ) : (
              <div key={i} className={`bubble ${m.role}`}>
                <Linkified text={m.content} />
              </div>
            ),
          )}
          {busy && (toolNote || messages[messages.length - 1]?.role === "user") && (
            <div className="bubble assistant pending">{toolNote ?? "thinking…"}</div>
          )}
          <div ref={bottomRef} />
        </section>

        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <input autoFocus value={draft} onChange={(e) => setDraft(e.target.value)} placeholder={`Message ${name}…`} />
          <button disabled={busy || !draft.trim()}>Send</button>
        </form>
      </div>
    </main>
  );
}
