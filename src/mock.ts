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
    active: focusing ? { id: 1, kind: "focus", task_id: 1, started_at: ts(-7), ends_at: ts(18) } : null,
    task_title: focusing ? "Finish Q4 budget report" : null,
    blocks_today: 2,
    focus_minutes: 25,
    break_minutes: 5,
  }),
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
