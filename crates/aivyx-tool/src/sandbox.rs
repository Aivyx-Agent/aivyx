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

/// Build the conservative-but-functional preset for `backend`.
/// `ro_extra` is bound read-only (the tool's command-binary dir,
/// so a binary outside `/usr` — e.g. `target/release` — is
/// reachable). `writable` is bound read-write (the per-tool data
/// dir, so the tool can read/write its OAuth token).
pub fn preset_for(
    backend: SandboxBackend,
    ro_extra: &[PathBuf],
    writable: &[PathBuf],
) -> SandboxConfig {
    match backend {
        SandboxBackend::Bubblewrap => {
            bubblewrap_preset(ro_extra, writable)
        }
        SandboxBackend::Firejail => firejail_preset(ro_extra, writable),
    }
}

/// Bubblewrap preset: read-only system (+ `ro_extra`), private
/// `/tmp`, isolated PID namespace, `$HOME` hidden except the
/// `writable` binds, network on. The strongest of the two
/// backends.
pub fn bubblewrap_preset(
    ro_extra: &[PathBuf],
    writable: &[PathBuf],
) -> SandboxConfig {
    let mut args: Vec<String> = Vec::new();
    for dir in RO_SYSTEM_DIRS {
        // `--ro-bind-try` so an absent dir (merged-/usr) is a
        // no-op instead of a spawn failure.
        args.push("--ro-bind-try".into());
        args.push((*dir).into());
        args.push((*dir).into());
    }
    // The command-binary dir (and any operator extras) read-only,
    // so a tool binary outside the system dirs is reachable.
    for path in ro_extra {
        let p = path_string(path);
        args.push("--ro-bind-try".into());
        args.push(p.clone());
        args.push(p);
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
/// fallback ordering. `ro_extra` / `writable` are accepted for a
/// uniform signature but not needed (home + system stay visible).
pub fn firejail_preset(
    _ro_extra: &[PathBuf],
    _writable: &[PathBuf],
) -> SandboxConfig {
    SandboxConfig {
        wrapper: SandboxBackend::Firejail.program().to_string(),
        args: vec![
            "--quiet".into(),
            "--noroot".into(),
            "--private-tmp".into(),
        ],
    }
}

/// The operator-facing `[sandbox].default_backend` choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxChoice {
    /// Use a detected backend (bwrap → firejail), else none.
    Auto,
    /// Force a specific backend.
    Backend(SandboxBackend),
    /// No default sandbox.
    None,
}

/// Resolve the effective sandbox for one tool-process spawn.
///
/// Precedence (highest first):
///
/// 1. `explicit` — an operator-supplied per-tool `sandbox` block
///    wins outright.
/// 2. `disabled` — a per-tool `disable_sandbox = true` opt-out.
/// 3. `global` — the `[sandbox].default_backend` choice (`Auto`
///    resolves through `detected`).
///
/// `ro_extra` (the command-binary dir) and `writable` (the
/// per-tool data dir) feed the chosen preset. Pure + testable.
pub fn resolve_sandbox(
    explicit: Option<SandboxConfig>,
    disabled: bool,
    global: SandboxChoice,
    detected: Option<SandboxBackend>,
    ro_extra: &[PathBuf],
    writable: &[PathBuf],
) -> Option<SandboxConfig> {
    if let Some(e) = explicit {
        return Some(e);
    }
    if disabled {
        return None;
    }
    let backend = match global {
        SandboxChoice::None => return None,
        SandboxChoice::Backend(b) => b,
        SandboxChoice::Auto => detected?,
    };
    Some(preset_for(backend, ro_extra, writable))
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
        let cmd_dir = PathBuf::from("/opt/aivyx/bin");
        let token_dir =
            PathBuf::from("/home/op/.aivyx/tool-processes/gmail");
        let c = bubblewrap_preset(&[cmd_dir], &[token_dir]);
        assert_eq!(c.wrapper, "bwrap");
        let a = c.args.join(" ");
        // Read-only system + the command-binary dir.
        assert!(a.contains("--ro-bind-try /usr /usr"));
        assert!(a.contains("--ro-bind-try /etc /etc"));
        assert!(a.contains("--ro-bind-try /opt/aivyx/bin /opt/aivyx/bin"));
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
    fn bubblewrap_preset_with_no_binds_nothing_extra() {
        let c = bubblewrap_preset(&[], &[]);
        assert!(!c.args.iter().any(|s| s == "--bind"));
        // Still isolates + keeps network.
        assert!(c.args.iter().any(|s| s == "--tmpfs"));
        assert!(c.args.iter().any(|s| s == "--share-net"));
    }

    #[test]
    fn firejail_preset_hardens_without_binds() {
        let c = firejail_preset(
            &[PathBuf::from("/ignored")],
            &[PathBuf::from("/ignored2")],
        );
        assert_eq!(c.wrapper, "firejail");
        assert_eq!(c.args, vec!["--quiet", "--noroot", "--private-tmp"]);
    }

    #[test]
    fn preset_for_dispatches_by_backend() {
        let ro = [PathBuf::from("/opt/aivyx/bin")];
        let w = [PathBuf::from("/home/op/.aivyx/tool-processes/notion")];
        assert_eq!(
            preset_for(SandboxBackend::Bubblewrap, &ro, &w).wrapper,
            "bwrap"
        );
        assert_eq!(
            preset_for(SandboxBackend::Firejail, &ro, &w).wrapper,
            "firejail"
        );
    }

    // ---- resolve_sandbox precedence ---------------------------

    fn explicit_cfg() -> SandboxConfig {
        SandboxConfig {
            wrapper: "operator-wrapper".into(),
            args: vec!["--custom".into()],
        }
    }

    #[test]
    fn resolve_explicit_wins_over_everything() {
        let got = resolve_sandbox(
            Some(explicit_cfg()),
            true, // even with disable set
            SandboxChoice::Backend(SandboxBackend::Bubblewrap),
            Some(SandboxBackend::Bubblewrap),
            &[],
            &[],
        )
        .expect("explicit");
        assert_eq!(got.wrapper, "operator-wrapper");
    }

    #[test]
    fn resolve_disable_opts_out_of_the_default() {
        assert!(resolve_sandbox(
            None,
            true,
            SandboxChoice::Auto,
            Some(SandboxBackend::Bubblewrap),
            &[],
            &[],
        )
        .is_none());
    }

    #[test]
    fn resolve_auto_uses_detected_else_none() {
        // Detected → preset.
        let got = resolve_sandbox(
            None,
            false,
            SandboxChoice::Auto,
            Some(SandboxBackend::Firejail),
            &[],
            &[],
        )
        .expect("preset");
        assert_eq!(got.wrapper, "firejail");
        // Nothing detected → None (graceful fallback).
        assert!(resolve_sandbox(
            None,
            false,
            SandboxChoice::Auto,
            None,
            &[],
            &[],
        )
        .is_none());
    }

    #[test]
    fn resolve_forced_backend_ignores_detection() {
        let got = resolve_sandbox(
            None,
            false,
            SandboxChoice::Backend(SandboxBackend::Bubblewrap),
            None, // nothing detected, but forced
            &[PathBuf::from("/opt/bin")],
            &[PathBuf::from("/home/op/.aivyx/tool-processes/x")],
        )
        .expect("forced");
        assert_eq!(got.wrapper, "bwrap");
    }

    #[test]
    fn resolve_global_none_means_no_sandbox() {
        assert!(resolve_sandbox(
            None,
            false,
            SandboxChoice::None,
            Some(SandboxBackend::Bubblewrap),
            &[],
            &[],
        )
        .is_none());
    }
}
