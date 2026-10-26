import { describe, expect, it } from "vitest";
import { feel } from "./useMood";

describe("feel", () => {
  it.each([
    ["hey fanta", "happy"],
    ["thanks!", "shy"],
    ["we shipped it 🚀", "excited"],
    ["haha", "laugh"],
    ["I'm so tired today", "caring"],
    ["love you", "love"],
  ])("%s → %s", (text, mood) => {
    expect(feel(text)).toBe(mood);
  });

  it("pouts at rudeness before caring about the sigh in it", () => {
    expect(feel("ugh you're useless")).toBe("pout");
  });

  it("only treats a greeting at the start as one", () => {
    expect(feel("say hi to Ravi")).toBeNull();
  });

  it("has no feeling about plain requests", () => {
    expect(feel("remind me at 4 to send the report")).toBeNull();
  });
});
