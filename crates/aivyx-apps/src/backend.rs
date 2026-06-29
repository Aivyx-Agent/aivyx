//! Chapter Deckhand — the Linux/X11 desktop backend.
//!
//! v1 shells out to the standard `xdotool` for window enumeration / focus /
//! input, and a screenshot CLI for captures. The **argv builders and output
//! parsers are pure** so they unit-test without a display (the dogfood rig and
//! CI are headless); the `run_*` functions are the thin impure layer that
//! actually spawns the commands.
//!
//! ## Platform reality
//! `xdotool` drives X11 and **Xwayland** windows. Native Wayland windows are
//! isolated by the compositor and are NOT reachable this way — a documented
//! limitation (see `docs/APPLICATIONS.md`). The tools surface a clear error
//! rather than silently doing nothing when `xdotool` is missing.

use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

/// One open window, as reported by `app.list`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    /// The X11 window id (numeric, e.g. "29360131").
    pub id: String,
    /// The window title (`WM_NAME`).
    pub title: String,
    /// Whether this is the currently-focused window.
    pub active: bool,
}

// ---- pure argv builders -------------------------------------------------

/// `xdotool type` argv for literal text. `--clearmodifiers` so a held key
/// doesn't corrupt it; `--` so text starting with `-` isn't parsed as a flag.
/// Text is passed as a single argv element (never through a shell), so no
/// escaping/injection concern.
pub fn type_argv(text: &str) -> Vec<String> {
    vec![
        "type".into(),
        "--clearmodifiers".into(),
        "--".into(),
        text.to_string(),
    ]
}

/// `xdotool key` argv for a key or chord, e.g. `"ctrl+s"`, `"Return"`,
/// `"alt+Tab"`. Validated to key-name characters only — defence-in-depth even
/// though we never pass through a shell.
pub fn key_argv(combo: &str) -> Result<Vec<String>, String> {
    let combo = combo.trim();
    if combo.is_empty() {
        return Err("empty key combo".into());
    }
    if !combo
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-'))
    {
        return Err(format!(
            "invalid key combo {combo:?}: only letters, digits, '+', '_', '-' \
             (e.g. \"ctrl+s\", \"Return\", \"alt+Tab\")"
        ));
    }
    Ok(vec!["key".into(), "--clearmodifiers".into(), combo.into()])
}

/// `xdotool` argv for a mouse click at `(x, y)`. `button`: 1=left, 2=middle,
/// 3=right.
pub fn click_argv(x: i64, y: i64, button: u8) -> Result<Vec<String>, String> {
    if !(1..=3).contains(&button) {
        return Err(format!(
            "button must be 1 (left), 2 (middle), or 3 (right), got {button}"
        ));
    }
    if x < 0 || y < 0 {
        return Err(format!("coordinates must be non-negative, got ({x}, {y})"));
    }
    Ok(vec![
        "mousemove".into(),
        x.to_string(),
        y.to_string(),
        "click".into(),
        button.to_string(),
    ])
}

/// `xdotool windowactivate --sync` argv for raising/focusing a window.
pub fn focus_argv(window_id: &str) -> Result<Vec<String>, String> {
    validate_window_id(window_id)?;
    Ok(vec![
        "windowactivate".into(),
        "--sync".into(),
        window_id.to_string(),
    ])
}

/// A window id must be a bare decimal — both for xdotool and so a crafted
/// "id" can never smuggle extra argv.
pub fn validate_window_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "invalid window id {id:?}: expected a numeric xdotool window id \
             (use app.list to get one)"
        ));
    }
    Ok(())
}

/// Parse the newline-separated window ids from `xdotool search`. Blank lines
/// and non-numeric noise are dropped.
pub fn parse_window_ids(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
        .map(String::from)
        .collect()
}

// ---- impure runners -----------------------------------------------------

