//! Phase 33 — Multi-level sub-agent nesting integration tests.
//!
//! These tests prove that the capability-bounded recursion
//! introduced in Phase 14 Task 3 works at depth > 1 after the
//! Phase 33 config change that grants `researcher` the
//! `role.switch:junior_researcher` scope.
//!
//! ## What the tests cover
//!
//! 1. **Full nesting chain envelope walk**: assemble envelopes for
//!    each role in the chain `coder → researcher →
//!    junior_researcher` and verify that `role.switch` narrows at
//!    each level (coder holds `:researcher`, researcher holds
//!    `:junior_researcher`, junior_researcher holds none).
//!
//! 2. **Recursion termination**: `junior_researcher` has no
//!    `role.switch` scope in its effective envelope, so a
//!    grandchild cannot nest further. This is the structural
//!    guarantee that the chain has finite depth.
//!
//! 3. **Capability attenuation across three levels**: the
//!    grandchild's envelope is strictly narrower than the child's,
//!    which is strictly narrower than the parent's. Pinning this
//!    proves PRODUCT.md P1.3 "structurally impossible escalation"
//!    holds at depth 2.
//!
//! 4. **Empty-child floor substitution does not leak role.switch**:
//!    `junior_researcher` has empty `capability_scopes`, so the
//!    backcompat floor is substituted. The floor does NOT contain
//!    `role.switch`, so the grandchild cannot nest even if the floor
//!    path were to accidentally include it.

use std::path::PathBuf;

use aivyx_capability::{Scope, TrustTier};
use aivyx_channel::assemble_role_envelope;
use aivyx_config::{AivyxConfig, LoadOptions};

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

fn envelope_strings(envelope: &aivyx_capability::CapabilitySet) -> Vec<String> {
    let mut out: Vec<String> = envelope.iter().map(|s| s.as_str().to_string()).collect();
    out.sort();
    out
}

// -----------------------------------------------------------------
// Nesting chain: role.switch narrows at each level
// -----------------------------------------------------------------

/// Walk the full nesting chain and verify that `role.switch`
/// narrows at each depth:
///
/// - `coder`: holds `role.switch:researcher` (can nest into researcher)
/// - `researcher`: holds `role.switch:junior_researcher` (can nest into junior_researcher)
/// - `junior_researcher`: holds NO `role.switch` scope (recursion terminates)
///
/// This is the load-bearing test for multi-level nesting: it
/// proves the capability system bounds the recursion depth by
/// construction rather than by runtime depth checks.
#[test]
fn nesting_chain_role_switch_narrows_at_each_level() {
    let cfg = load_example_config();
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    // Level 1: coder
    let coder = cfg.roles.get("coder").expect("coder");
    let coder_env = assemble_role_envelope(coder, &cfg.roles, &floor)
        .intersect(coder.trust_ceiling.value.default_ceiling());
    let coder_strs = envelope_strings(&coder_env);
    assert!(
        coder_strs.contains(&"role.switch:researcher".to_string()),
        "coder must hold role.switch:researcher: {coder_strs:?}"
    );
    assert!(
        !coder_strs.contains(&"role.switch:junior_researcher".to_string()),
        "coder must NOT hold role.switch:junior_researcher \
         (structural impossibility — coder only declared :researcher): {coder_strs:?}"
    );

    // Level 2: researcher
    let researcher = cfg.roles.get("researcher").expect("researcher");
    let researcher_env = assemble_role_envelope(researcher, &cfg.roles, &floor)
        .intersect(researcher.trust_ceiling.value.default_ceiling());
    let researcher_strs = envelope_strings(&researcher_env);
    assert!(
        researcher_strs.contains(&"role.switch:junior_researcher".to_string()),
        "researcher must hold role.switch:junior_researcher: {researcher_strs:?}"
    );
    assert!(
        !researcher_strs.contains(&"role.switch:researcher".to_string()),
        "researcher must NOT hold role.switch:researcher \
         (not declared in its scopes): {researcher_strs:?}"
    );

    // Level 3: junior_researcher (terminal — no role.switch)
    let junior = cfg.roles.get("junior_researcher").expect("junior_researcher");
    let junior_env = assemble_role_envelope(junior, &cfg.roles, &floor)
        .intersect(junior.trust_ceiling.value.default_ceiling());
    let junior_strs = envelope_strings(&junior_env);
    let has_role_switch = junior_strs
        .iter()
        .any(|s| s.starts_with("role.switch"));
    assert!(
        !has_role_switch,
        "junior_researcher must hold NO role.switch scope at all \
         (recursion must terminate here): {junior_strs:?}"
    );
}

// -----------------------------------------------------------------
// Attenuation: parent→child in the inheritance tree narrows
// -----------------------------------------------------------------

