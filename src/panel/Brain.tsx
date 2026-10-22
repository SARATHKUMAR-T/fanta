import { useEffect, useState } from "react";
import { api, BrainStatus, Provider } from "../api";

const PROVIDERS: { id: Provider; name: string; badge: string; blurb: string; keyHint: string; keyUrl: string }[] = [
  { id: "local", name: "On this computer", badge: "🏠", blurb: "Private and free. Slower on CPU.", keyHint: "", keyUrl: "" },
  { id: "anthropic", name: "Claude", badge: "✳️", blurb: "Anthropic · great with tools", keyHint: "sk-ant-…", keyUrl: "https://console.anthropic.com/settings/keys" },
  { id: "openai", name: "OpenAI", badge: "◎", blurb: "GPT models", keyHint: "sk-…", keyUrl: "https://platform.openai.com/api-keys" },
  { id: "gemini", name: "Gemini", badge: "✦", blurb: "Google AI Studio", keyHint: "AIza…", keyUrl: "https://aistudio.google.com/apikey" },
];

export default function Brain() {
  const [status, setStatus] = useState<BrainStatus | null>(null);
  const [selected, setSelected] = useState<Provider>("local");
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [background, setBackground] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  async function refresh(pick?: Provider) {
    const s = await api.brainStatus();
    setStatus(s);
    setBackground(s.background);
    const p = pick ?? s.provider;
    setSelected(p);
    setModel(p === "local" ? s.local_model : s.models[p] ?? "");
  }
  useEffect(() => {
    refresh();
  }, []);

  function choose(p: Provider) {
    setSelected(p);
    setKey("");
    setModels([]);
    setMessage(null);
    setModel(p === "local" ? status?.local_model ?? "" : status?.models[p] ?? "");
  }

  async function run(action: () => Promise<string | void>) {
    setBusy(true);
    setMessage(null);
    try {
      const text = await action();
      if (text) setMessage({ ok: true, text });
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }

  const info = PROVIDERS.find((p) => p.id === selected)!;
  const hasKey = !!status?.keys[selected];
  const active = status?.provider === selected;

  return (
    <section className="card brain">
      <div className="card-title">
        <span>🧠 Brain</span>
        {status && <span className="chip ok">using {PROVIDERS.find((p) => p.id === status.provider)?.name}</span>}
      </div>
      <p className="muted small">Pick the model Fanta thinks with. Cloud models are faster and smarter with plugins; you bring your own API key.</p>

      <div className="provider-grid">
        {PROVIDERS.map((p) => (
          <button key={p.id} className={`provider ${selected === p.id ? "selected" : ""}`} onClick={() => choose(p.id)}>
            <span className="provider-badge">{p.badge}</span>
            <span className="provider-name">{p.name}</span>
            <span className="muted tiny">{p.blurb}</span>
            {status?.provider === p.id && <span className="provider-on">●</span>}
          </button>
        ))}
      </div>

      {selected === "local" ? (
        <p className="small">
          Using <code>{status?.local_model}</code> through Ollama. Nothing leaves this computer. Change the model in <code>persona.yaml</code>.
        </p>
      ) : (
        <>
          <label>
            <span>
              API key {hasKey && <span className="ok-text">· saved in your keyring ✓</span>}
            </span>
            <div className="inline">
              <input type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder={hasKey ? "•••••••• (leave empty to keep)" : info.keyHint} />
              <button
                className="btn ghost"
                disabled={busy || !key.trim()}
                onClick={() =>
                  run(async () => {
                    const list = await api.brainModels(selected, key);
                    await api.brainSaveKey(selected, key);
                    setKey("");
                    setModels(list);
                    if (!model && list.length) setModel(list[0]);
                    await refresh(selected);
                    return `Key works - ${list.length} models available.`;
                  })
                }
              >
                Save
              </button>
            </div>
          </label>
          <button className="btn link small" onClick={() => api.openLink(info.keyUrl)}>
            Get a {info.name} key ↗
          </button>

          <label>
            Model
            <div className="inline">
              <input list={`models-${selected}`} value={model} onChange={(e) => setModel(e.target.value)} placeholder="Fetch the list, or type a model id" />
              <datalist id={`models-${selected}`}>
                {models.map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
              <button
                className="btn ghost"
                disabled={busy || !hasKey}
                onClick={() =>
                  run(async () => {
                    const list = await api.brainModels(selected);
                    setModels(list);
                    return `${list.length} models - pick one from the list.`;
                  })
                }
              >
                Fetch
              </button>
            </div>
          </label>
        </>
      )}

      <label className="check">
        <input type="checkbox" checked={background} disabled={selected === "local"} onChange={(e) => setBackground(e.target.checked)} />
        <span>Also use it for morning briefs, nudges and focus digests</span>
      </label>
      {selected !== "local" && (
        <p className="muted tiny">
          🔒 When chatting, your messages, open tasks, today's meetings and recent message previews go to {info.name}. Sorting your notifications always stays
          on this computer.
        </p>
      )}

      <div className="form-actions">
        <button
          className="btn primary"
          disabled={busy || (selected !== "local" && (!hasKey || !model.trim()))}
          onClick={() =>
            run(async () => {
              await api.brainConfigure(selected, selected === "local" ? null : model, background && selected !== "local");
              await refresh(selected);
              return `Fanta is now thinking with ${info.name}${selected === "local" ? "" : ` (${model})`}.`;
            })
          }
        >
          {active ? "Save" : `Use ${info.name}`}
        </button>
        {hasKey && selected !== "local" && (
          <button
            className="btn ghost danger"
            disabled={busy}
            onClick={() =>
              run(async () => {
                if (status?.provider === selected) await api.brainConfigure("local", null, false);
                await api.brainSaveKey(selected, "");
                await refresh(selected);
                return "Key removed.";
              })
            }
          >
            Remove key
          </button>
        )}
      </div>
      {message && <p className={message.ok ? "ok-text" : "error-text"}>{message.text}</p>}
    </section>
  );
}