/// Run `xdotool` with `args`, returning trimmed stdout. A missing binary is
/// mapped to an actionable install hint.
pub async fn run_xdotool(args: &[String]) -> Result<String, String> {
    let out = Command::new("xdotool")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "`xdotool` is not installed — it is required for the \
                 applications tool process on Linux/X11 (install it via your \
                 package manager, e.g. `pacman -S xdotool` / `apt install \
                 xdotool`)"
                    .to_string()
            } else {
                format!("failed to run xdotool: {e}")
            }
        })?;
    if !out.status.success() {
        return Err(format!(
            "xdotool {} failed: {}",
            args.first().map(String::as_str).unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Enumerate visible windows with titles + the active flag. N+1 xdotool calls
/// (one search + one name lookup per window) — fine for a desktop's handful of
/// windows. Windows whose name lookup fails (just closed) are skipped.
pub async fn list_windows() -> Result<Vec<Window>, String> {
    let ids = parse_window_ids(
        &run_xdotool(&[
            "search".into(),
            "--onlyvisible".into(),
            "--name".into(),
            ".+".into(),
        ])
        .await?,
    );
    // Best-effort active id; an empty desktop has none.
    let active = run_xdotool(&["getactivewindow".into()])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut windows = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(title) =
            run_xdotool(&["getwindowname".into(), id.clone()]).await
        {
            let active = active.as_deref() == Some(id.as_str());
            windows.push(Window { id, title, active });
        }
    }
    Ok(windows)
}

/// Screenshot CLIs tried in order (full-screen capture to a file). The first
/// one present on the system wins. Covers Wayland (`grim`) and X11
/// (`maim`/`scrot`/ImageMagick `import`).
pub const SCREENSHOT_TOOLS: &[(&str, &[&str])] = &[
    ("grim", &[]),                      // grim <file>
    ("maim", &[]),                      // maim <file>
    ("scrot", &[]),                     // scrot <file>
    ("import", &["-window", "root"]),   // import -window root <file>
];

/// Capture a full-screen screenshot to `out_path` using the first available
/// tool. Returns the tool that succeeded. Errors (with an install hint) when
/// none is present.
pub async fn capture_screenshot(out_path: &std::path::Path) -> Result<String, String> {
    let mut last_err = String::new();
    for (cmd, fixed) in SCREENSHOT_TOOLS {
        let mut args: Vec<std::ffi::OsString> =
            fixed.iter().map(Into::into).collect();
        args.push(out_path.as_os_str().to_owned());
        match Command::new(cmd)
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .await
        {
            Ok(o) if o.status.success() && out_path.exists() => {
                return Ok((*cmd).to_string());
            }
            Ok(o) => {
                last_err = format!(
                    "{cmd} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            // NotFound → try the next tool.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => last_err = format!("{cmd}: {e}"),
        }
    }
    Err(format!(
        "no screenshot tool available (tried grim / maim / scrot / import) — \
         install one (e.g. `grim` on Wayland, `maim` or `scrot` on X11).{}",
        if last_err.is_empty() {
            String::new()
        } else {
            format!(" Last error: {last_err}")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_argv_passes_text_as_one_element_after_dashdash() {
        let a = type_argv("hello -world");
        assert_eq!(a, vec!["type", "--clearmodifiers", "--", "hello -world"]);
        // The text is a single argv element — no shell, no splitting.
        assert_eq!(a.last().unwrap(), "hello -world");
    }

    #[test]
    fn key_argv_accepts_chords_rejects_junk() {
        assert_eq!(
            key_argv("ctrl+s").unwrap(),
            vec!["key", "--clearmodifiers", "ctrl+s"]
        );
        assert_eq!(key_argv("Return").unwrap()[2], "Return");
        assert!(key_argv("").is_err());
        assert!(key_argv("ctrl+s; rm -rf").is_err(), "rejects shell metachars");
        assert!(key_argv("$(whoami)").is_err());
    }

    #[test]
    fn click_argv_validates_button_and_coords() {
        assert_eq!(
            click_argv(10, 20, 1).unwrap(),
            vec!["mousemove", "10", "20", "click", "1"]
        );
        assert!(click_argv(0, 0, 0).is_err(), "button 0 invalid");
        assert!(click_argv(0, 0, 4).is_err(), "button 4 invalid");
        assert!(click_argv(-1, 0, 1).is_err(), "negative coord invalid");
    }

    #[test]
    fn focus_argv_requires_numeric_id() {
        assert_eq!(
            focus_argv("12345").unwrap(),
            vec!["windowactivate", "--sync", "12345"]
        );
        assert!(focus_argv("not-an-id").is_err());
        assert!(focus_argv("12345; xdotool key Return").is_err());
        assert!(focus_argv("").is_err());
    }

    #[test]
    fn parse_window_ids_keeps_only_numeric_lines() {
        let out = "29360131\n  41943043 \n\ngarbage\n0x0123\n83886082\n";
        assert_eq!(
            parse_window_ids(out),
            vec!["29360131", "41943043", "83886082"]
        );
    }
}
