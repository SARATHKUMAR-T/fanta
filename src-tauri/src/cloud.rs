//! Cloud model clients over raw HTTP (no official Rust SDKs): Anthropic's
//! Messages API, and the OpenAI Chat Completions API (also used for Gemini via
//! Google's OpenAI-compatible endpoint). Both stream, both support tools.

use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use crate::llm::{Message, ToolCall, ToolCallFunction};

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1";
pub const OPENAI_URL: &str = "https://api.openai.com/v1";
pub const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

/// Reads a server-sent-events body, calling `on_data` with each `data:` payload.
async fn for_each_sse(resp: reqwest::Response, mut on_data: impl FnMut(&str) -> anyhow::Result<bool>) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(bytes) = stream.next().await {
        buf.extend_from_slice(&bytes?);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end();
            if let Some(data) = line.strip_prefix("data:") {
                if !on_data(data.trim_start())? {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

async fn error_text(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let msg = body["error"]["message"].as_str().or(body[0]["error"]["message"].as_str()).unwrap_or("request failed");
    format!("{status}: {msg}")
}

/// Tool definitions arrive in OpenAI/Ollama shape: {type, function: {name, description, parameters}}.
fn tool_parts(t: &Value) -> (&str, &str, Value) {
    let f = &t["function"];
    (
        f["name"].as_str().unwrap_or(""),
        f["description"].as_str().unwrap_or(""),
        f.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object" })),
    )
}

// ---------------- Anthropic ----------------

/// Models that take `output_config.effort`; older ones reject it.
fn anthropic_supports_effort(model: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-fable", "claude-mythos", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// Models where a policy decline should be retried on a fallback model server-side.
fn anthropic_fallback_eligible(model: &str) -> bool {
    matches!(model, "claude-fable-5-1" | "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5")
}

fn anthropic_messages(messages: &[Message]) -> (String, Vec<Value>) {
    let system = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n\n");
    let mut out: Vec<Value> = vec![];
    for m in messages.iter().filter(|m| m.role != "system") {
        let (role, content) = match m.role.as_str() {
            "assistant" => {
                let content = if let Some(raw) = &m.raw {
                    raw.clone()
                } else if m.tool_calls.is_empty() {
                    if m.content.trim().is_empty() {
                        continue;
                    }
                    json!([{ "type": "text", "text": m.content }])
                } else {
                    let mut blocks = vec![];
                    if !m.content.trim().is_empty() {
                        blocks.push(json!({ "type": "text", "text": m.content }));
                    }
                    for c in &m.tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": c.id.clone().unwrap_or_default(),
                            "name": c.function.name,
                            "input": if c.function.arguments.is_object() { c.function.arguments.clone() } else { json!({}) }
                        }));
                    }
                    Value::Array(blocks)
                };
                ("assistant", content)
            }
            "tool" => {
                let content = if m.images.is_empty() {
                    json!(m.content)
                } else {
                    let mut blocks = vec![json!({ "type": "text", "text": m.content })];
                    blocks.extend(m.images.iter().map(|i| json!({ "type": "image", "source": { "type": "base64", "media_type": i.mime, "data": i.data } })));
                    Value::Array(blocks)
                };
                ("user", json!([{ "type": "tool_result", "tool_use_id": m.tool_call_id.clone().unwrap_or_default(), "content": content }]))
            }
            _ => ("user", json!([{ "type": "text", "text": m.content }])),
        };
        // Merge consecutive same-role turns (all tool results go back in one user message).
        match out.last_mut() {
            Some(last) if last["role"] == role => {
                if let (Some(a), Some(b)) = (last["content"].as_array_mut(), content.as_array()) {
                    a.extend(b.iter().cloned());
                }
            }
            _ => out.push(json!({ "role": role, "content": content })),
        }
    }
    // The conversation must open with a user turn (Fanta's own check-ins can come first).
    while out.first().is_some_and(|m| m["role"] != "user") {
        out.remove(0);
    }
    (system, out)
}

fn anthropic_request(http: &reqwest::Client, key: &str, path: &str) -> reqwest::RequestBuilder {
    http.post(format!("{ANTHROPIC_URL}/{path}"))
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
}

