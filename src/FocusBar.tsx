import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, FocusStatus, parseTs } from "./api";

const NOTE_EVENT = "fanta:focus-note";

/** Starts focus and surfaces "shortened for a meeting" notes or refusals in the focus bar. */
export async function startFocus(minutes?: number, taskId?: number) {
  let note: string | null;
  try {
    note = await api.startFocus(minutes, taskId);
  } catch (e) {
    note = String(e);
  }
  if (note) window.dispatchEvent(new CustomEvent(NOTE_EVENT, { detail: note }));
}

function mmss(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export default function FocusBar() {
  const [status, setStatus] = useState<FocusStatus | null>(null);
  const [now, setNow] = useState(Date.now());
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let timer: number | undefined;
    const onNote = (e: Event) => {
      setNote((e as CustomEvent<string>).detail);
      clearTimeout(timer);
      timer = window.setTimeout(() => setNote(null), 8000);
    };
    window.addEventListener(NOTE_EVENT, onNote);
    return () => {
      window.removeEventListener(NOTE_EVENT, onNote);
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    const refresh = () => api.focusStatus().then(setStatus);
    refresh();
    const un = listen("focus://changed", refresh);
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      un.then((f) => f());
      clearInterval(tick);
    };
  }, []);

  if (!status) return null;
  const s = status.active;
  const noteLine = note && <div className="focus-note">{note}</div>;

  if (!s) {
    return (
      <div className="focusbar">
        <div className="focus-idle">
          <button className="primary" onClick={() => startFocus()}>
            Start focus · {status.focus_minutes} min
          </button>
          <span className="muted">
            {status.blocks_today === 0
              ? "no focus blocks yet today"
              : `${status.blocks_today} block${status.blocks_today > 1 ? "s" : ""} today`}
          </span>
        </div>
        {noteLine}
      </div>
    );
  }

  const start = parseTs(s.started_at).getTime();
  const end = parseTs(s.ends_at).getTime();
  const progress = Math.min(1, Math.max(0, (now - start) / (end - start)));
  const isFocus = s.kind === "focus";

  return (
    <div className={`focusbar ${s.kind}`}>
      <div className="focus-row">
        <div>
          <div className="focus-label">
            {isFocus ? "Focusing" : "Break"}
            {status.task_title && <span className="muted"> · {status.task_title}</span>}
          </div>
          <div className="countdown">{mmss(end - now)}</div>
        </div>
        <button className="ghost" onClick={() => api.stopFocus()}>
          {isFocus ? "Stop" : "Skip break"}
        </button>
      </div>
      <div className="progress">
        <div style={{ width: `${progress * 100}%` }} />
      </div>
      {noteLine}
    </div>
  );
}
