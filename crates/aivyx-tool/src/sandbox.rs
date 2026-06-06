//! Phase 180 — bundled default sandbox presets.
//!
//! Phase 52/55 made the `[[tool_process]]` sandbox an
//! operator-supplied wrapper (`SandboxConfig { wrapper, args }`).
//! That left the out-of-the-box posture *unsandboxed*: a tool
//! process with no `sandbox` block spawned with the operator's
//! full UID. This module ships **bundled default presets** —
//! detected automatically and applied to new launches — so a
//! security-focused product is secure by default.
//!
//! The presets are **isolating but functional**: a tool that
//! reads its OAuth token from `$HOME/.aivyx/tool-processes/<tool>/`
//! and makes network calls must still work. So the preset gives
//! read-only system directories, a private `/tmp`, no `$HOME`
//! except a writable bind of the per-tool data dir, and leaves
//! network on (it is already capability-gated at the IPC
//! boundary). A tool needing more declares an explicit `sandbox`
//! block — the existing escape hatch.
//!
//! The argv these builders produce is the unit-testable core.
//! Whether `bwrap` actually contains a given process is verified
//! on the operator's machine (no sandbox backend in CI) — the
//! same operator-verification posture the threat model uses.

use std::path::{Path, PathBuf};

use crate::bridge::SandboxConfig;

/// A detected OS-level sandbox backend. `Bubblewrap` is preferred
/// (it can hide `$HOME` and bind only the per-tool dir);
/// `Firejail` is the fallback (process + `/tmp` hardening, but
/// `$HOME` stays readable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    Bubblewrap,
    Firejail,
}

impl SandboxBackend {
    /// The wrapper program name (also the `PATH` lookup key).
    pub fn program(self) -> &'static str {
        match self {
            SandboxBackend::Bubblewrap => "bwrap",
            SandboxBackend::Firejail => "firejail",
        }
    }

    /// Stable config / log label.
    pub fn label(self) -> &'static str {
        match self {
            SandboxBackend::Bubblewrap => "bubblewrap",
            SandboxBackend::Firejail => "firejail",
        }
    }

    /// Parse the `[sandbox].default_backend` label. `None` for an
    /// unknown string (the config layer turns that into an error).
    pub fn parse(s: &str) -> Option<SandboxBackend> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bubblewrap" | "bwrap" => Some(SandboxBackend::Bubblewrap),
            "firejail" => Some(SandboxBackend::Firejail),
            _ => None,
        }
    }
}

/// Detect an available backend on `PATH`. Prefers `bwrap`, then
/// `firejail`. `None` if neither is installed (the caller warns
/// and falls back to no sandbox).
pub fn detect_sandbox_backend() -> Option<SandboxBackend> {
    detect_with(binary_on_path)
}

/// Detection seam — `on_path(program)` reports whether a wrapper
/// is available. Public for unit tests (the real `PATH` lookup is
/// not reproducible in CI).
pub fn detect_with(
    on_path: impl Fn(&str) -> bool,
) -> Option<SandboxBackend> {
    if on_path(SandboxBackend::Bubblewrap.program()) {
        Some(SandboxBackend::Bubblewrap)
    } else if on_path(SandboxBackend::Firejail.program()) {
        Some(SandboxBackend::Firejail)
    } else {
        None
    }
}

fn binary_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).exists())
}

/// Read-only system directories every preset binds. `*-try`
/// variants tolerate absence (e.g. merged-`/usr` hosts where
/// `/bin` and `/lib` are symlinks already covered by `/usr`).
const RO_SYSTEM_DIRS: &[&str] =
    &["/usr", "/bin", "/lib", "/lib64", "/etc"];

/// Build the conservative-but-functional preset for `backend`,
/// binding each path in `writable` read-write (the per-tool data
/// dir, so the tool can read/write its OAuth token).
pub fn preset_for(
    backend: SandboxBackend,
    writable: &[PathBuf],
) -> SandboxConfig {
    match backend {
        SandboxBackend::Bubblewrap => bubblewrap_preset(writable),
        SandboxBackend::Firejail => firejail_preset(writable),
    }
}

