//! Plugin host: runs MCP servers listed in ~/.config/fanta/mcp.json (same
//! `mcpServers` format as Claude Desktop / Cline) and offers their tools to the model.

pub mod mcp;

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::llm::Image;
use crate::{persona, AppState};
use mcp::{Connection, Event, McpTool};

/// npx/uvx may download the server on first start.
const INIT_TIMEOUT: Duration = Duration::from_secs(120);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Small models get confused by many tools; below this, plugin tools always load.
const ALWAYS_LOAD_LIMIT: usize = 8;
/// Tool output beyond this is cut so it fits the model's context. Cloud models
/// get more room: a browser page snapshot alone is often 10k+ characters.
const MAX_RESULT_CHARS: usize = 4000;
const MAX_CLOUD_RESULT_CHARS: usize = 24000;
const SEP: &str = "__";

pub fn result_limit(cloud: bool) -> usize {
    if cloud {
        MAX_CLOUD_RESULT_CHARS
    } else {
        MAX_RESULT_CHARS
    }
}

/// Fanta's own desktop-control MCP server (AT-SPI + screenshots), written to the
/// config folder so the plugin entry can run it with the system python3.
const DESKTOP_SCRIPT: &str = include_str!("../../resources/desktop_mcp.py");

fn desktop_script_path() -> PathBuf {
    persona::config_dir().join("desktop_mcp.py")
}

/// Keeps the installed desktop script in step with this build.
fn write_desktop_script() -> anyhow::Result<()> {
    let path = desktop_script_path();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(DESKTOP_SCRIPT) {
        std::fs::create_dir_all(persona::config_dir())?;
        std::fs::write(&path, DESKTOP_SCRIPT)?;
    }
    Ok(())
}

/// Adds the built-in "desktop" plugin: read and operate any accessible app window.
pub fn add_desktop() -> anyhow::Result<()> {
    write_desktop_script()?;
    let cmd = shell_words::join(["python3", &desktop_script_path().to_string_lossy()]);
    add("desktop", &cmd, "app, apps, window, screen, screenshot, click, button, desktop, editor, type, menu")
}

pub fn config_path() -> PathBuf {
    persona::config_dir().join("mcp.json")
}

#[derive(Debug, Clone, Default)]
struct ServerConfig {
    command: Option<String>,
    args: Vec<String>,
    env: HashMap<String, String>,
    url: Option<String>,
    disabled: bool,
    auto_approve: Vec<String>,
    triggers: Vec<String>,
}

fn parse_server(v: &Value) -> ServerConfig {
    let strings = |k: &str| -> Vec<String> {
        v[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
    };
    ServerConfig {
        command: v["command"].as_str().map(String::from),
        args: strings("args"),
        env: v["env"]
            .as_object()
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
        url: v["url"].as_str().or(v["serverUrl"].as_str()).map(String::from),
        disabled: v["disabled"].as_bool().unwrap_or(false),
        auto_approve: strings("autoApprove"),
        triggers: strings("triggers"),
    }
}

/// Reads mcp.json as raw JSON so edits keep keys Fanta doesn't know about.
fn read_config() -> anyhow::Result<Value> {
    let path = config_path();
    if !path.exists() {
        write_config(&json!({ "mcpServers": {} }))?;
    }
    let text = std::fs::read_to_string(&path)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("mcp.json isn't valid JSON: {e}"))?;
    Ok(v)
}

fn write_config(v: &Value) -> anyhow::Result<()> {
    std::fs::create_dir_all(persona::config_dir())?;
    let path = config_path();
    std::fs::write(&path, serde_json::to_string_pretty(v)? + "\n")?;
    // May contain API tokens in `env`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn servers(v: &Value) -> BTreeMap<String, Value> {
    v["mcpServers"].as_object().map(|m| m.clone().into_iter().collect()).unwrap_or_default()
}

/// Tool names the model sees must match ^[a-zA-Z0-9_-]{1,64}$.
fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

fn exposed_name(plugin: &str, tool: &str) -> String {
    let mut n = format!("{}{SEP}{}", sanitize(plugin), sanitize(tool));
    n.truncate(64);
    n
}

#[derive(Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub auto_approve: bool,
}

#[derive(Clone, Serialize)]
pub struct PluginInfo {
    pub name: String,
    /// "disabled", "starting", "running", "error"
    pub status: String,
    pub detail: Option<String>,
    pub command: String,
    pub triggers: Vec<String>,
    pub tools: Vec<ToolInfo>,
    pub log: Vec<String>,
}

struct Plugin {
    name: String,
    config: ServerConfig,
    status: Mutex<(String, Option<String>)>,
    tools: Mutex<Vec<McpTool>>,
    conn: Mutex<Option<Arc<Connection>>>,
}

