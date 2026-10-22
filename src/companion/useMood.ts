import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { Mood } from "./Character";

/** Things that make Fanta react. Panel and companion both emit/listen to these. */
export const UI_EVENTS = {
  typing: "ui://typing", // the user is typing to her
  userMessage: "ui://user-message", // payload: the text they sent
  thinking: "ui://thinking",
  error: "ui://error",
  newMessage: "ui://new-message", // payload: "urgent" | "message"
};

const KIND_WORDS = /\b(thank(s| you)?|thx|love (you|it)|awesome|great job|good job|you'?re the best|nice one)\b/i;

/**
 * Fanta's current expression: a base mood from what's going on (focus, time of
 * day) plus short reactions to events (a reply streaming, a task ticked off...).
 */
export function useMood(): Mood {
  const [base, setBase] = useState<Mood>("idle");
  const [reaction, setReaction] = useState<Mood | null>(null);
  // While a voice exchange is happening, that wins over everything else.
  const [voiceMood, setVoiceMood] = useState<Mood | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const holding = useRef(false); // an approval is waiting: stay worried until answered
  const baseRef = useRef<Mood>("idle");
  baseRef.current = base;

  useEffect(() => {
    const react = (mood: Mood, ms: number, force = false) => {
      if (holding.current && !force) return;
      setReaction(mood);
      clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setReaction(null), ms);
    };

    let openTasks: number | null = null;
    let lastKind: string | null = null;
    const refreshBase = async () => {
      const f = await api.focusStatus();
      const kind = f.active?.kind ?? null;
      if (lastKind === "focus" && kind === "break") react("proud", 3500); // finished a block
      lastKind = kind;
      const h = new Date().getHours();
      setBase(kind === "focus" ? "focus" : h >= 23 || h < 6 ? "sleepy" : "idle");
    };
    const refreshTasks = async () => {
      const n = (await api.listTasks(false)).length;
      if (openTasks !== null && n < openTasks) react("proud", 3000); // ticked something off
      openTasks = n;
    };
    refreshBase();
    refreshTasks();

    const subs = [
      listen("focus://changed", refreshBase),
      listen("tasks://changed", refreshTasks),
      listen<string>("chat://token", () => react("talking", 1400)),
      listen<string>("chat://tool", () => react("thinking", 20_000)),
      listen(UI_EVENTS.thinking, () => react("thinking", 20_000)),
      listen(UI_EVENTS.typing, () => react("listening", 2500)),
      listen<string>(UI_EVENTS.userMessage, (e) => KIND_WORDS.test(e.payload) && react("love", 3200)),
      listen(UI_EVENTS.error, () => react("sad", 4000)),
      listen<string>(UI_EVENTS.newMessage, (e) => (e.payload === "urgent" ? react("alert", 5000) : react("surprised", 1600))),
      listen<string>("chat://proactive", (e) =>
        e.payload.startsWith("⏰") || e.payload.startsWith("📅") ? react("alert", 4000) : react("happy", 3000),
      ),
      listen("approval://request", () => {
        react("worried", 300_000, true);
        holding.current = true;
      }),
      listen<{ approved: boolean }>("approval://resolved", (e) => {
        holding.current = false;
        e.payload.approved ? react("wink", 2000, true) : react("idle", 10, true);
      }),
      listen<boolean>("panel://visible", (e) => e.payload && react("wave", 1800)),
      listen<string>("voice://state", (e) =>
        setVoiceMood(e.payload === "listening" ? "listening" : e.payload === "transcribing" ? "thinking" : null),
      ),
      listen<boolean>("voice://speaking", (e) => setVoiceMood(e.payload ? "talking" : null)),
      listen("voice://error", () => react("sad", 2500)),
    ];

    // Every so often, a small idle flourish so she feels alive.
    const flourish = window.setInterval(() => {
      if (!holding.current && baseRef.current === "idle" && Math.random() < 0.35) react(Math.random() < 0.5 ? "happy" : "wink", 1100);
    }, 45_000);
    const clock = window.setInterval(refreshBase, 60_000);

    return () => {
      subs.forEach((p) => p.then((f) => f()));
      clearTimeout(timer.current);
      clearInterval(flourish);
      clearInterval(clock);
    };
  }, []);

  return voiceMood ?? reaction ?? base;
}
