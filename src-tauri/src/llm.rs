//! Minimal streaming client for Ollama's /api/chat.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Room for history, tool definitions, and plugin output (~1.2 GB KV cache for a 4B model).
const CONTEXT_TOKENS: u32 = 8192;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider's id for the call; cloud APIs match tool results by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub function: ToolCallFunction,
    /// Provider fields that must be echoed back with the call, e.g. Gemini's
    /// `extra_content.google.thought_signature`. Never sent to Ollama.
    #[serde(skip)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// For role "tool": which call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The provider's own assistant content (e.g. Claude's thinking blocks),
    /// replayed unchanged within a tool loop. Never sent to Ollama.
    #[serde(skip)]
    pub raw: Option<Value>,
    /// Images a tool returned (screenshots), shown to cloud models. Never sent to Ollama.
    #[serde(skip)]
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub mime: String,
    /// Base64, as MCP servers send it.
    pub data: String,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
            tool_call_id: None,
            raw: None,
            images: vec![],
        }
    }
}

#[derive(Deserialize)]
struct Chunk {
    message: Option<Message>,
    #[serde(default)]
    done: bool,
    error: Option<String>,
}

pub struct Ollama {
    http: reqwest::Client,
    base_url: String,
}

impl Ollama {
    pub fn new(base_url: &str) -> Self {
        Self { http: reqwest::Client::new(), base_url: base_url.trim_end_matches('/').into() }
    }

    /// Streams one assistant turn. `on_token` receives content deltas.
    /// Returns the full assistant message, including any tool calls.
    pub async fn chat(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        mut on_token: impl FnMut(&str),
    ) -> anyhow::Result<Message> {
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "stream": true,
            "options": { "num_ctx": CONTEXT_TOKENS },
        });
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Can't reach Ollama at {} ({e}). Is it running?", self.base_url))?;
        if !resp.status().is_success() {
            anyhow::bail!("Ollama error {}: {}", resp.status(), resp.text().await.unwrap_or_default());
        }

        let mut out = Message::new("assistant", "");
        let mut buf = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(bytes) = stream.next().await {
            buf.extend_from_slice(&bytes?);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let chunk: Chunk = serde_json::from_slice(&line)?;
                if let Some(err) = chunk.error {
                    anyhow::bail!("Ollama: {err}");
                }
                if let Some(msg) = chunk.message {
                    if !msg.content.is_empty() {
                        on_token(&msg.content);
                        out.content.push_str(&msg.content);
                    }
                    out.tool_calls.extend(msg.tool_calls);
                }
                if chunk.done {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    /// One non-streaming call constrained to a JSON schema (Ollama structured outputs).
    pub async fn complete_json(&self, model: &str, messages: &[Message], schema: &Value) -> anyhow::Result<Value> {
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "format": schema,
            "stream": false,
            "options": { "temperature": 0, "num_ctx": CONTEXT_TOKENS },
        });
        let resp =
            self.http.post(format!("{}/api/chat", self.base_url)).json(&body).send().await?.error_for_status()?;
        let chunk: Chunk = resp.json().await?;
        let content = chunk.message.map(|m| m.content).unwrap_or_default();
        Ok(serde_json::from_str(&content)?)
    }
}