impl Plugin {
    fn set_status(&self, status: &str, detail: Option<String>) {
        *self.status.lock().unwrap() = (status.to_string(), detail);
    }

    fn triggers(&self) -> Vec<String> {
        if self.config.triggers.is_empty() {
            vec![self.name.to_lowercase()]
        } else {
            self.config.triggers.iter().map(|t| t.to_lowercase()).collect()
        }
    }

    fn info(&self) -> PluginInfo {
        let (status, detail) = self.status.lock().unwrap().clone();
        let command = match (&self.config.command, &self.config.url) {
            (Some(c), _) => {
                std::iter::once(c.clone()).chain(self.config.args.iter().cloned()).collect::<Vec<_>>().join(" ")
            }
            (None, Some(u)) => u.clone(),
            _ => String::new(),
        };
        PluginInfo {
            name: self.name.clone(),
            status,
            detail,
            command,
            triggers: self.triggers(),
            tools: self
                .tools
                .lock()
                .unwrap()
                .iter()
                .map(|t| ToolInfo {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    read_only: t.read_only,
                    auto_approve: self.config.auto_approve.contains(&t.name),
                })
                .collect(),
            log: self.conn.lock().unwrap().as_ref().map(|c| c.stderr_lines()).unwrap_or_default(),
        }
    }
}

#[derive(Default)]
pub struct Host {
    plugins: Mutex<BTreeMap<String, Arc<Plugin>>>,
}

fn changed(app: &AppHandle) {
    let _ = app.emit("plugins://changed", ());
}

/// (Re)starts every plugin from mcp.json. Safe to call repeatedly.
pub async fn reload(app: &AppHandle) -> anyhow::Result<()> {
    let host = &app.state::<AppState>().plugins;
    let old: Vec<Arc<Plugin>> = host.plugins.lock().unwrap().values().cloned().collect();
    for p in old {
        if let Some(c) = p.conn.lock().unwrap().take() {
            c.stop();
        }
    }
    host.plugins.lock().unwrap().clear();

    if let Err(e) = write_desktop_script() {
        eprintln!("couldn't update the desktop plugin script: {e}");
    }
    let cfg = read_config()?;
    for (name, v) in servers(&cfg) {
        let plugin = Arc::new(Plugin {
            config: parse_server(&v),
            name: name.clone(),
            status: Mutex::new(("starting".into(), None)),
            tools: Mutex::new(vec![]),
            conn: Mutex::new(None),
        });
        host.plugins.lock().unwrap().insert(name, plugin.clone());
        if plugin.config.disabled {
            plugin.set_status("disabled", None);
            continue;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = start(&app, &plugin).await {
                plugin.set_status("error", Some(e.to_string()));
            }
            changed(&app);
        });
    }
    changed(app);
    Ok(())
}

async fn start(app: &AppHandle, plugin: &Arc<Plugin>) -> anyhow::Result<()> {
    let Some(command) = plugin.config.command.clone() else {
        if plugin.config.url.is_some() {
            anyhow::bail!("remote (HTTP) MCP servers aren't supported yet - use a local command");
        }
        anyhow::bail!("no `command` set");
    };
    let weak = Arc::downgrade(plugin);
    let app2 = app.clone();
    let conn = Connection::start(&command, &plugin.config.args, &plugin.config.env, INIT_TIMEOUT, move |event| {
        let Some(p) = weak.upgrade() else { return };
        let app = app2.clone();
        match event {
            Event::ToolsChanged => {
                tauri::async_runtime::spawn(async move {
                    let conn = p.conn.lock().unwrap().clone();
                    if let Some(c) = conn {
                        if let Ok(tools) = c.list_tools().await {
                            *p.tools.lock().unwrap() = tools;
                            changed(&app);
                        }
                    }
                });
            }
            Event::Exited => {
                // A reload stops the connection first, so only unexpected exits land here.
                if p.conn.lock().unwrap().is_some() {
                    let hint = p.conn.lock().unwrap().as_ref().map(|c| c.stderr_hint()).unwrap_or_default();
                    p.set_status("error", Some(format!("plugin exited{hint}")));
                    changed(&app);
                }
            }
        }
    })
    .await?;
    *plugin.conn.lock().unwrap() = Some(conn.clone());
    let tools = conn.list_tools().await?;
    *plugin.tools.lock().unwrap() = tools;
    plugin.set_status("running", None);
    Ok(())
}

impl Host {
    pub fn info(&self) -> Vec<PluginInfo> {
        self.plugins.lock().unwrap().values().map(|p| p.info()).collect()
    }

