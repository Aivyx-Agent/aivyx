//! `aivyx daemon install` — first-class persistent service (Chapter Anchor).
//!
//! A local-first agent meant to run for *days* needs a supported way to stay
//! running. Today the only paths are the Docker appliance (Chapter Harbor) and
//! the desktop app's autostart; the **bare daemon** — the default install — has
//! none, so on the dogfood rig we hand-rolled `loginctl enable-linger` +
//! `systemd-run`. No real user will do that. This module turns it into one
//! command.
//!
//! ## AN.0 design decisions
//!
//! - **Linux = a systemd *user* unit** at `~/.config/systemd/user/aivyx-daemon.service`,
//!   plus `loginctl enable-linger <user>` so it runs **without an active login
//!   session** (the runs-for-days requirement). Not a system unit — no root, no
//!   sudo; the agent is the user's, not the machine's.
//! - **macOS = a launchd `LaunchAgent`** plist at
//!   `~/Library/LaunchAgents/com.aivyx.daemon.plist` (rendered in AN.2).
//! - **Windows = out of scope** for now (documented; the desktop app covers it).
//! - **Command surface:** `aivyx daemon install [--web-ui] [--no-start]` and
//!   `aivyx daemon uninstall` (wired in AN.1).
//! - **Secret handling (the real decision):** the unit references an *optional*
//!   env file (`EnvironmentFile=-<path>`, mode `0o600`) rather than baking the
//!   passphrase into the unit (the hand-rolled rig version put `AIVYX_PASSPHRASE`
//!   plaintext into the unit's `--setenv`, world-readable in `systemctl cat`).
//!   `daemon install` captures the passphrase (from the env or a prompt) and
//!   writes the `0o600` env file (AN.1); the **rendered unit carries no secret**,
//!   only the file path. Plaintext-at-rest under `0o600` is the same protection
//!   level as the redb store passphrase and the federation key; a keyring
//!   backend is a future enhancement.
//! - **Idempotent:** re-install overwrites the unit + reloads; uninstall stops,
//!   disables, and removes it. **No new capability base / no P10** — this is
//!   operator-facing management that shells out to `systemctl`/`launchctl`, not
//!   an agent tool.
//!
//! AN.0 ships the pure, tested render/plan layer below; AN.1 wires the CLI and
//! performs the side effects (write the unit, enable linger, `enable --now`).

// macOS items (LAUNCHD_LABEL, the launchd plan) are wired in AN.2; suppress the
// dead-code warning for them until then.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The host service manager Anchor targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// systemd user unit + linger.
    Linux,
    /// launchd LaunchAgent (AN.2).
    MacOs,
    /// No supported service manager — `install` errors with guidance.
    Unsupported,
}

impl Platform {
    /// Detect the current host's service manager.
    pub fn detect() -> Self {
        if cfg!(target_os = "linux") {
            Platform::Linux
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Unsupported
        }
    }
}

/// The unit / plist name (stable across platforms for `status`/`uninstall`).
pub const SERVICE_UNIT: &str = "aivyx-daemon.service";
/// The launchd label (macOS, AN.2).
pub const LAUNCHD_LABEL: &str = "com.aivyx.daemon";

/// Where the secret env file lives (referenced by the unit, written 0o600 by
/// `install`). Relative to the user's config dir.
pub const ENV_FILE_REL: &str = "aivyx/daemon.env";

/// A resolved install plan — the concrete paths + contents an install will
/// write. Pure data so a `--dry-run`/preview (AN.1) can show exactly what will
/// happen before any side effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePlan {
    /// Where the unit/plist file goes.
    pub unit_path: PathBuf,
    /// The rendered unit/plist contents (carries **no secret**).
    pub unit_contents: String,
    /// The 0o600 env file the unit references (written separately by install).
    pub env_file_path: PathBuf,
}

