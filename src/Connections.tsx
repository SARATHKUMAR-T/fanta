import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CalendarStatus, DeviceCode, friendlyTime } from "./api";

export default function Connections() {
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [tenant, setTenant] = useState("");
  const [icsUrl, setIcsUrl] = useState("");
  const [code, setCode] = useState<DeviceCode | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [showGuide, setShowGuide] = useState(false);

  useEffect(() => {
    const refresh = () =>
      api.calendarStatus().then((s) => {
        setStatus(s);
        if (s.client_id) setClientId(s.client_id);
        if (s.tenant && s.tenant !== "organizations") setTenant(s.tenant);
      });
    refresh();
    const unChanged = listen("calendar://changed", refresh);
    const unAuth = listen<{ ok: boolean; message: string }>("calendar://auth", (e) => {
      setCode(null);
      setMessage({ ok: e.payload.ok, text: e.payload.message });
      refresh();
    });
    return () => {
      unChanged.then((f) => f());
      unAuth.then((f) => f());
    };
  }, []);

  async function signIn() {
    setBusy(true);
    setMessage(null);
    try {
      const c = await api.calendarSignIn(clientId, tenant);
      setCode(c);
      await navigator.clipboard.writeText(c.user_code).catch(() => {});
      api.openLink(c.verification_uri);
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }

  async function connectIcs() {
    setBusy(true);
    setMessage(null);
    try {
      const n = await api.calendarConnectIcs(icsUrl);
      setMessage({ ok: true, text: `Connected - found ${n} events in the next two weeks.` });
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }

  const connected = status && status.provider !== "none";

  return (
    <section className="conn">
      <div className="card">
        <div className="card-title">
          <span>📅 Microsoft 365 · Outlook + Teams</span>
          {connected && <span className="chip ok">{status.provider === "graph" ? "connected" : "read-only"}</span>}
        </div>

        {connected ? (
          <>
            <p>{status.account}</p>
            <p className="muted">
              {status.last_sync ? `Last synced ${friendlyTime(status.last_sync)}` : "Syncing…"}
              {status.provider === "ics" && " · ICS links can't create or answer invites"}
            </p>
            {status.error && <p className="error-text">{status.error}</p>}
            <div className="row">
              <button className="btn ghost" onClick={() => api.calendarSync()}>
                Sync now
              </button>
              <button className="btn ghost" onClick={() => api.calendarDisconnect()}>
                Disconnect
              </button>
            </div>
          </>
        ) : code ? (
          <div className="device-code">
            <p>Sign in at the page that just opened and enter this code (it's on your clipboard):</p>
            <div className="code">{code.user_code}</div>
            <div className="row">
              <button className="btn ghost" onClick={() => api.openLink(code.verification_uri)}>
                Open sign-in page again
              </button>
            </div>
            <p className="muted">Waiting for you to finish signing in…</p>
          </div>
        ) : (
          <>
            <h3>Sign in with Microsoft</h3>
            <p className="muted">
              One sign-in for your Outlook calendar and Teams chats: meeting heads-ups, creating or answering invites, and replying to Teams messages. Fanta
              always asks before sending anything.
            </p>
            <label>
              Application (client) ID
              <input value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" />
            </label>
            <label>
              Directory (tenant) ID
              <input value={tenant} onChange={(e) => setTenant(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" />
            </label>
            <div className="row">
              <button className="btn primary" disabled={busy || !clientId.trim() || !tenant.trim()} onClick={signIn}>
                Sign in
              </button>
              <button className="btn link" onClick={() => setShowGuide(!showGuide)}>
                {showGuide ? "Hide" : "How do I get a client ID?"}
              </button>
            </div>
            {showGuide && (
              <ol className="guide">
                <li>
                  Open <b>entra.microsoft.com</b> → Applications → App registrations → <b>New registration</b>.
                </li>
                <li>Name it “Fanta”, choose “Accounts in this organizational directory only”, leave Redirect URI empty, Register.</li>
                <li>
                  Authentication → Advanced settings → <b>Allow public client flows: Yes</b> → Save.
                </li>
                <li>
                  API permissions → Add → Microsoft Graph → Delegated: <b>Calendars.ReadWrite</b>, <b>Chat.ReadBasic</b>, <b>ChatMessage.Send</b>,{" "}
                  <b>User.Read</b>, <b>offline_access</b>.
                </li>
                <li>
                  Copy the <b>Application (client) ID</b> and <b>Directory (tenant) ID</b> from Overview into the fields above.
                </li>
                <li>If sign-in says an admin must approve, ask IT to grant consent - or use the ICS link below.</li>
              </ol>
            )}

            <h3>Or use a published calendar link</h3>
            <p className="muted">
              Outlook web → Settings → Calendar → Shared calendars → Publish a calendar → copy the ICS link. Read-only, may lag a few minutes.
            </p>
            <label>
              ICS link
              <input value={icsUrl} onChange={(e) => setIcsUrl(e.target.value)} placeholder="https://outlook.office365.com/owa/calendar/…/calendar.ics" />
            </label>
            <div className="row">
              <button className="btn ghost" disabled={busy || !icsUrl.trim()} onClick={connectIcs}>
                Connect link
              </button>
            </div>
          </>
        )}
        {message && <p className={message.ok ? "ok-text" : "error-text"}>{message.text}</p>}
        <p className="muted tiny privacy">
          🔒 Stays on this computer: the sign-in token lives in your GNOME keyring, calendar data in Fanta's local database.
        </p>
      </div>
    </section>
  );
}
