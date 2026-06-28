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

// AN.0 skeleton: the render/plan layer is exercised by its own tests and wired
// to the `aivyx daemon install` command in AN.1.
#![allow(dead_code)]

use std::path::PathBuf;

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
}