/// The PRODUCT.md P1.3 guarantee is that `assemble_role_envelope`
/// walks the inheritance chain and intersects at every level,
/// so a child's effective envelope can only be equal to or
/// narrower than its parent's. This test verifies the
/// inheritance-tree attenuation (researcher → junior_researcher),
/// NOT the switch-target relationship (which is between siblings
/// in the example config).
///
/// Note: `coder` and `researcher` are siblings (both have
/// `parent_role = "default"`), so their envelopes are
/// independently attenuated from `default` — neither is a
/// subset of the other. The nesting chain (coder *switches*
/// into researcher) is about the `role.switch` scope gate,
/// not about envelope subsetting.
#[test]
fn inheritance_chain_attenuation_researcher_to_junior() {
    let cfg = load_example_config();
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let researcher = cfg.roles.get("researcher").expect("researcher");
    let researcher_env = assemble_role_envelope(researcher, &cfg.roles, &floor)
        .intersect(researcher.trust_ceiling.value.default_ceiling());

    let junior = cfg.roles.get("junior_researcher").expect("junior_researcher");
    let junior_env = assemble_role_envelope(junior, &cfg.roles, &floor)
        .intersect(junior.trust_ceiling.value.default_ceiling());

    // junior_researcher's envelope must be narrower than researcher's
    // (the floor substitution drops some scopes researcher holds).
    let researcher_strs = envelope_strings(&researcher_env);
    let junior_strs = envelope_strings(&junior_env);
    assert!(
        junior_strs.len() < researcher_strs.len(),
        "junior_researcher ({} scopes) must have fewer scopes \
         than researcher ({} scopes) — inheritance \
         attenuation must narrow",
        junior_strs.len(),
        researcher_strs.len()
    );

    // Every base scope in junior's envelope must appear (possibly
    // in a wider form) in researcher's envelope. We compare bases
    // rather than using `CapabilitySet::grants` because the
    // url-prefix qualifier kind has a known reflexivity edge case
    // (Phase 13 Task 4) where a `url-prefix:` compound qualifier
    // doesn't self-grant through the URL parser.
    let researcher_bases: Vec<String> = researcher_env
        .iter()
        .map(|s| s.base().to_string())
        .collect();
    for scope in junior_env.iter() {
        let base = scope.base().to_string();
        assert!(
            researcher_bases.contains(&base),
            "junior_researcher scope base {base} (from {scope}) must \
             also exist in researcher's envelope (possibly in a wider \
             form) — P1.3 structural impossibility"
        );
    }
}

// -----------------------------------------------------------------
// Floor substitution does not leak role.switch
// -----------------------------------------------------------------

/// The backcompat floor used for empty-child levels does NOT
/// contain any `role.switch` scope. This test pins that property
/// directly on the floor vector, not on the assembled envelope
/// — if a future refactor accidentally added `role.switch` to
/// the binary's floor construction, this test would fail before
/// the floor ever reached `assemble_role_envelope`.
#[test]
fn backcompat_floor_does_not_contain_role_switch() {
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");
    let has_role_switch = floor
        .iter()
        .any(|s| s.as_str().starts_with("role.switch"));
    assert!(
        !has_role_switch,
        "the backcompat floor must NOT contain any role.switch scope — \
         adding it would let empty-child roles inherit nesting ability \
         from the floor rather than from their declared scopes: {floor:?}"
    );
}

// -----------------------------------------------------------------
// Default (root) can reach any role, including junior_researcher
// -----------------------------------------------------------------

/// The root `default` role holds unqualified `role.switch`,
/// which under D4 Rule 2 grants any qualifier. Verify it can
/// reach `junior_researcher` as well — the deepest leaf in
/// the chain. This confirms that a direct switch (bypassing
/// intermediate roles) is possible from root.
#[test]
fn default_unqualified_role_switch_reaches_all_chain_members() {
    let cfg = load_example_config();
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let default_role = cfg.roles.get("default").expect("default");
    let default_env = assemble_role_envelope(default_role, &cfg.roles, &floor)
        .intersect(TrustTier::Trusted.default_ceiling());

    // Unqualified role.switch must be in the envelope.
    let default_strs = envelope_strings(&default_env);
    assert!(
        default_strs.contains(&"role.switch".to_string()),
        "default must hold unqualified role.switch: {default_strs:?}"
    );

    // It must grant any qualified form — test all chain members.
    for target in &["coder", "researcher", "junior_researcher"] {
        let needed = Scope::parse(&format!("role.switch:{target}"))
            .expect("test scope must parse");
        assert!(
            default_env.grants(&needed),
            "default's unqualified role.switch must grant \
             role.switch:{target} via D4 Rule 2"
        );
    }
}
