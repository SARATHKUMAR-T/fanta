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
  newMessage: "ui://new-message", // payload: NewMessage
  hover: "ui://hover", // the pointer moved onto her
  lessonReady: "ui://lesson-ready",
  quiz: "ui://quiz", // payload: whether the answer was right
};

export type NewMessage = { kind: "urgent" | "watched" | "message"; text: string };

// What a line of text feels like, checked in this order: hurt first, so "ugh you're
// useless" pouts rather than cares, and worry before cheer.
const FEELINGS: [RegExp, Mood][] = [
  [/\b(stupid|dumb|useless|idiot|shut up|hate you|you suck|annoying)\b/i, "pout"],
  [/\b(tired|exhausted|stressed|stress|sad|overwhelmed|frustrat\w*|anxious|burn(ed|t)? ?out|bad day|sick|headache|ugh+|sigh|lonely|depressed|worried)\b|😞|😢|😭|😩/i, "caring"],
  [/\b(love (you|u)|you'?re the best|cute|adorable)\b|❤️|🥰|😍/i, "love"],
  [/\b(thank(s| you)?|thx|ty|good (job|girl|bot)|nice one|well done|awesome|you'?re great)\b/i, "shy"],
  [/\b(done|finished|shipped|merged|deployed|released|nailed it|passed|fixed it|it works|we did it|approved|congrat\w*)\b|🎉|🚀|🥳/i, "excited"],
  [/\b(lol|lmao|rofl|haha+|hehe+)\b|😂|🤣/i, "laugh"],
  [/^\s*(hi+|hey+|hello|yo|hiya|good (morning|afternoon|evening)|gm)\b/i, "happy"],
];

/** The mood a message evokes, if any. */
export function feel(text: string): Mood | null {
  return FEELINGS.find(([re]) => re.test(text))?.[1] ?? null;
}

// How long each feeling lingers; caring stays a while, a laugh is quick.
const FEEL_MS: Partial<Record<Mood, number>> = { caring: 7000, pout: 4000, love: 3500, shy: 3000, excited: 3200, laugh: 2400 };

// Someone else's message, seen through her: their stress worries her, their thanks
// and good news make her smile. Greetings and rudeness aren't aimed at her.
const ECHO: Partial<Record<Mood, Mood>> = { caring: "worried", shy: "happy", love: "happy", excited: "excited", laugh: "laugh" };
const TROUBLE = /\b(broken|failed|failing|error|down|bug|issue|crash\w*|blocked|sorry|asap|urgent)\b/i;

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
    /** Shows `mood` for `ms`, then `then` (another mood and duration) or back to the base. */
    const react = (mood: Mood, ms: number, force = false, then?: [Mood, number]) => {
      if (holding.current && !force) return;
      setReaction(mood);
      clearTimeout(timer.current);
      timer.current = window.setTimeout(() => (then ? react(then[0], then[1], force) : setReaction(null)), ms);
    };
    const onMessage = ({ kind, text }: NewMessage) => {
      const echo = TROUBLE.test(text) ? "worried" : ECHO[feel(text) ?? "idle"];
      if (kind === "urgent") return react("alert", 5000, false, ["worried", 2500]);
      if (kind === "watched") return react("excited", 3000, false, echo && echo !== "excited" ? [echo, 2200] : undefined);
      react("surprised", 1100, false, echo ? [echo, 2200] : undefined);
    };

    let openTasks: number | null = null;
    let lastKind: string | null = null;
    const refreshBase = async () => {
      const f = await api.focusStatus();
      const kind = f.active?.kind ?? null;
      if (lastKind === "focus" && kind !== null && kind !== "focus") react("proud", 3500); // finished a block
      lastKind = kind;
      const h = new Date().getHours();
      setBase(kind === "focus" && !f.active?.paused_at ? "focus" : h >= 23 || h < 6 ? "sleepy" : "idle");
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
      listen<string>(UI_EVENTS.userMessage, (e) => {
        const felt = feel(e.payload);
        // Hurt feelings pass: a short pout, then a small sad look, then she's over it.
        if (felt === "pout") react("pout", FEEL_MS.pout!, false, ["sad", 1800]);
        else if (felt) react(felt, FEEL_MS[felt] ?? 3000);
      }),
      listen(UI_EVENTS.error, () => react("sad", 4000)),
      listen<NewMessage>(UI_EVENTS.newMessage, (e) => onMessage(e.payload)),
      listen(UI_EVENTS.lessonReady, () => react("excited", 2600)),
      listen<boolean>(UI_EVENTS.quiz, (e) => (e.payload ? react("proud", 2400) : react("caring", 2800))),
      listen(UI_EVENTS.hover, () => {
        if (!holding.current && baseRef.current === "idle") react(Math.random() < 0.3 ? "shy" : "happy", 1300);
      }),
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
      listen<boolean>("panel://visible", (e) => e.payload && react("happy", 1500)),
      listen<string>("voice://state", (e) =>
        setVoiceMood(e.payload === "listening" ? "listening" : e.payload === "transcribing" ? "thinking" : null),
      ),
      listen<boolean>("voice://speaking", (e) => setVoiceMood(e.payload ? "talking" : null)),
      listen("voice://error", () => react("sad", 2500)),
    ];

    // Every so often, a small idle flourish so she feels alive.
    const IDLE_FLOURISHES: Mood[] = ["happy", "wink", "happy", "shy", "excited", "laugh"];
    const flourish = window.setInterval(() => {
      if (!holding.current && baseRef.current === "idle" && Math.random() < 0.4)
        react(IDLE_FLOURISHES[Math.floor(Math.random() * IDLE_FLOURISHES.length)], 1200);
    }, 40_000);
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
