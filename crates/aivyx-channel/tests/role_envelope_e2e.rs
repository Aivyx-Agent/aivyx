//! Phase 15 Task 2 — cross-crate integration test for
//! `aivyx_channel::assemble_role_envelope`.
//!
//! ## Why this file exists
//!
//! Phase 14 Task 1 lifted `assemble_role_envelope` out of
//! `crates/aivyx-channel/src/bin/aivyx.rs` and into
//! `crates/aivyx-channel/src/role_envelope.rs`, so the walker
//! became a library-reachable function on the channel crate.
//! That lift was structurally motivated (Phase 14's
//! `RoleSwitchTool` needed a library-reachable envelope
//! assembler to honour PRODUCT.md P1.3 "structurally impossible
//! escalation"), but no test ever proved the new public surface
//! is actually reachable from *outside* `aivyx-channel`. The
//! Phase 13 Task 3 deferral recorded this gap:
//!
//!   > "Lift `assemble_role_envelope` from the binary into
//!   > `aivyx-channel/src/lib.rs` for cross-crate integration
//!   > tests."
//!
//! Phase 14 Task 1 closed the lift half; this file closes the
//! *test* half. It lives in the `tests/` integration-test
//! directory, which compiles as an **external** crate against
//! `aivyx-channel`'s public API — if the walker were still
//! crate-private, this file would not link. The binary-internal
//! `examples/aivyx.toml` regression tests in
//! `src/bin/aivyx.rs` continue to pin the same envelopes from
//! the binary's side; this file is the matching pin from the
//! library's side.
//!
//! ## Relationship to the binary-internal regression tests
//!
//! The three envelope assertions below (coder, researcher,
//! junior_researcher) duplicate the `example_aivyx_toml_*`
//! tests in `src/bin/aivyx.rs` by design — the value added is
//! not a new envelope claim but the structural proof that the
//! *same* envelope math runs through `aivyx_channel`'s public
//! API. If a future refactor removes `pub` from
//! `assemble_role_envelope`, this file fails to compile and
//! the binary-internal tests still pass — that is the signal
//! we want.
//!
//! Three deliberate additions beyond pure duplication:
//!
//! 1. The `default` role gets an envelope assertion here; the
//!    binary-internal tests skipped it because `default` is
//!    the root and every other test implicitly exercises it.
//!    Pinning it here documents the root envelope directly.
//! 2. `MAX_INHERITANCE_DEPTH` is asserted to be reachable as
//!    `aivyx_channel::MAX_INHERITANCE_DEPTH`, which is the
//!    belt-and-suspenders constant the walker uses to bail
//!    out on a validator regression. Reaching it through the
//!    public API pins that the constant is re-exported, not
//!    just the function.
//! 3. A divergence cross-check: `junior_researcher` and
//!    `researcher` share the same parent chain but produce
//!    different envelopes because of the empty-child floor
//!    substitution. The binary-internal test pins this too,
//!    but the library-side pin makes the surprise visible to
//!    anyone auditing the channel crate's public surface.

use std::path::PathBuf;

use aivyx_capability::{Scope, TrustTier};
use aivyx_channel::{assemble_role_envelope, MAX_INHERITANCE_DEPTH};
use aivyx_config::{AivyxConfig, LoadOptions};

/// Build the Local-channel backcompat floor the binary uses at
/// startup. Mirrors `local_channel_floor_with_sandbox` in
/// `src/bin/aivyx.rs`'s `mod tests`. The path `"/tmp/sandbox"`
/// is a stand-in for the production binary's canonicalized
/// `fs_root` — the integration test does not touch the real
/// filesystem, it just feeds this vector into
/// `assemble_role_envelope` to match the runtime shape.
fn local_channel_floor_with_sandbox(sandbox: &str) -> Vec<Scope> {
    vec![
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        Scope::parse(&format!("fs.read:{sandbox}/**")).unwrap(),
        Scope::parse(&format!("fs.write:{sandbox}/**")).unwrap(),
        Scope::parse("net.fetch").unwrap(),
        Scope::parse("shell.exec").unwrap(),
    ]
}

