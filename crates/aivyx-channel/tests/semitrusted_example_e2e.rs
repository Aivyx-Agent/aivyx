//! Phase 15 Task 4 — cross-crate integration test for the
//! SemiTrusted worked example (`examples/aivyx-semitrusted.toml`).
//!
//! ## Why this file exists
//!
//! `examples/aivyx-pa.toml` teaches the **Trusted-tier** attenuation
//! story (D4 Rule 2: an unqualified-held parent scope grants a
//! qualified-needed child scope). That example deliberately keeps
//! every descendant at `Trusted` so `CEILING_TRUSTED`'s permissive
//! shape does not interfere with the inheritance teaching point.
//!
//! This file is the companion that teaches the **SemiTrusted-tier
//! ▲-row footgun**: what happens to roles that declare `fs.read` /
//! `fs.write` (bare or qualified) when they run under the
//! `CEILING_SEMITRUSTED` intersection. The D5 table in
//! `crates/aivyx-capability/src/lib.rs` omits the entire
//! `fs.read`, `fs.write`, `net.post`, `memory.forget`,
//! `channel.send`, `channel.receive`, and `audit.read` **bases**
//! from the SemiTrusted ceiling (▲ rows). The practical
//! consequence — pinned mechanically below — is that *any* scope
//! with one of those bases vanishes under intersection, whether
//! the role declared it unqualified or path-qualified. There is
//! no "qualified rescue path" for those bases at SemiTrusted.
//!
//! This is a surprising result for an operator coming from the
//! Trusted story, where `fs.read:/tmp/notes/**` would survive
//! under `fs.read` in the parent and `fs.read` in the ceiling via
//! D4 Rule 2. At SemiTrusted the ceiling's *base* is missing, so
//! D4 Rule 1 ("bases must match exactly") short-circuits the
//! check before the qualifier rules even run. Pinning this
//! mechanically means that if a future ceiling rewrite ever adds
//! `fs.read` back to `CEILING_SEMITRUSTED` (or changes its base
//! set at all), the test fails loud — and the operator who reads
//! the example learns the real rule, not the one they expected.
//!
//! ## Relationship to `examples/aivyx-pa.toml`'s tests
//!
//! `role_envelope_e2e.rs` and `role_render_e2e.rs` pin the
//! Trusted-tier story's envelopes. This file pins the
//! SemiTrusted-tier story's envelopes against a dedicated,
//! minimally-named config so the two stories do not shadow each
//! other in a single file. The two examples together cover the
//! two directions an operator's mental model can fail: over-
//! permissive parent inheritance (Trusted) and over-restrictive
//! ceiling intersection (SemiTrusted).

use std::path::PathBuf;

use aivyx_capability::{Scope, TrustTier};
use aivyx_channel::assemble_role_envelope;
use aivyx_config::{AivyxConfig, LoadOptions};

/// Build the same Local-channel backcompat floor shape the
/// binary uses at startup. The integration test does not touch
/// the filesystem — the floor is handed to
/// `assemble_role_envelope` purely to match the runtime shape
/// the production binary would see.
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

/// Load `examples/aivyx-semitrusted.toml` via the same
/// `LoadOptions` shape the Trusted example test uses. The file
/// is designed to load without real secrets, so
/// `require_api_key: false` and `require_telegram_token: false`
/// let it parse cleanly. `role_override` is set to `"default"`
/// only to satisfy the loader's active-role pick; each test re-
/// resolves whichever role it cares about out of `cfg.roles`.
fn load_semitrusted_example_config() -> AivyxConfig {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let example_path = PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("examples")
        .join("aivyx-semitrusted.toml");
    assert!(
        example_path.exists(),
        "examples/aivyx-semitrusted.toml must exist at {example_path:?}"
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
        .expect("examples/aivyx-semitrusted.toml must load cleanly via aivyx-config")
}

/// Collect an envelope's scopes as a sorted `Vec<String>` for
/// order-independent comparison, same helper shape as
/// `role_envelope_e2e.rs`.
fn envelope_strings(envelope: &aivyx_capability::CapabilitySet) -> Vec<String> {
    let mut out: Vec<String> = envelope.iter().map(|s| s.as_str().to_string()).collect();
    out.sort();
    out
}

/// `default` is the Trusted root of the SemiTrusted example
/// tree. Its declared scopes include unqualified `fs.read` and
/// `fs.write`, which survive `CEILING_TRUSTED` because the
/// Trusted ceiling holds both bases unqualified. This test is
/// the *contrast baseline*: it pins that the parent is fine, so
/// the surprises the child tests below assert are specifically
/// the SemiTrusted ceiling's doing — not an upstream bug in the
/// walker.
#[test]
fn semitrusted_example_default_root_envelope_matches_declared_set() {
    let cfg = load_semitrusted_example_config();
    let default_role = cfg
        .roles
        .get("default")
        .expect("default root declared in semitrusted example");
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
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "default root (Trusted) must keep all six declared scopes: \
         the Trusted ceiling holds fs.read, fs.write, memory.forget, \
         memory.read, memory.write, net.fetch unqualified, so every \
         declared scope survives intersection. This is the baseline \
         the SemiTrusted child tests contrast against."
    );
}

