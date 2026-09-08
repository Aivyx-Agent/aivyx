//! Phase 15 Task 3 — cross-crate integration tests for
//! `aivyx_channel::render_role_envelope`.
//!
//! These seven tests moved out of
//! `crates/aivyx-channel/src/bin/aivyx.rs`'s `mod tests` when
//! Phase 15 Task 3 lifted `render_role_envelope`,
//! `build_display_floor`, `drop_reason_for`, and the
//! `ChannelKind` enum out of the binary and into
//! `crates/aivyx-channel/src/role_render.rs`. The binary kept
//! only the CLI **parse-time** tests (which exercise
//! `parse_cli_args`, still binary-private); everything that
//! drives the renderer directly now lives here.
//!
//! ## Why the move
//!
//! The functional assertions don't depend on anything the
//! binary owns — they load `examples/aivyx-pa.toml`, call
//! `render_role_envelope`, and assert on substrings of the
//! returned string. That is exactly the shape Rust integration
//! tests are best at: compiling against the public surface of
//! `aivyx-channel` as an external crate. Keeping them in the
//! binary's `mod tests` was an artifact of the renderer being
//! binary-private; Phase 15 Task 3's lift removes that
//! constraint and the tests follow their code home.
//!
//! ## Coverage
//!
//! Seven tests in three groups:
//!
//! - **Rendered envelope per role** (3 tests): coder with no
//!   drops, junior_researcher with visible floor drops,
//!   researcher with no drops. These pin the rendered-output
//!   structure (header line, parent chain, level breakdown,
//!   effective envelope, dropped block).
//!
//! - **Reachable role.switch targets** (3 tests): case 3
//!   (single qualified target for coder, pinning the PRODUCT.md
//!   P1.3 structural-impossibility guarantee), case 2
//!   (unqualified role.switch for default, expanding to all
//!   other roles), and case 1 / empty-child (researcher and
//!   junior_researcher both show "<none>" — the empty-child
//!   case pins that the floor substitution does not
//!   transitively leak `default`'s unqualified `role.switch`
//!   into the child).
//!
//! - **Unknown role error** (1 test): an unknown role name
//!   produces a typed error that echoes the input and lists the
//!   known roles, mirroring the load-time `UnknownRole` shape
//!   the `--role` flag already exposes.

use std::path::PathBuf;

use aivyx_channel::{render_role_envelope, ChannelKind};
use aivyx_config::{AivyxConfig, LoadOptions};

/// Load `examples/aivyx-pa.toml` via `aivyx_config::LoadOptions`.
/// Mirrors the binary-internal `load_example_config` helper but
/// runs from an integration-test crate's context — the
/// `CARGO_MANIFEST_DIR` anchor is still `crates/aivyx-channel/`,
/// so the relative `../../examples/aivyx-pa.toml` walk is
/// identical.
fn load_example_config() -> AivyxConfig {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let example_path = PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("examples")
        .join("aivyx-pa.toml");
    assert!(
        example_path.exists(),
        "examples/aivyx-pa.toml must exist at {example_path:?}"
    );
    let opts = LoadOptions {
        toml_path: Some(example_path),
        require_api_key: false,
        require_telegram_token: false,
        require_discord_token: false,
        require_slack_tokens: false,
        role_override: Some("default".to_string()),
    };
    AivyxConfig::load_from_env_and_toml(&opts)
        .expect("examples/aivyx-pa.toml must load cleanly via aivyx-config")
}

// --------------------------------------------------------------
// Rendered-envelope-per-role tests
// --------------------------------------------------------------

/// Render `coder` against `examples/aivyx-pa.toml`. Verifies the
/// structural elements (header, parent chain, level breakdown,
/// effective envelope) and pins that the documented seven
/// scopes each appear in the rendered output. Coder has no
/// dropped scopes, so the dropped block reads "<none - every
/// declared scope survived intersection>".
///
/// Phase 14 Task 2 widened the documented set from six to
/// seven: coder now declares `role.switch:researcher` as well,
/// which survives intersection with `default`'s unqualified
/// `role.switch` (Rule 2) and with CEILING_TRUSTED's
/// unqualified `role.switch` (also Rule 2).
#[test]
fn print_role_renders_coder_envelope_against_example_config() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("coder", &cfg, ChannelKind::Local)
        .expect("coder must render");

    assert!(rendered.contains("role: coder"), "header line: {rendered}");
    assert!(
        rendered.contains("parent chain: coder -> default"),
        "parent chain line: {rendered}"
    );
    assert!(
        rendered.contains("level 1 - coder (declared)"),
        "level header for coder: {rendered}"
    );
    assert!(
        rendered.contains("level 2 - default (declared)"),
        "level header for default: {rendered}"
    );
    assert!(
        rendered.contains("trust_ceiling: Trusted"),
        "trust ceiling label: {rendered}"
    );
    for scope in &[
        "fs.read",
        "fs.write",
        "memory.read",
        "memory.write",
        "memory.forget",
        "shell.exec",
        "role.switch:researcher",
    ] {
        assert!(
            rendered.contains(scope),
            "expected scope `{scope}` somewhere in render: {rendered}"
        );
    }
    assert!(
        rendered.contains("<none - every scope the active role declared survived intersection>"),
        "coder has no dropped scopes; render must say so: {rendered}"
    );
}

