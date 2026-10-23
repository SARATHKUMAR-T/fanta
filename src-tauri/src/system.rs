//! Hands on the computer: read and write files, run shell commands, launch apps.
//! Reads run freely inside $HOME (secrets excluded); everything else is approved in tools.rs.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::tools::{arg_i64, arg_str};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_CHARS: usize = 12000;
const DEFAULT_READ_CHARS: usize = 20000;
const MAX_LIST_ENTRIES: usize = 300;

/// Paths under $HOME that hold credentials. Fanta never reads or writes these,
/// and refuses commands that mention them.
const SECRET_DIRS: &[&str] = &[".ssh", ".gnupg", ".password-store", ".aws", ".kube", ".docker", ".local/share/keyrings", ".mozilla", ".config/google-chrome", ".config/chromium"];
const SECRET_FILES: &[&str] = &[".config/fanta/mcp.json", ".netrc", ".git-credentials", ".pgpass"];

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Expands `~`, makes relative paths relative to $HOME, and resolves `..` without touching the disk.
fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    let raw = if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        home().join(path)
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn is_secret(rel: &Path) -> bool {
    let name = rel.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    SECRET_DIRS.iter().any(|d| rel.starts_with(d))
        || SECRET_FILES.iter().any(|f| rel == Path::new(f))
        || name == ".env"
        || name.starts_with(".env.")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}

/// A path Fanta may touch: inside $HOME (symlinks resolved) and not a secret.
pub fn allowed_path(path: &str) -> anyhow::Result<PathBuf> {
    let p = expand(path);
    // Resolve symlinks for whatever part already exists, so a link can't point outside.
    let resolved = match p.canonicalize() {
        Ok(c) => c,
        Err(_) => match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
            (Some(dir), Some(name)) => dir.join(name),
            _ => p.clone(),
        },
    };
    let home = home().canonicalize().unwrap_or_else(|_| home());
    let rel = resolved.strip_prefix(&home).map_err(|_| anyhow::anyhow!("{} is outside your home folder", p.display()))?;
    if is_secret(rel) {
        anyhow::bail!("{} may hold passwords or keys; I don't touch those", p.display());
    }
    Ok(resolved)
}

/// `~/x` instead of `/home/me/x`, for messages and approval cards.
pub fn pretty(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".into(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => p.display().to_string(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        text.chars().take(max).collect::<String>() + "\n…(truncated)"
    } else {
        text.to_string()
    }
}

pub fn read_file(args: &Value) -> anyhow::Result<String> {
    let path = allowed_path(arg_str(args, "path").ok_or_else(|| anyhow::anyhow!("path is required"))?)?;
    let max = arg_i64(args, "max_chars").map(|m| m.max(100) as usize).unwrap_or(DEFAULT_READ_CHARS);
    let bytes = std::fs::read(&path).map_err(|e| anyhow::anyhow!("can't read {}: {e}", pretty(&path)))?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        anyhow::bail!("{} is a binary file ({} bytes)", pretty(&path), bytes.len());
    }
    Ok(truncate(&String::from_utf8_lossy(&bytes), max))
}

pub fn list_dir(args: &Value) -> anyhow::Result<String> {
    let path = allowed_path(arg_str(args, "path").unwrap_or("~"))?;
    let mut entries: Vec<(bool, String, u64)> = std::fs::read_dir(&path)
        .map_err(|e| anyhow::anyhow!("can't list {}: {e}", pretty(&path)))?
        .filter_map(Result::ok)
        .map(|e| {
            let meta = e.metadata().ok();
            (meta.as_ref().is_some_and(|m| m.is_dir()), e.file_name().to_string_lossy().to_string(), meta.map(|m| m.len()).unwrap_or(0))
        })
        .collect();
    let hidden = args.get("show_hidden").and_then(Value::as_bool).unwrap_or(false);
    entries.retain(|(_, name, _)| hidden || !name.starts_with('.'));
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    if entries.is_empty() {
        return Ok(format!("{} is empty.", pretty(&path)));
    }
    let total = entries.len();
    let mut out: Vec<String> = entries
        .into_iter()
        .take(MAX_LIST_ENTRIES)
        .map(|(dir, name, len)| if dir { format!("{name}/") } else { format!("{name}  ({})", human_size(len)) })
        .collect();
    if total > MAX_LIST_ENTRIES {
        out.push(format!("…and {} more", total - MAX_LIST_ENTRIES));
    }
    Ok(format!("{}:\n{}", pretty(&path), out.join("\n")))
}

