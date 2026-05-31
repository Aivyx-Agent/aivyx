//! Operator-supplied OAuth client configuration.
//!
//! Phase 123 Task 2 — Q1a Recommended (operator-provided OAuth
//! app). The operator creates an OAuth client in their own
//! Google Cloud project, configures the Calendar API + consent
//! screen, and pastes `client_id` + `client_secret` into the
//! tool-process config file (default location:
//! `~/.aivyx/tool-processes/calendar/config.toml`).
//!
//! No serde shape pinned in Task 2 — the file-format wiring
//! lands in Task 3 (`aivyx-calendar auth init`). Task 2 ships the
//! in-memory primitive so the OAuth substrate compiles and
//! tests independently.

use serde::{Deserialize, Serialize};

/// Default Calendar OAuth scopes covering the Task 4-8
/// tool surface (5 tools — Q3b operator-picked).
///
/// - `auth/calendar` — full read+write across all
///   calendars accessible to the authenticated user.
///   Covers `list_events`, `get_event`, `create_event`,
///   `update_event`, `delete_event` against any calendar
///   the operator has access to.
///
/// Per the Phase 128 sign-off scope notes, the default
/// is the BROAD `auth/calendar` rather than the narrower
/// `auth/calendar.events` (events-only) — fewer "re-auth
/// with new scope" loops for operators. Operators who
/// want a narrower posture can supply
/// `["https://www.googleapis.com/auth/calendar.events"]`
/// (or `auth/calendar.events.readonly` for a read-only
/// tool surface) via the `scopes` field in their config
/// file. The tools whose scopes weren't granted will
/// fail at the first call with a clear error.
pub const DEFAULT_CALENDAR_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
];

/// Operator-provided OAuth client configuration.
///
/// **Lifecycle:** loaded once at tool-process startup from
/// `~/.aivyx/tool-processes/calendar/config.toml`; passed by-value
/// to [`crate::oauth::exchange::exchange_code`] +
/// [`crate::oauth::exchange::refresh_access_token`] as needed.
///
/// **Privacy posture:** `client_secret` is sensitive but not
/// cryptographically valuable on its own — without `client_id`
/// it's useless, and Google's OAuth flow binds the secret to
/// the redirect URI registered against the OAuth app. Still
/// stored in a 0600 file alongside the tokens; not logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// Google Cloud OAuth 2.0 client ID. Format:
    /// `XXXXXX-XXXXXXXX.apps.googleusercontent.com`.
    pub client_id: String,
    /// Google Cloud OAuth 2.0 client secret. Bound to
    /// `redirect_uri`.
    pub client_secret: String,
    /// Redirect URI registered against the OAuth app in Google
    /// Cloud Console. For Aivyx-local use this is always a
    /// loopback URI like `http://127.0.0.1:<port>/callback`
    /// (Task 3 spins up the local listener on this port).
    /// Operators may also use `urn:ietf:wg:oauth:2.0:oob` for
    /// headless setups but it's deprecated by Google.
    pub redirect_uri: String,
    /// OAuth scopes to request at consent time. Defaults to
    /// [`DEFAULT_CALENDAR_SCOPES`]; operators may narrow.
    #[serde(default = "default_scopes_owned")]
    pub scopes: Vec<String>,
}

fn default_scopes_owned() -> Vec<String> {
    DEFAULT_CALENDAR_SCOPES.iter().map(|s| (*s).to_string()).collect()
}

impl OAuthConfig {
    /// Build an OAuthConfig with the default scope set.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            scopes: default_scopes_owned(),
        }
    }

    /// Replace the default scope set with operator-narrowed scopes.
    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Render the scope set as a space-delimited string, as
    /// required by both Google's authorization endpoint and the
    /// token endpoint.
    pub fn scopes_space_delimited(&self) -> String {
        self.scopes.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_populates_default_scopes() {
        let cfg = OAuthConfig::new("id", "secret", "http://127.0.0.1:1234/cb");
        assert_eq!(cfg.client_id, "id");
        assert_eq!(cfg.client_secret, "secret");
        assert_eq!(cfg.redirect_uri, "http://127.0.0.1:1234/cb");
        assert_eq!(cfg.scopes.len(), DEFAULT_CALENDAR_SCOPES.len());
    }

    #[test]
    fn with_scopes_narrows() {
        let cfg = OAuthConfig::new("id", "secret", "http://localhost:0/cb")
            .with_scopes([
                "https://www.googleapis.com/auth/calendar.readonly",
            ]);
        assert_eq!(cfg.scopes.len(), 1);
        assert!(cfg.scopes[0].ends_with("calendar.readonly"));
    }

    #[test]
    fn scopes_space_delimited_joins_with_single_space() {
        // Two-scope smoke test — the default is one scope
        // so we narrow to verify the join logic.
        let cfg = OAuthConfig::new("id", "secret", "http://localhost:0/cb")
            .with_scopes([
                "https://www.googleapis.com/auth/calendar",
                "https://www.googleapis.com/auth/calendar.events",
            ]);
        let joined = cfg.scopes_space_delimited();
        assert_eq!(joined.matches(' ').count(), 1);
        for scope in &cfg.scopes {
            assert!(joined.contains(scope), "missing {scope}");
        }
    }

    #[test]
    fn default_scopes_contain_the_broad_calendar_scope() {
        // Per the Phase 128 sign-off scope notes, the
        // default is the BROAD `auth/calendar` scope.
        assert!(
            DEFAULT_CALENDAR_SCOPES.contains(&"https://www.googleapis.com/auth/calendar"),
            "broad `auth/calendar` scope must be in the default set"
        );
    }

    #[test]
    fn config_serde_roundtrips_through_toml_shape() {
        // The config file will be TOML in Task 3; verify the
        // serde derive accepts a JSON shape that matches what
        // TOML deserialization will produce (TOML maps cleanly
        // to JSON for our flat field set).
        let cfg = OAuthConfig::new("id-x", "secret-x", "http://127.0.0.1:8765/cb");
        let json = serde_json::to_value(&cfg).unwrap();
        let back: OAuthConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn config_deserialize_without_scopes_uses_default() {
        // Operator omits scopes field → default scope set kicks in.
        let json = serde_json::json!({
            "client_id": "id",
            "client_secret": "secret",
            "redirect_uri": "http://127.0.0.1:0/cb",
        });
        let cfg: OAuthConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.scopes.len(), DEFAULT_CALENDAR_SCOPES.len());
    }
}
