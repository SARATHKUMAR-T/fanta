import { describe, expect, it } from "vitest";
import type { DiagramSpec } from "../api";
import { candidates, toFlowchart } from "./Diagram";

const spec = (kind: DiagramSpec["kind"], nodes: [string, string][], edges: [string, string, string?][]): DiagramSpec => ({
  kind,
  nodes: nodes.map(([id, label]) => ({ id, label })),
  edges: edges.map(([from, to, label = ""]) => ({ from, to, label })),
});

describe("toFlowchart", () => {
  it("lays a short flow out sideways with pill-shaped ends", () => {
    const src = toFlowchart(spec("flow", [["a", "Start"], ["b", "Work"], ["c", "Done"]], [["a", "b"], ["b", "c"]]));
    expect(src.split("\n")[0]).toBe("flowchart LR");
    expect(src).toContain('n0(["Start"])');
    expect(src).toContain('n1("Work")');
    expect(src).toContain('n2(["Done"])');
    expect(src).toContain("n0 --> n1");
  });

  it("stacks longer diagrams vertically", () => {
    const nodes: [string, string][] = ["a", "b", "c", "d"].map((id) => [id, id.toUpperCase()]);
    expect(toFlowchart(spec("flow", nodes, [])).startsWith("flowchart TD")).toBe(true);
  });

  it("escapes quotes and newlines in labels", () => {
    const src = toFlowchart(spec("flow", [["a", 'say "hi"\nnow']], []));
    expect(src).toContain('n0(["say #quot;hi#quot; now"])');
  });

  it("dots the edge that closes a cycle", () => {
    const src = toFlowchart(spec("cycle", [["a", "Plan"], ["b", "Do"]], [["a", "b", "then"], ["b", "a"]]));
    expect(src).toContain('n0 -->|"then"| n1');
    expect(src).toContain("n1 -.-> n0");
  });

  it("maps unknown edge ends to the first node instead of inventing ids", () => {
    expect(toFlowchart(spec("flow", [["a", "A"]], [["a", "ghost"]]))).toContain("n0 --> n0");
  });
});

describe("candidates", () => {
  it("tries a sequence diagram first, then a flowchart", () => {
    const [first, second] = candidates(spec("sequence", [["u", "User"], ["s", "Server: API"]], [["u", "s", "GET /x"]]));
    expect(first).toContain("sequenceDiagram");
    expect(first).toContain("participant n1 as Server API");
    expect(second.startsWith("flowchart")).toBe(true);
  });

  it("draws a single-rooted tree as a mind map", () => {
    const [first] = candidates(spec("tree", [["r", "Root"], ["a", "A"], ["b", "B"]], [["r", "a"], ["r", "b"]]));
    expect(first.split("\n")).toEqual(["mindmap", "  n0((Root))", "    n1(A)", "    n2(B)"]);
  });

  it("falls back to a flowchart when the tree has two roots", () => {
    const all = candidates(spec("tree", [["a", "A"], ["b", "B"]], []));
    expect(all).toHaveLength(1);
    expect(all[0].startsWith("flowchart")).toBe(true);
  });
});
