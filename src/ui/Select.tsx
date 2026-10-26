import { ReactNode, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import "./select.css";

// WebKitGTK draws native <select> and <datalist> popups with the GTK theme: white,
// unstyleable, and our light-on-dark text inherits into them. These replace both.

export type Option<T> = { value: T; label: string; hint?: string; icon?: ReactNode };

type Place = { left: number; top?: number; bottom?: number; width: number; maxHeight: number };

/** Where the list goes: under the anchor, or above it when there's no room below. */
function place(anchor: HTMLElement, minWidth: number, align: "start" | "end"): Place {
  const r = anchor.getBoundingClientRect();
  const width = Math.min(Math.max(r.width, minWidth), window.innerWidth - 16);
  const left = Math.max(8, Math.min(align === "end" ? r.right - width : r.left, window.innerWidth - width - 8));
  const below = window.innerHeight - r.bottom - 12;
  const above = r.top - 12;
  return below >= 180 || below >= above
    ? { left, top: r.bottom + 6, width, maxHeight: Math.min(300, below) }
    : { left, bottom: window.innerHeight - r.top + 6, width, maxHeight: Math.min(300, above) };
}

/** The open list, shared by Select and Combo. Lives in a portal so scroll areas can't clip it. */
function useListbox(count: number, minWidth: number, align: "start" | "end") {
  const anchor = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [pos, setPos] = useState<Place | null>(null);

  useLayoutEffect(() => {
    if (open && anchor.current) setPos(place(anchor.current, minWidth, align));
  }, [open, count, minWidth, align]);

  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!anchor.current?.contains(t) && !list.current?.contains(t)) setOpen(false);
    };
    // Scrolling the page under it would leave the list floating in the wrong place.
    const scrolled = (e: Event) => !list.current?.contains(e.target as Node) && setOpen(false);
    const resized = () => setOpen(false);
    document.addEventListener("mousedown", outside);
    document.addEventListener("scroll", scrolled, true);
    window.addEventListener("resize", resized);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("scroll", scrolled, true);
      window.removeEventListener("resize", resized);
    };
  }, [open]);

  useEffect(() => {
    if (open && active >= 0) list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  return { anchor, list, open, setOpen, active, setActive, pos };
}

function Listbox<T>({
  id,
  box,
  options,
  selected,
  onPick,
  empty,
}: {
  id: string;
  box: ReturnType<typeof useListbox>;
  options: Option<T>[];
  selected?: T;
  onPick: (o: Option<T>) => void;
  empty?: string;
}) {
  if (!box.open || !box.pos) return null;
  const { maxHeight, ...at } = box.pos;
  return createPortal(
    <ul id={id} ref={box.list} role="listbox" className={`select-list ${at.bottom !== undefined ? "up" : ""}`} style={{ ...at, maxHeight }}>
      {options.length === 0 && empty && <li className="select-empty">{empty}</li>}
      {options.map((o, i) => (
        <li
          key={String(o.value)}
          id={`${id}-${i}`}
          role="option"
          aria-selected={o.value === selected}
          className={`${i === box.active ? "active" : ""} ${o.value === selected ? "selected" : ""}`}
          // mousedown, not click: keeps focus on the trigger or input.
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(o);
          }}
          onMouseEnter={() => box.setActive(i)}
        >
          {o.icon && <span className="select-icon">{o.icon}</span>}
          <span className="select-text">
            <span className="select-label">{o.label}</span>
            {o.hint && <span className="select-hint">{o.hint}</span>}
          </span>
          {o.value === selected && <Check />}
        </li>
      ))}
    </ul>,
    document.body,
  );
}

const Check = () => (
  <svg className="select-check" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="m5 12.5 4.5 4.5L19 7.5" />
  </svg>
);

