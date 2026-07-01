//! Chapter Rampart — an egress guard for the network tools.
//!
//! Ward ([`crate::sensitive_paths`]) closed the *read* half of exfiltration;
//! this closes the *send* half's sharpest, default-safe edge. Two concerns:
//!
//! 1. **SSRF / private-network reach (default-on).** A network-capable agent —
//!    especially one steered by injected web/email content — fetching
//!    `http://169.254.169.254/…` (the AWS/GCP/Azure metadata endpoint →
//!    cloud-credential theft), `http://localhost:7843/…` (local services,
//!    including Aivyx's own daemon), or an RFC-1918 LAN address is a real
//!    exfiltration / pivot vector that neither `fs_root` nor the read guard
//!    touches. Public-web research uses public hostnames, so refusing
//!    loopback / link-local / private / unique-local targets by default costs
//!    normal use nothing.
//! 2. **Host allow-list (opt-in).** `[access] allow_egress_hosts` restricts the
//!    network tools to named hosts — a hard gate (unlike the model-cooperative
//!    `confirm_destructive`) for the privacy-conscious operator.
//!
//! Applied to the initial URL AND every redirect hop of `web.fetch` /
//! `web.extract` / `web.post`, so a public URL that 3xx-redirects to
//! `169.254.169.254` is caught too.
//!
//! ## Honest scope
//!
//! This checks the URL's **host literal**. A public hostname that *resolves*
//! to a private address (DNS rebinding, or attacker-controlled DNS) is not
//! caught here — robust defense resolves and re-checks the connected IP, a
//! documented follow-on. It stops the direct-address and localhost cases,
//! which are the common metadata/local-pivot attacks.

use std::net::IpAddr;

/// The egress policy: the default-on SSRF guard plus an optional operator
/// host allow-list. Cheap to clone; the network tools hold it behind `Arc`.
#[derive(Debug, Clone)]
pub struct EgressPolicy {
    /// When true (the default), refuse loopback / link-local / private /
    /// unique-local targets. `[access] allow_private_egress = true` flips it
    /// off (for operators who genuinely want the agent to reach localhost/LAN).
    block_private: bool,
    /// When non-empty, ONLY these hosts (exact or a dot-suffix subdomain
    /// match) are reachable. Empty ⇒ any public host.
    allow_hosts: Vec<String>,
}

impl Default for EgressPolicy {
    /// The default posture: SSRF guard on, no host restriction.
    fn default() -> Self {
        EgressPolicy { block_private: true, allow_hosts: Vec::new() }
    }
}

impl EgressPolicy {
    pub fn new(block_private: bool, allow_hosts: Vec<String>) -> Self {
        // Normalize allow-list to lowercase for case-insensitive host match.
        let allow_hosts =
            allow_hosts.into_iter().map(|h| h.trim().to_ascii_lowercase()).collect();
        EgressPolicy { block_private, allow_hosts }
    }

    /// A fully-permissive policy (SSRF guard off, no allow-list) — the escape
    /// hatch when the operator sets `allow_private_egress` and no host list.
    pub fn permissive() -> Self {
        EgressPolicy { block_private: false, allow_hosts: Vec::new() }
    }

    /// Classify a URL. `Some(reason)` ⇒ the request must be refused. Pure.
    pub fn classify(&self, url: &str) -> Option<String> {
        let host = host_of(url)?;
        let host_l = host.to_ascii_lowercase();

        if self.block_private {
            // Literal IP → range checks.
            if let Ok(ip) = host_l.parse::<IpAddr>() {
                if is_blocked_ip(&ip) {
                    return Some(format!(
                        "target {host} is a private/loopback/link-local address \
                         (blocked to prevent SSRF + cloud-metadata theft)"
                    ));
                }
            } else if is_local_hostname(&host_l) {
                return Some(format!(
                    "target {host} is a local hostname (blocked; set \
                     `[access] allow_private_egress` to permit localhost/LAN)"
                ));
            }
        }

        if !self.allow_hosts.is_empty() && !self.host_allowed(&host_l) {
            return Some(format!(
                "target {host} is not in `[access] allow_egress_hosts`"
            ));
        }
        None
    }

    /// Exact host match, or a dot-boundary subdomain of an allowed host
    /// (`api.github.com` is allowed by `github.com`, but `evilgithub.com`
    /// is not).
    fn host_allowed(&self, host_l: &str) -> bool {
        self.allow_hosts.iter().any(|a| {
            host_l == a || host_l.ends_with(&format!(".{a}"))
        })
    }
}

