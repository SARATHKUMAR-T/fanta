import { useEffect, useState } from "react";
import type { DiagramSpec } from "../api";
import { ExpandIcon, CloseIcon } from "../ui/Icons";

type Mermaid = typeof import("mermaid").default;

// Mermaid is large, so it's only loaded the first time a diagram is drawn.
let mermaidLoad: Promise<Mermaid> | null = null;
function loadMermaid(): Promise<Mermaid> {
  mermaidLoad ??= import("mermaid").then((m) => m.default);
  return mermaidLoad;
}

const dark = () => window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Mermaid's theme, in the app's violet. Mermaid needs hex colours here, not oklch. */
function themeVariables() {
  const d = dark();
  const ink = d ? "#f5f3ff" : "#2e1065";
  return {
    fontFamily: '"Nunito Variable", Nunito, system-ui, sans-serif',
    fontSize: "14px",
    background: "transparent",
    primaryColor: d ? "#3b1a78" : "#ede9fe",
    primaryBorderColor: d ? "#a78bfa" : "#7c3aed",
    primaryTextColor: ink,
    secondaryColor: d ? "#4c1d95" : "#ddd6fe",
    tertiaryColor: d ? "#2e1065" : "#f5f3ff",
    lineColor: d ? "#a78bfa" : "#8b5cf6",
    textColor: ink,
    edgeLabelBackground: d ? "#1c0d40" : "#ffffff",
    actorBkg: d ? "#3b1a78" : "#ede9fe",
    actorBorder: d ? "#a78bfa" : "#7c3aed",
    actorTextColor: ink,
    actorLineColor: d ? "#6d28d9" : "#c4b5fd",
    signalColor: d ? "#c4b5fd" : "#6d28d9",
    signalTextColor: ink,
    // mind map branches, from the root outwards
    cScale0: "#6d28d9",
    cScaleLabel0: "#ffffff",
    cScale1: d ? "#5b21b6" : "#ddd6fe",
    cScaleLabel1: ink,
    cScale2: d ? "#4c1d95" : "#ede9fe",
    cScaleLabel2: ink,
    cScale3: d ? "#3b1a78" : "#f5f3ff",
    cScaleLabel3: ink,
  };
}