/// Render the systemd **user** unit. Pure over its inputs so the exact bytes are
/// testable. The unit references the env file via `EnvironmentFile=-` (the `-`
/// makes it optional, so a missing/rotated secret file degrades to a clear
/// startup error rather than a unit that won't load).
pub fn render_systemd_unit(
    bin_path: &str,
    web_ui: bool,
    env_file: &str,
    working_dir: &str,
) -> String {
    let web_ui_flag = if web_ui { " --web-ui" } else { "" };
    format!(
        "[Unit]\n\
         Description=Aivyx personal-assistant daemon\n\
         Documentation=https://github.com/Aivyx-Agent/aivyx\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin_path} daemon run{web_ui_flag}\n\
         WorkingDirectory={working_dir}\n\
         EnvironmentFile=-{env_file}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// Render the 0o600 env file the unit references. The **only** place the
/// passphrase is written, and it lives at rest under owner-only permissions
/// (set by the caller) — never in the unit, never in the process table.
pub fn render_env_file(passphrase: &str) -> String {
    format!("AIVYX_PASSPHRASE={passphrase}\n")
}

/// Compute the concrete Linux install plan — pure over its inputs so the paths
/// and unit contents are testable without touching the real home or running any
/// command.
pub fn plan_linux(
    config_dir: &Path,
    bin_path: &str,
    working_dir: &str,
    web_ui: bool,
) -> ServicePlan {
    let unit_path = config_dir.join("systemd/user").join(SERVICE_UNIT);
    let env_file_path = config_dir.join(ENV_FILE_REL);
    let unit_contents = render_systemd_unit(
        bin_path,
        web_ui,
        &env_file_path.display().to_string(),
        working_dir,
    );
    ServicePlan { unit_path, unit_contents, env_file_path }
}

/// `aivyx daemon install` — install + (by default) start the daemon as a
/// persistent user service. Linux now; macOS in AN.2.
pub fn run_install(web_ui: bool, start: bool) -> Result<(), String> {
    match Platform::detect() {
        Platform::Linux => install_linux(web_ui, start),
        Platform::MacOs => Err(
            "macOS service install lands in Anchor AN.2 — for now use the desktop \
             app's autostart or run `aivyx daemon run`."
                .into(),
        ),
        Platform::Unsupported => Err(
            "no supported service manager on this platform — run `aivyx daemon run` \
             directly, or use the Docker appliance (docs/INSTALL.md)."
                .into(),
        ),
    }
}

/// `aivyx daemon uninstall` — stop, disable, and remove the service.
pub fn run_uninstall() -> Result<(), String> {
    match Platform::detect() {
        Platform::Linux => uninstall_linux(),
        Platform::MacOs => Err("macOS service uninstall lands in Anchor AN.2.".into()),
        Platform::Unsupported => {
            Err("no service was installed by aivyx on this platform.".into())
        }
    }
}

fn install_linux(web_ui: bool, start: bool) -> Result<(), String> {
    let bin = current_exe_path()?;
    let config_dir = user_config_dir()?;
    let working_dir = install_working_dir();
    let plan = plan_linux(&config_dir, &bin, &working_dir, web_ui);

    // The passphrase: env first (the established policy), else a no-echo prompt.
    let passphrase = resolve_passphrase()?;

    // Write the 0o600 env file (the only secret on disk), then the unit.
    if let Some(parent) = plan.env_file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create config dir {}: {e}", parent.display()))?;
    }
    std::fs::write(&plan.env_file_path, render_env_file(&passphrase))
        .map_err(|e| format!("write env file: {e}"))?;
    set_permissions_600(&plan.env_file_path)?;

    if let Some(parent) = plan.unit_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create unit dir {}: {e}", parent.display()))?;
    }
    std::fs::write(&plan.unit_path, &plan.unit_contents)
        .map_err(|e| format!("write unit {}: {e}", plan.unit_path.display()))?;

    // Linger so the service runs without an active login session (runs-for-days).
    run_cmd("loginctl", &["enable-linger", &current_user()])?;
    run_cmd("systemctl", &["--user", "daemon-reload"])?;
    run_cmd("systemctl", &["--user", "enable", SERVICE_UNIT])?;
    if start {
        // restart (not just start) so a re-install picks up the new unit/env.
        run_cmd("systemctl", &["--user", "restart", SERVICE_UNIT])?;
    }

    eprintln!(
        "aivyx daemon: installed as a user service.\n  \
         unit:   {}\n  \
         env:    {} (0600)\n  \
         status: systemctl --user status aivyx-daemon\n  \
         logs:   journalctl --user -u aivyx-daemon -f{}",
        plan.unit_path.display(),
        plan.env_file_path.display(),
        if start { "\n  (started; runs across reboots via linger)" } else { "\n  (enabled; start with `systemctl --user start aivyx-daemon`)" },
    );
    Ok(())
}

fn uninstall_linux() -> Result<(), String> {
    let config_dir = user_config_dir()?;
    let unit_path = config_dir.join("systemd/user").join(SERVICE_UNIT);
    let env_file_path = config_dir.join(ENV_FILE_REL);

    // Stop + disable; ignore failures (the unit may already be gone/stopped).
    let _ = run_cmd("systemctl", &["--user", "disable", "--now", SERVICE_UNIT]);

    let mut removed = false;
    if unit_path.exists() {
        std::fs::remove_file(&unit_path)
            .map_err(|e| format!("remove unit {}: {e}", unit_path.display()))?;
        removed = true;
    }
    // The env file holds the passphrase — remove it on uninstall (hygiene).
    if env_file_path.exists() {
        std::fs::remove_file(&env_file_path)
            .map_err(|e| format!("remove env file {}: {e}", env_file_path.display()))?;
    }
    let _ = run_cmd("systemctl", &["--user", "daemon-reload"]);

    if removed {
        eprintln!(
            "aivyx daemon: service uninstalled (unit + env file removed). \
             Linger was left enabled; disable with `loginctl disable-linger`."
        );
    } else {
        eprintln!("aivyx daemon: no installed service found — nothing to remove.");
    }
    Ok(())
}