pub async fn anthropic_chat(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    mut on_token: impl FnMut(&str),
) -> anyhow::Result<Message> {
    let (system, msgs) = anthropic_messages(messages);
    let mut body = json!({
        "model": model,
        "max_tokens": 32000,
        "stream": true,
        "system": system,
        "messages": msgs,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    let (name, description, schema) = tool_parts(t);
                    // Streamed tool input arrives unbuffered; we parse it strictly below.
                    json!({ "name": name, "description": description, "input_schema": schema, "eager_input_streaming": true })
                })
                .collect(),
        );
    }
    if anthropic_supports_effort(model) {
        body["output_config"] = json!({ "effort": "medium" });
    }
    let mut req = anthropic_request(http, key, "messages");
    if anthropic_fallback_eligible(model) {
        // On a safety decline, the API re-runs the request on a suitable fallback model.
        body["fallbacks"] = json!("default");
        req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    let resp = req.json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Claude {}", error_text(resp).await);
    }

    // Rebuild every content block as it streams (text, tool_use, thinking, fallback...).
    let mut blocks: BTreeMap<u64, Value> = BTreeMap::new();
    let mut partial_json: BTreeMap<u64, String> = BTreeMap::new();
    let mut stop_reason = String::new();
    for_each_sse(resp, |data| {
        let ev: Value = serde_json::from_str(data)?;
        let i = ev["index"].as_u64().unwrap_or(0);
        match ev["type"].as_str() {
            Some("content_block_start") => {
                blocks.insert(i, ev["content_block"].clone());
            }
            Some("content_block_delta") => {
                let d = &ev["delta"];
                let block = blocks.entry(i).or_insert_with(|| json!({}));
                match d["type"].as_str() {
                    Some("text_delta") => {
                        let t = d["text"].as_str().unwrap_or("");
                        on_token(t);
                        let cur = block["text"].as_str().unwrap_or("").to_string();
                        block["text"] = json!(cur + t);
                    }
                    Some("input_json_delta") => partial_json.entry(i).or_default().push_str(d["partial_json"].as_str().unwrap_or("")),
                    Some("thinking_delta") => {
                        let cur = block["thinking"].as_str().unwrap_or("").to_string();
                        block["thinking"] = json!(cur + d["thinking"].as_str().unwrap_or(""));
                    }
                    Some("signature_delta") => {
                        let cur = block["signature"].as_str().unwrap_or("").to_string();
                        block["signature"] = json!(cur + d["signature"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                    stop_reason = r.to_string();
                }
            }
            Some("error") => anyhow::bail!("Claude: {}", ev["error"]["message"].as_str().unwrap_or("stream error")),
            Some("message_stop") => return Ok(false),
            _ => {}
        }
        Ok(true)
    })
    .await?;

    let mut out = Message::new("assistant", "");
    for (i, block) in blocks.iter_mut() {
        match block["type"].as_str() {
            Some("text") => out.content.push_str(block["text"].as_str().unwrap_or("")),
            Some("tool_use") => {
                let raw = partial_json.get(i).map(String::as_str).unwrap_or("");
                // Unbuffered input isn't validated by the API: parse strictly and
                // hand malformed input to the tool as an error marker, never guess.
                let input = if raw.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "__invalid_json": true }))
                };
                block["input"] = input.clone();
                out.tool_calls.push(ToolCall {
                    id: block["id"].as_str().map(String::from),
                    function: ToolCallFunction { name: block["name"].as_str().unwrap_or("").to_string(), arguments: input },
                    extra: Map::new(),
                });
            }
            _ => {}
        }
    }
    if stop_reason == "refusal" && out.content.trim().is_empty() {
        out.content = "I can't help with that one.".into();
        out.tool_calls.clear();
    }
    out.raw = Some(Value::Array(blocks.into_values().collect()));
    if out.tool_calls.is_empty() {
        // A finished answer is stored as text only; no need to keep provider blocks.
        out.raw = None;
    }
    Ok(out)
}

pub async fn anthropic_models(http: &reqwest::Client, key: &str) -> anyhow::Result<Vec<String>> {
    let resp = http
        .get(format!("{ANTHROPIC_URL}/models?limit=100"))
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Claude {}", error_text(resp).await);
    }
    let v: Value = resp.json().await?;
    Ok(v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)).collect())
}

// ---------------- OpenAI-compatible (OpenAI, Gemini) ----------------