/// Extract the bare host from an `http(s)://` URL — no scheme, no userinfo, no
/// port, IPv6 brackets stripped. `None` if there's no authority.
fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1)?;
    let authority = after.split(['/', '?', '#']).next()?;
    // Drop any `userinfo@`.
    let hostport = authority.rsplit('@').next()?;
    if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal `[::1]:port`.
        let end = rest.find(']')?;
        return Some(rest[..end].to_string());
    }
    let host = hostport.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Filter resolved socket addresses down to those safe to connect to,
/// dropping any that land on a blocked (private/loopback/link-local) IP.
/// This is the TOCTOU-safe half of the SSRF guard: a custom reqwest DNS
/// resolver (in `tools::web_fetch`) runs every hostname through this, so a
/// public name that *resolves* to `127.0.0.1` / `169.254.169.254` / an
/// RFC-1918 address (DNS rebinding) is never connected to — reqwest only ever
/// sees the vetted addresses. Pure + testable.
pub(crate) fn filter_public_addrs(
    addrs: impl Iterator<Item = std::net::SocketAddr>,
) -> Vec<std::net::SocketAddr> {
    addrs.filter(|a| !is_blocked_ip(&a.ip())).collect()
}

/// Whether an IP is one the egress guard blocks by default: loopback,
/// link-local (incl. the `169.254.169.254` cloud-metadata IP), private, or
/// IPv6 unique-local (`fc00::/7`).
pub(crate) fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // Unique-local fc00::/7.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Hostnames that name the local machine.
fn is_local_hostname(host_l: &str) -> bool {
    host_l == "localhost"
        || host_l.ends_with(".localhost")
        || host_l == "ip6-localhost"
        || host_l == "localhost.localdomain"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_metadata_and_local_targets_by_default() {
        let p = EgressPolicy::default();
        for url in [
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/",
            "http://127.0.0.1:7843/api",
            "http://localhost:8080/",
            "https://LocalHost/x",
            "http://10.1.2.3/internal",
            "http://192.168.1.1/",
            "http://172.16.0.9/",
            "http://[::1]:9000/",
            "http://0.0.0.0/",
        ] {
            assert!(p.classify(url).is_some(), "should block {url}");
        }
    }

    #[test]
    fn allows_public_hosts_by_default() {
        let p = EgressPolicy::default();
        for url in [
            "https://example.com/page",
            "https://api.github.com/repos/x/y",
            "http://93.184.216.34/", // a public literal IP
        ] {
            assert!(p.classify(url).is_none(), "should allow {url}");
        }
    }

    #[test]
    fn allow_private_flag_permits_localhost() {
        let p = EgressPolicy::new(false, Vec::new());
        assert!(p.classify("http://localhost:7843/").is_none());
        assert!(p.classify("http://127.0.0.1/").is_none());
    }

    #[test]
    fn host_allowlist_restricts_to_named_hosts_and_subdomains() {
        let p = EgressPolicy::new(true, vec!["github.com".into(), "example.com".into()]);
        assert!(p.classify("https://github.com/x").is_none());
        assert!(p.classify("https://api.github.com/x").is_none()); // subdomain ok
        assert!(p.classify("https://example.com/y").is_none());
        // Not in the list → blocked.
        assert!(p.classify("https://evil.com/x").is_some());
        // Look-alike must not match by suffix trick.
        assert!(p.classify("https://evilgithub.com/x").is_some());
        // Allow-list still layered under the SSRF guard.
        assert!(p.classify("http://127.0.0.1/").is_some());
    }

    #[test]
    fn filter_public_addrs_drops_private_and_keeps_public() {
        use std::net::SocketAddr;
        let addrs: Vec<SocketAddr> = [
            "127.0.0.1:80",
            "169.254.169.254:80",
            "10.0.0.5:80",
            "93.184.216.34:80", // public
            "[::1]:80",
        ]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
        let kept = filter_public_addrs(addrs.into_iter());
        assert_eq!(kept.len(), 1, "only the public address survives");
        assert_eq!(kept[0].ip().to_string(), "93.184.216.34");
    }

    #[test]
    fn host_extraction_handles_userinfo_ports_ipv6() {
        assert_eq!(host_of("https://user:pw@example.com:443/p").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[2606:2800:220:1::]:80/").as_deref(), Some("2606:2800:220:1::"));
        assert_eq!(host_of("https://example.com").as_deref(), Some("example.com"));
        assert_eq!(host_of("not a url"), None);
    }
}
