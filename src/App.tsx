import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Persona = { name: string; tagline: string; model: string };
type Msg = { role: "user" | "assistant" | "error"; content: string };

export default function App() {
  const [persona, setPersona] = useState<Persona | null>(null);
  const [messages, setMessages] = useState<Msg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<Persona>("get_persona").then(setPersona);
    invoke<{ role: string; content: string }[]>("get_history").then((h) =>
      setMessages(h.map((m) => ({ role: m.role as Msg["role"], content: m.content }))),
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
    const unTool = listen<string>("chat://tool", (e) => setToolNote(`using ${e.payload}…`));
    return () => {
      unToken.then((f) => f());
      unTool.then((f) => f());
    };
  }, []);

  useEffect(() => bottomRef.current?.scrollIntoView({ behavior: "smooth" }), [messages, toolNote]);

  async function send() {
    const text = draft.trim();
    if (!text || busy) return;
    setDraft("");
    setBusy(true);
    setMessages((prev) => [...prev, { role: "user", content: text }]);
    try {
      await invoke<string>("send_message", { text });
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

      <section className="chat">
        {messages.length === 0 && (
          <div className="empty">Hey, I'm {name}. What are we focusing on today?</div>
        )}
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
    </main>
  );
}
