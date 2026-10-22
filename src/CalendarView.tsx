import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CalEvent, CalendarStatus, hm, parseTs, toTs } from "./api";

function dayStart(offset: number) {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() + offset);
  return d;
}

export default function CalendarView({ onConnect }: { onConnect: () => void }) {
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [days, setDays] = useState<{ label: string; events: CalEvent[] }[]>([]);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    const refresh = async () => {
      setStatus(await api.calendarStatus());
      const labels = ["Today", "Tomorrow"];
      setDays(
        await Promise.all(
          labels.map(async (label, i) => ({
            label,
            events: await api.calendarEvents(toTs(dayStart(i)), toTs(dayStart(i + 1))),
          })),
        ),
      );
    };
    refresh();
    const un = listen("calendar://changed", refresh);
    const tick = setInterval(() => setNow(Date.now()), 30_000);
    return () => {
      un.then((f) => f());
      clearInterval(tick);
    };
  }, []);

  if (status && status.provider === "none") {
    return (
      <section className="calendar">
        <div className="empty">
          <p>Connect your Outlook calendar and I'll give you a heads-up before meetings, keep focus blocks clear of them, and answer “when am I free?”.</p>
          <button className="primary" onClick={onConnect}>
            Connect Outlook
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="calendar">
      {status?.error && <p className="error-text small">Sync problem: {status.error}</p>}
      {days.map((d) => (
        <div key={d.label} className="day">
          <h3>{d.label}</h3>
          {d.events.length === 0 && <p className="muted">No meetings.</p>}
          <ul>
            {d.events.map((e) => {
              const start = parseTs(e.start_at).getTime();
              const end = parseTs(e.end_at).getTime();
              const live = !e.all_day && now >= start && now < end;
              const past = !e.all_day && now >= end;
              const soon = !e.all_day && start > now && start - now < 15 * 60_000;
              const declined = e.response === "declined";
              return (
                <li key={e.num} className={`event ${live ? "live" : ""} ${past ? "past" : ""} ${declined ? "declined" : ""}`}>
                  <div className="event-time">{e.all_day ? "all day" : `${hm(e.start_at)}–${hm(e.end_at)}`}</div>
                  <div className="event-body">
                    <div className="event-title">{e.subject}</div>
                    <div className="muted small">
                      {[
                        e.organizer,
                        e.location,
                        e.response === "tentativelyAccepted" && "tentative",
                        (e.response === "notResponded" || e.response === "none") && "not responded",
                        declined && "declined",
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </div>
                  </div>
                  {e.join_url && !past && (
                    <button className={live || soon ? "primary" : "ghost"} onClick={() => api.openLink(e.join_url!)}>
                      Join
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </section>
  );
}
