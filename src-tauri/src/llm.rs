//! Minimal streaming client for Ollama's /api/chat.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
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
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), tool_calls: vec![], tool_name: None }
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
}
