// Dev-only: fakes the Tauri backend so the UI can be rendered in a plain browser
// (http://localhost:1420/?view=panel&mock=1). Never loaded in the app.
const ts = (minsFromNow: number) => {
  const d = new Date(Date.now() + minsFromNow * 60_000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:00`;
};
const params = new URLSearchParams(location.search);
const empty = params.has("empty");
const focusing = params.has("focus");

const data: Record<string, (args: any) => unknown> = {
  get_persona: () => ({ name: "Fanta", tagline: "your calm focus coach", model: "qwen3:4b-instruct", notifications: { bubbles: "messages" } }),
  brain_status: () => ({
    provider: "anthropic",
    model: "claude-opus-5-5",
    background: false,
    keys: { anthropic: true, openai: false, gemini: false },
    models: { anthropic: "claude-opus-5-5", openai: "", gemini: "" },
    local_model: "qwen3:4b-instruct",
  }),
  get_history: () =>
    empty
      ? []
      : [
          { role: "assistant", content: "Good afternoon! ☀️ You've got the design review at 4 and two tasks open. Want to knock out the Q4 report first?" },
          { role: "user", content: "yes, remind me at 3:45 to prep for the review" },
          { role: "assistant", content: "Done - I'll nudge you at 15:45. Want a 25-minute focus block on the report until then?" },
          { role: "user", content: "let's do it" },
          { role: "assistant", content: "📅 Quick sync with Ravi in 5 min. Join: https://teams.microsoft.com/l/meetup-join/abc" },
        ],
  list_tasks: () => [
    { id: 1, title: "Finish Q4 budget report", done: false, due_at: ts(120), remind_at: null, created_at: ts(-300) },
    { id: 2, title: "Prep for design review", done: false, due_at: null, remind_at: ts(40), created_at: ts(-200) },
    { id: 3, title: "Review PR #42", done: true, due_at: null, remind_at: null, created_at: ts(-500) },
  ],
  focus_status: () => ({
    active: focusing ? { id: 1, kind: "focus", task_id: 1, started_at: ts(-7), ends_at: ts(18), planned_minutes: 25, paused_at: null } : null,
    task_title: focusing ? "Finish Q4 budget report" : null,
    blocks_today: 5,
    focus_minutes: 25,
    break_minutes: 5,
    config: { focus_minutes: 25, short_break_minutes: 5, long_break_minutes: 15, long_break_every: 4, daily_goal: 8, auto_start_breaks: true, auto_start_focus: false },
    cycle_done: 1,
    next_break: "short_break",
  }),
  focus_stats: () => {
    const today = new Date();
    const monday = new Date(today);
    monday.setDate(today.getDate() - ((today.getDay() + 6) % 7));
    const counts = [6, 8, 4, 5, 0, 0, 0];
    const week = counts.map((n, i) => {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      const iso = d.toISOString().slice(0, 10);
      return { date: iso, label: d.toLocaleDateString("en", { weekday: "short" }), pomodoros: n, minutes: n * 25, is_today: d.toDateString() === today.toDateString() };
    });
    const hist = (offset: number, kind: string, task: string | null, start: number, mins: number, done = true) => ({
      id: offset * 100 + start, kind, task_title: task, started_at: ts(-offset * 1440 - start), ended_at: ts(-offset * 1440 - start + mins), planned_minutes: 25, actual_minutes: mins, completed: done,
    });
    return {
      today: { pomodoros: 5, focus_minutes: 137, break_minutes: 30, interrupted: 1, goal: 8 },
      week, week_pomodoros: 23, week_minutes: 575, streak: 6, best_streak: 11,
      last30: { pomodoros: 96, focus_minutes: 2460, completion_rate: 0.91, active_days: 18, avg_per_active_day: 5.3 },
      by_task: [
        { title: "Finish Q4 budget report", pomodoros: 21, minutes: 540 },
        { title: "Creator studio refactor", pomodoros: 17, minutes: 430 },
        { title: "No task", pomodoros: 12, minutes: 300 },
        { title: "Review PRs", pomodoros: 6, minutes: 150 },
      ],
      by_hour: [0,0,0,0,0,0,0,0,2,9,14,11,4,3,8,10,7,5,2,1,0,0,0,0],
      history: [
        hist(0, "short_break", null, 35, 5), hist(0, "focus", "Finish Q4 budget report", 62, 25), hist(0, "focus", "Creator studio refactor", 110, 12, false),
        hist(0, "long_break", null, 140, 15), hist(1, "focus", "Review PRs", 60, 25), hist(1, "short_break", null, 90, 5),
      ],
    };
  },
  inbox_unseen: () => 3,
  list_notifications: () => [
    { id: 3, app: "Google Chrome", source: "Teams", title: "Lakshmi", body: "Need the Q4 numbers before the 3:30 sync", sender: "Lakshmi", chat: null, text: "Need the Q4 numbers before the 3:30 sync - can you send them now?", category: "urgent", summary: "Lakshmi needs Q4 numbers before 3:30 sync", reason: "manager request", during_focus: true, seen: false, received_at: ts(-12) },
    { id: 2, app: "Google Chrome", source: "Teams", title: "Aarthi Selvakumari, +3", body: "Akilesh: customer grievance agent?", sender: "Akilesh", chat: "Aarthi Selvakumari, +3", text: "customer grievance agent? or should we keep it generic", category: "fyi", summary: "Akilesh asks about grievance agent", reason: "update", during_focus: false, seen: false, received_at: ts(-40) },
    { id: 1, app: "Thunderbird", source: "Thunderbird", title: "Invoice #881", body: "Your invoice is ready", sender: "Invoice #881", chat: null, text: "Your invoice is ready", category: "fyi", summary: "Invoice #881 is ready", reason: "email", during_focus: false, seen: true, received_at: ts(-90) },
  ],
  calendar_status: () => ({ provider: "graph", account: "Sarath <sarath@example.com>", last_sync: ts(-2), error: null, can_write: true, client_id: "x", tenant: "y" }),
  calendar_events: () => [
    { num: 1, subject: "Quick sync with Ravi", start_at: ts(5), end_at: ts(20), all_day: false, location: null, join_url: "https://teams.microsoft.com/l/x", organizer: "Ravi Kumar", response: "accepted", show_as: "busy" },
    { num: 2, subject: "Design review", start_at: ts(45), end_at: ts(75), all_day: false, location: "Room 4B", join_url: null, organizer: "Kiran", response: "tentativelyAccepted", show_as: "busy" },
  ],
  plugins_info: () => [
    { name: "files", status: "running", detail: null, command: "npx -y @modelcontextprotocol/server-filesystem ~/Documents", triggers: ["files", "folder"], tools: [{ name: "read_text_file", description: "", read_only: true, auto_approve: false }, { name: "write_file", description: "", read_only: false, auto_approve: false }], log: [] },
    { name: "github", status: "error", detail: "GITHUB_PERSONAL_ACCESS_TOKEN not set", command: "npx -y @modelcontextprotocol/server-github", triggers: ["github"], tools: [], log: [] },
  ],
};

let cb = 0;
(window as any).__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: params.get("view") ?? "panel" }, currentWebview: { label: params.get("view") ?? "panel" } },
  transformCallback: () => ++cb,
  invoke: async (cmd: string, args: any) => {
    if (cmd.startsWith("plugin:event")) return cb;
    const f = data[cmd];
    return f ? f(args) : null;
  },
};
export {};
