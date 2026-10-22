import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, VoiceStatus } from "../api";
import { SpeakerIcon } from "../ui/Icons";

const MODES: { id: VoiceStatus["speak_mode"]; label: string }[] = [
  { id: "voice", label: "When I talk to her" },
  { id: "always", label: "Always" },
  { id: "never", label: "Never" },
];

export default function VoiceSettings() {
  const [status, setStatus] = useState<VoiceStatus | null>(null);
  const [progress, setProgress] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => api.voiceStatus().then(setStatus);
    refresh();
    const subs = [
      listen("voice://changed", refresh),
      listen<{ label: string; done: number; total: number }>("voice://progress", (e) =>
        setProgress(`Downloading ${e.payload.label}… ${Math.round(e.payload.done / 1e6)} / ${Math.round(e.payload.total / 1e6)} MB`),
      ),
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  }, []);

  async function install() {
    setError(null);
    setProgress("Starting download…");
    try {
      await api.voiceInstall();
      setProgress(null);
    } catch (e) {
      setError(String(e));
      setProgress(null);
    }
    setStatus(await api.voiceStatus());
  }

  if (!status) return null;
  return (
    <section className="card">
      <div className="card-title">
        <span>🎙️ Voice</span>
        {status.installed && <span className="chip ok">ready</span>}
      </div>
      {status.installed ? (
        <>
          <p className="muted small">
            Click the mic in the chat, <b>right-click Fanta</b>, or press <code>Ctrl+Alt+Space</code> anywhere, then just talk. She stops listening when you pause.
            Everything runs on this computer.
          </p>
          <label>
            Read replies aloud
            <div className="seg-inline">
              {MODES.map((m) => (
                <button
                  key={m.id}
                  className={status.speak_mode === m.id ? "active" : ""}
                  onClick={async () => {
                    await api.voiceSetMode(m.id);
                    setStatus(await api.voiceStatus());
                  }}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </label>
          <div className="form-actions">
            <button className="btn soft" onClick={() => api.voiceSpeak("Hi! I'm Fanta. Want to plan your day together?")}>
              <SpeakerIcon size={15} /> Hear Fanta
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="muted small">
            Talk to Fanta and hear her answer. Speech recognition and her voice run fully offline after a one-time ~{status.download_mb} MB download.
          </p>
          <div className="form-actions">
            <button className="btn primary" disabled={status.installing || !!progress} onClick={install}>
              {status.installing || progress ? "Downloading…" : "Download voice"}
            </button>
          </div>
          {progress && <p className="muted small">{progress}</p>}
        </>
      )}
      {error && <p className="error-text">{error}</p>}
    </section>
  );
}