fn openai_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = vec![];
    let system = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n\n");
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    // Tool messages can't carry images here, so screenshots follow the tool results as a user message.
    let mut pending_images: Vec<Value> = vec![];
    for m in messages.iter().filter(|m| m.role != "system") {
        if m.role != "tool" && !pending_images.is_empty() {
            out.push(image_message(std::mem::take(&mut pending_images)));
        }
        match m.role.as_str() {
            "assistant" if !m.tool_calls.is_empty() => out.push(json!({
                "role": "assistant",
                "content": if m.content.is_empty() { Value::Null } else { json!(m.content) },
                "tool_calls": m.tool_calls.iter().map(|c| {
                    let mut call = json!({
                        "id": c.id.clone().unwrap_or_default(),
                        "type": "function",
                        "function": { "name": c.function.name, "arguments": c.function.arguments.to_string() }
                    });
                    // Echo provider extras unchanged (Gemini rejects tool calls without its thought signature).
                    for (k, v) in &c.extra {
                        call[k] = v.clone();
                    }
                    call
                }).collect::<Vec<_>>()
            })),
            "assistant" => {
                if !m.content.trim().is_empty() {
                    out.push(json!({ "role": "assistant", "content": m.content }));
                }
            }
            "tool" => {
                out.push(json!({ "role": "tool", "tool_call_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.content }));
                pending_images.extend(m.images.iter().map(|i| json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", i.mime, i.data) } })));
            }
            _ => out.push(json!({ "role": "user", "content": m.content })),
        }
    }
    if !pending_images.is_empty() {
        out.push(image_message(pending_images));
    }
    out
}

fn image_message(images: Vec<Value>) -> Value {
    let mut content = vec![json!({ "type": "text", "text": "Screenshot from the tool call above:" })];
    content.extend(images);
    json!({ "role": "user", "content": content })
}

pub async fn openai_chat(
    http: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    mut on_token: impl FnMut(&str),
) -> anyhow::Result<Message> {
    let mut body = json!({ "model": model, "messages": openai_messages(messages), "stream": true });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let resp = http.post(format!("{base}/chat/completions")).bearer_auth(key).json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_text(resp).await);
    }
    let mut out = Message::new("assistant", "");
    // Tool calls stream in pieces keyed by index: id and name first, then argument fragments.
    // Gemini omits `index`, sending each call whole; a new id then means a new call.
    let mut calls: BTreeMap<u64, (Option<String>, String, String, Map<String, Value>)> = BTreeMap::new();
    for_each_sse(resp, |data| {
        if data == "[DONE]" {
            return Ok(false);
        }
        let ev: Value = serde_json::from_str(data)?;
        if let Some(e) = ev.get("error") {
            anyhow::bail!("{}", e["message"].as_str().unwrap_or("stream error"));
        }
        let delta = &ev["choices"][0]["delta"];
        if let Some(t) = delta["content"].as_str() {
            on_token(t);
            out.content.push_str(t);
        }
        for tc in delta["tool_calls"].as_array().into_iter().flatten() {
            let id = tc["id"].as_str().filter(|s| !s.is_empty());
            let key = match tc["index"].as_u64() {
                Some(i) => i,
                None => match (id, calls.iter().next_back()) {
                    (Some(id), Some((&k, last))) if last.0.as_deref() == Some(id) => k,
                    (Some(_), _) => calls.len() as u64,
                    (None, Some((&k, _))) => k,
                    (None, None) => 0,
                },
            };
            let entry = calls.entry(key).or_default();
            if let Some(id) = id {
                entry.0 = Some(id.to_string());
            }
            if let Some(obj) = tc.as_object() {
                for (k, v) in obj {
                    if !["index", "id", "type", "function"].contains(&k.as_str()) {
                        entry.3.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(n) = tc["function"]["name"].as_str() {
                entry.1.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                entry.2.push_str(a);
            }
        }
        Ok(true)
    })
    .await?;
    for (i, (id, name, args, extra)) in calls {
        let arguments = if args.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&args).unwrap_or_else(|_| json!({ "__invalid_json": true }))
        };
        out.tool_calls.push(ToolCall {
            id: Some(id.unwrap_or_else(|| format!("call_{i}"))),
            function: ToolCallFunction { name, arguments },
            extra,
        });
    }
    Ok(out)
}

pub async fn openai_models(http: &reqwest::Client, base: &str, key: &str) -> anyhow::Result<Vec<String>> {
    let resp = http.get(format!("{base}/models")).bearer_auth(key).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("{}", error_text(resp).await);
    }
    let v: Value = resp.json().await?;
    let skip = ["embedding", "tts", "whisper", "audio", "realtime", "transcribe", "image", "dall-e", "moderation", "search", "imagen", "veo", "aqa"];
    let mut ids: Vec<String> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .map(|id| id.trim_start_matches("models/").to_string())
        .filter(|id| !skip.iter().any(|s| id.contains(s)))
        .collect();
    ids.sort();
    Ok(ids)
}

/// Marker set when a model sent tool arguments that weren't valid JSON.
pub fn invalid_args(args: &Value) -> bool {
    args.as_object().is_some_and(|m: &Map<String, Value>| m.contains_key("__invalid_json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_history_shape() {
        let mut call = Message::new("assistant", "Checking.");
        call.tool_calls.push(ToolCall { id: Some("toolu_1".into()), function: ToolCallFunction { name: "list_tasks".into(), arguments: json!({}) }, extra: Map::new() });
        let mut result = Message::new("tool", "#1 report");
        result.tool_call_id = Some("toolu_1".into());
        let msgs = vec![
            Message::new("system", "be nice"),
            Message::new("assistant", "Good morning!"), // proactive, before any user turn
            Message::new("user", "what's on my list?"),
            call,
            result,
        ];
        let (system, out) = anthropic_messages(&msgs);
        assert_eq!(system, "be nice");
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[1]["content"][1]["type"], "tool_use");
        assert_eq!(out[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn screenshots_reach_both_apis() {
        let mut call = Message::new("assistant", "");
        call.tool_calls.push(ToolCall { id: Some("c1".into()), function: ToolCallFunction { name: "shot".into(), arguments: json!({}) }, extra: Map::new() });
        let mut result = Message::new("tool", "screen");
        result.tool_call_id = Some("c1".into());
        result.images.push(crate::llm::Image { mime: "image/png".into(), data: "AAAA".into() });
        let msgs = vec![Message::new("user", "look"), call, result, Message::new("user", "and now?")];

        let (_, out) = anthropic_messages(&msgs);
        let blocks = &out[2]["content"][0]["content"];
        assert_eq!(blocks[1]["source"]["data"], "AAAA");
        assert_eq!(out[2]["content"][1]["text"], "and now?"); // merged into the same user turn

        let out = openai_messages(&msgs);
        assert_eq!(out[2]["role"], "tool");
        assert_eq!(out[3]["content"][1]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(out[4]["content"], "and now?");
    }

    #[test]
    fn openai_history_shape() {
        let mut call = Message::new("assistant", "");
        let mut extra = Map::new();
        extra.insert("extra_content".into(), json!({ "google": { "thought_signature": "sig" } }));
        call.tool_calls.push(ToolCall { id: Some("c1".into()), function: ToolCallFunction { name: "x".into(), arguments: json!({"a": 1}) }, extra });
        let mut result = Message::new("tool", "ok");
        result.tool_call_id = Some("c1".into());
        let out = openai_messages(&[Message::new("system", "s"), Message::new("user", "hi"), call, result]);
        assert_eq!(out[2]["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert_eq!(out[2]["content"], Value::Null);
        assert_eq!(out[3]["tool_call_id"], "c1");
        assert_eq!(out[2]["tool_calls"][0]["extra_content"]["google"]["thought_signature"], "sig");
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Uses the Gemini key saved in Fanta: `cargo test -- --ignored gemini_tool_loop --nocapture`
    #[tokio::test]
    #[ignore]
    async fn gemini_tool_loop() {
        let key = keyring::Entry::new("fanta", "gemini-api-key").unwrap().get_password().unwrap();
        let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash".into());
        let http = reqwest::Client::new();
        let tools = vec![json!({ "type": "function", "function": {
            "name": "start_focus", "description": "Start a focus block",
            "parameters": { "type": "object", "properties": { "minutes": { "type": "integer" } } } } })];
        let mut msgs = vec![Message::new("system", "Use tools when asked."), Message::new("user", "ok lets focus on something")];

        let first = openai_chat(&http, GEMINI_URL, &key, &model, &msgs, &tools, |_| {}).await.unwrap();
        assert!(!first.tool_calls.is_empty(), "expected a tool call, got {:?}", first.content);
        println!("extras kept: {:?}", first.tool_calls[0].extra.keys().collect::<Vec<_>>());
        let call = first.tool_calls[0].clone();
        msgs.push(first);
        let mut result = Message::new("tool", "Focus block started, ends at 18:30.");
        result.tool_call_id = call.id.clone();
        result.tool_name = Some(call.function.name.clone());
        msgs.push(result);

        // This second request is the one that failed with "missing a thought_signature".
        let second = openai_chat(&http, GEMINI_URL, &key, &model, &msgs, &tools, |_| {}).await.unwrap();
        println!("reply: {}", second.content);
        assert!(!second.content.trim().is_empty());
    }
}
