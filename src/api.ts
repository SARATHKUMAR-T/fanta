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

export type Category = "urgent" | "fyi" | "ignore";

export type InboxItem = {
  id: number;
  app: string;
  source: string;
  title: string;
  body: string;
  category: Category | "pending";
  summary: string | null;
  reason: string | null;
  during_focus: boolean;
  seen: boolean;
  received_at: string;
};

export type CalEvent = {
  num: number;
  subject: string;
  start_at: string;
  end_at: string;
  all_day: boolean;
  location: string | null;
  join_url: string | null;
  organizer: string | null;
  response: string | null;
  show_as: string | null;
};

export type CalendarStatus = {
  provider: "none" | "graph" | "ics";
  account: string | null;
  last_sync: string | null;
  error: string | null;
  can_write: boolean;
  client_id: string | null;
  tenant: string | null;
};

export type DeviceCode = {
  user_code: string;
  verification_uri: string;
  expires_in: number;
  message: string;
};

export type ApprovalRequest = { id: number; tool: string; text: string };

export const api = {
  persona: () => invoke<Persona>("get_persona"),
  history: () => invoke<{ role: string; content: string }[]>("get_history"),
  send: (text: string) => invoke<string>("send_message", { text }),
  listTasks: (includeDone: boolean) => invoke<Task[]>("list_tasks", { includeDone }),
  addTask: (title: string) => invoke<Task>("add_task", { title }),
  setTaskDone: (id: number, done: boolean) => invoke<void>("set_task_done", { id, done }),
  deleteTask: (id: number) => invoke<void>("delete_task", { id }),
  focusStatus: () => invoke<FocusStatus>("focus_status"),
  /** Resolves to a note when the block was shortened for a meeting. */
  startFocus: (minutes?: number, taskId?: number) => invoke<string | null>("start_focus", { minutes, taskId }),
  stopFocus: () => invoke<void>("stop_focus"),
  listNotifications: (category?: Category) => invoke<InboxItem[]>("list_notifications", { category }),
  inboxUnseen: () => invoke<number>("inbox_unseen"),
  markInboxSeen: () => invoke<void>("mark_inbox_seen"),
  setNotificationCategory: (id: number, category: Category) =>
    invoke<void>("set_notification_category", { id, category }),
  answerApproval: (id: number, approved: boolean) => invoke<void>("answer_approval", { id, approved }),
  calendarStatus: () => invoke<CalendarStatus>("calendar_status"),
  calendarSignIn: (clientId: string, tenant: string) => invoke<DeviceCode>("calendar_sign_in", { clientId, tenant }),
  calendarConnectIcs: (url: string) => invoke<number>("calendar_connect_ics", { url }),
  calendarDisconnect: () => invoke<void>("calendar_disconnect"),
  calendarSync: () => invoke<void>("calendar_sync"),
  calendarEvents: (from: string, to: string) => invoke<CalEvent[]>("calendar_events", { from, to }),
  openLink: (url: string) => invoke<void>("open_link", { url }),
};

/** Local "YYYY-MM-DD HH:MM:SS" for a Date, matching the backend. */
export function toTs(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export function hm(ts: string): string {
  return parseTs(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

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
