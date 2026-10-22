import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, friendlyTime, parseTs, Task } from "./api";

export default function Tasks({ onCount }: { onCount: (n: number) => void }) {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [draft, setDraft] = useState("");
  const [showDone, setShowDone] = useState(false);

  useEffect(() => {
    const refresh = () =>
      api.listTasks(true).then((t) => {
        setTasks(t);
        onCount(t.filter((x) => !x.done).length);
      });
    refresh();
    const un = listen("tasks://changed", refresh);
    return () => {
      un.then((f) => f());
    };
  }, [onCount]);

  const open = tasks.filter((t) => !t.done);
  const done = tasks.filter((t) => t.done);

  async function add() {
    const title = draft.trim();
    if (!title) return;
    setDraft("");
    await api.addTask(title);
  }

  return (
    <section className="tasks">
      <form
        className="task-add"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Add a task…" />
      </form>

      {open.length === 0 && (
        <div className="empty">
          All clear. Add a task here, or just tell me in chat: “remind me at 4 to send the report”.
        </div>
      )}
      <ul>
        {open.map((t) => (
          <TaskRow key={t.id} task={t} />
        ))}
      </ul>

      {done.length > 0 && (
        <>
          <button className="link" onClick={() => setShowDone(!showDone)}>
            {showDone ? "Hide" : "Show"} done ({done.length})
          </button>
          {showDone && (
            <ul className="done">
              {done.map((t) => (
                <TaskRow key={t.id} task={t} />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

function TaskRow({ task: t }: { task: Task }) {
  const overdue = !t.done && t.due_at && parseTs(t.due_at).getTime() < Date.now();
  return (
    <li className="task">
      <input type="checkbox" checked={t.done} onChange={(e) => api.setTaskDone(t.id, e.target.checked)} />
      <div className="task-body">
        <span className="task-title">{t.title}</span>
        <span className="chips">
          {t.due_at && <span className={`chip ${overdue ? "overdue" : ""}`}>due {friendlyTime(t.due_at)}</span>}
          {t.remind_at && <span className="chip">⏰ {friendlyTime(t.remind_at)}</span>}
        </span>
      </div>
      {!t.done && (
        <button className="icon" title="Focus on this" onClick={() => api.startFocus(undefined, t.id)}>
          ▶
        </button>
      )}
      <button className="icon" title="Delete" onClick={() => api.deleteTask(t.id)}>
        ×
      </button>
    </li>
  );
}
