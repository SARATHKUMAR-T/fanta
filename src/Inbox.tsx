import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, Category, friendlyTime, InboxItem } from "./api";

type Filter = "all" | Category;

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
              <select
                className={`cat ${n.category}`}
                value={n.category}
                title={n.reason ?? undefined}
                onChange={(e) => api.setNotificationCategory(n.id, e.target.value as Category)}
              >
                {n.category === "pending" && <option value="pending">sorting…</option>}
                <option value="urgent">urgent</option>
                <option value="fyi">fyi</option>
                <option value="ignore">ignore</option>
              </select>
            </div>
            <div className="notif-title">{n.summary ?? n.title}</div>
            {n.summary && (
              <div className="notif-raw muted">
                {n.title}: {n.body}
              </div>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
