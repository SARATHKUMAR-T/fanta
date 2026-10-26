# Security policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Please don't open a public issue for a security problem. Report it privately through [GitHub's private vulnerability reporting](https://github.com/SARATHKUMAR-T/fanta/security/advisories/new) instead.

Include what you found, how to reproduce it, and what an attacker could do with it. You'll get a reply within a week. Once a fix ships, the advisory is published with credit to you unless you'd rather stay anonymous.

## Scope

Fanta can run shell commands, edit files and operate other apps, always after the user approves. These are especially in scope:

- Any way to run a tool with side effects without approval, including through text in notifications, emails, web pages, calendar events or plugin output (prompt injection).
- Leaking API keys from the system keyring, or sending local data to a provider the user didn't choose.
- Escaping the app's Content Security Policy or reaching Tauri commands from untrusted content.
