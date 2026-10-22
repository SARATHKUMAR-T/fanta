use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_PERSONA: &str = r#"# Fanta's persona. Edit freely; restart the app to apply.
name: Fanta
tagline: your calm focus coach
model: qwen3:4b-instruct
ollama_url: http://localhost:11434

style:
  tone: casual
  emoji: minimal
  verbosity: short

proactive: true

workday:
  start: "09:00"
  end: "18:00"

pomodoro:
  focus_minutes: 25
  break_minutes: 5

system_prompt: |
  You are Fanta, a calm, grounded productivity coach who lives on the user's
  Linux machine. You speak casually, like a supportive friend who keeps them
  on track. Keep replies short (1-4 sentences) unless asked for detail.
  You help with tasks, focus, reminders, and staying on top of messages.
  When the user drifts, nudge them gently back - never nag or guilt-trip.
  Use tools when they help; never invent tool results.
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Style {
    pub tone: String,
    pub emoji: String,
    pub verbosity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workday {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pomodoro {
    pub focus_minutes: u32,
    pub break_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub name: String,
    pub tagline: String,
    pub model: String,
    pub ollama_url: String,
    pub style: Style,
    pub proactive: bool,
    pub workday: Workday,
    pub pomodoro: Pomodoro,
    pub system_prompt: String,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("fanta")
}

/// Loads ~/.config/fanta/persona.yaml, writing the default on first run.
pub fn load() -> anyhow::Result<Persona> {
    let path = config_dir().join("persona.yaml");
    if !path.exists() {
        std::fs::create_dir_all(config_dir())?;
        std::fs::write(&path, DEFAULT_PERSONA)?;
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_yaml::from_str(&text)?)
}

impl Persona {
    pub fn full_system_prompt(&self) -> String {
        format!(
            "{}\nStyle: {} tone, {} emoji, {} replies.\nUser's workday: {}-{}. Pomodoro: {}min focus / {}min break.\nCurrent local time: {}.",
            self.system_prompt.trim(),
            self.style.tone,
            self.style.emoji,
            self.style.verbosity,
            self.workday.start,
            self.workday.end,
            self.pomodoro.focus_minutes,
            self.pomodoro.break_minutes,
            chrono::Local::now().format("%A %Y-%m-%d %H:%M"),
        )
    }
}
