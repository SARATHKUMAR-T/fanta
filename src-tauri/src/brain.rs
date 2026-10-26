//! Which model Fanta thinks with: the local Ollama model (default), or a cloud
//! model with the user's own API key. Keys live in the GNOME keyring.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager};

use crate::cloud;
use crate::llm::Message;
use crate::{db, AppState};

pub const PROVIDERS: [&str; 4] = ["local", "anthropic", "openai", "gemini"];
const KEYRING_SERVICE: &str = "fanta";

// settings keys
const PROVIDER: &str = "brain_provider";
const BACKGROUND: &str = "brain_background";
fn model_key(provider: &str) -> String {
    format!("brain_model_{provider}")
}

fn default_model(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("claude-opus-5-5"),
        _ => None, // OpenAI/Gemini: picked from the user's own model list
    }
}

#[derive(Serialize)]
pub struct Status {
    /// "local", "anthropic", "openai", or "gemini"
    pub provider: String,
    pub model: String,
    /// Use the cloud model for briefs, nudges and focus digests too.
    pub background: bool,
    /// Which providers have a key saved, and the chosen model for each.
    pub keys: BTreeMap<String, bool>,
    pub models: BTreeMap<String, String>,
    pub local_model: String,
}

fn keyring(provider: &str) -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, &format!("{provider}-api-key"))?)
}

fn key_for(provider: &str) -> anyhow::Result<String> {
    keyring(provider)?
        .get_password()
        .map_err(|_| anyhow::anyhow!("no API key saved for {provider} - add one in Settings → Brain"))
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    db::get_setting(&app.state::<AppState>().db.lock().unwrap(), key).ok().flatten()
}

fn model_for(app: &AppHandle, provider: &str) -> Option<String> {
    if provider == "local" {
        return Some(app.state::<AppState>().persona.model.clone());
    }
    setting(app, &model_key(provider)).or_else(|| default_model(provider).map(String::from))
}

pub fn provider(app: &AppHandle) -> String {
    setting(app, PROVIDER).filter(|p| PROVIDERS.contains(&p.as_str())).unwrap_or_else(|| "local".into())
}

pub fn is_cloud(app: &AppHandle) -> bool {
    provider(app) != "local"
}

pub fn status(app: &AppHandle) -> Status {
    let provider = provider(app);
    let mut keys = BTreeMap::new();
    let mut models = BTreeMap::new();
    for p in &PROVIDERS[1..] {
        keys.insert(p.to_string(), keyring(p).map(|k| k.get_password().is_ok()).unwrap_or(false));
        models.insert(p.to_string(), model_for(app, p).unwrap_or_default());
    }
    Status {
        model: model_for(app, &provider).unwrap_or_default(),
        background: setting(app, BACKGROUND).as_deref() == Some("1"),
        local_model: app.state::<AppState>().persona.model.clone(),
        provider,
        keys,
        models,
    }
}

pub fn save_key(provider: &str, key: &str) -> anyhow::Result<()> {
    if !PROVIDERS[1..].contains(&provider) {
        anyhow::bail!("unknown provider {provider}");
    }
    let key = key.trim();
    if key.is_empty() {
        let _ = keyring(provider)?.delete_credential();
    } else {
        keyring(provider)?.set_password(key)?;
    }
    Ok(())
}

pub fn configure(app: &AppHandle, provider: &str, model: Option<&str>, background: bool) -> anyhow::Result<()> {
    if !PROVIDERS.contains(&provider) {
        anyhow::bail!("unknown provider {provider}");
    }
    if provider != "local" {
        key_for(provider)?;
    }
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::set_setting(&conn, PROVIDER, provider)?;
    db::set_setting(&conn, BACKGROUND, if background { "1" } else { "0" })?;
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        if provider != "local" {
            db::set_setting(&conn, &model_key(provider), m)?;
        }
    }
    Ok(())
}

pub async fn list_models(app: &AppHandle, provider: &str, key: Option<&str>) -> anyhow::Result<Vec<String>> {
    let key = match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => k.to_string(),
        None => key_for(provider)?,
    };
    let http = app.state::<AppState>().http.clone();
    match provider {
        "anthropic" => cloud::anthropic_models(&http, &key).await,
        "openai" => cloud::openai_models(&http, cloud::OPENAI_URL, &key).await,
        "gemini" => cloud::openai_models(&http, cloud::GEMINI_URL, &key).await,
        _ => anyhow::bail!("{provider} has no model list"),
    }
}

async fn chat_with(
    app: &AppHandle,
    provider: &str,
    messages: &[Message],
    tools: &[Value],
    on_token: impl FnMut(&str),
) -> anyhow::Result<Message> {
    let state = app.state::<AppState>();
    let model =
        model_for(app, provider).ok_or_else(|| anyhow::anyhow!("pick a {provider} model in Settings → Brain"))?;
    match provider {
        "anthropic" => cloud::anthropic_chat(&state.http, &key_for(provider)?, &model, messages, tools, on_token).await,
        "openai" => {
            cloud::openai_chat(&state.http, cloud::OPENAI_URL, &key_for(provider)?, &model, messages, tools, on_token)
                .await
        }
        "gemini" => {
            cloud::openai_chat(&state.http, cloud::GEMINI_URL, &key_for(provider)?, &model, messages, tools, on_token)
                .await
        }
        _ => state.ollama.chat(&model, messages, tools, on_token).await,
    }
}

/// A chat turn with the user's chosen model.
pub async fn chat(
    app: &AppHandle,
    messages: &[Message],
    tools: &[Value],
    on_token: impl FnMut(&str),
) -> anyhow::Result<Message> {
    chat_with(app, &provider(app), messages, tools, on_token).await
}

/// Background writing (briefs, nudges, digests): local unless the user opted in.
pub async fn background(app: &AppHandle, messages: &[Message]) -> anyhow::Result<String> {
    let s = status(app);
    let provider = if s.background { s.provider } else { "local".into() };
    Ok(chat_with(app, &provider, messages, &[], |_| {}).await?.content.trim().to_string())
}
