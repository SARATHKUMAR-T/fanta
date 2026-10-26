# Contributing to Fanta

Thanks for helping. This guide covers how to set up the project, what a good pull request looks like, and how releases are made.

## Before you start

- **Bugs**: search the [issues](https://github.com/SARATHKUMAR-T/fanta/issues) first, then open one with the bug report template.
- **Features**: open a feature request before writing code for anything bigger than a small fix, so we can agree on the approach first.
- **Security problems**: don't open an issue. Follow [SECURITY.md](SECURITY.md).

By contributing you agree that your work is released under the [MIT License](LICENSE), and to follow the [code of conduct](CODE_OF_CONDUCT.md).

## Development setup

Follow [Getting started](README.md#getting-started) in the README to install the system packages, then:

```sh
npm install
./scripts/setup-voice-tools.sh   # once per clone
npm run tauri dev
```

### Working on the UI without the backend

The frontend can run in a plain browser against a mock backend (`src/mock.ts`):

```sh
npm run dev
# http://localhost:1420/?mock=1                     the panel
# http://localhost:1420/?mock=1&expanded=1&tab=teach  enlarged, on a given tab
# http://localhost:1420/?view=companion&mock=1      the floating companion
# http://localhost:1420/?view=preview               every mood side by side
```

If you add a backend command, add a mock for it too.

## Checks

CI runs all of these on every pull request. Run them locally first:

| What | Command |
| --- | --- |
| Frontend types | `npm run typecheck` |
| Frontend tests | `npm test` |
| Frontend build | `npm run build` |
| Rust formatting | `cd src-tauri && cargo fmt --check` |
| Rust lints | `cd src-tauri && cargo clippy --all-targets -- -D warnings` |
| Rust tests | `cd src-tauri && cargo test` |

Tests that need real hardware or downloaded models are marked `#[ignore]`. Run them with `cargo test -- --ignored` when you touch voice or desktop control.

## Code style

- **Rust**: `cargo fmt` with the repo's `rustfmt.toml`, and no clippy warnings. Return `anyhow::Result` from fallible functions. Outside tests, use `unwrap()` only where failure is a bug (such as mutex locks), never on input, files or network results.
- **TypeScript and React**: strict TypeScript, function components and hooks. Keep lines under about 160 characters, like the existing code.
- **CSS**: use the theme tokens in `src/ui/theme.css` (`--surface`, `--text`, `--accent` and so on) instead of hard-coded colours, and check both light and dark mode.
- **Comments**: explain *why*, not *what*. Keep them short.
- **Approvals**: any new tool with a side effect (writes files, runs commands, sends messages, changes settings) must go through the approval flow in `src-tauri/src/approvals.rs`.
- **Privacy**: don't send user data to a new service, and don't add telemetry.

## Tests

- Bug fixes come with a test that fails without the fix, where that's practical.
- New logic gets unit tests: Rust tests in a `#[cfg(test)] mod tests` beside the code, and frontend tests in `*.test.ts(x)` files beside the source (Vitest and Testing Library).
- UI changes include before and after screenshots in the pull request, in light and dark mode.

## Branches and commits

- Branch from `main`, using a name like `fix/panel-reattach`, `feat/teach-quiz` or `docs/readme-install`.
- Write commit subjects as `Area: what changed`, in the imperative mood and under about 72 characters, for example `Focus: pause the timer when the screen locks`. Use the body to explain why.
- Keep each commit focused. Squash fixup commits before review.

## Pull requests

1. Keep each pull request to one change. Split unrelated fixes into separate ones.
2. Fill in the template: what changed, why, how you tested it, and screenshots for UI changes.
3. Make sure CI is green. A pull request with failing checks won't be reviewed.
4. Update `CHANGELOG.md` under **Unreleased** for anything users will notice.
5. Update the README or docs when behaviour or setup changes.
6. Be ready for review comments. The maintainer merges with **squash and merge** once it's approved, so the pull request title becomes the commit subject: use the `Area: what changed` form.

## Releases (maintainers)

Fanta follows [Semantic Versioning](https://semver.org/).

1. Set the new version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, then run `cargo check` in `src-tauri` to update `Cargo.lock`.
2. In `CHANGELOG.md`, move the **Unreleased** entries into a new version section dated today, and update the links at the bottom.
3. Commit with the message `Release vX.Y.Z` and open a pull request.
4. After it merges, tag and push the tag:
   ```sh
   git tag -a vX.Y.Z -m "Fanta vX.Y.Z"
   git push origin vX.Y.Z
   ```
5. The **Release** workflow checks that the tag matches the versions, runs the tests, builds the `.deb`, `.rpm` and `.AppImage` packages, and attaches them with `SHA256SUMS` to a **draft** release.
6. Install one of the packages to check it, edit the release notes, then publish the release.

A tag with a suffix, such as `v0.2.0-beta.1`, is marked as a pre-release.
