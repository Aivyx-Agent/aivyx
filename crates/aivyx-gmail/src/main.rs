//! `aivyx-gmail` binary entry point.
//!
//! Phase 123 Task 2 ships only the OAuth substrate; this main is
//! a deliberate stub. Task 3 lands the operator-facing CLI
//! surface (`aivyx-gmail auth init / status / revoke`); Tasks
//! 4-7 land the tool implementations and the multi-tool harness
//! loop that wires them to the daemon over IPC.
//!
//! Until Task 3 lands, running the binary prints a usage hint
//! and exits with a non-zero status so an operator who tries to
//! run it doesn't silently get a no-op.

fn main() {
    eprintln!(
        "aivyx-gmail: Phase 123 Task 2 ships only the OAuth substrate.\n\
         The operator-facing CLI (`auth init / status / revoke`) lands\n\
         in Task 3; the Gmail tools and IPC harness land in Tasks 4-7.\n\
         Try again after Phase 123 has fully shipped."
    );
    std::process::exit(2);
}