/// `telegram_researcher` is the "declared-carefully" SemiTrusted
/// role. It declares path-qualified `fs.read:/tmp/notes/**` and
/// `fs.write:/tmp/notes/**` — the shape an operator coming from
/// the Trusted example would expect to survive.
///
/// The pinned surprise: it does **not**. `CEILING_SEMITRUSTED`
/// omits both `fs.read` and `fs.write` as ▲-row bases. D4 Rule 1
/// ("bases must match exactly") short-circuits the check — there
/// is no ceiling scope with base `fs.read`, so the qualified
/// form has nothing to intersect against. `fs.metadata` is a
/// *different base*, not a parent of `fs.read`, so it does not
/// rescue the scope either.
///
/// What does survive:
/// - `memory.read`, `memory.write` — both unqualified in the
///   ceiling, granted by the role's declared unqualified forms.
/// - `net.fetch:url-prefix:https://en.wikipedia.org/` — the
///   ceiling holds unqualified `net.fetch`, which grants the
///   qualified form via D4 Rule 2.
///
/// The test therefore pins the "right-way" role to exactly three
/// scopes, and the example file's teaching comment must reflect
/// that the real fix for fs access at SemiTrusted is to raise
/// the ceiling (via an operator override) rather than to rely on
/// path qualification alone.
#[test]
fn semitrusted_example_telegram_researcher_envelope_demonstrates_base_absence() {
    let cfg = load_semitrusted_example_config();
    let researcher = cfg
        .roles
        .get("telegram_researcher")
        .expect("telegram_researcher role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(researcher, &cfg.roles, &floor);
    let effective = envelope.intersect(TrustTier::SemiTrusted.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "memory.read".to_string(),
        "memory.write".to_string(),
        "net.fetch:url-prefix:https://en.wikipedia.org/".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "telegram_researcher (SemiTrusted) must drop both \
         fs.read:/tmp/notes/** and fs.write:/tmp/notes/** because \
         CEILING_SEMITRUSTED has no fs.read/fs.write base at all — \
         D4 Rule 1 short-circuits before the qualifier rules run. \
         Only the memory/net scopes whose bases exist unqualified in \
         the ceiling survive. If a future ceiling edit ever adds \
         fs.read or fs.write back to CEILING_SEMITRUSTED, this test \
         breaks loud — by design."
    );
}

/// `telegram_footgun` is the "declared-naively" SemiTrusted
/// role. It declares bare unqualified `fs.read` and `fs.write` —
/// the exact same shape that works perfectly at `Trusted`, but
/// which disappears at `SemiTrusted` for the same base-absence
/// reason pinned in the previous test.
///
/// The effective envelope is exactly the same three non-fs
/// scopes `telegram_researcher` ended up with, *minus* the
/// net.fetch (which this role never declared). So the test pins
/// the two roles to different envelopes to drive home the point
/// that the path-qualification discipline *does* matter — not
/// for fs (those fail either way here), but for net.fetch (which
/// only survives if declared at all).
#[test]
fn semitrusted_example_telegram_footgun_envelope_drops_unqualified_fs() {
    let cfg = load_semitrusted_example_config();
    let footgun = cfg
        .roles
        .get("telegram_footgun")
        .expect("telegram_footgun role declared");
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let envelope = assemble_role_envelope(footgun, &cfg.roles, &floor);
    let effective = envelope.intersect(TrustTier::SemiTrusted.default_ceiling());

    let got = envelope_strings(&effective);
    let mut expected = vec![
        "memory.read".to_string(),
        "memory.write".to_string(),
    ];
    expected.sort();
    assert_eq!(
        got, expected,
        "telegram_footgun (SemiTrusted) must have only memory.read \
         and memory.write in its effective envelope: the unqualified \
         fs.read and fs.write vanish because CEILING_SEMITRUSTED \
         omits both bases entirely (D5 ▲ rows). The operator's \
         declared filesystem surface is *gone* — exactly the \
         footgun the example is teaching."
    );
}

/// Cross-check: the two SemiTrusted roles produce different
/// effective envelopes even though both lose their filesystem
/// surface. `telegram_researcher` keeps its narrow web-fetch
/// allowance (because its `net.fetch:url-prefix:...` scope
/// survives under the ceiling's unqualified `net.fetch`);
/// `telegram_footgun` has no net.fetch at all. Pinning this
/// divergence makes it mechanically obvious that declaring
/// scopes *still matters* at SemiTrusted — just not in the way
/// an operator migrating from the Trusted example would expect.
#[test]
fn semitrusted_example_researcher_and_footgun_diverge_on_net_fetch() {
    let cfg = load_semitrusted_example_config();
    let researcher = cfg.roles.get("telegram_researcher").unwrap();
    let footgun = cfg.roles.get("telegram_footgun").unwrap();
    let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

    let researcher_env = assemble_role_envelope(researcher, &cfg.roles, &floor)
        .intersect(TrustTier::SemiTrusted.default_ceiling());
    let footgun_env = assemble_role_envelope(footgun, &cfg.roles, &floor)
        .intersect(TrustTier::SemiTrusted.default_ceiling());

    let researcher_strs = envelope_strings(&researcher_env);
    let footgun_strs = envelope_strings(&footgun_env);

    assert_ne!(
        researcher_strs, footgun_strs,
        "researcher and footgun must diverge: researcher still holds \
         the url-prefix net.fetch it explicitly declared, footgun \
         declared none: researcher={researcher_strs:?}, \
         footgun={footgun_strs:?}"
    );
    assert!(
        researcher_strs.contains(&"net.fetch:url-prefix:https://en.wikipedia.org/".to_string()),
        "researcher must retain the qualified net.fetch: {researcher_strs:?}"
    );
    assert!(
        !footgun_strs
            .iter()
            .any(|s| s.starts_with("net.fetch")),
        "footgun must hold no net.fetch scope at all: {footgun_strs:?}"
    );
}
