// Dev-only gallery for checking the character and panel in a plain browser.
import Character, { Mood } from "./companion/Character";
import "./companion/character.css";

const moods: Mood[] = ["idle", "happy", "talking", "thinking", "focus", "alert", "sleepy", "surprised", "proud", "love", "sad", "wink", "listening", "worried", "excited", "shy", "caring", "pout", "laugh"];

export default function Preview() {
  // ?mood=excited renders just that one, large.
  const only = new URLSearchParams(location.search).get("mood") as Mood | null;
  if (only) return <Character mood={only} size={300} />;
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 24, padding: 24, background: "#f5f3ff", fontFamily: "sans-serif" }}>
      {moods.map((m) => (
        <div key={m} style={{ textAlign: "center" }}>
          <Character mood={m} size={150} />
          <div>{m}</div>
        </div>
      ))}
    </div>
  );
}
