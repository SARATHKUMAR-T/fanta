import { useEffect, useState } from "react";
import { api } from "../api";
import { SpeakerIcon } from "../ui/Icons";

export default function SoundSettings() {
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => {
    api.soundsGet().then(setOn);
  }, []);
  if (on === null) return null;
  return (
    <section className="card">
      <div className="card-head">
        <div className="card-title">🔔 Sounds</div>
        <label className="switch">
          <input
            type="checkbox"
            checked={on}
            onChange={async (e) => {
              await api.soundsSet(e.target.checked);
              setOn(e.target.checked);
            }}
          />
          <span />
        </label>
      </div>
      <p className="muted small">A soft chime when a focus block ends and when your break is over - it plays even during Do Not Disturb.</p>
      <div className="form-actions">
        <button className="btn soft small" onClick={() => api.soundsPreview("focus")}>
          <SpeakerIcon size={14} /> Focus done
        </button>
        <button className="btn soft small" onClick={() => api.soundsPreview("break")}>
          <SpeakerIcon size={14} /> Break over
        </button>
      </div>
    </section>
  );
}
