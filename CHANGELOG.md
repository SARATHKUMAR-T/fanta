# Changelog

All notable changes to Fanta are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-26

The first public release, for Linux (x86_64), packaged as `.deb`, `.rpm` and `.AppImage`.

### Added

- **Companion**: a floating character with expressive moods, a chat panel and an inbox badge. Click once to open or close the panel, drag to move her.
- **Panel**: can be enlarged to fill the screen, with a sidebar, or minimized back beside the companion.
- **Tasks and reminders**: a morning brief, nudges and recurring reminders, stored in SQLite.
- **Pomodoro**: focus and break cycles with GNOME Do Not Disturb, chimes, history and analytics.
- **Notification hub**: local triage of incoming notifications, instant Teams alerts, and a watch on chosen chats with recent message context.
- **Calendar**: Outlook through Microsoft Graph, or any ICS feed.
- **Teacher mode**: lessons on any topic with Mermaid diagrams, step-by-step explanations and a quiz.
- **Voice**: push-to-talk (`Ctrl+Alt+Space`) with on-device whisper.cpp and Piper.
- **Models**: Ollama by default (offline), or Claude, OpenAI and Gemini with keys kept in the system keyring.
- **Plugins and control**: an MCP host, shell and file tools, AT-SPI desktop control, and CLI-Anything tools. Every action with a side effect asks for approval first.

### Security

- A strict Content Security Policy for the app windows.

[Unreleased]: https://github.com/SARATHKUMAR-T/fanta/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SARATHKUMAR-T/fanta/releases/tag/v0.1.0
