//! CLI-Anything app CLIs (github.com/HKUDS/CLI-Anything): `cli-anything-<app>`
//! commands that drive GIMP, LibreOffice, Blender... with JSON output for agents.
//! Fanta finds the installed ones on PATH and runs them as plain argv (no shell).

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::tools::arg_str;

const PREFIX: &str = "cli-anything-";
const HELP_TIMEOUT: Duration = Duration::from_secs(30);
const RUN_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_OUTPUT_CHARS: usize = 16000;

#[derive(Debug, Clone, Serialize)]
pub struct AppCli {
    /// The part after `cli-anything-`, e.g. "gimp".
    pub app: String,
    pub path: String,
    /// From the SKILL.md front matter, when the package ships one.
    pub description: Option<String>,
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // pip --user installs land here, and a GUI-launched app may not have it on PATH.
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".local/bin"));
    }
    dirs
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Installed app CLIs with their descriptions, first match on PATH wins.
pub fn discover() -> Vec<AppCli> {
    let mut all = discover_in(&search_dirs());
    for c in all.iter_mut() {
        c.description = skill_md(Path::new(&c.path), &c.app).as_deref().and_then(skill_description);
    }
    all
}

fn discover_in(dirs: &[PathBuf]) -> Vec<AppCli> {
    let mut out: Vec<AppCli> = vec![];
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(app) = name.strip_prefix(PREFIX) else { continue };
            if app.is_empty() || out.iter().any(|c| c.app == app) || !is_executable(&e.path()) {
                continue;
            }
            out.push(AppCli { app: app.to_string(), path: e.path().to_string_lossy().to_string(), description: None });
        }
    }
    out.sort_by(|a, b| a.app.cmp(&b.app));
    out
}

fn find(app: &str) -> anyhow::Result<AppCli> {
    let app = app.trim().to_lowercase();
    let app = app.strip_prefix(PREFIX).unwrap_or(&app);
    let all = discover_in(&search_dirs());
    if let Some(c) = all.iter().find(|c| c.app == app) {
        return Ok(c.clone());
    }
    let installed: Vec<&str> = all.iter().map(|c| c.app.as_str()).collect();
    anyhow::bail!(
        "no CLI-Anything app '{app}' installed. Installed: {}. The user can add one with `cli-hub install {app}`.",
        if installed.is_empty() { "none".to_string() } else { installed.join(", ") }
    )
}

/// The SKILL.md packaged at `cli_anything/<app>/skills/SKILL.md`, located via
/// the Python interpreter in the console script's shebang.
fn skill_md(script: &Path, app: &str) -> Option<String> {
    let first = std::fs::read_to_string(script).ok()?.lines().next()?.to_string();
    let python = first.strip_prefix("#!")?.trim();
    if !python.contains("python") || python.contains(' ') {
        return None;
    }
    let module = format!("cli_anything.{}", app.replace('-', "_"));
    let out = std::process::Command::new(python)
        .args(["-I", "-c", "import importlib.util,sys;s=importlib.util.find_spec(sys.argv[1]);print(list(s.submodule_search_locations)[0])", &module])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dir.is_empty() {
        return None;
    }
    std::fs::read_to_string(Path::new(&dir).join("skills/SKILL.md")).ok()
}

/// `description:` from the YAML front matter.
fn skill_description(md: &str) -> Option<String> {
    let body = md.strip_prefix("---")?;
    let front = &body[..body.find("\n---")?];
    front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().trim_matches('"').to_string())
        .filter(|d| !d.is_empty())
}

async fn run(cli: &AppCli, args: &[String], timeout: Duration) -> anyhow::Result<String> {
    let child = tokio::process::Command::new(&cli.path)
        .args(args)
        .current_dir(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => out?,
        Err(_) => anyhow::bail!("{PREFIX}{} took over {}s, so I stopped it", cli.app, timeout.as_secs()),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        text = format!("error (exit {}): {}\n{text}", out.status.code().unwrap_or(-1), err.trim());
    }
    if text.chars().count() > MAX_OUTPUT_CHARS {
        text = text.chars().take(MAX_OUTPUT_CHARS).collect::<String>() + "\n…(truncated)";
    }
    Ok(if text.trim().is_empty() { "(no output)".into() } else { text })
}