/** Text that's safe inside a quoted Mermaid label. */
function esc(s: string) {
  return s.replace(/"/g, "#quot;").replace(/[\r\n]+/g, " ").trim();
}

/** Text for sequence messages and mind map nodes, where some characters end the line. */
function plain(s: string) {
  return s.replace(/[;#:"()[\]{}<>]/g, " ").replace(/[\r\n]+/g, " ").replace(/\s+/g, " ").trim();
}

function ids(d: DiagramSpec) {
  const map = new Map(d.nodes.map((n, i) => [n.id, `n${i}`]));
  return (id: string) => map.get(id) ?? "n0";
}

export function toFlowchart(d: DiagramSpec): string {
  const id = ids(d);
  // The panel is narrow: side by side only fits a few short boxes.
  const dir = d.kind === "tree" || d.nodes.length > 3 ? "TD" : "LR";
  const first = d.nodes[0]?.id;
  const sinks = new Set(d.nodes.map((n) => n.id).filter((n) => !d.edges.some((e) => e.from === n)));
  const lines = [`flowchart ${dir}`];
  for (const n of d.nodes) {
    const label = esc(n.label);
    // Start and end of a process are pill-shaped; everything else is a rounded box.
    const shape = d.kind === "flow" && (n.id === first || sinks.has(n.id)) ? `(["${label}"])` : `("${label}")`;
    lines.push(`  ${id(n.id)}${shape}`);
  }
  for (const e of d.edges) {
    // In a cycle, the edge back to the start is dotted, so the loop reads clearly.
    const arrow = d.kind === "cycle" && e.to === first ? "-.->" : "-->";
    lines.push(e.label.trim() ? `  ${id(e.from)} ${arrow}|"${esc(e.label)}"| ${id(e.to)}` : `  ${id(e.from)} ${arrow} ${id(e.to)}`);
  }
  if (first) lines.push(`  classDef start stroke-width:2.5px`, `  class ${id(first)} start`);
  return lines.join("\n");
}

function toSequence(d: DiagramSpec): string {
  const id = ids(d);
  const lines = ["sequenceDiagram", "  autonumber"];
  for (const n of d.nodes) lines.push(`  participant ${id(n.id)} as ${plain(n.label) || n.id}`);
  for (const e of d.edges) lines.push(`  ${id(e.from)}->>${id(e.to)}: ${plain(e.label) || " "}`);
  return lines.join("\n");
}

/** A mind map, if the edges form a single tree; otherwise null. */
function toMindmap(d: DiagramSpec): string | null {
  const parent = new Map<string, string>();
  for (const e of d.edges) {
    if (parent.has(e.to) || e.to === e.from) return null;
    parent.set(e.to, e.from);
  }
  const roots = d.nodes.filter((n) => !parent.has(n.id));
  if (roots.length !== 1) return null;
  const id = ids(d);
  const label = new Map(d.nodes.map((n) => [n.id, plain(n.label) || n.id]));
  const lines = ["mindmap"];
  const walk = (node: string, depth: number, seen: Set<string>) => {
    if (seen.has(node)) return;
    seen.add(node);
    const text = label.get(node) ?? node;
    lines.push(`${"  ".repeat(depth + 1)}${id(node)}${depth === 0 ? `((${text}))` : `(${text})`}`);
    for (const e of d.edges) if (e.from === node) walk(e.to, depth + 1, seen);
  };
  walk(roots[0].id, 0, new Set());
  return lines.length - 1 === d.nodes.length ? lines.join("\n") : null;
}

/** Mermaid sources to try in order: the best fit first, a plain flowchart as backup. */
export function candidates(d: DiagramSpec): string[] {
  const flow = toFlowchart(d);
  if (d.kind === "sequence") return [toSequence(d), flow];
  if (d.kind === "tree") return [toMindmap(d), flow].filter((s): s is string => !!s);
  return [flow];
}

let renders = 0;
// Mermaid keeps global state while it draws, so overlapping renders break each other.
let queue: Promise<unknown> = Promise.resolve();

function render(d: DiagramSpec): Promise<string> {
  const next = queue.then(() => draw(d));
  queue = next.catch(() => {});
  return next;
}

async function draw(d: DiagramSpec): Promise<string> {
  const mermaid = await loadMermaid();
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    theme: "base",
    themeVariables: themeVariables(),
    flowchart: { curve: "basis", padding: 14, nodeSpacing: 34, rankSpacing: 42, useMaxWidth: true },
    sequence: { useMaxWidth: true, mirrorActors: false, actorMargin: 40, messageMargin: 30 },
    mindmap: { useMaxWidth: true, padding: 12 },
  });
  let lastError: unknown = null;
  for (const source of candidates(d)) {
    const id = `diagram-${++renders}`;
    try {
      return (await mermaid.render(id, source)).svg;
    } catch (e) {
      lastError = e;
      document.getElementById(id)?.remove();
      document.getElementById(`d${id}`)?.remove();
    }
  }
  throw lastError ?? new Error("nothing to draw");
}

/** When Mermaid can't draw it: the nodes in order, joined by their edges. */
function Fallback({ d }: { d: DiagramSpec }) {
  const label = new Map(d.nodes.map((n) => [n.id, n.label]));
  return (
    <div className="diagram-fallback">
      {d.edges.length
        ? d.edges.map((e, i) => (
            <div key={i} className="fb-edge">
              <span className="fb-node">{label.get(e.from) ?? e.from}</span>
              <span className="fb-arrow">→{e.label && <em>{e.label}</em>}</span>
              <span className="fb-node">{label.get(e.to) ?? e.to}</span>
            </div>
          ))
        : d.nodes.map((n) => (
            <span key={n.id} className="fb-node">
              {n.label}
            </span>
          ))}
    </div>
  );
}

/** A diagram that animates in, with a button to see it full size. */
export default function Diagram({ spec, caption }: { spec: DiagramSpec; caption?: string }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [zoomed, setZoomed] = useState(false);
  const [scale, setScale] = useState(1);

  useEffect(() => {
    let live = true;
    setSvg(null);
    setFailed(false);
    if (!spec.nodes.length) return;
    render(spec)
      .then((s) => live && setSvg(s))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [spec]);

  if (!spec.nodes.length) return null;

  return (
    <figure className={`diagram kind-${spec.kind}`} aria-label={caption ?? "Diagram"}>
      {svg ? (
        <>
          <div className="diagram-svg" dangerouslySetInnerHTML={{ __html: svg }} />
          <button className="diagram-zoom" title="View larger" onClick={() => setZoomed(true)}>
            <ExpandIcon size={15} />
          </button>
        </>
      ) : failed ? (
        <Fallback d={spec} />
      ) : (
        <div className="diagram-loading">
          <span />
          <span />
          <span />
        </div>
      )}
      {caption && <figcaption>{caption}</figcaption>}

      {zoomed && svg && (
        <div className="diagram-modal" onClick={() => setZoomed(false)}>
          <div className="diagram-modal-bar" onClick={(e) => e.stopPropagation()}>
            <button className="btn small ghost" onClick={() => setScale((s) => Math.max(0.6, s - 0.25))}>
              −
            </button>
            <span className="tiny muted">{Math.round(scale * 100)}%</span>
            <button className="btn small ghost" onClick={() => setScale((s) => Math.min(3, s + 0.25))}>
              +
            </button>
            <button className="icon-btn" title="Close" onClick={() => setZoomed(false)}>
              <CloseIcon size={16} />
            </button>
          </div>
          <div className="diagram-modal-body" onClick={(e) => e.stopPropagation()}>
            <div className="diagram-svg big" style={{ width: `${scale * 100}%` }} dangerouslySetInnerHTML={{ __html: svg }} />
          </div>
        </div>
      )}
    </figure>
  );
}
