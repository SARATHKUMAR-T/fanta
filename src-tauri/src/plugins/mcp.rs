//! Minimal MCP client over stdio: JSON-RPC 2.0, one message per line.
//! Implements what a tool host needs: initialize, tools/list (+ list_changed),
//! tools/call, and polite answers to server-initiated requests.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const STDERR_TAIL: usize = 40;

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// From the server's `readOnlyHint` annotation.
    pub read_only: bool,
}

pub struct Connection {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    child: Mutex<Option<Child>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    pub server_name: Mutex<String>,
}

/// Events the reader task reports back to the owner.
pub enum Event {
    ToolsChanged,
    Exited,
}

impl Connection {
    /// Spawns the server process and performs the MCP handshake.
    pub async fn start(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        init_timeout: Duration,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> anyhow::Result<Arc<Self>> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("couldn't start `{command}`: {e}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let conn = Arc::new(Self {
            stdin: tokio::sync::Mutex::new(stdin),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            child: Mutex::new(Some(child)),
            stderr_tail: Arc::new(Mutex::new(VecDeque::new())),
            server_name: Mutex::new(command.to_string()),
        });

        let tail = conn.stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut t = tail.lock().unwrap();
                if t.len() == STDERR_TAIL {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });

        let reader_conn = Arc::downgrade(&conn);
        let pending = conn.pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let method = msg.get("method").and_then(Value::as_str);
                match (method, msg.get("id")) {
                    // Response to one of our requests.
                    (None, Some(id)) => {
                        if let Some(tx) = id.as_u64().and_then(|id| pending.lock().unwrap().remove(&id)) {
                            let result = match msg.get("error") {
                                Some(e) => Err(e["message"].as_str().unwrap_or("request failed").to_string()),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = tx.send(result);
                        }
                    }
                    // Request from the server (ping, roots/list, sampling...).
                    (Some(method), Some(id)) => {
                        let reply = match method {
                            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                            "roots/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [] } }),
                            _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "not supported by Fanta" } }),
                        };
                        if let Some(c) = reader_conn.upgrade() {
                            let _ = c.write(&reply).await;
                        }
                    }
                    (Some("notifications/tools/list_changed"), None) => on_event(Event::ToolsChanged),
                    _ => {}
                }
            }
            // Server exited or closed stdout: fail anything still waiting.
            for (_, tx) in pending.lock().unwrap().drain() {
                let _ = tx.send(Err("plugin exited".into()));
            }
            on_event(Event::Exited);
        });

        let init = conn
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "fanta", "version": env!("CARGO_PKG_VERSION") }
                }),
                init_timeout,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}{}", conn.stderr_hint()))?;
        if let Some(name) = init["serverInfo"]["name"].as_str() {
            *conn.server_name.lock().unwrap() = name.to_string();
        }
        conn.notify("notifications/initialized", json!({})).await?;
        Ok(conn)
    }

    async fn write(&self, msg: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }

    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
            self.pending.lock().unwrap().remove(&id);
            anyhow::bail!("plugin isn't accepting input: {e}");
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => anyhow::bail!("{e}"),
            Ok(Err(_)) => anyhow::bail!("plugin exited"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                anyhow::bail!("{method} timed out after {}s", timeout.as_secs())
            }
        }
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self.request("tools/list", params, Duration::from_secs(30)).await?;
            for t in page["tools"].as_array().into_iter().flatten() {
                let Some(name) = t["name"].as_str() else { continue };
                tools.push(McpTool {
                    name: name.to_string(),
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                    read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                });
            }
            match page["nextCursor"].as_str() {
                Some(c) => cursor = Some(c.to_string()),
                None => break,
            }
        }
        Ok(tools)
    }

    /// Calls a tool and flattens its content to text for the model.
    pub async fn call_tool(&self, name: &str, args: &Value, timeout: Duration) -> anyhow::Result<(String, bool)> {
        let result = self.call_tool_raw(name, args, timeout).await?;
        Ok((content_to_text(&result), result["isError"].as_bool().unwrap_or(false)))
    }

    /// The whole `tools/call` result, for callers that want more than the text.
    pub async fn call_tool_raw(&self, name: &str, args: &Value, timeout: Duration) -> anyhow::Result<Value> {
        let args = if args.is_object() { args.clone() } else { json!({}) };
        self.request("tools/call", json!({ "name": name, "arguments": args }), timeout).await
    }

    pub fn stderr_hint(&self) -> String {
        let tail = self.stderr_tail.lock().unwrap();
        match tail.back() {
            Some(last) => format!(" - plugin said: {last}"),
            None => String::new(),
        }
    }

    pub fn stderr_lines(&self) -> Vec<String> {
        self.stderr_tail.lock().unwrap().iter().cloned().collect()
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }
}

/// Image blocks in a tool result.
pub fn content_images(result: &Value) -> Vec<crate::llm::Image> {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "image")
        .filter_map(|c| Some(crate::llm::Image { mime: c["mimeType"].as_str()?.to_string(), data: c["data"].as_str()?.to_string() }))
        .collect()
}