/** Arrow keys, Enter and Escape for an open list. Returns true when it used the key. */
function navigate<T>(e: React.KeyboardEvent, box: ReturnType<typeof useListbox>, options: Option<T>[], pick: (o: Option<T>) => void) {
  const n = options.length;
  if (e.key === "Escape" && box.open) {
    // Don't let the panel's own Escape handler close the whole window.
    e.stopPropagation();
    box.setOpen(false);
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    if (!box.open) box.setOpen(true);
    else if (n) box.setActive((a) => (e.key === "ArrowDown" ? (a + 1) % n : (a - 1 + n) % n));
  } else if (e.key === "Home" && box.open) box.setActive(0);
  else if (e.key === "End" && box.open) box.setActive(n - 1);
  else if (e.key === "Enter" && box.open && box.active >= 0 && options[box.active]) pick(options[box.active]);
  else if (e.key === "Tab") {
    box.setOpen(false);
    return false;
  } else return false;
  e.preventDefault();
  return true;
}

/** A dropdown that matches the app, in place of a native <select>. */
export function Select<T extends string | number>({
  value,
  options,
  onChange,
  className = "",
  title,
  size = "md",
  align = "start",
  minWidth = 0,
}: {
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  className?: string;
  title?: string;
  size?: "sm" | "md";
  align?: "start" | "end";
  minWidth?: number;
}) {
  const id = useId();
  const box = useListbox(options.length, minWidth, align);
  const current = options.find((o) => o.value === value);
  const pick = (o: Option<T>) => {
    box.setOpen(false);
    if (o.value !== value) onChange(o.value);
  };

  useEffect(() => {
    if (box.open) box.setActive(Math.max(0, options.findIndex((o) => o.value === value)));
  }, [box.open]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div ref={box.anchor} className={`select ${size} ${box.open ? "open" : ""} ${className}`}>
      <button
        type="button"
        className="select-trigger"
        title={title}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={box.open}
        aria-controls={id}
        aria-activedescendant={box.open && box.active >= 0 ? `${id}-${box.active}` : undefined}
        onClick={() => box.setOpen(!box.open)}
        onKeyDown={(e) => {
          if (!box.open && (e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            box.setOpen(true);
          } else navigate(e, box, options, pick);
        }}
      >
        {current?.icon && <span className="select-icon">{current.icon}</span>}
        <span className="select-value">{current?.label ?? ""}</span>
        <Chevron />
      </button>
      <Listbox id={id} box={box} options={options} selected={value} onPick={pick} />
    </div>
  );
}

const Chevron = () => (
  <svg className="select-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="m6 9 6 6 6-6" />
  </svg>
);

/** A text input with suggestions, in place of <input list> + <datalist>. */
export function Combo({
  value,
  suggestions,
  onChange,
  placeholder,
}: {
  value: string;
  suggestions: string[];
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  const id = useId();
  // Show everything until the user types something that narrows it.
  const [typed, setTyped] = useState(false);
  const q = value.trim().toLowerCase();
  const options: Option<string>[] = (typed && q ? suggestions.filter((s) => s.toLowerCase().includes(q)) : suggestions).map((s) => ({ value: s, label: s }));
  const box = useListbox(options.length, 0, "start");
  const pick = (o: Option<string>) => {
    onChange(o.value);
    setTyped(false);
    box.setOpen(false);
  };

  // A fresh list (e.g. just fetched) opens so the user sees it arrived.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) first.current = false;
    else if (suggestions.length) {
      setTyped(false);
      box.setOpen(true);
    }
  }, [suggestions]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div ref={box.anchor} className={`select combo ${box.open && options.length ? "open" : ""}`}>
      <input
        value={value}
        placeholder={placeholder}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={box.open}
        aria-controls={id}
        aria-activedescendant={box.open && box.active >= 0 ? `${id}-${box.active}` : undefined}
        onChange={(e) => {
          onChange(e.target.value);
          setTyped(true);
          box.setActive(-1);
          box.setOpen(true);
        }}
        onFocus={() => suggestions.length && box.setOpen(true)}
        onKeyDown={(e) => navigate(e, box, options, pick)}
      />
      {suggestions.length > 0 && (
        <button type="button" tabIndex={-1} className="combo-toggle" title="Show models" onMouseDown={(e) => e.preventDefault()} onClick={() => (setTyped(false), box.setOpen(!box.open))}>
          <Chevron />
        </button>
      )}
      {options.length > 0 && <Listbox id={id} box={box} options={options} selected={value} onPick={pick} />}
    </div>
  );
}