fn human_size(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

// ---- write_file ----

pub fn describe_write(args: &Value) -> anyhow::Result<String> {
    let path = allowed_path(arg_str(args, "path").ok_or_else(|| anyhow::anyhow!("path is required"))?)?;
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
    let verb = if append {
        "Add to"
    } else if path.exists() {
        "Overwrite"
    } else {
        "Create"
    };
    let preview: Vec<&str> = content.lines().take(6).collect();
    let more = if content.lines().count() > 6 { "\n…" } else { "" };
    Ok(format!("{verb} {} ({} bytes):\n{}{more}", pretty(&path), content.len(), preview.join("\n")))
}

pub fn write_file(args: &Value) -> anyhow::Result<String> {
    use std::io::Write;
    let path = allowed_path(arg_str(args, "path").ok_or_else(|| anyhow::anyhow!("path is required"))?)?;
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path)?;
    f.write_all(content.as_bytes())?;
    Ok(format!("{} {} ({} bytes).", if append { "Appended to" } else { "Wrote" }, pretty(&path), content.len()))
}

// ---- run_command ----

fn command_cwd(args: &Value) -> anyhow::Result<PathBuf> {
    let cwd = allowed_path(arg_str(args, "cwd").unwrap_or("~"))?;
    if !cwd.is_dir() {
        anyhow::bail!("{} isn't a folder", pretty(&cwd));
    }
    Ok(cwd)
}

pub fn describe_command(args: &Value) -> anyhow::Result<String> {
    let command = arg_str(args, "command").ok_or_else(|| anyhow::anyhow!("command is required"))?;
    if SECRET_DIRS.iter().chain(SECRET_FILES).any(|s| command.contains(s)) || command.contains("secret-tool") {
        anyhow::bail!("that command touches stored passwords or keys; I won't run it");
    }
    Ok(format!("Run in {}:\n$ {command}", pretty(&command_cwd(args)?)))
}

pub async fn run_command(args: &Value) -> anyhow::Result<String> {
    let command = arg_str(args, "command").ok_or_else(|| anyhow::anyhow!("command is required"))?;
    let child = tokio::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(command_cwd(args)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let out = match tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output()).await {
        Ok(out) => out?,
        Err(_) => anyhow::bail!("still running after {}s, so I stopped it. For apps use open_app.", COMMAND_TIMEOUT.as_secs()),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text += &format!("\n[stderr]\n{err}");
    }
    let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "killed".into());
    let text = if text.trim().is_empty() { "(no output)".to_string() } else { truncate(text.trim(), MAX_OUTPUT_CHARS) };
    Ok(format!("exit {code}\n{text}"))
}

// ---- open_app ----

fn application_dirs() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".local/share/applications"),
        h.join(".local/share/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/usr/share/applications"),
    ]
}

/// (desktop id, display name) of launchable apps.
fn installed_apps() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.filter_map(Result::ok) {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let id = path.file_stem().unwrap().to_string_lossy().to_string();
            if out.iter().any(|(i, _)| *i == id) {
                continue; // earlier dirs (the user's own) win
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if let Some(name) = parse_desktop_entry(&text) {
                out.push((id, name));
            }
        }
    }
    out
}

/// The app's Name from a .desktop file, or `None` if it's hidden or not an app.
fn parse_desktop_entry(text: &str) -> Option<String> {
    let mut in_entry = false;
    let mut name = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        match line.split_once('=') {
            Some(("Name", v)) => name = Some(v.trim().to_string()),
            Some(("NoDisplay" | "Hidden", v)) if v.trim() == "true" => return None,
            Some(("Type", v)) if v.trim() != "Application" => return None,
            _ => {}
        }
    }
    name
}

