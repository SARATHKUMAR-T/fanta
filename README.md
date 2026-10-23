# Fanta

A local AI companion for the Linux desktop. Fanta lives in a small floating window and in the system tray. She keeps your tasks, reminders, and calendar, filters notifications, runs Pomodoro focus sessions, and can operate other apps for you. Every action with a side effect needs your approval first.

By default Fanta runs fully offline on [Ollama](https://ollama.com). You can switch her to Claude, OpenAI, or Gemini when you want a stronger model.

## Features

- **Companion UI**: a floating anime-style character with expressive eyes, plus a chat panel and an inbox badge.
- **Tasks and reminders**: a morning brief, nudges, and recurring reminders, stored in SQLite.
- **Pomodoro**: full focus and break cycles that turn on GNOME Do Not Disturb, with a chime when each phase ends, plus history and analytics.
- **Notification hub**: local triage of incoming notifications, with a digest when focus ends.
- **Calendar**: Outlook through Microsoft Graph, or any ICS feed.
- **Teams**: replies to Teams chats from Fanta (chats only, not channels).
- **Voice**: hold `Ctrl+Alt+Space` to talk. Speech recognition uses whisper.cpp and speech output uses Piper, both on-device.
- **Plugins**: an MCP host over stdio, with trigger-based tool routing.
- **Computer control**: shell and file tools, a browser preset, and a Stop button.
- **Desktop control**: a bundled AT-SPI plugin reads and operates any app window, including on Wayland, and takes screenshots through xdg-desktop-portal. Screenshots go only to cloud models, never to Ollama.
- **CLI-Anything**: discovers `cli-anything-*` tools on your `PATH`, reads their guides, and runs them with your approval.

## Requirements

- Linux (developed on GNOME/Wayland; x86_64)
- [Rust](https://rustup.rs) and Node.js 20+
- [Tauri 2 system dependencies](https://tauri.app/start/prerequisites/#linux) (webkit2gtk, etc.)
- [Ollama](https://ollama.com) with the default model:
  ```sh
  ollama pull qwen3:4b-instruct
  ```
- Optional, for desktop control: `python3-gi` with AT-SPI enabled

## Getting started

```sh
npm install
./scripts/setup-voice-tools.sh   # fetches the CMake that whisper.cpp needs at build time
npm run tauri dev
```

The first voice request downloads the Whisper model and the Piper voice.

To build a release bundle:

```sh
npm run tauri build
```

## Configuration

| Path | Purpose |
| --- | --- |
| `~/.config/fanta/persona.yaml` | Persona, model, Ollama URL, focus defaults, and urgency keywords. Fanta writes a default copy on first run. |
| `~/.config/fanta/mcp.json` | MCP plugin servers |
| `~/.local/share/fanta/fanta.db` | SQLite database (tasks, reminders, history) |
| `~/.local/share/fanta/voice/` | Downloaded voice models |

Cloud API keys are stored in the system keyring (Secret Service), never in files.

## Project layout

```
src/                 React + TypeScript frontend (companion, panel, tasks, inbox, calendar)
src-tauri/src/       Rust backend
  brain.rs           chat loop and tool routing
  llm.rs, cloud.rs   Ollama and cloud model clients
  tools.rs           built-in tool registry
  approvals.rs       approval flow for side-effecting tools
  plugins/           MCP plugin host
  calendar/          Microsoft Graph and ICS
  voice.rs           whisper.cpp + Piper
  focus.rs           Pomodoro and Do Not Disturb
src-tauri/resources/ bundled desktop-control MCP server (AT-SPI)
```

## Tests

```sh
cd src-tauri && cargo test
```