/// Load `examples/aivyx.toml` from the repo root. The path is
/// resolved via `CARGO_MANIFEST_DIR`, which for this integration
/// test crate is `crates/aivyx-channel/` — the same anchor the
/// binary-internal tests use, so the relative `../../examples/`
/// walk is identical.
///
/// The example file is designed to load without any real
/// secrets, so `require_api_key: false` and
/// `require_telegram_token: false` let it parse cleanly without
/// env vars. `role_override` is set to `"default"` only to
/// satisfy the loader's active-role pick; each test re-resolves
/// the role it actually wants out of `cfg.roles`.
fn load_example_config() -> AivyxConfig {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let example_path = PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("examples")
        .join("aivyx.toml");
    assert!(
        example_path.exists(),
        "examples/aivyx.toml must exist at {example_path:?}"
    );
    let opts = LoadOptions {
        toml_path: Some(example_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: Some("default".to_string()),
    };
    AivyxConfig::load_from_env_and_toml(&opts)
        .expect("examples/aivyx.toml must load cleanly via aivyx-config")
}

/// Collect an envelope's scopes as a sorted `Vec<String>` for
/// order-independent comparison.
fn envelope_strings(envelope: &aivyx_capability::CapabilitySet) -> Vec<String> {
    let mut out: Vec<String> = envelope.iter().map(|s| s.as_str().to_string()).collect();
    out.sort();
    out
}

/// `default` is the root role in `examples/aivyx.toml`. It has
/// no parent, so `assemble_role_envelope` returns its declared
/// set verbatim (no intersection upward). Composed with
/// `CEILING_TRUSTED` it keeps every declared scope, since each
/// one is an unqualified base that CEILING_TRUSTED also holds.
///
/// Pinning `default` here is the one assertion the binary-
/// internal regression tests do not cover directly — every
/// other test exercises `default` only as a transitive parent.
#[test]
fn cross_crate_assemble_envelope_for_default_matches_declared_set() {
    let cfg = load_example_config();
    let default_role = cfg.roles.get("default").expect("default role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(default_role, &cfg.roles, &floor);
    let effective = envelope.intersect(TrustTier::Trusted.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "memory.read".to_string(),
        "memory.write".to_string(),
        "memory.forget".to_string(),
        "fs.read".to_string(),
        "fs.write".to_string(),
        "net.fetch".to_string(),
        "shell.exec".to_string(),
        "role.switch".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "default role's envelope (root, no parent) must be its \
         declared eight scopes, including the unqualified \
         role.switch that Phase 14 Task 2 added as the ancestor \
         grant for descendants' qualified forms"
    );
}

/// `coder` attenuates `default` by dropping `net.fetch` and
/// narrows `role.switch` to a single target. Identical envelope
/// claim to the binary-internal `example_aivyx_toml_coder_*`
/// test — the value added is that this assertion runs through
/// `aivyx_channel::assemble_role_envelope`'s **public** surface,
/// proving Phase 14 Task 1's lift actually exposed the walker.
#[test]
fn cross_crate_assemble_envelope_for_coder_matches_documented_set() {
    let cfg = load_example_config();
    let coder = cfg.roles.get("coder").expect("coder role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(coder, &cfg.roles, &floor);
    let effective = envelope.intersect(coder.trust_ceiling.value.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "fs.read".to_string(),
        "fs.write".to_string(),
        "memory.read".to_string(),
        "memory.write".to_string(),
        "memory.forget".to_string(),
        "shell.exec".to_string(),
        "role.switch:researcher".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "coder runtime envelope (via cross-crate public API) \
         must match the seven scopes the example file documents \
         — identical math to the binary-internal regression test"
    );
}

/// `researcher` attenuates `default` by dropping `fs.write` /
/// `shell.exec` and narrowing `net.fetch` to a URL prefix. Runs
/// at `Trusted` so unqualified `fs.read` survives the ceiling
/// intersection (CEILING_SEMITRUSTED would strip it, but the
/// example deliberately stays at Trusted to keep the
/// attenuation teaching point clean).
#[test]
fn cross_crate_assemble_envelope_for_researcher_matches_documented_set() {
    let cfg = load_example_config();
    let researcher = cfg
        .roles
        .get("researcher")
        .expect("researcher role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(researcher, &cfg.roles, &floor);
    let effective = envelope.intersect(researcher.trust_ceiling.value.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "fs.read".to_string(),
        "memory.read".to_string(),
        "memory.write".to_string(),
        "memory.forget".to_string(),
        "net.fetch:url-prefix:https://httpbin.org/".to_string(),
        "role.switch:junior_researcher".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "researcher runtime envelope (via cross-crate public API) \
         must match the six scopes the example file documents — \
         includes role.switch:junior_researcher added in Phase 33"
    );
}

/// `junior_researcher` is the **empty-child surprise** case,
/// pinned from outside the channel crate. Its
/// `capability_scopes = []` triggers backcompat-floor
/// substitution at the child level, so the walker intersects
/// the floor against `researcher`'s declared set upward. Two
/// teaching points the assertion locks in:
///
/// - `fs.read:/tmp/sandbox/**` (path-qualified, from the
///   floor) survives because `researcher.fs.read` (unqualified)
///   grants it under D4 Rule 2.
/// - `researcher.fs.read` (unqualified) does **not** survive,
///   because `floor.fs.read:/tmp/sandbox/**` (qualified-held)
///   cannot grant the unqualified form back under D4 Rule 4.
///
/// This asymmetry is the whole reason the empty-child surprise
/// is worth documenting — an operator's mental model of "empty
/// inherits parent" is quietly wrong, and the test mechanically
/// pins the actual runtime shape so any future refactor that
/// changed the intersection direction would break loud.
#[test]
fn cross_crate_assemble_envelope_for_junior_researcher_demonstrates_floor_substitution() {
    let cfg = load_example_config();
    let junior = cfg
        .roles
        .get("junior_researcher")
        .expect("junior_researcher role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(junior, &cfg.roles, &floor);
    let effective = envelope.intersect(junior.trust_ceiling.value.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "fs.read:/tmp/sandbox/**".to_string(),
        "memory.read".to_string(),
        "memory.write".to_string(),
        "memory.forget".to_string(),
        "net.fetch:url-prefix:https://httpbin.org/".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "junior_researcher runtime envelope demonstrates the \
         empty-child floor substitution: the path-qualified \
         fs.read from the floor survives; researcher's \
         unqualified fs.read does NOT carry through under D4 \
         Rule 4"
    );
}

/// Cross-check the divergence: `junior_researcher` and
/// `researcher` produce different runtime envelopes even
/// though an operator's mental model would expect them
/// identical. This is the same divergence the binary-internal
/// test asserts; duplicating it here pins the surprise on the
/// library-side public surface as well.
#[test]
fn cross_crate_junior_researcher_envelope_diverges_from_researcher() {
    let cfg = load_example_config();
    let junior = cfg
        .roles
        .get("junior_researcher")
        .expect("junior_researcher role declared");
    let researcher = cfg
        .roles
        .get("researcher")
        .expect("researcher role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let junior_envelope = assemble_role_envelope(junior, &cfg.roles, &floor)
        .intersect(junior.trust_ceiling.value.default_ceiling());
    let researcher_envelope = assemble_role_envelope(researcher, &cfg.roles, &floor)
        .intersect(researcher.trust_ceiling.value.default_ceiling());

    let junior_strs = envelope_strings(&junior_envelope);
    let researcher_strs = envelope_strings(&researcher_envelope);

    assert_ne!(
        junior_strs, researcher_strs,
        "junior_researcher and researcher must diverge at runtime: \
         junior holds fs.read:/tmp/sandbox/** (path-qualified from \
         the floor), researcher holds fs.read (unqualified from its \
         own declared set). Same base, different envelopes."
    );
    assert!(
        junior_strs.contains(&"fs.read:/tmp/sandbox/**".to_string()),
        "junior must hold the path-qualified fs.read from the floor: {junior_strs:?}"
    );
    assert!(
        researcher_strs.contains(&"fs.read".to_string()),
        "researcher must hold the unqualified fs.read from its declared set: {researcher_strs:?}"
    );
}

/// Pin that `MAX_INHERITANCE_DEPTH` is reachable via
/// `aivyx_channel::MAX_INHERITANCE_DEPTH`. Phase 14 Task 1 added
/// the re-export alongside `assemble_role_envelope`; this
/// assertion locks the re-export into the public-API contract
/// so a future cleanup that removed the `pub use` would fail
/// here. The actual value (`64`) is not load-bearing and is
/// asserted as a non-zero sanity check, not a pinned number —
/// widening the depth bound is a legitimate future change that
/// should not require a test edit.
#[test]
fn cross_crate_max_inheritance_depth_is_reachable_via_public_api() {
    // `const`-eval the bound so clippy does not flag the check
    // as a tautological runtime assertion. The real purpose of
    // this test is not the numerical inequality — it is the
    // fact that the reference compiles at all, which proves
    // `MAX_INHERITANCE_DEPTH` is reachable through
    // `aivyx_channel`'s public API. Without the re-export in
    // `lib.rs`, the `use` at the top of the file would fail.
    const _: () = assert!(
        MAX_INHERITANCE_DEPTH >= 8,
        "MAX_INHERITANCE_DEPTH must comfortably exceed realistic role tree depth"
    );
    let _ = MAX_INHERITANCE_DEPTH;
}
