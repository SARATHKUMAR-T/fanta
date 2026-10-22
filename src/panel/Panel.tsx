import React, { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CalEvent, Persona, toTs } from "../api";
import CalendarView from "../CalendarView";
import Character from "../companion/Character";
import "../companion/character.css";
import { useMood } from "../companion/useMood";
import Connections from "../Connections";
import Inbox from "../Inbox";
import Tasks from "../Tasks";
import { CalendarIcon, ChatIcon, CloseIcon, GearIcon, InboxIcon, TasksIcon } from "../ui/Icons";
import Chat from "./Chat";
import { FocusPill, mmss, useFocusNote, useFocusStatus } from "./focus";
import Brain from "./Brain";
import Plugins from "./Plugins";
import VoiceSettings from "./VoiceSettings";
import "./panel.css";

type View = "chat" | "tasks" | "inbox" | "calendar" | "settings";

function useStatusLine(name: string, openTasks: number) {
  const { status, remaining } = useFocusStatus();
  const [next, setNext] = useState<CalEvent | null>(null);
  useEffect(() => {
    const refresh = async () => {
      const now = new Date();
      const later = new Date(now.getTime() + 2 * 3600_000);
      const events = await api.calendarEvents(toTs(now), toTs(later)).catch(() => []);
      setNext(events.find((e) => !e.all_day && e.response !== "declined" && e.start_at >= toTs(now)) ?? null);
    };
    refresh();
    const un = listen("calendar://changed", refresh);
    const t = setInterval(refresh, 60_000);
    return () => {
      un.then((f) => f());
      clearInterval(t);
    };
  }, []);
  const s = status?.active;
  if (s?.kind === "focus") return `Focusing · ${mmss(remaining)} left`;
  if (s?.kind === "break") return `On a break · ${mmss(remaining)}`;
  if (next) return `Next: ${next.subject} at ${next.start_at.slice(11, 16)}`;
  if (openTasks > 0) return `${openTasks} task${openTasks > 1 ? "s" : ""} open`;
  return `${name} is here for you`;
}

export default function Panel() {
  const [persona, setPersona] = useState<Persona | null>(null);
  // ?tab= lets the dev preview open a specific view.
  const [view, setView] = useState<View>(() => (new URLSearchParams(location.search).get("tab") as View) || "chat");
  const [openTasks, setOpenTasks] = useState(0);
  const [unseen, setUnseen] = useState(0);
  const note = useFocusNote();
  // The panel window stays mounted while hidden; only count the Inbox as "read" when it's on screen.
  const [shown, setShown] = useState(true);
  const mood = useMood();
  const name = persona?.name ?? "Fanta";
  const statusLine = useStatusLine(name, openTasks);
  const onCount = useCallback((n: number) => setOpenTasks(n), []);

  useEffect(() => {
    api.persona().then(setPersona);
    const refreshUnseen = () => api.inboxUnseen().then(setUnseen);
    refreshUnseen();
    const subs = [
      listen("inbox://changed", refreshUnseen),
      listen<boolean>("panel://visible", (e) => setShown(e.payload)),
      listen("ui://reply-to", () => setView("chat")),
      listen("ui://open-settings", () => setView("settings")),
    ];
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && api.hidePanel();
    window.addEventListener("keydown", onKey);
    return () => {
      subs.forEach((p) => p.then((f) => f()));
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const tabs: { id: View; label: string; icon: React.ReactNode; badge?: number }[] = [
    { id: "chat", label: "Chat", icon: <ChatIcon size={16} /> },
    { id: "tasks", label: "Tasks", icon: <TasksIcon size={16} />, badge: openTasks },
    { id: "inbox", label: "Inbox", icon: <InboxIcon size={16} />, badge: unseen },
    { id: "calendar", label: "Calendar", icon: <CalendarIcon size={16} /> },
  ];

  return (
    <div className="panel">
      <header className="panel-head" data-tauri-drag-region>
        <div className="head-avatar" data-tauri-drag-region>
          <Character mood={mood} size={44} face />
        </div>
        <div className="head-text" data-tauri-drag-region>
          <div className="head-name" data-tauri-drag-region>
            {name}
            <span className="online" />
          </div>
          <div className="head-status" data-tauri-drag-region>
            {statusLine}
          </div>
        </div>
        <FocusPill />
        <button className={`icon-btn ${view === "settings" ? "active" : ""}`} title="Settings" onClick={() => setView(view === "settings" ? "chat" : "settings")}>
          <GearIcon size={18} />
        </button>
        <button className="icon-btn" title="Close (Esc)" onClick={() => api.hidePanel()}>
          <CloseIcon size={18} />
        </button>
      </header>

      {view !== "settings" && (
        <nav className="seg">
          {tabs.map((t) => (
            <button key={t.id} className={view === t.id ? "active" : ""} onClick={() => setView(t.id)}>
              {t.icon}
              <span>{t.label}</span>
              {!!t.badge && <em>{t.badge > 9 ? "9+" : t.badge}</em>}
            </button>
          ))}
        </nav>
      )}

      {note && <div className="toast">{note}</div>}

      <main className="panel-body">
        <div className={view === "chat" ? "view" : "view hidden"}>
          <Chat name={name} />
        </div>
        <div className={view === "tasks" ? "view scroll" : "view hidden"}>
          <Tasks onCount={onCount} />
        </div>
        <div className={view === "inbox" ? "view scroll" : "view hidden"}>
          <Inbox visible={shown && view === "inbox"} />
        </div>
        <div className={view === "calendar" ? "view scroll" : "view hidden"}>
          <CalendarView onConnect={() => setView("settings")} />
        </div>
        {view === "settings" && (
          <div className="view scroll settings">
            <div className="settings-head">
              <button className="btn ghost" onClick={() => setView("chat")}>
                ← Back
              </button>
              <h2>Settings</h2>
            </div>
            <Brain />
            <VoiceSettings />
            <Connections />
            <Plugins />
            <section className="card">
              <div className="card-title">✨ {name}'s personality</div>
              <p className="muted small">
                Name, tone, workday, pomodoro length, notification rules and the model live in <code>persona.yaml</code>. Edit it and restart {name}.
              </p>
              <div className="form-actions">
                <button className="btn ghost" onClick={() => api.openConfigDir()}>
                  Open config folder
                </button>
              </div>
            </section>
          </div>
        )}
      </main>
    </div>
  );
}
