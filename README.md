# Fanta

[![CI](https://github.com/SARATHKUMAR-T/fanta/actions/workflows/ci.yml/badge.svg)](https://github.com/SARATHKUMAR-T/fanta/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/SARATHKUMAR-T/fanta)](https://github.com/SARATHKUMAR-T/fanta/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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

## Install

Download the package for your distribution from the [latest release](https://github.com/SARATHKUMAR-T/fanta/releases/latest) (Linux x86_64):

| Distribution | Package | Install |
| --- | --- | --- |
| Ubuntu, Debian, Mint, Pop!_OS | `.deb` | `sudo apt install ./Fanta_*_amd64.deb` |
| Fedora, openSUSE, RHEL | `.rpm` | `sudo dnf install ./Fanta-*.x86_64.rpm` |
| Any other | `.AppImage` | `chmod +x Fanta_*_amd64.AppImage && ./Fanta_*_amd64.AppImage` |

Check the download against `SHA256SUMS` from the same release: `sha256sum -c SHA256SUMS --ignore-missing`.

Then install [Ollama](https://ollama.com) and pull the default model:

```sh
ollama pull qwen3:4b-instruct
```

Voice needs `arecord` and `aplay` from `alsa-utils`, which the `.deb` and `.rpm` install for you. Desktop control also needs `python3-gi` with AT-SPI.

## Building from source

### Requirements

- Linux (developed on GNOME/Wayland; x86_64)
- [Rust](https://rustup.rs) and Node.js 20+
- [Tauri 2 system dependencies](https://tauri.app/start/prerequisites/#linux). On Ubuntu/Debian:
  ```sh
  sudo apt install build-essential pkg-config libssl-dev libdbus-1-dev libxdo-dev \
    libayatana-appindicator3-dev librsvg2-dev libwebkit2gtk-4.1-dev
  ```
- [Ollama](https://ollama.com) with the default model:
  ```sh
  ollama pull qwen3:4b-instruct
  ```
- Optional, for desktop control: `python3-gi` with AT-SPI enabled

### Getting started

```sh
npm install
./scripts/setup-voice-tools.sh   # fetches the CMake that whisper.cpp needs at build time
npm run tauri dev
```

Run the setup script once per clone. `src-tauri/.cargo/config.toml` points the whisper.cpp build at `src-tauri/.tools/cmake`, which is gitignored, so without it the build fails with ``failed to run custom build command for `whisper-rs-sys` `` (`is cmake not installed?`).

The first voice request downloads the Whisper model and the Piper voice.

To build the release packages (`src-tauri/target/release/bundle/`):

```sh
npm run tauri build -- --bundles deb,rpm,appimage
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
npm test                    # frontend (Vitest)
cd src-tauri && cargo test  # backend
```

[CONTRIBUTING.md](CONTRIBUTING.md#checks) lists every check that CI runs.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, code style and pull request guidelines. Report security problems privately as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © 2026 Sarath Kumar
