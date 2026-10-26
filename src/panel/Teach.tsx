import { useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { api, friendlyTime, Lesson, LessonAnswer, LessonInfo, Level } from "../api";
import Character, { Mood } from "../companion/Character";
import { UI_EVENTS } from "../companion/useMood";
import { SendIcon, TrashIcon } from "../ui/Icons";
import Diagram from "./Diagram";
import "./teach.css";

const LEVELS: { id: Level; label: string; hint: string }[] = [
  { id: "kid", label: "🧒 Simple", hint: "like I'm 10" },
  { id: "beginner", label: "🌱 Beginner", hint: "new to it" },
  { id: "intermediate", label: "🚀 Deeper", hint: "I know the basics" },
];

const SUGGESTIONS = ["How does HTTPS work?", "What is a REST API?", "How does Git branching work?", "How do vaccines train the body?", "What happens when I type a URL?", "How does a neural network learn?"];

const LOADING_LINES = ["Finding the big picture…", "Thinking of a good analogy…", "Drawing the diagrams…", "Breaking it into small steps…", "Writing a quick quiz…"];

type Phase = { kind: "home" } | { kind: "loading"; topic: string; level: Level } | { kind: "lesson"; lesson: Lesson };

export default function Teach({ mood }: { mood: Mood }) {
  const [phase, setPhase] = useState<Phase>({ kind: "home" });
  const [recent, setRecent] = useState<LessonInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Bumped when the user leaves a loading lesson, so a late result is dropped.
  const request = useRef(0);

  const refreshRecent = () => api.teachRecent().then(setRecent).catch(() => {});
  useEffect(() => {
    refreshRecent();
  }, []);

  async function start(topic: string, level: Level) {
    const t = topic.trim();
    if (!t) return;
    const mine = ++request.current;
    setError(null);
    setPhase({ kind: "loading", topic: t, level });
    emit(UI_EVENTS.thinking);
    try {
      const lesson = await api.teachLesson(t, level);
      if (mine !== request.current) return;
      setPhase({ kind: "lesson", lesson });
      emit(UI_EVENTS.lessonReady);
      refreshRecent();
    } catch (e) {
      if (mine !== request.current) return;
      setError(String(e));
      setPhase({ kind: "home" });
      emit(UI_EVENTS.error);
    }
  }

  async function open(id: number) {
    try {
      setPhase({ kind: "lesson", lesson: await api.teachGet(id) });
    } catch (e) {
      setError(String(e));
    }
  }

  function home() {
    request.current++;
    setPhase({ kind: "home" });
  }

  if (phase.kind === "loading") return <Loading topic={phase.topic} mood={mood} onCancel={home} />;
  if (phase.kind === "lesson")
    return <LessonView key={phase.lesson.id} lesson={phase.lesson} onHome={home} onRelevel={(level) => start(phase.lesson.topic, level)} />;
  return (
    <Home
      mood={mood}
      recent={recent}
      error={error}
      onStart={start}
      onOpen={open}
      onDelete={async (id) => {
        await api.teachDelete(id);
        refreshRecent();
      }}
    />
  );
}

function Home({
  mood,
  recent,
  error,
  onStart,
  onOpen,
  onDelete,
}: {
  mood: Mood;
  recent: LessonInfo[];
  error: string | null;
  onStart: (topic: string, level: Level) => void;
  onOpen: (id: number) => void;
  onDelete: (id: number) => void;
}) {
  const [topic, setTopic] = useState("");
  const [level, setLevel] = useState<Level>("beginner");

  return (
    <div className="view scroll teach-home">
      <div className="teach-hero">
        <div className="teach-hero-art">
          <div className="teach-hero-face">
            <Character mood={mood} size={78} face />
          </div>
          <span className="teach-cap" aria-hidden>
            🎓
          </span>
        </div>
        <h2>What shall we learn?</h2>
        <p>Give me any topic. I'll explain it in pictures, one small step at a time, then check it stuck with a quick quiz.</p>
      </div>

      <form
        className="teach-ask-box"
        onSubmit={(e) => {
          e.preventDefault();
          onStart(topic, level);
        }}
      >
        <input autoFocus value={topic} onChange={(e) => setTopic(e.target.value)} placeholder="e.g. How does DNS work?" aria-label="Topic" />
        <button className="send" type="submit" disabled={!topic.trim()} title="Teach me">
          <SendIcon size={18} />
        </button>
      </form>

      <div className="level-pick" role="radiogroup" aria-label="How deep">
        {LEVELS.map((l) => (
          <button key={l.id} role="radio" aria-checked={level === l.id} className={level === l.id ? "on" : ""} onClick={() => setLevel(l.id)} title={l.hint}>
            {l.label}
          </button>
        ))}
      </div>

      {error && <div className="msg-error teach-error">{error}</div>}

      <div className="teach-section-title">Try one</div>
      <div className="teach-chips">
        {SUGGESTIONS.map((s) => (
          <button key={s} onClick={() => onStart(s, level)}>
            {s}
          </button>
        ))}
      </div>

      {recent.length > 0 && (
        <>
          <div className="teach-section-title">Your lessons</div>
          <div className="lesson-list">
            {recent.map((r) => (
              <div key={r.id} className="lesson-item" onClick={() => onOpen(r.id)}>
                <span className="lesson-dot" aria-hidden>
                  {LEVELS.find((l) => l.id === r.level)?.label.split(" ")[0] ?? "📘"}
                </span>
                <span className="lesson-item-text">
                  <b>{r.title || r.topic}</b>
                  <span className="tiny muted">{friendlyTime(r.created_at)}</span>
                </span>
                <button
                  className="icon-btn"
                  title="Delete lesson"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete(r.id);
                  }}
                >
                  <TrashIcon size={15} />
                </button>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function Loading({ topic, mood, onCancel }: { topic: string; mood: Mood; onCancel: () => void }) {
  const [line, setLine] = useState(0);
  const [progress, setProgress] = useState<number | null>(null);
  useEffect(() => {
    const t = setInterval(() => setLine((l) => (l + 1) % LOADING_LINES.length), 2600);
    // Cloud models stream, so there's a real progress figure; the local model doesn't.
    const un = listen<number>("teach://progress", (e) => setProgress(e.payload));
    return () => {
      clearInterval(t);
      un.then((f) => f());
    };
  }, []);
  return (
    <div className="view scroll teach-loading">
      <Character mood={mood === "idle" ? "thinking" : mood} size={96} />
      <div className="teach-loading-topic">{topic}</div>
      <div className="teach-loading-line" key={line}>
        {LOADING_LINES[line]}
      </div>
      <div className={`teach-progress ${progress === null ? "indeterminate" : ""}`}>
        <span style={progress === null ? undefined : { width: `${Math.round(progress * 100)}%` }} />
      </div>
      <div className="skeleton-card">
        <i style={{ width: "60%" }} />
        <i style={{ width: "92%" }} />
        <i style={{ width: "80%" }} />
        <div className="skeleton-diagram" />
      </div>
      <button className="btn link" onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}

type Page = { kind: "overview" } | { kind: "step"; index: number } | { kind: "recap" } | { kind: "quiz" };

function LessonView({ lesson, onHome, onRelevel }: { lesson: Lesson; onHome: () => void; onRelevel: (level: Level) => void }) {
  const pages: Page[] = [
    { kind: "overview" },
    ...lesson.steps.map((_, index) => ({ kind: "step" as const, index })),
    { kind: "recap" },
    ...(lesson.quiz.length ? [{ kind: "quiz" as const }] : []),
  ];
  const [at, setAt] = useState(0);
  const [asking, setAsking] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const page = pages[at];
  const go = (i: number) => {
    setAt(Math.max(0, Math.min(pages.length - 1, i)));
    scroller.current?.scrollTo({ top: 0 });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest("input, textarea")) return;
      if (e.key === "ArrowRight") go(at + 1);
      if (e.key === "ArrowLeft") go(at - 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const level = LEVELS.find((l) => l.id === lesson.level);
  const simpler = lesson.level === "intermediate" ? "beginner" : "kid";
  const deeper = lesson.level === "kid" ? "beginner" : "intermediate";

  return (
    <div className="lesson">
      <div className="lesson-top">
        <button className="btn link" onClick={onHome}>
          ← Topics
        </button>
        <div className="lesson-progress" aria-label={`Page ${at + 1} of ${pages.length}`}>
          <span style={{ width: `${((at + 1) / pages.length) * 100}%` }} />
        </div>
        <span className="level-chip" title={level?.hint}>
          {level?.label ?? lesson.level}
        </span>
        <button className="ask-btn" onClick={() => setAsking(true)} title="Ask a question about this lesson">
          💬 Ask
        </button>
      </div>

      <div className="lesson-body" ref={scroller}>
        <div className="lesson-page" key={at}>
          {page.kind === "overview" && (
            <>
              <div className="eyebrow">The big picture</div>
              <h2 className="lesson-title">{lesson.title}</h2>
              <p className="lesson-summary">{lesson.summary}</p>
              <Diagram spec={lesson.overview} />
              {lesson.analogy && (
                <div className="callout analogy">
                  <span className="callout-icon">💡</span>
                  <div>
                    <b>Think of it like this</b>
                    <p>{lesson.analogy}</p>
                  </div>
                </div>
              )}
              <div className="step-map">
                {lesson.steps.map((s, i) => (
                  <button key={i} onClick={() => go(i + 1)}>
                    <span className="step-num">{i + 1}</span>
                    {s.title}
                  </button>
                ))}
              </div>
            </>
          )}

          {page.kind === "step" && <StepPage step={lesson.steps[page.index]} index={page.index} total={lesson.steps.length} />}

          {page.kind === "recap" && (
            <>
              <div className="eyebrow">Recap</div>
              <h2 className="lesson-title">What to remember</h2>
              <ul className="key-points">
                {lesson.steps.map((s, i) => (
                  <li key={i}>
                    <span className="step-num">{i + 1}</span>
                    {s.key_point || s.title}
                  </li>
                ))}
              </ul>
              {lesson.terms.length > 0 && (
                <>
                  <div className="teach-section-title">Key words</div>
                  <div className="terms">
                    {lesson.terms.map((t) => (
                      <details key={t.term} className="term">
                        <summary>{t.term}</summary>
                        <p>{t.meaning}</p>
                      </details>
                    ))}
                  </div>
                </>
              )}
              <div className="relevel">
                <span className="small muted">Didn't click, or too easy?</span>
                <div>
                  {lesson.level !== "kid" && (
                    <button className="btn small soft" onClick={() => onRelevel(simpler)}>
                      Explain simpler
                    </button>
                  )}
                  {lesson.level !== "intermediate" && (
                    <button className="btn small soft" onClick={() => onRelevel(deeper)}>
                      Go deeper
                    </button>
                  )}
                </div>
              </div>
            </>
          )}

          {page.kind === "quiz" && <Quiz quiz={lesson.quiz} onDone={onHome} />}
        </div>
      </div>

      <div className="lesson-nav">
        <button className="btn ghost" disabled={at === 0} onClick={() => go(at - 1)}>
          ‹ Back
        </button>
        <div className="page-dots">
          {pages.map((p, i) => (
            <button key={i} className={`dot ${i === at ? "on" : ""} ${i < at ? "done" : ""} ${p.kind}`} onClick={() => go(i)} aria-label={`Go to page ${i + 1}`} />
          ))}
        </div>
        {at < pages.length - 1 ? (
          <button className="btn primary" onClick={() => go(at + 1)}>
            {pages[at + 1].kind === "quiz" ? "Quiz ›" : "Next ›"}
          </button>
        ) : (
          <button className="btn primary" onClick={onHome}>
            Done
          </button>
        )}
      </div>

      {asking && <AskSheet lessonId={lesson.id} onClose={() => setAsking(false)} />}
    </div>
  );
}

function StepPage({ step, index, total }: { step: Lesson["steps"][number]; index: number; total: number }) {
  return (
    <>
      <div className="eyebrow">
        Step {index + 1} of {total}
      </div>
      <h2 className="lesson-title">{step.title}</h2>
      <Diagram spec={step.diagram} />
      <p className="lesson-text">{step.explain}</p>
      {step.example && (
        <div className="callout example">
          <span className="callout-icon">🌍</span>
          <div>
            <b>For example</b>
            <p>{step.example}</p>
          </div>
        </div>
      )}
      {step.key_point && (
        <div className="callout remember">
          <span className="callout-icon">📌</span>
          <div>
            <b>Remember</b>
            <p>{step.key_point}</p>
          </div>
        </div>
      )}
    </>
  );
}

function Quiz({ quiz, onDone }: { quiz: Lesson["quiz"]; onDone: () => void }) {
  const [picked, setPicked] = useState<(number | null)[]>(() => quiz.map(() => null));
  const answered = picked.filter((p) => p !== null).length;
  const score = picked.filter((p, i) => p === quiz[i].answer).length;
  const finished = answered === quiz.length;

  function pick(q: number, option: number) {
    if (picked[q] !== null) return;
    setPicked((p) => p.map((v, i) => (i === q ? option : v)));
    emit(UI_EVENTS.quiz, option === quiz[q].answer);
  }

  return (
    <>
      <div className="eyebrow">Check yourself</div>
      <h2 className="lesson-title">Quick quiz</h2>
      {quiz.map((q, qi) => {
        const chosen = picked[qi];
        return (
          <div key={qi} className="quiz-q">
            <div className="quiz-question">
              <span className="step-num">{qi + 1}</span>
              {q.question}
            </div>
            <div className="quiz-options">
              {q.options.map((o, oi) => {
                const state = chosen === null ? "" : oi === q.answer ? "right" : oi === chosen ? "wrong" : "dim";
                return (
                  <button key={oi} className={`quiz-option ${state}`} disabled={chosen !== null} onClick={() => pick(qi, oi)}>
                    <span className="quiz-letter">{String.fromCharCode(65 + oi)}</span>
                    {o}
                  </button>
                );
              })}
            </div>
            {chosen !== null && (
              <div className={`quiz-why ${chosen === q.answer ? "right" : "wrong"}`}>
                <b>{chosen === q.answer ? "Yes! " : "Not quite. "}</b>
                {q.why}
              </div>
            )}
          </div>
        );
      })}
      {finished && (
        <div className="quiz-score">
          <div className="quiz-score-num">
            {score}/{quiz.length}
          </div>
          <div>
            <b>{score === quiz.length ? "Perfect - you've got it!" : score >= quiz.length / 2 ? "Nice work, nearly there." : "Worth another look at the steps."}</b>
            <div className="quiz-score-actions">
              <button className="btn small ghost" onClick={() => setPicked(quiz.map(() => null))}>
                Try again
              </button>
              <button className="btn small soft" onClick={onDone}>
                New topic
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

function AskSheet({ lessonId, onClose }: { lessonId: number; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [answers, setAnswers] = useState<LessonAnswer[]>([]);
  const [error, setError] = useState<string | null>(null);
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "smooth" });
  }, [answers, busy]);

  async function ask() {
    const question = q.trim();
    if (!question || busy) return;
    setBusy(true);
    setError(null);
    emit(UI_EVENTS.thinking);
    try {
      const a = await api.teachAsk(lessonId, question);
      setAnswers((all) => [...all, a]);
      setQ("");
      emit(UI_EVENTS.lessonReady);
    } catch (e) {
      setError(String(e));
      emit(UI_EVENTS.error);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="ask-sheet-backdrop" onClick={onClose}>
      <div className="ask-sheet" onClick={(e) => e.stopPropagation()}>
        <div className="ask-sheet-head">
          <b>Ask about this lesson</b>
          <button className="btn link" onClick={onClose}>
            Done
          </button>
        </div>
        <div className="ask-sheet-body">
          {answers.length === 0 && !busy && <p className="muted small">Anything unclear? Ask, and I'll answer with a picture.</p>}
          {answers.map((a, i) => (
            <div key={i} className="ask-answer">
              <div className="ask-q">{a.question}</div>
              <p>{a.answer}</p>
              <Diagram spec={a.diagram} />
            </div>
          ))}
          {busy && (
            <div className="dots ask-busy">
              <i />
              <i />
              <i />
            </div>
          )}
          {error && <div className="msg-error">{error}</div>}
          <div ref={end} />
        </div>
        <form
          className="teach-ask-box"
          onSubmit={(e) => {
            e.preventDefault();
            ask();
          }}
        >
          <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Why does…? What if…?" aria-label="Your question" />
          <button className="send" type="submit" disabled={!q.trim() || busy} title="Ask">
            <SendIcon size={18} />
          </button>
        </form>
      </div>
    </div>
  );
}