pub fn content_to_text(result: &Value) -> String {
    let mut parts = vec![];
    for c in result["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_string()),
            Some("image") => parts.push("[image]".into()),
            Some("audio") => parts.push("[audio]".into()),
            Some("resource") => parts.push(
                c["resource"]["text"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("[resource {}]", c["resource"]["uri"].as_str().unwrap_or(""))),
            ),
            Some("resource_link") => parts.push(format!("[link {}]", c["uri"].as_str().unwrap_or(""))),
            _ => {}
        }
    }
    if parts.is_empty() {
        if let Some(s) = result.get("structuredContent") {
            return s.to_string();
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn fake_server() -> (String, Vec<String>) {
        let script = format!("{}/tests/fake_mcp_server.py", env!("CARGO_MANIFEST_DIR"));
        ("python3".into(), vec!["-u".into(), script])
    }

    #[tokio::test]
    async fn talks_to_an_mcp_server() {
        let (cmd, args) = fake_server();
        let changes = Arc::new(AtomicUsize::new(0));
        let exits = Arc::new(AtomicUsize::new(0));
        let (c2, e2) = (changes.clone(), exits.clone());
        let conn = Connection::start(&cmd, &args, &HashMap::new(), Duration::from_secs(10), move |ev| match ev {
            Event::ToolsChanged => {
                c2.fetch_add(1, Ordering::SeqCst);
            }
            Event::Exited => {
                e2.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();
        assert_eq!(*conn.server_name.lock().unwrap(), "fake-server");

        // Pagination joins both pages; annotations are read.
        let tools = conn.list_tools().await.unwrap();
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["echo", "fail", "add_tool"]);
        assert!(tools[0].read_only && !tools[1].read_only);

        // The server pinged us after initialize; the echo reports whether we answered.
        let (text, is_error) = conn.call_tool("echo", &json!({ "text": "hi" }), Duration::from_secs(5)).await.unwrap();
        assert_eq!(text, "hi (ping answered: True)");
        assert!(!is_error);

        let (text, is_error) = conn.call_tool("fail", &json!({}), Duration::from_secs(5)).await.unwrap();
        assert_eq!((text.as_str(), is_error), ("boom", true));

        assert!(conn.call_tool("nope", &json!({}), Duration::from_secs(5)).await.is_err());

        conn.call_tool("add_tool", &json!({}), Duration::from_secs(5)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        assert_eq!(conn.list_tools().await.unwrap().len(), 4);
        assert_eq!(conn.stderr_lines(), vec!["initialized"]);

        // A crash fails the in-flight call instead of hanging, and reports the exit.
        let err = conn.call_tool("crash", &json!({}), Duration::from_secs(5)).await.unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(exits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reports_a_missing_command() {
        let err = Connection::start("definitely-not-a-command", &[], &HashMap::new(), Duration::from_secs(2), |_| {})
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("couldn't start"));
    }
}

#[cfg(test)]
mod real_server_tests {
    use super::*;

    /// Needs python3-gi and a desktop session: `cargo test -- --ignored desktop_server`
    #[tokio::test]
    #[ignore]
    async fn desktop_server() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/desktop_mcp.py");
        let conn = Connection::start("python3", &[script.into()], &HashMap::new(), Duration::from_secs(10), |_| {}).await.unwrap();
        let tools = conn.list_tools().await.unwrap();
        let read_only: Vec<&str> = tools.iter().filter(|t| t.read_only).map(|t| t.name.as_str()).collect();
        assert_eq!(read_only, ["list_windows", "window_tree", "find", "read_text", "screenshot"]);
        let (text, is_error) = conn.call_tool("list_windows", &json!({}), Duration::from_secs(20)).await.unwrap();
        println!("{text}");
        assert!(!is_error);
        let (text, is_error) = conn.call_tool("click", &json!({ "id": "999.0" }), Duration::from_secs(20)).await.unwrap();
        assert!(is_error && text.contains("gone"), "{text}");
    }

    /// Needs network for npx: `cargo test -- --ignored real_filesystem_server`
    #[tokio::test]
    #[ignore]
    async fn real_filesystem_server() {
        let dir = std::env::temp_dir().join("fanta-mcp-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "standup at 10").unwrap();
        let args = vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(), dir.to_string_lossy().to_string()];
        let conn = Connection::start("npx", &args, &HashMap::new(), Duration::from_secs(180), |_| {}).await.unwrap();
        let tools = conn.list_tools().await.unwrap();
        println!("server: {}", conn.server_name.lock().unwrap());
        for t in &tools {
            println!("tool: {} read_only={}", t.name, t.read_only);
        }
        let path = dir.join("notes.txt").to_string_lossy().to_string();
        let tool = if tools.iter().any(|t| t.name == "read_text_file") { "read_text_file" } else { "read_file" };
        let (text, is_error) = conn.call_tool(tool, &json!({ "path": path }), Duration::from_secs(30)).await.unwrap();
        println!("{tool} -> {text}");
        assert!(!is_error && text.contains("standup at 10"));
    }
}