    /// Finds the plugin and tool behind an exposed name like `github__create_issue`.
    fn resolve(&self, exposed: &str) -> Option<(Arc<Plugin>, McpTool)> {
        let plugins = self.plugins.lock().unwrap();
        for p in plugins.values() {
            if let Some(t) = p.tools.lock().unwrap().iter().find(|t| exposed_name(&p.name, &t.name) == exposed) {
                return Some((p.clone(), t.clone()));
            }
        }
        None
    }

    pub fn is_plugin_tool(&self, exposed: &str) -> bool {
        exposed.contains(SEP) && self.resolve(exposed).is_some()
    }

    /// Plugin tools to offer this turn. `context` is the recent user text; a
    /// plugin loads when it mentions one of its trigger words.
    pub fn schemas_for(&self, context: &str) -> Vec<Value> {
        let plugins = self.plugins.lock().unwrap();
        let running: Vec<&Arc<Plugin>> = plugins.values().filter(|p| p.status.lock().unwrap().0 == "running").collect();
        let total: usize = running.iter().map(|p| p.tools.lock().unwrap().len()).sum();
        let words: Vec<String> = context
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '-')
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect();
        let mut out = vec![];
        for p in running {
            let wanted = total <= ALWAYS_LOAD_LIMIT
                || context == "*"
                || p.triggers().iter().any(|t| t == "*" || words.iter().any(|w| w == t));
            if !wanted {
                continue;
            }
            for t in p.tools.lock().unwrap().iter() {
                let mut params = t.input_schema.clone();
                if params.get("type").is_none() {
                    params["type"] = json!("object");
                }
                if let Some(obj) = params.as_object_mut() {
                    obj.remove("$schema");
                }
                out.push(json!({
                    "type": "function",
                    "function": {
                        "name": exposed_name(&p.name, &t.name),
                        "description": format!("[{} plugin] {}", p.name, t.description),
                        "parameters": params
                    }
                }));
            }
        }
        out
    }

    /// Every running plugin's tools (for cloud models, which cope with many).
    pub fn all_schemas(&self) -> Vec<Value> {
        self.schemas_for("*")
    }

    /// One line per plugin for the system prompt.
    pub fn summary(&self) -> String {
        self.plugins
            .lock()
            .unwrap()
            .values()
            .map(|p| {
                let (status, _) = p.status.lock().unwrap().clone();
                let tools = p.tools.lock().unwrap().len();
                format!(
                    "- {} ({status}, {tools} tools; loads for the local model when the user says: {})",
                    p.name,
                    p.triggers().join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `None` if no approval is needed; otherwise what the approval card should say.
    pub fn approval_text(&self, exposed: &str, args: &Value) -> Option<String> {
        let (p, t) = self.resolve(exposed)?;
        if t.read_only || p.config.auto_approve.contains(&t.name) {
            return None;
        }
        if let Some(action) = describe_action(&t.name, args) {
            return Some(format!("{} › {action}", p.name));
        }
        let mut shown = serde_json::to_string(args).unwrap_or_default();
        if shown.chars().count() > 400 {
            shown = shown.chars().take(400).collect::<String>() + "…";
        }
        Some(format!("Run {} › {} with {shown}", p.name, t.name))
    }

    /// The tool's text output, plus any images it returned.
    pub async fn call(&self, exposed: &str, args: &Value, max_chars: usize) -> (String, Vec<Image>) {
        let Some((p, t)) = self.resolve(exposed) else {
            return (format!("error: unknown plugin tool '{exposed}'"), vec![]);
        };
        let conn = p.conn.lock().unwrap().clone();
        let Some(conn) = conn else {
            return (format!("error: plugin '{}' isn't running", p.name), vec![]);
        };
        let result = conn.call_tool_raw(&t.name, args, CALL_TIMEOUT).await;
        let images = result.as_ref().map(mcp::content_images).unwrap_or_default();
        let text = match result.map(|r| (mcp::content_to_text(&r), r["isError"].as_bool().unwrap_or(false))) {
            Ok((text, is_error)) => {
                let mut text = if is_error { format!("error: {text}") } else { text };
                if text.chars().count() > max_chars {
                    text = text.chars().take(max_chars).collect::<String>() + "\n…(truncated)";
                }
                if text.trim().is_empty() && images.is_empty() {
                    "(no output)".into()
                } else {
                    text
                }
            }
            Err(e) => format!("error: {e}"),
        };
        (text, images)
    }
}

/// Plain-language approval text for common browser/desktop actions, which
/// describe their target in an `element` argument. `None` falls back to raw JSON.
fn describe_action(tool: &str, args: &Value) -> Option<String> {
    let s = |k: &str| args[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    let short =
        |v: &str| if v.chars().count() > 300 { v.chars().take(300).collect::<String>() + "…" } else { v.to_string() };
    let action = tool.strip_prefix("browser_").unwrap_or(tool);
    Some(match action {
        "navigate" => format!("open {}", s("url")?),
        "click" => format!("click “{}”", s("element")?),
        "hover" => format!("hover over “{}”", s("element")?),
        "type" | "set_text" => {
            format!("type “{}” into “{}”", short(s("text")?), s("element").unwrap_or("the focused field"))
        }
        "press_key" | "press_keys" => format!("press {}", s("key").or(s("keys"))?),
        "select_option" => format!("choose {} in “{}”", args["values"], s("element")?),
        "fill_form" => format!("fill in a form: {}", short(&args["fields"].to_string())),
        _ => return None,
    })
}

// ---- config edits from the UI ----

fn edit_server(name: &str, f: impl FnOnce(&mut Map<String, Value>)) -> anyhow::Result<()> {
    let mut cfg = read_config()?;
    let entry = cfg["mcpServers"]
        .as_object_mut()
        .and_then(|m| m.get_mut(name))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("no plugin named '{name}'"))?;
    f(entry);
    write_config(&cfg)
}

pub fn set_enabled(name: &str, enabled: bool) -> anyhow::Result<()> {
    edit_server(name, |e| {
        if enabled {
            e.remove("disabled");
        } else {
            e.insert("disabled".into(), json!(true));
        }
    })
}

pub fn set_auto_approve(name: &str, tool: &str, auto: bool) -> anyhow::Result<()> {
    edit_server(name, |e| {
        let mut list: Vec<String> = e
            .get("autoApprove")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        list.retain(|t| t != tool);
        if auto {
            list.push(tool.to_string());
        }
        e.insert("autoApprove".into(), json!(list));
    })
}

/// Adds a plugin from a command line such as `npx -y @modelcontextprotocol/server-filesystem ~/Documents`.
pub fn add(name: &str, command_line: &str, triggers: &str) -> anyhow::Result<()> {
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("use a short name with letters, numbers, - or _");
    }
    let mut parts = shell_words::split(command_line.trim())?;
    if parts.is_empty() {
        anyhow::bail!("enter the command that starts the plugin");
    }
    // Expand ~ since there's no shell in between.
    let home = dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_default();
    for p in parts.iter_mut() {
        if p == "~" || p.starts_with("~/") {
            *p = p.replacen('~', &home, 1);
        }
    }
    let command = parts.remove(0);
    let mut entry = json!({ "command": command, "args": parts });
    let triggers: Vec<String> = triggers
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if !triggers.is_empty() {
        entry["triggers"] = json!(triggers);
    }
    let mut cfg = read_config()?;
    let servers =
        cfg["mcpServers"].as_object_mut().ok_or_else(|| anyhow::anyhow!("mcp.json has no mcpServers object"))?;
    if servers.contains_key(name) {
        anyhow::bail!("a plugin named '{name}' already exists");
    }
    servers.insert(name.to_string(), entry);
    write_config(&cfg)
}

pub fn remove(name: &str) -> anyhow::Result<()> {
    let mut cfg = read_config()?;
    if let Some(m) = cfg["mcpServers"].as_object_mut() {
        m.remove(name);
    }
    write_config(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_browser_actions() {
        assert_eq!(
            describe_action("browser_click", &json!({ "element": "Sign in button", "target": "e12" })).unwrap(),
            "click “Sign in button”"
        );
        assert_eq!(
            describe_action("browser_navigate", &json!({ "url": "https://example.com" })).unwrap(),
            "open https://example.com"
        );
        assert_eq!(
            describe_action("browser_type", &json!({ "text": "hi" })).unwrap(),
            "type “hi” into “the focused field”"
        );
        // Missing the describing argument: fall back to raw JSON.
        assert!(describe_action("browser_click", &json!({ "target": "e12" })).is_none());
        assert!(describe_action("create_issue", &json!({})).is_none());
    }

    #[test]
    fn exposed_names_are_valid() {
        assert_eq!(exposed_name("git hub", "create.issue"), "git_hub__create_issue");
        assert!(exposed_name(&"x".repeat(50), &"y".repeat(50)).len() <= 64);
    }

    #[test]
    fn parses_claude_desktop_entry() {
        let v = json!({
            "command": "npx", "args": ["-y", "pkg"], "env": { "TOKEN": "t" },
            "disabled": true, "autoApprove": ["read_file"], "someOtherClientKey": 1
        });
        let c = parse_server(&v);
        assert_eq!(c.command.as_deref(), Some("npx"));
        assert_eq!(c.args, vec!["-y", "pkg"]);
        assert_eq!(c.env.get("TOKEN").map(String::as_str), Some("t"));
        assert!(c.disabled);
        assert_eq!(c.auto_approve, vec!["read_file"]);
    }
}