/// Render `junior_researcher`. This is the load-bearing test:
/// it pins that the rendered output mechanically surfaces the
/// "empty-child surprise" by showing both
/// (a) the empty `capability_scopes` line for the junior
/// level, and
/// (b) at least one *dropped* entry in the dropped section
/// sourced from the backcompat floor, because researcher
/// declares no `fs.write` and no `shell.exec` at all — so
/// those floor scopes cannot be granted upward and get
/// reported as drops. (Note: the floor's unqualified
/// `net.fetch` does NOT drop, because researcher's
/// `net.fetch:url-prefix:...` is granted by it via D4 Rule 2.
/// The surprise is specifically that fs.write and shell.exec
/// silently disappear, not net.fetch.) If a future refactor
/// makes `--print-role` claim "no dropped scopes" for a
/// junior_researcher run, the operator loses the only
/// surface that exposes the surprise *before* production.
#[test]
fn print_role_renders_junior_researcher_with_visible_drops() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("junior_researcher", &cfg, ChannelKind::Local)
        .expect("junior_researcher must render");

    assert!(
        rendered.contains("role: junior_researcher"),
        "header: {rendered}"
    );
    assert!(
        rendered.contains("parent chain: junior_researcher -> researcher -> default"),
        "three-level parent chain: {rendered}"
    );
    assert!(
        rendered.contains("<empty - backcompat floor will be substituted at runtime>"),
        "empty-capability-scopes line for junior level: {rendered}"
    );
    // The dropped block must mention at least one of the
    // surprises documented in `examples/aivyx-pa.toml`. We assert
    // the strongest signal: shell.exec from the floor gets
    // dropped (researcher does not declare it), with a reason
    // line that names the floor.
    assert!(
        rendered.contains("shell.exec [floor]"),
        "shell.exec must be reported as dropped from the floor: {rendered}"
    );
    // fs.write from the floor should also drop: researcher
    // declares no fs.write at all (not even qualified), so
    // the floor's `fs.write:<sandbox>/**` cannot be granted
    // upward through researcher's declared set.
    assert!(
        rendered.contains("fs.write") && rendered.contains("[floor]"),
        "fs.write (floor-qualified) must be reported as dropped from the floor: {rendered}"
    );
}

/// Render `researcher` and verify the dropped block stays
/// empty for it (researcher's declared scopes are all granted
/// by default and survive CEILING_TRUSTED intact). This test
/// exists as a *contrast* to the junior_researcher test: it
/// pins that a "cleanly attenuated" role does not produce
/// false-positive drop noise.
///
/// Phase 33 added `role.switch:junior_researcher` to
/// researcher's declared scopes — the second link in the
/// multi-level nesting chain.
#[test]
fn print_role_renders_researcher_with_no_drops() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("researcher", &cfg, ChannelKind::Local)
        .expect("researcher must render");

    assert!(rendered.contains("role: researcher"));
    assert!(rendered.contains("parent chain: researcher -> default"));
    assert!(
        rendered.contains("role.switch:junior_researcher"),
        "researcher must show role.switch:junior_researcher in effective \
         envelope (Phase 33): {rendered}"
    );
    assert!(
        rendered.contains("<none - every scope the active role declared survived intersection>"),
        "researcher's declared scopes all survive intersection; \
         the dropped block must say so explicitly to avoid false-positive \
         noise: {rendered}"
    );
}

// --------------------------------------------------------------
// Reachable role.switch targets tests
//
// These pin the three shapes the enumerator can produce and the
// structural-impossibility guarantee from PRODUCT.md P1.3: a
// leaf-declared `role.switch:<X>` that doesn't survive
// intersection must NOT show up as reachable.
// --------------------------------------------------------------

/// Case 3 (single qualified target) + structural impossibility.
///
/// `coder` in `examples/aivyx-pa.toml` declares
/// `role.switch:researcher` and inherits unqualified
/// `role.switch` from `default`. Intersection picks the
/// narrower form, so `coder`'s effective envelope holds
/// exactly `role.switch:researcher` — and the rendered
/// reachable-targets section must list `researcher` and
/// *only* `researcher`. Listing any other role would mean
/// `coder` could escape its declared qualifier — exactly
/// the escalation that P1.3 forbids.
#[test]
fn print_role_lists_role_switch_targets_for_coder() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("coder", &cfg, ChannelKind::Local)
        .expect("coder must render");

    assert!(
        rendered.contains("reachable role.switch targets"),
        "section header must be present: {rendered}"
    );
    // Find the section and assert content within it. We slice
    // from the section header to end-of-string so an unrelated
    // earlier mention of "researcher" (e.g. in the effective
    // envelope listing of `role.switch:researcher`) does not
    // satisfy the assertion.
    let section_start = rendered
        .find("reachable role.switch targets")
        .expect("section header must be present");
    let section = &rendered[section_start..];
    assert!(
        section.contains("\n  researcher\n"),
        "researcher must be listed as a reachable target on its own line: {section}"
    );
    // Structural impossibility: no other role names may appear
    // in the section, even though all of them exist in the
    // config.
    for forbidden in &["default", "coder", "junior_researcher"] {
        // Use a tight pattern that matches the indented bullet
        // form the enumerator emits, so we don't trip on the
        // word appearing inside an unrelated noun phrase.
        let pattern = format!("\n  {forbidden}");
        assert!(
            !section.contains(&pattern),
            "`{forbidden}` must NOT be listed as a reachable target for coder \
             (would violate PRODUCT.md P1.3 structural impossibility): {section}"
        );
    }
}

