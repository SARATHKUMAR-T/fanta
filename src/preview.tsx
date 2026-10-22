// Dev-only gallery for checking the character and panel in a plain browser.
import Character, { Mood } from "./companion/Character";
import "./companion/character.css";

const moods: Mood[] = ["idle", "happy", "talking", "thinking", "focus", "alert", "sleepy"];

export default function Preview() {
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 24, padding: 24, background: "#f3eee8", fontFamily: "sans-serif" }}>
      {moods.map((m) => (
        <div key={m} style={{ textAlign: "center" }}>
          <Character mood={m} size={150} />
          <div>{m}</div>
        </div>
      ))}
    </div>
  );
}
