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
import { CalendarIcon, ChatIcon, CloseIcon, GearIcon, InboxIcon, MaximizeIcon, MinimizeIcon, RestoreIcon, TasksIcon, TeachIcon, TomatoIcon } from "../ui/Icons";
import Chat from "./Chat";
import { FocusPill, mmss, OPEN_FOCUS_EVENT, useFocusNote, useFocusStatus } from "./focus";
import FocusView from "./FocusView";
import Brain from "./Brain";
import Plugins from "./Plugins";
import SoundSettings from "./SoundSettings";
import Teach from "./Teach";
import VoiceSettings from "./VoiceSettings";
import "./panel.css";

type View = "chat" | "focus" | "tasks" | "inbox" | "calendar" | "teach" | "settings";

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
  if (s?.paused_at) return `Paused · ${mmss(remaining)} left`;
  if (s?.kind === "focus") return `Focusing · ${mmss(remaining)} left`;
  if (s) return `${s.kind === "long_break" ? "Long break" : "On a break"} · ${mmss(remaining)}`;
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
  // Enlarged: the panel fills the screen, with the tabs down the side.
  const [expanded, setExpanded] = useState(false);
  const mood = useMood();
  const name = persona?.name ?? "Fanta";
  const statusLine = useStatusLine(name, openTasks);
  const onCount = useCallback((n: number) => setOpenTasks(n), []);

  useEffect(() => {
    api.persona().then(setPersona);
    api.panelExpanded().then((e) => setExpanded(!!e));
    const refreshUnseen = () => api.inboxUnseen().then(setUnseen);
    refreshUnseen();
    const subs = [
      listen("inbox://changed", refreshUnseen),
      listen<boolean>("panel://visible", (e) => setShown(e.payload)),
      listen<boolean>("panel://expanded", (e) => setExpanded(e.payload)),
      listen("ui://reply-to", () => setView("chat")),
      listen("ui://open-settings", () => setView("settings")),
    ];
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && api.hidePanel();
    window.addEventListener("keydown", onKey);
    const openFocus = () => setView("focus");
    window.addEventListener(OPEN_FOCUS_EVENT, openFocus);
    return () => {
      subs.forEach((p) => p.then((f) => f()));
      window.removeEventListener("keydown", onKey);
      window.removeEventListener(OPEN_FOCUS_EVENT, openFocus);
    };
  }, []);

  const tabs: {
    id: View;
    label: string;
    icon: React.ReactNode;
    badge?: number;
    about?: string;
  }[] = [
    {
      id: "chat",
      label: "Chat",
      icon: <ChatIcon size={16} />,
      about: `Ask ${name} to plan, remind or take care of something`,
    },
    {
      id: "focus",
      label: "Focus",
      icon: <TomatoIcon size={16} />,
      about: "Pomodoro timer and how your focus is going",
    },
    {
      id: "tasks",
      label: "Tasks",
      icon: <TasksIcon size={16} />,
      badge: openTasks,
      about: openTasks ? `${openTasks} open` : "All clear",
    },
    {
      id: "inbox",
      label: "Inbox",
      icon: <InboxIcon size={16} />,
      badge: unseen,
      about: unseen ? `${unseen} new since you last looked` : "Messages from your notifications, sorted",
    },
    {
      id: "calendar",
      label: "Calendar",
      icon: <CalendarIcon size={16} />,
      about: "Today and tomorrow",
    },
    {
      id: "teach",
      label: "Teach",
      icon: <TeachIcon size={16} />,
      about: "Learn anything through diagrams and small steps",
    },
  ];
  const page =
    view === "settings"
      ? {
          label: "Settings",
          about: "Brain, voice, sounds, connections and plugins",
        }
      : tabs.find((t) => t.id === view)!;

  // Enlarged, the window fills the screen; dragging it around would only shift it off.
  const drag = expanded ? "false" : "true";
  const toggleSize = () => {
    setExpanded(!expanded);
    api.expandPanel(!expanded);
  };
  // Minimize tucks the panel back into Fanta; shrink it out first so it doesn't just blink away.
  const [tucking, setTucking] = useState(false);
  const minimize = () => {
    setTucking(true);
    setTimeout(() => api.minimizePanel().finally(() => setTucking(false)), 150);
  };

  return (
    <div className={`panel ${expanded ? "expanded" : ""} ${tucking ? "tucking" : ""}`}>
      <header
        className="panel-head"
        data-tauri-drag-region={drag}
        onMouseDown={(e) => {
          // A double-click on the drag area would natively maximize the window; enlarge our way instead.
          if (e.detail === 2 && !(e.target as HTMLElement).closest("button")) {
            e.stopPropagation();
            toggleSize();
          }
        }}
      >
        <div className="head-avatar" data-tauri-drag-region={drag}>
          <Character mood={mood} size={44} face />
        </div>
        <div className="head-text" data-tauri-drag-region={drag}>
          <div className="head-name" data-tauri-drag-region={drag}>
            {name}
            <span className="online" />
          </div>
          <div className="head-status" data-tauri-drag-region={drag}>
            {statusLine}
          </div>
        </div>
        <FocusPill />
        {!expanded && (
          <button
            className={`icon-btn ${view === "settings" ? "active" : ""}`}
            title="Settings"
            onClick={() => setView(view === "settings" ? "chat" : "settings")}
          >
            <GearIcon size={18} />
          </button>
        )}
        <div className="win-controls">
          <button className="icon-btn" title={`Minimize into ${name}`} aria-label="Minimize" onClick={minimize}>
            <MinimizeIcon size={17} />
          </button>
          <button
            className="icon-btn"
            title={expanded ? "Back to normal size" : "Enlarge"}
            aria-label={expanded ? "Restore size" : "Enlarge"}
            onClick={toggleSize}
          >
            {expanded ? <RestoreIcon size={16} /> : <MaximizeIcon size={16} />}
          </button>
          <button className="icon-btn close" title="Close (Esc)" aria-label="Close" onClick={() => api.hidePanel()}>
            <CloseIcon size={17} />
          </button>
        </div>
      </header>

      {(view !== "settings" || expanded) && (
        <nav className="seg">
          {tabs.map((t) => (
            <button key={t.id} className={view === t.id ? "active" : ""} onClick={() => setView(t.id)} title={t.label}>
              {t.icon}
              {(view === t.id || expanded) && <span>{t.label}</span>}
              {!!t.badge && <em>{t.badge > 9 ? "9+" : t.badge}</em>}
            </button>
          ))}
          {expanded && (
            <>
              <div className="side-spacer" />
              <button className={view === "settings" ? "active" : ""} onClick={() => setView("settings")}>
                <GearIcon size={16} />
                <span>Settings</span>
              </button>
              <div className="side-buddy">
                <Character mood={mood} size={96} />
                <div className="side-buddy-text">
                  <b>{name}</b>
                  <span>{statusLine}</span>
                </div>
              </div>
            </>
          )}
        </nav>
      )}

      {expanded && (
        <div className="page-title">
          <h1>{page.label}</h1>
          {page.about && <p>{page.about}</p>}
        </div>
      )}

      {note && <div className="toast">{note}</div>}

      <main className="panel-body">
        <div className={view === "chat" ? "view" : "view hidden"}>
          <Chat name={name} />
        </div>
        <div className={view === "focus" ? "view scroll wide" : "view hidden"}>
          <FocusView visible={shown && view === "focus"} />
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
        <div className={view === "teach" ? "view" : "view hidden"}>
          <Teach mood={mood} />
        </div>
        {view === "settings" && (
          <div className="view scroll wide settings">
            <div className="settings-head">
              <button className="btn ghost" onClick={() => setView("chat")}>
                ← Back
              </button>
              <h2>Settings</h2>
            </div>
            <div className="settings-cards">
              <Brain />
              <VoiceSettings />
              <SoundSettings />
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
          </div>
        )}
      </main>
    </div>
  );
}
