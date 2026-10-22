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

# Nudges, morning brief, and check-ins during your workday.
proactive: true
# Turn on GNOME Do Not Disturb during focus blocks.
focus_dnd: true

workday:
  start: "09:00"
  end: "18:00"
  days: [mon, tue, wed, thu, fri]

pomodoro:
  focus_minutes: 25
  break_minutes: 5

# Notification hub: Fanta reads desktop notifications (Teams, Slack, mail...)
# locally, sorts them into urgent / fyi / ignore, and holds non-urgent ones
# back while you focus.
notifications:
  enabled: true
  # People whose messages always break through focus. Match on sender name.
  vip: []
  # Words that make a message urgent without asking the model.
  urgent_keywords: [urgent, asap, outage, incident, emergency, blocker, p1, sev1, prod down]
  # Apps to ignore entirely, e.g. [Spotify, Rhythmbox].
  muted_apps: []
  keep_days: 14
  # Speech bubbles for incoming messages: messages (chat + urgent), urgent, or off.
  bubbles: messages

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
    #[serde(default = "default_days")]
    pub days: Vec<String>,
}

fn default_days() -> Vec<String> {
    ["mon", "tue", "wed", "thu", "fri"].map(String::from).to_vec()
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pomodoro {
    pub focus_minutes: u32,
    pub break_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub vip: Vec<String>,
    pub urgent_keywords: Vec<String>,
    pub muted_apps: Vec<String>,
    pub keep_days: u32,
    /// Which messages Fanta reads out in a speech bubble: "messages" (chat
    /// messages and anything urgent), "urgent", or "off".
    pub bubbles: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            vip: vec![],
            urgent_keywords: ["urgent", "asap", "outage", "incident", "emergency", "blocker", "p1", "sev1", "prod down"]
                .map(String::from)
                .to_vec(),
            muted_apps: vec![],
            keep_days: 14,
            bubbles: "messages".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub name: String,
    pub tagline: String,
    pub model: String,
    pub ollama_url: String,
    pub style: Style,
    pub proactive: bool,
    #[serde(default = "yes")]
    pub focus_dnd: bool,
    pub workday: Workday,
    pub pomodoro: Pomodoro,
    #[serde(default)]
    pub notifications: NotificationConfig,
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

const TOOL_GUIDE: &str = "You can manage the user's tasks, reminders, and focus (pomodoro) \
sessions with your tools. When the user asks to be reminded, call set_reminder. When they want \
to focus or start a pomodoro, call start_focus. Check list_tasks before answering questions about \
their tasks. When the user says they finished something, call complete_task. \
You also see their desktop notifications (Teams, Slack, mail...): use list_notifications when they ask \
what they missed or about messages. \
To answer someone on Teams use teams_reply (the chat their message came from) or teams_send_message \
(a person's 1:1 chat or a named chat). Write the message in the user's own voice, first person, \
without adding anything they didn't ask for; the user approves it before it's sent. \
After a tool runs, confirm in one short sentence.";

impl Persona {
    /// Whether `now` falls inside the configured workday.
    pub fn in_workday(&self, now: chrono::DateTime<chrono::Local>) -> bool {
        let day = now.format("%a").to_string().to_lowercase();
        let hm = now.format("%H:%M").to_string();
        self.workday.days.iter().any(|d| d.to_lowercase() == day)
            && hm >= self.workday.start
            && hm < self.workday.end
    }

    pub fn full_system_prompt(&self) -> String {
        format!(
            "{}\n{TOOL_GUIDE}\nStyle: {} tone, {} emoji, {} replies.\nUser's workday: {}-{}. Pomodoro: {}min focus / {}min break.\nCurrent local time: {}.",
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
