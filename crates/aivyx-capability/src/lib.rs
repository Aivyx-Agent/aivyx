//! # aivyx-capability
//!
//! Capability-based security for Aivyx. Defines the `Scope` type, the
//! `CapabilitySet`, the `TrustTier` enum, and the attenuation rules
//! that let trust tiers cap what an agent can do per turn.
//!
//! See DESIGN.md Deliverable 4 (capability taxonomy) and Deliverable 5
//! (trust tier model) for the locked design this crate implements.
//!
//! ## Key rule
//!
//! Effective capabilities for a turn are computed as:
//! `effective = agent.capabilities().intersect(tier.default_ceiling())`.
//! This intersection happens **once per turn**, before any LLM call.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Scope registry — the v1 active namespace (21 scopes, per D4).
// ---------------------------------------------------------------------------

/// The v1 active scope bases. `Scope::parse` rejects anything not in this
/// set, so unknown scopes fail at parse time, not check time.
///
/// Phase 11 Task 4 adds `tool.allowlist` as a **synthetic dispatch-layer
/// base**: the turn loop synthesizes `tool.allowlist:<tool_name>` scopes
/// to represent role-allowlist rejections and routes them through
/// `ToolOutcome::Denied { scope, held }` unchanged. Auditors distinguish
/// "capability denial" from "role-allowlist denial" by reading
/// `scope_requested.base()`. No `TrustTier` ceiling includes
/// `tool.allowlist` — it exists as a parseable label only; the scope
/// gate never holds it.
const KNOWN_BASES: &[&str] = &[
    // fs
    "fs.read",
    "fs.write",
    "fs.delete",
    "fs.metadata",
    // net
    "net.fetch",
    "net.post",
    "net.dns",
    // shell
    "shell.exec",
    "shell.spawn",
    // llm
    "llm.call",
    "llm.embed",
    // memory
    "memory.read",
    "memory.write",
    "memory.forget",
    // channel
    "channel.send",
    "channel.receive",
    // audit
    "audit.read",
    // config
    "config.read",
    "config.write",
    // role allowlist (synthetic — Phase 11 Task 4)
    "tool.allowlist",
];

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// A capability scope.
///
/// Hierarchical string form: `base` or `base:qualifier`. The base is a dotted
/// identifier drawn from `KNOWN_BASES`; the qualifier is an optional free-form
/// attenuation string whose semantics are determined by the *needed* scope's
/// shape at check time (see `QualifierKind`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope(String);

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Scope {
    /// Parse a scope string. Returns `None` if the base is not a known v1
    /// scope, per D4: "unknown scopes fail at parse time, not check time."
    pub fn parse(s: &str) -> Option<Self> {
        let base = match s.find(':') {
            Some(idx) => &s[..idx],
            None => s,
        };
        if !KNOWN_BASES.contains(&base) {
            return None;
        }
        Some(Scope(s.to_string()))
    }

    /// The base portion (everything before the first `:`, or the whole string).
    pub fn base(&self) -> &str {
        match self.0.find(':') {
            Some(idx) => &self.0[..idx],
            None => &self.0,
        }
    }

    /// The qualifier portion (everything after the first `:`), if any.
    pub fn qualifier(&self) -> Option<&str> {
        self.0.find(':').map(|idx| &self.0[idx + 1..])
    }

    /// The full string form, useful for display and error messages.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Prefix-attenuation check per D4 rules 1–4: `true` iff `self` (the
    /// needed scope) is granted by `other` (a held capability).
    pub fn is_granted_by(&self, other: &Scope) -> bool {
        // Rule 1: bases must match exactly.
        if self.base() != other.base() {
            return false;
        }

        match (self.qualifier(), other.qualifier()) {
            // Held unqualified grants anything with the same base (rule 2).
            (_, None) => true,

            // Rule 4: qualified held cannot grant unqualified needed.
            (None, Some(_)) => false,

            // Rule 3: both qualified — dispatch by the *needed* qualifier's
            // shape so the tool's declared intent drives matching semantics.
            // Exception: allowlists live on the *held* side by convention
            // (`shell.exec:git,ls,cat` grants `shell.exec:git`), so a comma
            // on either side flips us into allowlist mode.
            (Some(needed_q), Some(held_q)) => {
                QualifierKind::of(needed_q, held_q).check(needed_q, held_q)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// QualifierKind — how to interpret a qualifier string.
// ---------------------------------------------------------------------------

/// Determines how to compare a needed qualifier against a held one.
///
/// Dispatch order (per D4 rule 3 and the decision in PHASE_1.md):
/// URL → path → allowlist → simple-glob. Dispatch is driven by the *needed*
/// scope's qualifier shape so that the tool's declared intent determines
/// matching semantics.
#[derive(Debug, Clone, Copy)]
enum QualifierKind {
    /// Contains `://` — URL prefix match.
    UrlPrefix,
    /// Contains `/` (and no `://`) — glob semantics, `**` crosses `/`.
    PathGlob,
    /// Contains `,` — comma-separated allowlist; needed set must be a subset
    /// of held set.
    Allowlist,
    /// Otherwise — simple glob against the whole string.
    SimpleGlob,
}

impl QualifierKind {
    /// Classify the qualifier pair.
    ///
    /// Dispatch rules, in order:
    /// 1. `://` on the *needed* side → URL prefix (URLs are almost always
    ///    declared by the tool, not the capability set).
    /// 2. `/` on *either* side → path glob. Paths always win over allowlist
    ///    so that brace alternation like `/home/{julian,root}/**` is
    ///    correctly treated as a path despite containing a comma.
    /// 3. `,` on either side → allowlist (allowlists live on the held side
    ///    by convention: `shell.exec:git,ls,cat` grants `shell.exec:git`).
    /// 4. Otherwise → simple glob against the whole string.
    fn of(needed_q: &str, held_q: &str) -> Self {
        if needed_q.contains("://") {
            QualifierKind::UrlPrefix
        } else if needed_q.contains('/') || held_q.contains('/') {
            QualifierKind::PathGlob
        } else if needed_q.contains(',') || held_q.contains(',') {
            QualifierKind::Allowlist
        } else {
            QualifierKind::SimpleGlob
        }
    }

    /// Returns `true` iff `held_q` grants `needed_q` under this kind.
    fn check(self, needed_q: &str, held_q: &str) -> bool {
        match self {
            QualifierKind::UrlPrefix => needed_q.starts_with(held_q),
            QualifierKind::PathGlob => glob_matches(held_q, needed_q),
            QualifierKind::Allowlist => {
                let held: Vec<&str> = held_q.split(',').map(str::trim).collect();
                needed_q
                    .split(',')
                    .map(str::trim)
                    .all(|item| held.contains(&item))
            }
            QualifierKind::SimpleGlob => glob_matches(held_q, needed_q),
        }
    }
}

/// Compile `pattern` as a glob and test it against `candidate`. Returns
/// `false` if the pattern fails to compile — a malformed qualifier cannot
/// grant anything.
fn glob_matches(pattern: &str, candidate: &str) -> bool {
    match globset::Glob::new(pattern) {
        Ok(g) => g.compile_matcher().is_match(candidate),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// CapabilitySet
// ---------------------------------------------------------------------------

/// A set of held scopes.
///
/// Backed by a sorted, deduplicated `Vec<Scope>` so that serialization is
/// deterministic — required because `CapabilitySet` appears inside
/// `AuditEvent::TurnStarted`, and audit events are HMAC-chained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    scopes: Vec<Scope>,
}

impl CapabilitySet {
    pub fn empty() -> Self {
        CapabilitySet { scopes: Vec::new() }
    }

    pub fn from_scopes(scopes: impl IntoIterator<Item = Scope>) -> Self {
        let mut v: Vec<Scope> = scopes.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.dedup();
        CapabilitySet { scopes: v }
    }

    /// Returns `true` iff any held scope grants `needed` per D4 rules 1–4.
    pub fn grants(&self, needed: &Scope) -> bool {
        self.scopes.iter().any(|held| needed.is_granted_by(held))
    }

    /// Intersection — every scope in the result is granted by *both* sides.
    ///
    /// Used exactly once per turn to cap agent capabilities by the trust tier
    /// ceiling: `effective = agent_caps.intersect(tier.default_ceiling())`.
    pub fn intersect(&self, other: &CapabilitySet) -> CapabilitySet {
        let mut out: Vec<Scope> = Vec::new();
        for s in &self.scopes {
            if other.grants(s) {
                out.push(s.clone());
            }
        }
        for s in &other.scopes {
            if self.grants(s) && !out.contains(s) {
                out.push(s.clone());
            }
        }
        CapabilitySet::from_scopes(out)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.iter()
    }
}

// ---------------------------------------------------------------------------
// TrustTier
// ---------------------------------------------------------------------------

/// The four trust tiers, per D5.
///
/// **Variant order is ascending trust**, so derived `Ord` gives
/// `Kernel > Trusted > SemiTrusted > Untrusted`. The display-label numbers
/// (Tier 0 = Kernel, Tier 3 = Untrusted) run *opposite* to `Ord` — those
/// labels are a naming convention; the ordering used in code is `Ord`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum TrustTier {
    /// Tier 3 — Untrusted. Public webhooks, anonymous HTTP, unknown senders.
    Untrusted,
    /// Tier 2 — SemiTrusted. Authenticated user on a remote channel.
    SemiTrusted,
    /// Tier 1 — Trusted. Authenticated user on an owned channel.
    Trusted,
    /// Tier 0 — Kernel. Unconditional. Never assigned to a user-facing channel.
    Kernel,
}

impl TrustTier {
    /// The default capability ceiling for this tier, per the D5 table.
    pub fn default_ceiling(self) -> &'static CapabilitySet {
        match self {
            TrustTier::Kernel => &CEILING_KERNEL,
            TrustTier::Trusted => &CEILING_TRUSTED,
            TrustTier::SemiTrusted => &CEILING_SEMITRUSTED,
            TrustTier::Untrusted => &CEILING_UNTRUSTED,
        }
    }

    pub fn is_more_trusted_than(self, other: TrustTier) -> bool {
        self > other
    }
}

// ---------------------------------------------------------------------------
// Ceiling tables — one `LazyLock<CapabilitySet>` per tier, per D5.
// ---------------------------------------------------------------------------

/// Helper: parse a list of scope strings, panicking on unknown bases. Panic
/// here is correct — these are compile-time-authored constants from D5, and a
/// typo should fail loudly on first access, not silently drop a scope.
fn caps(scopes: &[&str]) -> CapabilitySet {
    CapabilitySet::from_scopes(
        scopes
            .iter()
            .map(|s| Scope::parse(s).expect("ceiling scope must be valid")),
    )
}

/// Tier 0 — Kernel. Unlimited: holds every v1 active scope unqualified.
/// Exists so internal operations pass capability checks without special-casing.
static CEILING_KERNEL: LazyLock<CapabilitySet> = LazyLock::new(|| {
    CapabilitySet::from_scopes(
        KNOWN_BASES
            .iter()
            .map(|b| Scope::parse(b).expect("known base must parse")),
    )
});

/// Tier 1 — Trusted. Near-total. Every v1 scope granted unqualified.
static CEILING_TRUSTED: LazyLock<CapabilitySet> = LazyLock::new(|| {
    caps(&[
        "fs.read",
        "fs.write",
        "fs.delete",
        "fs.metadata",
        "net.fetch",
        "net.post",
        "net.dns",
        "shell.exec",
        "shell.spawn",
        "llm.call",
        "llm.embed",
        "memory.read",
        "memory.write",
        "memory.forget",
        "channel.send",
        "channel.receive",
        "audit.read",
        "config.read",
        "config.write",
    ])
});

/// Tier 2 — SemiTrusted. Per D5 table: ▲ rows are omitted from the unqualified
/// ceiling; an agent holding the corresponding *qualified* scope will still
/// match via intersection, but holding bare `fs.write` will not.
///
/// ⊘ rows: fs.delete, shell.exec, shell.spawn, config.write
/// ▲ rows (omitted from unqualified ceiling): fs.read, fs.write, net.post,
/// memory.forget, channel.send, channel.receive, audit.read
static CEILING_SEMITRUSTED: LazyLock<CapabilitySet> = LazyLock::new(|| {
    caps(&[
        "fs.metadata",
        "net.fetch",
        "net.dns",
        "llm.call",
        "llm.embed",
        "memory.read",
        "memory.write",
        "config.read",
    ])
});

/// Tier 3 — Untrusted. Near-empty. The two ▲ rows (`memory.read:scope:public:*`,
/// `audit.read:public`) are not representable as an unqualified ceiling entry —
/// intersection with a held narrow qualified scope would not match since rule
/// 2 (unqualified held grants qualified needed) is what we want here. So we
/// grant the specific narrow qualified forms directly.
static CEILING_UNTRUSTED: LazyLock<CapabilitySet> = LazyLock::new(|| {
    caps(&["memory.read:scope:public:*", "audit.read:public"])
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Scope {
        Scope::parse(x).expect("test scope must parse")
    }

    // ---- Scope::parse ----

    #[test]
    fn parse_accepts_known_bare_base() {
        assert!(Scope::parse("fs.read").is_some());
        assert!(Scope::parse("memory.write").is_some());
    }

    #[test]
    fn parse_accepts_known_base_with_qualifier() {
        assert!(Scope::parse("fs.read:/home/julian/**").is_some());
        assert!(Scope::parse("net.fetch:https://example.com").is_some());
    }

    #[test]
    fn parse_rejects_unknown_base() {
        assert!(Scope::parse("nonsense.base").is_none());
        assert!(Scope::parse("display.window_close").is_none(),
                "Reserved scopes are not v1 active");
    }

    #[test]
    fn base_and_qualifier_split() {
        let sc = s("fs.read:/etc/*");
        assert_eq!(sc.base(), "fs.read");
        assert_eq!(sc.qualifier(), Some("/etc/*"));

        let bare = s("fs.read");
        assert_eq!(bare.base(), "fs.read");
        assert_eq!(bare.qualifier(), None);
    }

    // ---- Attenuation rule 1: bases must match ----

    #[test]
    fn rule1_base_mismatch_denies() {
        assert!(!s("fs.read").is_granted_by(&s("fs.write")));
        assert!(!s("fs.read:/foo").is_granted_by(&s("fs.write:/foo")));
    }

    // ---- Attenuation rule 2: unqualified held grants qualified needed ----

    #[test]
    fn rule2_unqualified_grants_qualified() {
        assert!(s("fs.read:/any/path").is_granted_by(&s("fs.read")));
        assert!(s("net.fetch:https://example.com").is_granted_by(&s("net.fetch")));
    }

    #[test]
    fn rule2_unqualified_grants_unqualified() {
        assert!(s("fs.read").is_granted_by(&s("fs.read")));
    }

    // ---- Attenuation rule 3: qualified held grants qualified needed per kind ----

    #[test]
    fn rule3_path_glob_match() {
        assert!(s("fs.read:/home/julian/docs/note.md")
            .is_granted_by(&s("fs.read:/home/julian/**")));
        assert!(!s("fs.read:/etc/passwd")
            .is_granted_by(&s("fs.read:/home/julian/**")));
    }

    #[test]
    fn rule3_url_prefix_match() {
        assert!(s("net.fetch:https://api.example.com/v1/users")
            .is_granted_by(&s("net.fetch:https://api.example.com/")));
        assert!(!s("net.fetch:https://evil.example.com/")
            .is_granted_by(&s("net.fetch:https://api.example.com/")));
    }

    #[test]
    fn rule3_allowlist_subset_match() {
        // Needed is a single item present in held list.
        assert!(s("shell.exec:git").is_granted_by(&s("shell.exec:git,ls,cat")));
        // Needed as multi-item subset.
        assert!(s("shell.exec:git,ls").is_granted_by(&s("shell.exec:git,ls,cat")));
        // Needed item not in held list.
        assert!(!s("shell.exec:rm").is_granted_by(&s("shell.exec:git,ls,cat")));
    }

    #[test]
    fn rule3_simple_glob_match() {
        // model-name style qualifier — no slashes, no commas, no ://
        assert!(s("llm.call:claude-opus-4-6")
            .is_granted_by(&s("llm.call:claude-*")));
        assert!(!s("llm.call:gpt-5").is_granted_by(&s("llm.call:claude-*")));
    }

    // ---- Dispatch-collision regressions (Phase 1 Q1) ----

    #[test]
    fn dispatch_path_with_brace_alternation_beats_comma() {
        // Held qualifier uses globset brace alternation — contains both `/`
        // and `,`. Must dispatch as PathGlob, not Allowlist. The needed
        // scope here deliberately has no `/` so the *held* side is what
        // forces the path classification.
        let needed = s("fs.read:julian");
        let held = s("fs.read:/home/{julian,root}/**");
        // The glob won't actually match "julian" — that's fine; what we're
        // testing is that we hit the PathGlob branch, not Allowlist, which
        // would otherwise do a string-subset check and return a nonsense
        // answer. We assert PathGlob semantics: non-match.
        assert!(!needed.is_granted_by(&held));

        // And the glob *does* match a full path.
        assert!(s("fs.read:/home/julian/notes.md").is_granted_by(&held));
        assert!(s("fs.read:/home/root/.bashrc").is_granted_by(&held));
        assert!(!s("fs.read:/etc/passwd").is_granted_by(&held));
    }

    #[test]
    fn dispatch_url_with_comma_in_query_stays_url() {
        // A URL with a comma in a query parameter must remain URL-dispatched.
        // `://` on the needed side wins before the comma check fires.
        let needed = s("net.fetch:https://api.example.com/search?tags=a,b,c");
        let held = s("net.fetch:https://api.example.com/");
        assert!(needed.is_granted_by(&held));
    }

    #[test]
    fn dispatch_colon_in_qualifier_falls_through_to_simple_glob() {
        // A qualifier like `session:abc` (the memory-recall case from D4)
        // contains neither `/`, `,`, nor `://`. Scope::parse splits on the
        // first `:` only, so the qualifier is literally `session:abc`.
        // Must dispatch as SimpleGlob.
        let sc = s("memory.read:session:abc");
        assert_eq!(sc.qualifier(), Some("session:abc"));
        // Exact-match held grants exact needed.
        assert!(sc.is_granted_by(&s("memory.read:session:abc")));
        // Wildcard on the session suffix matches too.
        assert!(sc.is_granted_by(&s("memory.read:session:*")));
        // Different session is denied.
        assert!(!sc.is_granted_by(&s("memory.read:session:xyz")));
    }

    // ---- Attenuation rule 4: qualified held does NOT grant unqualified needed ----

    #[test]
    fn rule4_qualified_does_not_grant_unqualified() {
        assert!(!s("fs.read").is_granted_by(&s("fs.read:/home/julian/**")));
        assert!(!s("shell.exec").is_granted_by(&s("shell.exec:git,ls")));
    }

    // ---- CapabilitySet::grants / intersect ----

    #[test]
    fn capset_grants_any_held_match() {
        let held = CapabilitySet::from_scopes([
            s("fs.read:/home/julian/**"),
            s("net.fetch:https://api.example.com/"),
        ]);
        assert!(held.grants(&s("fs.read:/home/julian/notes.md")));
        assert!(held.grants(&s("net.fetch:https://api.example.com/v1")));
        assert!(!held.grants(&s("fs.write:/home/julian/notes.md")));
    }

    #[test]
    fn capset_from_scopes_sorts_and_dedupes() {
        let set = CapabilitySet::from_scopes([
            s("net.fetch"),
            s("fs.read"),
            s("fs.read"),
            s("llm.call"),
        ]);
        let collected: Vec<&str> = set.iter().map(|sc| sc.0.as_str()).collect();
        assert_eq!(collected, vec!["fs.read", "llm.call", "net.fetch"]);
    }

    #[test]
    fn capset_intersect_keeps_mutually_granted() {
        let agent = CapabilitySet::from_scopes([
            s("fs.read:/home/julian/**"),
            s("shell.exec"),
            s("llm.call"),
        ]);
        let ceiling = CapabilitySet::from_scopes([s("fs.read"), s("llm.call")]);

        let eff = agent.intersect(&ceiling);
        // fs.read:/home/julian/** is granted by unqualified fs.read, keep it.
        assert!(eff.grants(&s("fs.read:/home/julian/notes.md")));
        // llm.call is in both.
        assert!(eff.grants(&s("llm.call")));
        // shell.exec is not in the ceiling — dropped.
        assert!(!eff.grants(&s("shell.exec")));
    }

    #[test]
    fn capset_intersect_is_deterministic() {
        let a = CapabilitySet::from_scopes([s("fs.read"), s("llm.call")]);
        let b = CapabilitySet::from_scopes([s("llm.call"), s("fs.read")]);
        // Different insertion order, same resulting Vec order.
        assert_eq!(a, b);
    }

    // ---- TrustTier ordering and ceilings ----

    #[test]
    fn trust_tier_ord_matches_d5_note() {
        assert!(TrustTier::Kernel > TrustTier::Trusted);
        assert!(TrustTier::Trusted > TrustTier::SemiTrusted);
        assert!(TrustTier::SemiTrusted > TrustTier::Untrusted);
    }

    #[test]
    fn trust_tier_is_more_trusted_than() {
        assert!(TrustTier::Kernel.is_more_trusted_than(TrustTier::Untrusted));
        assert!(!TrustTier::Untrusted.is_more_trusted_than(TrustTier::Kernel));
    }

    #[test]
    fn ceiling_kernel_grants_everything() {
        let kernel = TrustTier::Kernel.default_ceiling();
        for b in KNOWN_BASES {
            assert!(kernel.grants(&Scope::parse(b).unwrap()));
        }
    }

    #[test]
    fn ceiling_trusted_grants_shell_exec() {
        let trusted = TrustTier::Trusted.default_ceiling();
        assert!(trusted.grants(&s("shell.exec")));
        assert!(trusted.grants(&s("fs.delete:/tmp/scratch")));
    }

    #[test]
    fn ceiling_semitrusted_denies_shell_and_delete() {
        // Scenario 3 from D1: "Run rm -rf from Telegram"
        let semi = TrustTier::SemiTrusted.default_ceiling();
        assert!(!semi.grants(&s("shell.exec")));
        assert!(!semi.grants(&s("shell.exec:rm")));
        assert!(!semi.grants(&s("fs.delete:/home/julian/notes.md")));
        assert!(!semi.grants(&s("config.write")));
        // But memory.read is still fine.
        assert!(semi.grants(&s("memory.read")));
    }

    #[test]
    fn ceiling_semitrusted_requires_qualifier_for_triangle_rows() {
        // D5's ▲ semantic: holding *bare* fs.write does NOT satisfy a Tier 2
        // check, because rule 4 says qualified held can't grant unqualified
        // needed — and the ceiling at Tier 2 has no unqualified fs.write.
        let semi = TrustTier::SemiTrusted.default_ceiling();
        assert!(!semi.grants(&s("fs.write")));
        assert!(!semi.grants(&s("fs.read")));

        // But an agent holding `fs.write:/tmp/**` and intersected with the
        // Tier 2 ceiling gets nothing — ceiling has no fs.write at all.
        // The ▲ semantic is enforced by *building a narrower tool* (per the
        // D5 design principle), not by loosening the ceiling.
        let agent = CapabilitySet::from_scopes([s("fs.write:/tmp/**")]);
        let eff = agent.intersect(semi);
        assert!(!eff.grants(&s("fs.write:/tmp/x.txt")));
    }

    #[test]
    fn ceiling_untrusted_is_minimal() {
        let u = TrustTier::Untrusted.default_ceiling();
        assert!(!u.grants(&s("fs.read")));
        assert!(!u.grants(&s("shell.exec")));
        assert!(!u.grants(&s("memory.write")));
        // The one explicit narrow grant.
        assert!(u.grants(&s("memory.read:scope:public:feed")));
    }

    // ---- Phase 11 Task 4: synthetic `tool.allowlist` base ----

    #[test]
    fn tool_allowlist_parses_and_is_absent_from_real_ceilings() {
        // The Phase 11 Task 4 role-allowlist gate synthesizes
        // `tool.allowlist:<tool_name>` scopes at the dispatch layer
        // and routes them through `ToolOutcome::Denied { scope,
        // held }` so auditors can distinguish "capability denial"
        // from "role allowlist denial" by reading
        // `scope_requested.base()`. The base must parse cleanly,
        // and NO real-tier ceiling (Trusted, SemiTrusted,
        // Untrusted) may hold it — otherwise an audit event for a
        // role rejection would show a held set that contradicts
        // the denial.
        let synthetic = Scope::parse("tool.allowlist:shell.exec")
            .expect("tool.allowlist base must parse");
        assert_eq!(synthetic.base(), "tool.allowlist");
        assert_eq!(synthetic.qualifier(), Some("shell.exec"));

        let bare = s("tool.allowlist");
        assert!(
            !TrustTier::Trusted.default_ceiling().grants(&bare),
            "Trusted ceiling must NOT hold tool.allowlist — it's \
             a synthetic dispatch-layer base, not a real capability"
        );
        assert!(
            !TrustTier::SemiTrusted.default_ceiling().grants(&bare),
            "SemiTrusted ceiling must NOT hold tool.allowlist"
        );
        assert!(
            !TrustTier::Untrusted.default_ceiling().grants(&bare),
            "Untrusted ceiling must NOT hold tool.allowlist"
        );

        // Kernel DOES hold it unqualified — that's fine; kernel is
        // an internal-only tier, no real binary dispatches through
        // it. The `ceiling_kernel_grants_everything` test already
        // pins the "kernel holds every base" invariant.
        assert!(
            TrustTier::Kernel.default_ceiling().grants(&bare),
            "Kernel holds every KNOWN_BASES entry including the \
             synthetic one, per the ceiling-kernel-grants-everything \
             invariant"
        );
    }

    // ---- End-to-end: D1 scenario 3 ("rm -rf from Telegram") ----

    #[test]
    fn d1_scenario3_rm_rf_from_telegram_is_denied() {
        // Agent is "near-full-power" — holds shell.exec.
        let agent = CapabilitySet::from_scopes([
            s("shell.exec"),
            s("fs.read"),
            s("fs.write"),
            s("fs.delete"),
            s("llm.call"),
        ]);
        // Channel reports SemiTrusted (Telegram).
        let tier = TrustTier::SemiTrusted;
        let effective = agent.intersect(tier.default_ceiling());

        // The attempted tool call: shell.exec:rm
        let needed = s("shell.exec:rm");
        assert!(
            !effective.grants(&needed),
            "Tier 2 ceiling must deny shell.exec regardless of agent caps"
        );
    }
}
