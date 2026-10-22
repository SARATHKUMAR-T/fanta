//! Tool registry. Each tool is a JSON-schema description plus a handler.
//! Phase 1 adds tasks/reminders/pomodoro here; Phase 3 bridges MCP plugins in.

use serde_json::{json, Value};

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub handler: fn(&Value) -> anyhow::Result<String>,
}

pub struct Registry {
    tools: Vec<Tool>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        Self {
            tools: vec![Tool {
                name: "get_current_time",
                description: "Get the current local date and time.",
                parameters: json!({ "type": "object", "properties": {} }),
                handler: |_| Ok(chrono::Local::now().format("%A %Y-%m-%d %H:%M:%S").to_string()),
            }],
        }
    }

    /// Tool definitions in Ollama/OpenAI function-calling format.
    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect()
    }

    pub fn call(&self, name: &str, args: &Value) -> String {
        match self.tools.iter().find(|t| t.name == name) {
            Some(t) => (t.handler)(args).unwrap_or_else(|e| format!("error: {e}")),
            None => format!("error: unknown tool '{name}'"),
        }
    }
}