// ---- tools ----

pub fn list(_: &Value) -> anyhow::Result<String> {
    let all = discover();
    if all.is_empty() {
        return Ok("No CLI-Anything apps installed. The user can install some with `pip install cli-anything-hub` then `cli-hub install <app>` (gimp, libreoffice, blender, inkscape, audacity, obs-studio...).".into());
    }
    Ok(all
        .iter()
        .map(|c| format!("- {}: {}", c.app, c.description.as_deref().unwrap_or("call app_cli_help for its commands")))
        .collect::<Vec<_>>()
        .join("\n"))
}

pub async fn help(args: &Value) -> anyhow::Result<String> {
    let cli = find(arg_str(args, "app").ok_or_else(|| anyhow::anyhow!("app is required"))?)?;
    match arg_str(args, "subcommand") {
        None => {
            if let Some(md) = skill_md(Path::new(&cli.path), &cli.app) {
                return Ok(md);
            }
            run(&cli, &["--help".into()], HELP_TIMEOUT).await
        }
        Some(sub) => {
            let mut argv = shell_words::split(sub)?;
            argv.push("--help".into());
            run(&cli, &argv, HELP_TIMEOUT).await
        }
    }
}

/// The CLI arguments after `--json`. Accepts an array or one string ("layer add -n Bg").
fn cli_args(args: &Value) -> anyhow::Result<Vec<String>> {
    let argv: Vec<String> = match args.get("args") {
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).collect()
        }
        Some(Value::String(s)) => shell_words::split(s)?,
        _ => vec![],
    };
    // With no subcommand the CLI opens its interactive REPL, which would just hang.
    if argv.is_empty() {
        anyhow::bail!("args is required, e.g. [\"project\", \"info\"]; see app_cli_help");
    }
    Ok(argv)
}

pub fn describe_run(args: &Value) -> anyhow::Result<String> {
    let cli = find(arg_str(args, "app").ok_or_else(|| anyhow::anyhow!("app is required"))?)?;
    let argv = cli_args(args)?;
    Ok(format!("Run {}:\n$ {PREFIX}{} --json {}", cli.app, cli.app, shell_words::join(&argv)))
}

pub async fn run_tool(args: &Value) -> anyhow::Result<String> {
    let cli = find(arg_str(args, "app").ok_or_else(|| anyhow::anyhow!("app is required"))?)?;
    let mut argv = vec!["--json".to_string()];
    argv.extend(cli_args(args)?);
    run(&cli, &argv, RUN_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_app_clis() {
        let dir = std::env::temp_dir().join(format!("fanta-apps-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("cli-anything-gimp");
        std::fs::write(&script, "#!/bin/sh\necho \"$@\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("cli-anything-notexec"), "").unwrap();
        std::fs::write(dir.join("other-tool"), "").unwrap();
        let found = discover_in(std::slice::from_ref(&dir));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].app, "gimp");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_args_and_refuses_repl() {
        assert_eq!(
            cli_args(&json!({ "args": ["layer", "add", "-n", "Big bg"] })).unwrap(),
            vec!["layer", "add", "-n", "Big bg"]
        );
        assert_eq!(
            cli_args(&json!({ "args": "layer add -n 'Big bg'" })).unwrap(),
            vec!["layer", "add", "-n", "Big bg"]
        );
        assert!(cli_args(&json!({ "args": [] })).is_err());
        assert!(cli_args(&json!({})).is_err());
    }

    #[test]
    fn reads_skill_description() {
        let md = "---\nname: cli-anything-gimp\ndescription: \"Edit images with GIMP\"\n---\n# GIMP";
        assert_eq!(skill_description(md).as_deref(), Some("Edit images with GIMP"));
        assert_eq!(skill_description("# no front matter"), None);
    }
}
