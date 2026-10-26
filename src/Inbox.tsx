import { useEffect, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { api, Category, friendlyTime, InboxItem } from "./api";
import { Option, Select } from "./ui/Select";

type Filter = "all" | Category;

const CATEGORIES: Option<Category | "pending">[] = [
  { value: "urgent", label: "Urgent", hint: "Interrupt me", icon: <span className="cat-dot urgent" /> },
  { value: "fyi", label: "FYI", hint: "Show it, no rush", icon: <span className="cat-dot fyi" /> },
  { value: "ignore", label: "Ignore", hint: "Keep it quiet", icon: <span className="cat-dot ignore" /> },
];
const SORTING: Option<Category | "pending"> = { value: "pending", label: "Sorting…", icon: <span className="cat-dot pending" /> };

const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "urgent", label: "Urgent" },
  { id: "fyi", label: "FYI" },
  { id: "ignore", label: "Ignored" },
];

export default function Inbox({ visible }: { visible: boolean }) {
  const [items, setItems] = useState<InboxItem[]>([]);
  const [filter, setFilter] = useState<Filter>("all");

  useEffect(() => {
    const refresh = () => api.listNotifications(filter === "all" ? undefined : filter).then(setItems);
    refresh();
    const un = listen("inbox://changed", refresh);
    return () => {
      un.then((f) => f());
    };
  }, [filter]);

  // Opening the tab counts as reading what's there.
  useEffect(() => {
    if (visible && items.some((n) => !n.seen)) api.markInboxSeen();
  }, [visible, items]);

  const shown = filter === "all" ? items.filter((n) => n.category !== "ignore") : items;

  return (
    <section className="inbox">
      <div className="filters">
        {FILTERS.map((f) => (
          <button key={f.id} className={filter === f.id ? "active" : ""} onClick={() => setFilter(f.id)}>
            {f.label}
          </button>
        ))}
      </div>

      {shown.length === 0 && (
        <div className="empty">
          {filter === "all"
            ? "Nothing yet. Notifications from Teams, Slack, mail and other apps show up here, sorted by what needs you."
            : "Nothing here."}
        </div>
      )}

      <ul>
        {shown.map((n) => (
          <li key={n.id} className={`notif ${n.category} ${n.seen ? "" : "unseen"}`}>
            <div className="notif-head">
              <span className="source">{n.source}</span>
              <span className="muted">{friendlyTime(n.received_at)}</span>
              {n.during_focus && <span className="chip">held during focus</span>}
              <Select
                size="sm"
                align="end"
                minWidth={190}
                className={`cat ${n.category}`}
                value={n.category}
                title={n.reason ? `Why: ${n.reason}` : "Change category"}
                options={n.category === "pending" ? [SORTING, ...CATEGORIES] : CATEGORIES}
                onChange={(c) => c !== "pending" && api.setNotificationCategory(n.id, c)}
              />
            </div>
            <div className="notif-from">
              <b>{n.sender ?? n.title}</b>
              {n.chat && <span className="muted"> in {n.chat}</span>}
            </div>
            <div className="notif-title">{n.text ?? n.body}</div>
            {n.summary && n.category !== "pending" && <div className="notif-raw muted">Fanta: {n.summary}</div>}
            {n.source === "Teams" && (
              <button
                className="btn soft small reply-btn"
                onClick={() => emit("ui://reply-to", { id: n.id, sender: n.sender ?? n.title, chat: n.chat, text: n.text ?? n.body })}
              >
                ↩ Reply
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