/// Best match for what the user called the app: exact name, then id, then a name containing it.
fn find_app<'a>(apps: &'a [(String, String)], query: &str) -> Option<&'a (String, String)> {
    let q = query.trim().to_lowercase();
    apps.iter()
        .find(|(_, n)| n.to_lowercase() == q)
        .or_else(|| apps.iter().find(|(id, _)| id.to_lowercase() == q || id.to_lowercase().rsplit('.').next() == Some(&q)))
        .or_else(|| apps.iter().find(|(_, n)| n.to_lowercase().contains(&q)))
}

fn is_url_or_path(target: &str) -> bool {
    target.contains("://") || target.starts_with('/') || target.starts_with("~/")
}

pub fn describe_open(args: &Value) -> anyhow::Result<String> {
    let target = arg_str(args, "name").ok_or_else(|| anyhow::anyhow!("name is required"))?;
    if is_url_or_path(target) {
        return Ok(format!("Open {target}"));
    }
    let apps = installed_apps();
    let (_, name) = find_app(&apps, target).ok_or_else(|| {
        let mut names: Vec<&str> = apps.iter().map(|(_, n)| n.as_str()).collect();
        names.sort_unstable();
        anyhow::anyhow!("no app called '{target}'. Installed: {}", names.join(", "))
    })?;
    Ok(format!("Open {name}"))
}

pub fn open_app(args: &Value) -> anyhow::Result<String> {
    let target = arg_str(args, "name").ok_or_else(|| anyhow::anyhow!("name is required"))?;
    if is_url_or_path(target) {
        let t = if target.contains("://") { target.to_string() } else { allowed_path(target)?.to_string_lossy().to_string() };
        std::process::Command::new("xdg-open").arg(&t).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        return Ok(format!("Opened {target}."));
    }
    let apps = installed_apps();
    let (id, name) = find_app(&apps, target).ok_or_else(|| anyhow::anyhow!("no app called '{target}'"))?;
    std::process::Command::new("gtk-launch").arg(id).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(format!("Opened {name}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_home_and_skip_secrets() {
        let h = home();
        assert_eq!(expand("~/a/../b"), h.join("b"));
        assert_eq!(expand("notes.txt"), h.join("notes.txt"));
        assert!(allowed_path("~/Documents/new-file.txt").is_ok());
        assert!(allowed_path("/etc/passwd").is_err());
        assert!(allowed_path("~/../../etc/passwd").is_err());
        assert!(allowed_path("~/.ssh/id_ed25519").is_err());
        assert!(allowed_path("~/projects/app/.env").is_err());
        assert!(allowed_path("~/.config/fanta/mcp.json").is_err());
    }

    #[test]
    fn commands_touching_secrets_are_refused() {
        assert!(describe_command(&serde_json::json!({ "command": "cat ~/.ssh/id_rsa" })).is_err());
        let ok = describe_command(&serde_json::json!({ "command": "ls -la" })).unwrap();
        assert!(ok.contains("$ ls -la") && ok.contains("Run in ~"));
    }

    #[tokio::test]
    async fn runs_a_command() {
        let out = run_command(&serde_json::json!({ "command": "echo hi; echo oops >&2; exit 3" })).await.unwrap();
        assert!(out.starts_with("exit 3") && out.contains("hi") && out.contains("[stderr]\noops"), "{out}");
    }

    #[test]
    fn desktop_entries() {
        assert_eq!(parse_desktop_entry("[Desktop Entry]\nType=Application\nName=Text Editor\n[Desktop Action new]\nName=New").as_deref(), Some("Text Editor"));
        assert_eq!(parse_desktop_entry("[Desktop Entry]\nName=Hidden\nNoDisplay=true"), None);
        let apps = vec![("org.gnome.TextEditor".to_string(), "Text Editor".to_string()), ("firefox".to_string(), "Firefox Web Browser".to_string())];
        assert_eq!(find_app(&apps, "firefox").unwrap().0, "firefox");
        assert_eq!(find_app(&apps, "texteditor").unwrap().0, "org.gnome.TextEditor");
        assert_eq!(find_app(&apps, "text editor").unwrap().0, "org.gnome.TextEditor");
        assert!(find_app(&apps, "gimp").is_none());
    }
}
