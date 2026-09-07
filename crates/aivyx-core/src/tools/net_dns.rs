//! Phase 109 — `net.dns` substrate tool. Closes one of Phase
//! 100's eight declared-but-toolless scopes.
//!
//! The `net.dns` scope base has been in `KNOWN_BASES` since
//! Phase 0; Phase 109 ships the tool that exercises it. No
//! amendment needed for the scope base itself — the only
//! contract change is A12's count update (10 → 13 substrate
//! tools) which acknowledges this addition alongside the two
//! `git.read` tools.
//!
//! ## What this tool does
//!
//! Resolves an operator-supplied hostname to one or more IP
//! addresses via `tokio::net::lookup_host`. Returns a JSON
//! object with a `host` field echoing the input and an
//! `addresses` array carrying the resolved strings.
//!
//! ## Why a separate tool and not folded into `web.fetch`
//!
//! `web.fetch` resolves DNS internally as part of its HTTP
//! GET, but the result is invisible to the agent — there's no
//! way to ask "what does host X resolve to?" without making a
//! full HTTP round-trip. DNS resolution is also a legitimate
//! diagnostic primitive on its own (does this hostname
//! exist? what IPs serve it?) that a code/ops agent benefits
//! from without paying the HTTP cost.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use aivyx_capability::Scope;

/// `net.dns` — resolve a hostname to one or more IP addresses.
#[derive(Debug)]
pub struct NetDnsTool {
    id: ToolId,
    schema: Value,
    /// Chapter Rampart — egress policy. A DNS lookup is itself an exfil channel
    /// (`<secret>.attacker.com` leaks to the attacker's nameserver), so net.dns
    /// honors the same allow-list / private-block as the web tools. Unset ⇒
    /// permissive (byte-identical; the binary installs the policy).
    egress: OnceLock<Arc<crate::egress::EgressPolicy>>,
}

impl Default for NetDnsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl NetDnsTool {
    pub fn new() -> Self {
        NetDnsTool {
            id: ToolId::new(),
            schema: input_schema(),
            egress: OnceLock::new(),
        }
    }

    /// Chapter Rampart — install the egress policy (unset ⇒ permissive).
    pub fn set_egress_policy(
        &self,
        policy: Arc<crate::egress::EgressPolicy>,
    ) -> Result<(), Arc<crate::egress::EgressPolicy>> {
        self.egress.set(policy)
    }
}

#[async_trait]
impl Tool for NetDnsTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "net.dns"
    }

    fn description(&self) -> &str {
        "Resolve a hostname to one or more IP addresses. Input \
         is a JSON object with a `host` field (the hostname, \
         without scheme, without port — just the bare host like \
         `example.com`). Returns a JSON object with `host` \
         (echoed) and `addresses` (array of resolved IP \
         strings)."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match validate_host(input) {
            Some(host) => Scope::parse(&format!("net.dns:{host}")).unwrap_or_else(deny_scope),
            None => deny_scope(),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let host = match validate_host(&input) {
            Some(h) => h,
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "net.dns: `host` field missing or malformed (must be a \
                             plain hostname — no scheme, no port, no slash, no `..`)"
                        .to_string(),
                });
            }
        };

        // Chapter Rampart — refuse a lookup the egress policy blocks (an
        // allow-listed deployment can't be DNS-tunneled; localhost/private
        // hostnames are refused too).
        if let Some(policy) = self.egress.get() {
            if let Some(reason) = policy.classify_host(&host) {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("net.dns: refusing to resolve {host} — {reason}."),
                });
            }
        }

        // tokio::net::lookup_host wants a `host:port` form. We
        // append `:0` because the port is irrelevant to the
        // resolution; the iterator yields SocketAddr entries
        // whose .ip() carries the address regardless of port.
        let lookup_target = format!("{host}:0");
        let addrs: Vec<String> = match tokio::net::lookup_host(lookup_target).await {
            Ok(iter) => {
                let mut seen: Vec<String> = Vec::new();
                for sock in iter {
                    let s = sock.ip().to_string();
                    if !seen.contains(&s) {
                        seen.push(s);
                    }
                }
                seen
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("net.dns: lookup failed for {host:?}: {e}"),
                });
            }
        };

        ToolOutcome::Completed {
            output: json!({ "host": host, "addresses": addrs }),
            verified: Verification::Verified,
        }
    }
}

