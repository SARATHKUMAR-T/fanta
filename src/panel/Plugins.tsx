import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, PluginInfo } from "../api";
import { PlugIcon, RefreshIcon, TrashIcon } from "../ui/Icons";

const EXAMPLES = [
  { name: "files", command: "npx -y @modelcontextprotocol/server-filesystem ~/Documents", triggers: "files, file, folder, document" },
  { name: "github", command: "npx -y @modelcontextprotocol/server-github", triggers: "github, pr, issue, repo" },
];

// Ready-made plugins that give Fanta hands on the computer. One click adds them.
const PRESETS = [
  {
    name: "browser",
    label: "Web browser",
    hint: "Opens sites, reads pages, clicks and fills forms in its own Chromium window.",
    command: "npx -y @playwright/mcp@0.0.83 --user-data-dir ~/.config/fanta/browser",
    triggers: "web, browser, site, website, page, open, search, google, url, link",
  },
];

export default function Plugins() {
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ name: "", command: "", triggers: "" });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => api.pluginsInfo().then(setPlugins);
    refresh();
    const un = listen("plugins://changed", refresh);
    return () => {
      un.then((f) => f());
    };
  }, []);

  async function run(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(String(e));
    }
  }

  async function add() {
    await run(async () => {
      await api.pluginAdd(form.name, form.command, form.triggers);
      setForm({ name: "", command: "", triggers: "" });
      setAdding(false);
    });
  }

  return (
    <section className="card">
      <div className="card-head">
        <div className="card-title">
          <PlugIcon size={18} /> Plugins
        </div>
        <div className="card-actions">
          <button className="icon-btn" title="Restart plugins" onClick={() => run(api.pluginsReload)}>
            <RefreshIcon size={16} />
          </button>
        </div>
      </div>
      <p className="muted small">
        Plugins are MCP servers - the same ones Claude Desktop uses. Their tools load when your message mentions a trigger word. Anything that
        changes things asks you first.
      </p>

      {plugins.length === 0 && !adding && <p className="muted small empty-line">No plugins yet.</p>}

      {PRESETS.filter((x) => !plugins.some((p) => p.name === x.name)).map((x) => (
        <div key={x.name} className="preset">
          <div className="plugin-main">
            <div className="plugin-name">{x.label}</div>
            <div className="muted tiny">{x.hint}</div>
          </div>
          <button className="btn soft" onClick={() => run(() => api.pluginAdd(x.name, x.command, x.triggers))}>
            Add
          </button>
        </div>
      ))}

      <ul className="plugin-list">
        {plugins.map((p) => (
          <li key={p.name} className="plugin">
            <div className="plugin-row" onClick={() => setOpen(open === p.name ? null : p.name)}>
              <span className={`dot ${p.status}`} />
              <div className="plugin-main">
                <div className="plugin-name">{p.name}</div>
                <div className="muted tiny">
                  {p.status === "running"
                    ? `${p.tools.length} tools · says “${p.triggers.join("”, “")}”`
                    : p.status === "error"
                      ? p.detail
                      : p.status}
                </div>
              </div>
              <label className="switch" onClick={(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={p.status !== "disabled"}
                  onChange={(e) => run(() => api.pluginSetEnabled(p.name, e.target.checked))}
                />
                <span />
              </label>
            </div>
            {open === p.name && (
              <div className="plugin-detail">
                <code className="cmd">{p.command}</code>
                {p.tools.length > 0 && (
                  <ul className="tool-list">
                    {p.tools.map((t) => (
                      <li key={t.name}>
                        <div>
                          <div className="tool-name">{t.name}</div>
                          <div className="muted tiny">{t.read_only ? "read-only · runs without asking" : t.auto_approve ? "runs without asking" : "asks first"}</div>
                        </div>
                        {!t.read_only && (
                          <label className="switch small" title="Run without asking">
                            <input type="checkbox" checked={t.auto_approve} onChange={(e) => run(() => api.pluginSetAutoApprove(p.name, t.name, e.target.checked))} />
                            <span />
                          </label>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
                {p.log.length > 0 && p.status === "error" && <pre className="log">{p.log.slice(-6).join("\n")}</pre>}
                <button className="btn ghost danger" onClick={() => run(() => api.pluginRemove(p.name))}>
                  <TrashIcon size={14} /> Remove
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>

      {adding ? (
        <div className="form">
          <label>
            Name
            <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="files" />
          </label>
          <label>
            Command
            <input value={form.command} onChange={(e) => setForm({ ...form, command: e.target.value })} placeholder="npx -y @modelcontextprotocol/server-filesystem ~/Documents" />
          </label>
          <label>
            Trigger words
            <input value={form.triggers} onChange={(e) => setForm({ ...form, triggers: e.target.value })} placeholder="files, folder, document" />
          </label>
          <div className="examples">
            {EXAMPLES.map((x) => (
              <button key={x.name} className="chip-btn" onClick={() => setForm(x)}>
                {x.name}
              </button>
            ))}
          </div>
          <div className="form-actions">
            <button className="btn primary" disabled={!form.name.trim() || !form.command.trim()} onClick={add}>
              Add plugin
            </button>
            <button className="btn ghost" onClick={() => setAdding(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="form-actions">
          <button className="btn soft" onClick={() => setAdding(true)}>
            + Add plugin
          </button>
          <button className="btn ghost" onClick={() => api.openConfigDir()}>
            Open config folder
          </button>
        </div>
      )}
      {error && <p className="error-text">{error}</p>}
    </section>
  );
}
