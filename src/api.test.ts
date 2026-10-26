import { describe, expect, it } from "vitest";
import { parseTs, toTs } from "./api";

describe("backend timestamps", () => {
  it("formats a Date as local YYYY-MM-DD HH:MM:SS", () => {
    expect(toTs(new Date(2026, 0, 5, 9, 3, 7))).toBe("2026-01-05 09:03:07");
  });

  it("parses what it formats", () => {
    const d = new Date(2026, 9, 26, 23, 59, 0);
    expect(parseTs(toTs(d)).getTime()).toBe(d.getTime());
  });
});