/// Validate that `host` is a plain hostname — no scheme prefix,
/// no port, no slash, no `..`, not empty. Returns the cleaned
/// host string if valid, `None` otherwise. The scope qualifier
/// is built from this string, so a strict validator here is
/// what keeps the scope-grant boundary honest.
fn validate_host(input: &Value) -> Option<String> {
    let raw = input.get("host").and_then(|v| v.as_str())?.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.contains("://") {
        return None;
    }
    if raw.contains('/') {
        return None;
    }
    if raw.contains(':') {
        return None;
    }
    if raw.contains("..") {
        return None;
    }
    // Allow IDN punycode (xn--) and standard hostname chars
    // (letters, digits, dots, hyphens). Reject other ASCII
    // control / punctuation. UTF-8 hostnames pre-IDN-encoding
    // are accepted too — `lookup_host` handles them via the
    // underlying resolver's IDNA rules.
    for ch in raw.chars() {
        if ch.is_ascii_control() || matches!(ch, ' ' | '"' | '\'' | '\\' | '?' | '#' | '@') {
            return None;
        }
    }
    Some(raw.to_string())
}

fn deny_scope() -> Scope {
    Scope::parse("net.dns:__aivyx_unresolvable__")
        .expect("net.dns:__aivyx_unresolvable__ must parse — net.dns is in KNOWN_BASES")
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Plain hostname (no scheme, no port, no slash)."
            }
        },
        "required": ["host"]
    })
}

// Suppress unused-import warning on `Arc` — pulled in by the
// `Tool` trait bound chain in the sibling modules' convention.
#[allow(dead_code)]
fn _unused_arc_ref<T>(_: &Arc<T>) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod net_dns_tests {
    use super::*;

    #[test]
    fn validate_host_accepts_typical_hostnames() {
        assert_eq!(
            validate_host(&json!({ "host": "example.com" })),
            Some("example.com".to_string())
        );
        assert_eq!(
            validate_host(&json!({ "host": "sub.domain.example.org" })),
            Some("sub.domain.example.org".to_string())
        );
        assert_eq!(
            validate_host(&json!({ "host": "xn--mxa.com" })), // IDN punycode
            Some("xn--mxa.com".to_string())
        );
    }

    #[test]
    fn validate_host_accepts_localhost() {
        assert_eq!(
            validate_host(&json!({ "host": "localhost" })),
            Some("localhost".to_string())
        );
    }

    #[test]
    fn validate_host_trims_whitespace() {
        assert_eq!(
            validate_host(&json!({ "host": "  example.com  " })),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn validate_host_rejects_missing_field() {
        assert!(validate_host(&json!({})).is_none());
    }

    #[test]
    fn validate_host_rejects_empty_string() {
        assert!(validate_host(&json!({ "host": "" })).is_none());
    }

    #[test]
    fn validate_host_rejects_scheme_prefix() {
        assert!(validate_host(&json!({ "host": "https://example.com" })).is_none());
        assert!(validate_host(&json!({ "host": "http://example.com" })).is_none());
    }

    #[test]
    fn validate_host_rejects_port() {
        assert!(validate_host(&json!({ "host": "example.com:80" })).is_none());
    }

    #[test]
    fn validate_host_rejects_slash() {
        assert!(validate_host(&json!({ "host": "example.com/path" })).is_none());
    }

    #[test]
    fn validate_host_rejects_dotdot() {
        assert!(validate_host(&json!({ "host": "..evil.com" })).is_none());
    }

    #[test]
    fn validate_host_rejects_control_chars() {
        assert!(validate_host(&json!({ "host": "evil\nhost.com" })).is_none());
        assert!(validate_host(&json!({ "host": "evil host.com" })).is_none());
    }

    #[test]
    fn deny_scope_parses_and_is_unsatisfiable() {
        let scope = deny_scope();
        assert_eq!(scope.base(), "net.dns");
        let real = Scope::parse("net.dns:example.com").unwrap();
        assert!(!scope.is_granted_by(&real));
    }

    #[test]
    fn tool_metadata_is_stable() {
        let t = NetDnsTool::new();
        assert_eq!(t.name(), "net.dns");
        assert!(!t.description().is_empty());
        let schema = t.input_schema();
        assert_eq!(schema["required"][0], "host");
    }

    #[test]
    fn required_scope_builds_from_validated_host() {
        let t = NetDnsTool::new();
        let scope = t.required_scope(&json!({ "host": "example.com" }));
        assert_eq!(scope.as_str(), "net.dns:example.com");
    }

    #[test]
    fn required_scope_returns_deny_on_malformed_input() {
        let t = NetDnsTool::new();
        let scope = t.required_scope(&json!({ "host": "https://evil.com" }));
        assert_eq!(scope.base(), "net.dns");
        // Confirms the qualifier is the deny sentinel, not the
        // raw input — a regression here would let an agent
        // smuggle a scheme into the scope grant.
        assert_eq!(scope.qualifier(), Some("__aivyx_unresolvable__"));
    }
}
