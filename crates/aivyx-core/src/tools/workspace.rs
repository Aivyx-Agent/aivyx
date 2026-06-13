//! Chapter O — the agent's personal workspace.
//!
//! A dedicated, always-available directory the agent OWNS (default
//! `~/.aivyx/workspace/`), for its own thoughts, ideas, plans, and multi-file
//! projects — the third leg alongside `memory.*` (recall facts) and the
//! operator's `fs.*` / `fs_root` (shared work, Chapter N access levels). It is
//! independent of the access level: even a fully-sandboxed agent has its own
//! notebook here.
//!
//! This module provides [`provision_workspace`] (idempotent startup seeding).
//! The `workspace.*` tools that operate within it land in O.2.

use std::path::Path;

/// Seed README written into a fresh workspace (only when absent — the agent's
/// own edits are never clobbered). Addressed to the agent itself.
const WORKSPACE_README: &str = "\
# Your workspace

This directory is **yours** — your own space, separate from the operator's
files. Use it freely for your own purposes: keep a journal, sketch ideas,
draft and revise plans, and scaffold your own projects. Nothing here is the
operator's; organize it however helps you think.

Suggested layout (just a starting point — make your own):

- `journal/` — dated entries; what you did, noticed, or are mulling over.
- `ideas/`   — half-formed thoughts and sketches worth keeping.
- `plans/`   — plans you draft and revise over time.
- `projects/`— your own multi-file projects.

Use the `workspace.*` tools to read, write, list, and append here. The
operator can see this space, so it is a window into your thinking — but it is
yours to use.
";

/// The seed subdirectories created in a fresh workspace.
pub const WORKSPACE_SUBDIRS: &[&str] = &["journal", "ideas", "plans", "projects"];

/// Create the workspace directory + seed structure, idempotently. Safe to call
/// on every startup: `create_dir_all` no-ops on existing dirs, and the README
/// is written only when absent so the agent's own edits survive.
pub fn provision_workspace(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    for sub in WORKSPACE_SUBDIRS {
        std::fs::create_dir_all(root.join(sub))?;
    }
    let readme = root.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, WORKSPACE_README)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("aivyx-ws-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn provision_creates_dir_seed_and_subdirs() {
        let root = tmp("provision");
        provision_workspace(&root).unwrap();
        assert!(root.join("README.md").is_file());
        for sub in WORKSPACE_SUBDIRS {
            assert!(root.join(sub).is_dir(), "{sub} should exist");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provision_is_idempotent_and_preserves_edits() {
        let root = tmp("idempotent");
        provision_workspace(&root).unwrap();
        // Operator/agent edits the README + adds a file.
        std::fs::write(root.join("README.md"), "my own notes").unwrap();
        std::fs::write(root.join("journal").join("day1.md"), "entry").unwrap();
        // Re-provision must not clobber either.
        provision_workspace(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("README.md")).unwrap(),
            "my own notes"
        );
        assert!(root.join("journal").join("day1.md").is_file());
        std::fs::remove_dir_all(&root).ok();
    }
}
