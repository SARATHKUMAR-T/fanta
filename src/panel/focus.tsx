import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, FocusStatus, parseTs } from "../api";
import { PauseIcon, PlayIcon } from "../ui/Icons";

const NOTE_EVENT = "fanta:focus-note";
/** Asks the panel to show the Focus tab. */
export const OPEN_FOCUS_EVENT = "fanta:open-focus";

/** Starts focus and surfaces "shortened for a meeting" notes or refusals as a toast. */
export async function startFocus(minutes?: number, taskId?: number) {
  let note: string | null;
  try {
    note = await api.startFocus(minutes, taskId);
  } catch (e) {
    note = String(e);
  }
  if (note) window.dispatchEvent(new CustomEvent(NOTE_EVENT, { detail: note }));
}

/** The latest focus note, cleared after a few seconds. */
export function useFocusNote() {
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => {
    let timer: number | undefined;
    const onNote = (e: Event) => {
      setNote((e as CustomEvent<string>).detail);
      clearTimeout(timer);
      timer = window.setTimeout(() => setNote(null), 7000);
    };
    window.addEventListener(NOTE_EVENT, onNote);
    return () => {
      window.removeEventListener(NOTE_EVENT, onNote);
      clearTimeout(timer);
    };
  }, []);
  return note;
}

export function useFocusStatus() {
  const [status, setStatus] = useState<FocusStatus | null>(null);
  const [now, setNow] = useState(Date.now());
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
  const s = status?.active;
  // Paused: freeze at the moment it was paused.
  const at = s?.paused_at ? parseTs(s.paused_at).getTime() : now;
  const remaining = s ? Math.max(0, parseTs(s.ends_at).getTime() - at) : 0;
  const total = s ? Math.max(1, s.planned_minutes) * 60_000 : 1;
  const progress = s ? Math.min(1, Math.max(0, 1 - remaining / total)) : 0;
  return { status, remaining, progress };
}

export function mmss(ms: number) {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Header control: "Focus 25" when idle, a countdown ring with stop when running. */
export function FocusPill() {
  const { status, remaining, progress } = useFocusStatus();
  if (!status) return null;
  const s = status.active;
  if (!s) {
    return (
      <button className="focus-pill" onClick={() => startFocus()} title={`Start a ${status.focus_minutes}-minute focus block`}>
        <PlayIcon size={13} /> Focus
      </button>
    );
  }
  const r = 9;
  const c = 2 * Math.PI * r;
  return (
    <button
      className={`focus-pill running ${s.kind === "focus" ? "focus" : "break"} ${s.paused_at ? "paused" : ""}`}
      onClick={() => window.dispatchEvent(new CustomEvent(OPEN_FOCUS_EVENT))}
      title="Open the Focus tab"
    >
      <svg width="22" height="22" viewBox="0 0 22 22" className="ring">
        <circle cx="11" cy="11" r={r} className="ring-bg" />
        <circle cx="11" cy="11" r={r} className="ring-fg" strokeDasharray={c} strokeDashoffset={c * progress} />
      </svg>
      <span className="pill-time">{mmss(remaining)}</span>
      {s.paused_at && <PauseIcon size={12} />}
    </button>
  );
}
