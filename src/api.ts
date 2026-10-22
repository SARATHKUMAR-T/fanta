import { invoke } from "@tauri-apps/api/core";

export type Persona = { name: string; tagline: string; model: string };

export type Task = {
  id: number;
  title: string;
  done: boolean;
  due_at: string | null;
  remind_at: string | null;
  created_at: string;
};

export type FocusSession = {
  id: number;
  kind: "focus" | "break";
  task_id: number | null;
  started_at: string;
  ends_at: string;
};

export type FocusStatus = {
  active: FocusSession | null;
  task_title: string | null;
  blocks_today: number;
  focus_minutes: number;
  break_minutes: number;
};

export const api = {
  persona: () => invoke<Persona>("get_persona"),
  history: () => invoke<{ role: string; content: string }[]>("get_history"),
  send: (text: string) => invoke<string>("send_message", { text }),
  listTasks: (includeDone: boolean) => invoke<Task[]>("list_tasks", { includeDone }),
  addTask: (title: string) => invoke<Task>("add_task", { title }),
  setTaskDone: (id: number, done: boolean) => invoke<void>("set_task_done", { id, done }),
  deleteTask: (id: number) => invoke<void>("delete_task", { id }),
  focusStatus: () => invoke<FocusStatus>("focus_status"),
  startFocus: (minutes?: number, taskId?: number) => invoke<void>("start_focus", { minutes, taskId }),
  stopFocus: () => invoke<void>("stop_focus"),
};

/** Backend timestamps are local "YYYY-MM-DD HH:MM:SS". */
export function parseTs(ts: string): Date {
  return new Date(ts.replace(" ", "T"));
}

export function friendlyTime(ts: string): string {
  const d = parseTs(ts);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const today = new Date();
  const tomorrow = new Date(today.getTime() + 86_400_000);
  if (d.toDateString() === today.toDateString()) return time;
  if (d.toDateString() === tomorrow.toDateString()) return `tomorrow ${time}`;
  return `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })} ${time}`;
}