/// The passphrase for the unattended service: `AIVYX_PASSPHRASE` if set+non-empty
/// (the established policy), else a no-echo prompt.
fn resolve_passphrase() -> Result<String, String> {
    if let Ok(v) = std::env::var("AIVYX_PASSPHRASE") {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    let p = rpassword::prompt_password(
        "Store passphrase for the unattended service (input hidden): ",
    )
    .map_err(|e| format!("failed to read passphrase: {e}"))?;
    if p.is_empty() {
        return Err("passphrase must not be empty".into());
    }
    Ok(p)
}

fn current_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|e| format!("resolve aivyx binary path: {e}"))
        .map(|p| p.display().to_string())
}

/// `$XDG_CONFIG_HOME` or `$HOME/.config`.
fn user_config_dir() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x));
        }
    }
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".config"))
}

/// The daemon loads `aivyx.toml` from its working directory — use the install
/// cwd when it holds a config, else `$HOME`.
fn install_working_dir() -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("aivyx.toml").exists() {
            return cwd.display().to_string();
        }
    }
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

fn current_user() -> String {
    std::env::var("USER").unwrap_or_default()
}

fn set_permissions_600(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set 0600 on {}: {e}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

/// Run a command, mapping a non-zero exit (or spawn failure) to a readable
/// error that names the command.
fn run_cmd(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("`{program} {}` failed to run: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!(
            "`{program} {}` exited with {}",
            args.join(" "),
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_picks_a_platform() {
        // On the build/CI host (Linux) detection is Linux; the point is it never
        // panics and returns a concrete platform.
        let p = Platform::detect();
        assert!(matches!(p, Platform::Linux | Platform::MacOs | Platform::Unsupported));
    }

    #[test]
    fn systemd_unit_has_the_load_bearing_directives() {
        let unit = render_systemd_unit(
            "/home/u/.local/bin/aivyx",
            false,
            "/home/u/.config/aivyx/daemon.env",
            "/home/u",
        );
        assert!(unit.contains("ExecStart=/home/u/.local/bin/aivyx daemon run\n"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target")); // user-unit auto-start target
        // optional env-file reference (the `-`), so a missing secret file doesn't
        // wedge the unit at load time.
        assert!(unit.contains("EnvironmentFile=-/home/u/.config/aivyx/daemon.env"));
        assert!(unit.contains("WorkingDirectory=/home/u"));
    }

    #[test]
    fn web_ui_flag_is_threaded_into_execstart() {
        let with = render_systemd_unit("/b/aivyx", true, "/e", "/w");
        assert!(with.contains("ExecStart=/b/aivyx daemon run --web-ui\n"));
        let without = render_systemd_unit("/b/aivyx", false, "/e", "/w");
        assert!(without.contains("ExecStart=/b/aivyx daemon run\n"));
        assert!(!without.contains("--web-ui"));
    }

    #[test]
    fn unit_never_contains_a_secret() {
        // The rendered unit references the env-file path but never a passphrase.
        let unit = render_systemd_unit("/b/aivyx", true, "/home/u/.config/aivyx/daemon.env", "/w");
        assert!(!unit.to_lowercase().contains("passphrase"));
        assert!(!unit.contains("AIVYX_PASSPHRASE"));
    }

    #[test]
    fn plan_resolves_unit_and_env_paths_under_config_dir() {
        let plan = plan_linux(Path::new("/home/u/.config"), "/home/u/.local/bin/aivyx", "/home/u", false);
        assert_eq!(
            plan.unit_path,
            PathBuf::from("/home/u/.config/systemd/user/aivyx-daemon.service")
        );
        assert_eq!(plan.env_file_path, PathBuf::from("/home/u/.config/aivyx/daemon.env"));
        // the unit references that exact env-file path
        assert!(plan.unit_contents.contains("EnvironmentFile=-/home/u/.config/aivyx/daemon.env"));
    }

    #[test]
    fn env_file_holds_the_passphrase_and_nothing_else() {
        assert_eq!(render_env_file("s3cr3t"), "AIVYX_PASSPHRASE=s3cr3t\n");
    }
}