/// Bubblewrap preset: read-only system, private `/tmp`, isolated
/// PID namespace, `$HOME` hidden except the writable binds,
/// network on. The strongest of the two backends.
pub fn bubblewrap_preset(writable: &[PathBuf]) -> SandboxConfig {
    let mut args: Vec<String> = Vec::new();
    for dir in RO_SYSTEM_DIRS {
        // `--ro-bind-try` so an absent dir (merged-/usr) is a
        // no-op instead of a spawn failure.
        args.push("--ro-bind-try".into());
        args.push((*dir).into());
        args.push((*dir).into());
    }
    args.push("--proc".into());
    args.push("/proc".into());
    args.push("--dev".into());
    args.push("/dev".into());
    args.push("--tmpfs".into());
    args.push("/tmp".into());
    // Writable binds — the per-tool data dir(s). `--bind` creates
    // intermediate mount points, so a nested token path works.
    for path in writable {
        let p = path_string(path);
        args.push("--bind".into());
        args.push(p.clone());
        args.push(p);
    }
    // Process isolation + clean teardown; network left on
    // (capability-gated at the IPC layer, and productivity tools
    // require it).
    args.push("--unshare-pid".into());
    args.push("--die-with-parent".into());
    args.push("--share-net".into());
    SandboxConfig {
        wrapper: SandboxBackend::Bubblewrap.program().to_string(),
        args,
    }
}

/// Firejail preset: drop root, private `/tmp`, default seccomp /
/// capability hardening. `$HOME` stays readable (firejail's model
/// differs), so the per-tool token is reachable without explicit
/// binds — weaker filesystem isolation than bubblewrap, hence the
/// fallback ordering. `writable` is accepted for a uniform
/// signature but not needed (home is visible).
pub fn firejail_preset(_writable: &[PathBuf]) -> SandboxConfig {
    SandboxConfig {
        wrapper: SandboxBackend::Firejail.program().to_string(),
        args: vec![
            "--quiet".into(),
            "--noroot".into(),
            "--private-tmp".into(),
        ],
    }
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_bwrap_then_firejail_then_none() {
        assert_eq!(detect_with(|_| true), Some(SandboxBackend::Bubblewrap));
        assert_eq!(
            detect_with(|p| p == "firejail"),
            Some(SandboxBackend::Firejail)
        );
        assert_eq!(detect_with(|_| false), None);
    }

    #[test]
    fn backend_parse_and_labels() {
        assert_eq!(
            SandboxBackend::parse("bubblewrap"),
            Some(SandboxBackend::Bubblewrap)
        );
        assert_eq!(
            SandboxBackend::parse(" BWRAP "),
            Some(SandboxBackend::Bubblewrap)
        );
        assert_eq!(
            SandboxBackend::parse("firejail"),
            Some(SandboxBackend::Firejail)
        );
        assert_eq!(SandboxBackend::parse("docker"), None);
        assert_eq!(SandboxBackend::Bubblewrap.label(), "bubblewrap");
        assert_eq!(SandboxBackend::Firejail.program(), "firejail");
    }

    #[test]
    fn bubblewrap_preset_argv_is_isolating_and_functional() {
        let token_dir =
            PathBuf::from("/home/op/.aivyx/tool-processes/gmail");
        let c = bubblewrap_preset(&[token_dir.clone()]);
        assert_eq!(c.wrapper, "bwrap");
        let a = c.args.join(" ");
        // Read-only system.
        assert!(a.contains("--ro-bind-try /usr /usr"));
        assert!(a.contains("--ro-bind-try /etc /etc"));
        // Private /tmp + proc/dev.
        assert!(a.contains("--tmpfs /tmp"));
        assert!(a.contains("--proc /proc"));
        // The per-tool token dir is the ONLY writable $HOME path.
        assert!(a.contains(
            "--bind /home/op/.aivyx/tool-processes/gmail \
             /home/op/.aivyx/tool-processes/gmail"
        ));
        assert!(!a.contains("--bind /home/op "));
        // Process isolation + network on.
        assert!(a.contains("--unshare-pid"));
        assert!(a.contains("--die-with-parent"));
        assert!(a.contains("--share-net"));
    }

    #[test]
    fn bubblewrap_preset_with_no_writable_binds_nothing_extra() {
        let c = bubblewrap_preset(&[]);
        assert!(!c.args.iter().any(|s| s == "--bind"));
        // Still isolates + keeps network.
        assert!(c.args.iter().any(|s| s == "--tmpfs"));
        assert!(c.args.iter().any(|s| s == "--share-net"));
    }

    #[test]
    fn firejail_preset_hardens_without_binds() {
        let c = firejail_preset(&[PathBuf::from("/ignored")]);
        assert_eq!(c.wrapper, "firejail");
        assert_eq!(c.args, vec!["--quiet", "--noroot", "--private-tmp"]);
    }

    #[test]
    fn preset_for_dispatches_by_backend() {
        let w = [PathBuf::from("/home/op/.aivyx/tool-processes/notion")];
        assert_eq!(
            preset_for(SandboxBackend::Bubblewrap, &w).wrapper,
            "bwrap"
        );
        assert_eq!(
            preset_for(SandboxBackend::Firejail, &w).wrapper,
            "firejail"
        );
    }
}
