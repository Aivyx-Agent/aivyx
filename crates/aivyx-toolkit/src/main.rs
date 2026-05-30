//! `aivyx-toolkit` binary entry point.
//!
//! Phase 125 Task 2 ships only the crate skeleton + capability
//! bases; this main is a deliberate stub. Tasks 3-6 land the
//! tools (web.search, task.*, health.check.*) and wire them
//! into the multi-tool IPC harness loop.
//!
//! Until Tasks 3-6 land, running the binary prints a usage
//! hint and exits with a non-zero status so a daemon
//! `[[tool_process]]` spawn doesn't silently hang waiting for
//! stdin.

fn main() {
    eprintln!(
        "aivyx-toolkit: Phase 125 Task 2 ships only the crate skeleton +\n\
         capability bases. The tools (web.search, task.*, health.check.*)\n\
         and the IPC harness wiring land in Tasks 3-6.\n\
         Try again after Phase 125 has fully shipped."
    );
    std::process::exit(2);
}