/// Case 2 (unqualified `role.switch` → any role).
///
/// `default` in `examples/aivyx-pa.toml` declares unqualified
/// `role.switch`. Under D4 Rule 2 the bare base grants any
/// qualifier, so the enumerator should emit the
/// "(any role - unqualified role.switch held)" line and list
/// every other role in the config. Pin both the line and the
/// presence of every non-self role name.
#[test]
fn print_role_lists_all_other_roles_when_unqualified_role_switch_held() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("default", &cfg, ChannelKind::Local)
        .expect("default must render");

    let section_start = rendered
        .find("reachable role.switch targets")
        .expect("section header must be present");
    let section = &rendered[section_start..];
    assert!(
        section.contains("(any role - unqualified role.switch held)"),
        "unqualified role.switch must trigger the any-role marker: {section}"
    );
    // Every other role name must appear as an indented bullet.
    // `default` itself must not, because the enumerator filters
    // out the active role from the list (a role switching to
    // itself is a no-op the dispatcher rejects).
    for other in &["coder", "researcher", "junior_researcher"] {
        let pattern = format!("\n    {other}\n");
        assert!(
            section.contains(&pattern),
            "`{other}` must be listed as a reachable target for default: {section}"
        );
    }
    let self_pattern = "\n    default\n";
    assert!(
        !section.contains(self_pattern),
        "default must not list itself as a reachable target: {section}"
    );
}

/// Case 3 for researcher (single qualified target).
///
/// Phase 33 added `role.switch:junior_researcher` to
/// `researcher`'s `capability_scopes`. The enumerator must
/// list `junior_researcher` as a reachable target and only
/// that target — mirroring the `coder` → `researcher` pattern
/// but one level deeper in the nesting chain.
#[test]
fn print_role_lists_junior_researcher_as_reachable_target_for_researcher() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("researcher", &cfg, ChannelKind::Local)
        .expect("researcher must render");

    let section_start = rendered
        .find("reachable role.switch targets")
        .expect("section header must be present");
    let section = &rendered[section_start..];
    assert!(
        section.contains("\n  junior_researcher\n"),
        "junior_researcher must be listed as a reachable target: {section}"
    );
    // Structural impossibility: no other role names may appear.
    for forbidden in &["default", "coder", "researcher"] {
        let pattern = format!("\n  {forbidden}");
        assert!(
            !section.contains(&pattern),
            "`{forbidden}` must NOT be listed as a reachable target for researcher \
             (would violate PRODUCT.md P1.3 structural impossibility): {section}"
        );
    }
}

/// `junior_researcher` is the empty-child case — its declared
/// `capability_scopes` is empty so the runtime substitutes the
/// backcompat floor for that level. The floor does NOT contain
/// `role.switch`, and `researcher` (the parent) does not
/// declare `role.switch` either, so the assembled envelope has
/// no `role.switch` scope. This test pins that the empty-child
/// substitution path also produces the case-1 "cannot start a
/// sub-session" output rather than silently inheriting
/// `default`'s unqualified `role.switch` through some
/// transitive accident.
#[test]
fn print_role_empty_child_does_not_inherit_role_switch_through_floor() {
    let cfg = load_example_config();
    let rendered = render_role_envelope("junior_researcher", &cfg, ChannelKind::Local)
        .expect("junior_researcher must render");

    let section_start = rendered
        .find("reachable role.switch targets")
        .expect("section header must be present");
    let section = &rendered[section_start..];
    assert!(
        section.contains("<none - this role cannot start a sub-session>"),
        "junior_researcher (empty child) must not transitively gain role.switch: {section}"
    );
}

// --------------------------------------------------------------
// Unknown role error
// --------------------------------------------------------------

/// Asking for an unknown role name produces a typed error
/// listing the known roles. Mirrors the load-time `UnknownRole`
/// behavior the `--role` flag already exposes.
#[test]
fn print_role_unknown_name_lists_known_roles_in_error() {
    let cfg = load_example_config();
    let err = render_role_envelope("nonsense", &cfg, ChannelKind::Local)
        .expect_err("unknown role must error");
    assert!(
        err.contains("nonsense"),
        "error must echo the requested name: {err}"
    );
    assert!(
        err.contains("coder"),
        "error must list known roles so the operator can spot a typo: {err}"
    );
}
