import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, FocusStats, hm, parseTs, PomodoroConfig, Task } from "../api";
import { PauseIcon, PlayIcon, SkipIcon, StopIcon } from "../ui/Icons";
import { Select } from "../ui/Select";
import { mmss, startFocus, useFocusStatus } from "./focus";

const KIND_LABEL = { focus: "Focus", short_break: "Short break", long_break: "Long break" } as const;

function fmtMinutes(m: number) {
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60 ? `${m % 60}m` : ""}`.trim();
}

/** The big countdown ring. */
function Ring({ progress, kind, children }: { progress: number; kind: string; children: React.ReactNode }) {
  const r = 78;
  const c = 2 * Math.PI * r;
  return (
    <div className={`big-ring ${kind}`}>
      <svg viewBox="0 0 180 180" width="180" height="180">
        <circle cx="90" cy="90" r={r} className="track" />
        <circle cx="90" cy="90" r={r} className="arc" strokeDasharray={c} strokeDashoffset={c * (1 - progress)} transform="rotate(-90 90 90)" />
      </svg>
      <div className="ring-center">{children}</div>
    </div>
  );
}

/** Today's pomodoros against the daily goal. */
function GoalRing({ done, goal }: { done: number; goal: number }) {
  const r = 26;
  const c = 2 * Math.PI * r;
  const p = Math.min(1, done / Math.max(1, goal));
  return (
    <svg viewBox="0 0 64 64" width="64" height="64" className="goal-ring" role="img" aria-label={`${done} of ${goal} pomodoros`}>
      <circle cx="32" cy="32" r={r} className="track" />
      <circle cx="32" cy="32" r={r} className="arc" strokeDasharray={c} strokeDashoffset={c * (1 - p)} transform="rotate(-90 32 32)" />
      <text x="32" y="37" textAnchor="middle">
        {Math.round(p * 100)}%
      </text>
    </svg>
  );
}

/** Pomodoros per day this week, with the daily goal as a dashed line. */
function WeekChart({ week, goal }: { week: FocusStats["week"]; goal: number }) {
  const [hover, setHover] = useState<number | null>(null);
  const W = 340;
  const H = 120;
  const top = 18;
  const base = H - 18;
  const max = Math.max(goal, ...week.map((d) => d.pomodoros), 1);
  const y = (v: number) => base - (v / max) * (base - top);
  const slot = W / 7;
  const bw = 22;
  return (
    <div className="chart-wrap">
      <svg viewBox={`0 0 ${W} ${H}`} className="week-chart" role="img" aria-label="Pomodoros per day this week">
        <line x1="0" x2={W} y1={base} y2={base} className="axis" />
        <line x1="0" x2={W} y1={y(goal)} y2={y(goal)} className="goal-line" />
        <text x={W} y={y(goal) - 4} textAnchor="end" className="goal-label">
          goal {goal}
        </text>
        {week.map((d, i) => {
          const x = i * slot + (slot - bw) / 2;
          const h = Math.max(0, base - y(d.pomodoros));
          return (
            <g key={d.date} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
              {/* hit target bigger than the bar */}
              <rect x={i * slot} y={0} width={slot} height={H} fill="transparent" />
              {h > 0 && <path className={`bar ${d.is_today ? "today" : ""}`} d={roundedTop(x, base, bw, h, 4)} />}
              {d.pomodoros > 0 && (
                <text x={x + bw / 2} y={base - h - 4} textAnchor="middle" className="bar-value">
                  {d.pomodoros}
                </text>
              )}
              <text x={x + bw / 2} y={H - 4} textAnchor="middle" className={`day-label ${d.is_today ? "today" : ""}`}>
                {d.label}
              </text>
            </g>
          );
        })}
      </svg>
      {hover !== null && (
        <div className="chart-tip" style={{ left: `${((hover + 0.5) / 7) * 100}%` }}>
          <b>{new Date(week[hover].date + "T12:00").toLocaleDateString([], { weekday: "long", day: "numeric", month: "short" })}</b>
          <span>
            {week[hover].pomodoros} pomodoro{week[hover].pomodoros === 1 ? "" : "s"} · {fmtMinutes(week[hover].minutes)} focused
          </span>
        </div>
      )}
    </div>
  );
}

/** A bar path with rounded top corners, anchored flat on the baseline. */
function roundedTop(x: number, base: number, w: number, h: number, r: number) {
  const rr = Math.min(r, h, w / 2);
  return `M${x} ${base} V${base - h + rr} Q${x} ${base - h} ${x + rr} ${base - h} H${x + w - rr} Q${x + w} ${base - h} ${x + w} ${base - h + rr} V${base} Z`;
}

function HourChart({ byHour }: { byHour: number[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const hours = byHour.slice(6, 24); // 6:00 to 23:00 - nights are empty for most people
  const max = Math.max(1, ...hours);
  const best = hours.indexOf(Math.max(...hours));
  return (
    <div className="chart-wrap">
      <div className="hour-chart" role="img" aria-label="Pomodoros by starting hour, last 30 days">
        {hours.map((n, i) => (
          <div key={i} className="hour-col" onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
            <div className={`hour-bar ${n === 0 ? "zero" : ""}`} style={{ height: `${n === 0 ? 3 : 8 + (n / max) * 44}px` }} />
            {/* every column keeps a label slot so all bars share one baseline */}
            <span className={i === best && n > 0 ? "best" : ""}>{i % 3 === 0 || (i === best && n > 0) ? i + 6 : ""}</span>
          </div>
        ))}
      </div>
      {hover !== null && (
        <div className="chart-tip" style={{ left: `${((hover + 0.5) / hours.length) * 100}%` }}>
          <b>
            {hover + 6}:00–{hover + 7}:00
          </b>
          <span>{hours[hover]} pomodoros started</span>
        </div>
      )}
    </div>
  );
}

function Settings({ config, onSaved }: { config: PomodoroConfig; onSaved: () => void }) {
  const [c, setC] = useState(config);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setC(config), [config]);
  const num = (key: keyof PomodoroConfig, label: string, unit: string, min: number, max: number) => (
    <label className="num-field">
      <span>{label}</span>
      <div className="stepper">
        <button onClick={() => setC({ ...c, [key]: Math.max(min, (c[key] as number) - 1) })}>−</button>
        <input
          type="number"
          min={min}
          max={max}
          value={c[key] as number}
          onChange={(e) => setC({ ...c, [key]: Math.min(max, Math.max(min, Number(e.target.value) || min)) })}
        />
        <button onClick={() => setC({ ...c, [key]: Math.min(max, (c[key] as number) + 1) })}>+</button>
        <em>{unit}</em>
      </div>
    </label>
  );
  return (
    <div className="timer-settings">
      {num("focus_minutes", "Focus", "min", 1, 180)}
      {num("short_break_minutes", "Short break", "min", 1, 60)}
      {num("long_break_minutes", "Long break", "min", 1, 120)}
      {num("long_break_every", "Long break every", "pomodoros", 2, 12)}
      {num("daily_goal", "Daily goal", "pomodoros", 1, 30)}
      <label className="check">
        <input type="checkbox" checked={c.auto_start_breaks} onChange={(e) => setC({ ...c, auto_start_breaks: e.target.checked })} />
        <span>Start breaks automatically</span>
      </label>
      <label className="check">
        <input type="checkbox" checked={c.auto_start_focus} onChange={(e) => setC({ ...c, auto_start_focus: e.target.checked })} />
        <span>Start the next pomodoro automatically after a break</span>
      </label>
      <div className="form-actions">
        <button
          className="btn primary"
          onClick={async () => {
            try {
              await api.setPomodoroConfig(c);
              setError(null);
              onSaved();
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          Save
        </button>
      </div>
      {error && <p className="error-text">{error}</p>}
    </div>
  );
}

export default function FocusView({ visible }: { visible: boolean }) {
  const { status, remaining, progress } = useFocusStatus();
  const [stats, setStats] = useState<FocusStats | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskId, setTaskId] = useState<number | undefined>(undefined);
  const [showSettings, setShowSettings] = useState(false);

  useEffect(() => {
    if (!visible) return;
    const refresh = () => api.focusStats().then(setStats);
    const refreshTasks = () => api.listTasks(false).then(setTasks);
    refresh();
    refreshTasks();
    const subs = [listen("focus://changed", refresh), listen("tasks://changed", refreshTasks)];
    const t = setInterval(refresh, 60_000);
    return () => {
      subs.forEach((p) => p.then((f) => f()));
      clearInterval(t);
    };
  }, [visible]);

  if (!status) return null;
  const s = status.active;
  const cfg = status.config;
  const paused = !!s?.paused_at;
  const kind = s?.kind ?? "idle";
  const inSet = status.cycle_done % cfg.long_break_every;

  // Session log grouped by day.
  const days: { day: string; items: FocusStats["history"] }[] = [];
  for (const h of stats?.history ?? []) {
    const day = parseTs(h.started_at).toDateString();
    const last = days[days.length - 1];
    if (last?.day === day) last.items.push(h);
    else days.push({ day, items: [h] });
  }
  const dayName = (d: string) => {
    const today = new Date().toDateString();
    const yesterday = new Date(Date.now() - 86_400_000).toDateString();
    return d === today ? "Today" : d === yesterday ? "Yesterday" : new Date(d).toLocaleDateString([], { weekday: "long", day: "numeric", month: "short" });
  };

  return (
    <section className="focus-view">
      {/* ---- timer ---- */}
      <div className={`card timer-card ${kind} ${paused ? "paused" : ""}`}>
        <Ring progress={s ? progress : 0} kind={kind}>
          <div className="ring-kind">{s ? (paused ? "Paused" : KIND_LABEL[s.kind]) : "Ready"}</div>
          <div className="ring-time">{s ? mmss(remaining) : `${cfg.focus_minutes}:00`}</div>
          {s && status.task_title && <div className="ring-task">{status.task_title}</div>}
        </Ring>

        <div className="cycle" title={`Long break after every ${cfg.long_break_every} pomodoros`}>
          {Array.from({ length: cfg.long_break_every }, (_, i) => (
            <span key={i} className={`cycle-dot ${i < inSet ? "done" : ""} ${i === inSet && kind === "focus" ? "now" : ""}`} />
          ))}
          <span className="muted tiny">
            {status.next_break === "long_break" && kind !== "focus" ? "long break earned" : `${cfg.long_break_every - inSet} to a long break`}
          </span>
        </div>

        {s ? (
          <div className="timer-controls">
            <button className="round-btn big" onClick={() => (paused ? api.resumeFocus() : api.pauseFocus())} title={paused ? "Resume" : "Pause"}>
              {paused ? <PlayIcon size={22} /> : <PauseIcon size={22} />}
            </button>
            <button className="round-btn" onClick={() => api.extendFocus(5)} title="Add 5 minutes">
              +5
            </button>
            <button className="round-btn" onClick={() => api.skipFocus()} title={s.kind === "focus" ? "Skip to break" : "End break"}>
              <SkipIcon size={18} />
            </button>
            <button className="round-btn" onClick={() => api.stopFocus()} title="Stop">
              <StopIcon size={16} />
            </button>
          </div>
        ) : (
          <>
            <Select
              className="task-pick"
              value={taskId ?? 0}
              onChange={(v) => setTaskId(v || undefined)}
              options={[{ value: 0, label: "No task - just focus", icon: "🎯" }, ...tasks.map((t) => ({ value: t.id, label: t.title, icon: "📝" }))]}
            />
            <div className="timer-controls">
              <button className="btn primary big-start" onClick={() => startFocus(undefined, taskId)}>
                <PlayIcon size={16} /> Start pomodoro
              </button>
            </div>
            <div className="break-buttons">
              <button className="btn soft small" onClick={() => api.startBreak(false)}>
                ☕ Short break · {cfg.short_break_minutes}m
              </button>
              <button className="btn soft small" onClick={() => api.startBreak(true)}>
                🌿 Long break · {cfg.long_break_minutes}m
              </button>
            </div>
          </>
        )}
      </div>

      {stats && (
        <>
          {/* ---- today ---- */}
          <div className="card today-card">
            <GoalRing done={stats.today.pomodoros} goal={stats.today.goal} />
            <div className="today-text">
              <div className="today-big">
                {stats.today.pomodoros} <span>/ {stats.today.goal} pomodoros today</span>
              </div>
              <div className="muted small">
                {fmtMinutes(stats.today.focus_minutes)} focused · {fmtMinutes(stats.today.break_minutes)} breaks
                {stats.today.interrupted > 0 && ` · ${stats.today.interrupted} stopped early`}
              </div>
            </div>
          </div>

          {/* ---- KPIs ---- */}
          <div className="kpis">
            <div className="kpi">
              <div className="kpi-value">🔥 {stats.streak}</div>
              <div className="kpi-label">day streak · best {stats.best_streak}</div>
            </div>
            <div className="kpi">
              <div className="kpi-value">{(stats.last30.focus_minutes / 60).toFixed(1)}h</div>
              <div className="kpi-label">focused · 30 days</div>
            </div>
            <div className="kpi">
              <div className="kpi-value">{Math.round(stats.last30.completion_rate * 100)}%</div>
              <div className="kpi-label">completed</div>
            </div>
            <div className="kpi">
              <div className="kpi-value">{stats.last30.avg_per_active_day.toFixed(1)}</div>
              <div className="kpi-label">per active day</div>
            </div>
          </div>

          {/* ---- week ---- */}
          <div className="card">
            <div className="card-head">
              <div className="card-title">This week</div>
              <span className="muted small">
                {stats.week_pomodoros} pomodoros · {fmtMinutes(stats.week_minutes)}
              </span>
            </div>
            <WeekChart week={stats.week} goal={stats.today.goal} />
          </div>

          {/* ---- tasks & hours ---- */}
          {stats.by_task.length > 0 && (
            <div className="card">
              <div className="card-head">
                <div className="card-title">Where your focus went</div>
                <span className="muted small">30 days</span>
              </div>
              <ul className="task-bars">
                {stats.by_task.map((t) => {
                  const max = Math.max(...stats.by_task.map((x) => x.minutes), 1);
                  return (
                    <li key={t.title} title={`${t.pomodoros} pomodoros · ${fmtMinutes(t.minutes)}`}>
                      <div className="task-bar-label">
                        <span>{t.title}</span>
                        <b>
                          {t.pomodoros} · {fmtMinutes(t.minutes)}
                        </b>
                      </div>
                      <div className="task-bar-track">
                        <div className="task-bar-fill" style={{ width: `${(t.minutes / max) * 100}%` }} />
                      </div>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}

          {stats.by_hour.some((n) => n > 0) && (
            <div className="card">
              <div className="card-head">
                <div className="card-title">Best focus hours</div>
                <span className="muted small">30 days</span>
              </div>
              <HourChart byHour={stats.by_hour} />
            </div>
          )}

          {/* ---- history ---- */}
          <div className="card">
            <div className="card-title">History</div>
            {days.length === 0 && <p className="muted small">Finished pomodoros and breaks show up here.</p>}
            {days.slice(0, 7).map((d) => (
              <div key={d.day} className="history-day">
                <div className="history-day-name">
                  {dayName(d.day)}
                  <span className="muted">
                    {" "}
                    · {d.items.filter((i) => i.kind === "focus" && i.completed).length} 🍅
                  </span>
                </div>
                <ul>
                  {d.items.map((h) => (
                    <li key={h.id} className={`history-item ${h.kind} ${h.completed ? "" : "stopped"}`}>
                      <span className="history-icon">{h.kind === "focus" ? "🍅" : h.kind === "long_break" ? "🌿" : "☕"}</span>
                      <span className="history-time" title={h.ended_at ? `${hm(h.started_at)}–${hm(h.ended_at)}` : undefined}>
                        {hm(h.started_at)}
                      </span>
                      <span className="history-what">{h.kind === "focus" ? h.task_title ?? "Focus" : KIND_LABEL[h.kind]}</span>
                      <span className="history-min">
                        {h.completed ? fmtMinutes(h.actual_minutes) : `stopped at ${fmtMinutes(h.actual_minutes)}`}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </>
      )}

      {/* ---- settings ---- */}
      <div className="card">
        <button className="settings-toggle" onClick={() => setShowSettings(!showSettings)}>
          <span>⚙ Timer settings</span>
          <span className="muted small">
            {cfg.focus_minutes}/{cfg.short_break_minutes}/{cfg.long_break_minutes} min {showSettings ? "▴" : "▾"}
          </span>
        </button>
        {showSettings && <Settings config={cfg} onSaved={() => setShowSettings(false)} />}
      </div>
    </section>
  );
}
