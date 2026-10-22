import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, Persona } from "./api";
import FocusBar from "./FocusBar";
import Inbox from "./Inbox";
import Tasks from "./Tasks";
import "./App.css";

type Msg = { role: "user" | "assistant" | "error"; content: string };
type Tab = "chat" | "tasks" | "inbox";

export default function App() {
  const [persona, setPersona] = useState<Persona | null>(null);
  const [messages, setMessages] = useState<Msg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("chat");
  const [openTasks, setOpenTasks] = useState(0);
  const [unseen, setUnseen] = useState(0);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.persona().then(setPersona);
    api.history().then((h) => setMessages(h.map((m) => ({ role: m.role as Msg["role"], content: m.content }))));
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
    const unTool = listen<string>("chat://tool", (e) => setToolNote(`using ${e.payload.replace(/_/g, " ")}…`));
    const unProactive = listen<string>("chat://proactive", (e) =>
      setMessages((prev) => [...prev, { role: "assistant", content: e.payload }]),
    );
    const refreshUnseen = () => api.inboxUnseen().then(setUnseen);
    refreshUnseen();
    const unInbox = listen("inbox://changed", refreshUnseen);
    return () => {
      for (const un of [unToken, unTool, unProactive, unInbox]) un.then((f) => f());
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

  return (
    <main className="app">
      <header>
        <div className="avatar">{name[0]}</div>
        <div>
          <h1>{name}</h1>
          <p>{persona ? `${persona.tagline} · ${persona.model}` : "waking up…"}</p>
        </div>
      </header>

      <FocusBar />

      <nav className="tabs">
        <button className={tab === "chat" ? "active" : ""} onClick={() => setTab("chat")}>
          Chat
        </button>
        <button className={tab === "tasks" ? "active" : ""} onClick={() => setTab("tasks")}>
          Tasks{openTasks > 0 && <span className="badge">{openTasks}</span>}
        </button>
        <button className={tab === "inbox" ? "active" : ""} onClick={() => setTab("inbox")}>
          Inbox{unseen > 0 && <span className="badge">{unseen}</span>}
        </button>
      </nav>

      <div className={tab === "tasks" ? "pane" : "pane hidden"}>
        <Tasks onCount={setOpenTasks} />
      </div>

      <div className={tab === "inbox" ? "pane" : "pane hidden"}>
        <Inbox visible={tab === "inbox"} />
      </div>

      <div className={tab === "chat" ? "pane" : "pane hidden"}>
        <section className="chat">
          {messages.length === 0 && <div className="empty">Hey, I'm {name}. What are we focusing on today?</div>}
          {messages.map((m, i) => (
            <div key={i} className={`bubble ${m.role}`}>
              {m.content}
            </div>
          ))}
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
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={`Message ${name}…`}
          />
          <button disabled={busy || !draft.trim()}>Send</button>
        </form>
      </div>
    </main>
  );
}
