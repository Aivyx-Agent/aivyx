//! `WebFetchTool` — Phase 12 Task 2, the first network-fetch tool.
//!
//! ## What it does
//!
//! Issues an HTTP GET against a user-supplied URL via
//! `reqwest::Client`, streams the response body through
//! `StreamEvent::ToolOutput` (Task 1's infra), and returns a
//! `{status, body}` pair to the planner. The agent must hold a
//! `net.fetch:<url-prefix>` scope; the capability layer's
//! Phase 12 Task 2 hardened `QualifierKind::UrlPrefix` matcher
//! enforces origin-exact + path-segment-boundary prefix
//! matching, so held `net.fetch:https://example.com/` does NOT
//! grant needed `net.fetch:https://example.com.evil.com/`.
//!
//! ## Design decisions pinned at Task 2 entry
//!
//! - **GET-only.** Phase 12 Q1 pinned this. There is no verb
//!   field in the schema; the tool unconditionally issues
//!   `client.get(url)`. Write verbs (POST/PUT/DELETE) are a
//!   later-phase concern that needs its own body-shape
//!   validator + capability base.
//! - **No redirects.** `reqwest::redirect::Policy::none()`.
//!   3xx responses surface as-is with `status` set to the
//!   3xx code and `body` empty (or whatever the origin sent).
//!   Following redirects without re-checking the scope on
//!   every hop is a privilege-escalation footgun — if the
//!   agent needs to follow a redirect, the planner issues a
//!   second tool call against the redirect target, and that
//!   call goes through the scope gate from scratch.
//! - **Hard body cap.** [`MAX_BODY_BYTES`] = 10 MiB. Exceeded
//!   bodies surface as `ToolOutcome::Failed` with a clear
//!   "body exceeded cap" detail — no silent truncation, no
//!   OOM risk, no "the LLM saw half a file" footgun.
//! - **Response headers audit-log-only.** Q3 pinned this. The
//!   tool's return value to the model is just
//!   `{status: u16, body: String}`. Headers are not in the
//!   return payload — the audit layer sees the
//!   `ToolCallFinished` with whatever the renderer produces,
//!   which for Task 2 is the same shape.
//! - **UTF-8 only.** The body is decoded via
//!   `String::from_utf8`. Non-UTF-8 responses fail with a
//!   clear detail. Phase 12's `StreamEvent::ToolOutput`
//!   variant carries `&str` (text-only), so a non-UTF-8
//!   body couldn't be streamed even if we wanted to.
//!
//! ## Defense in depth
//!
//! Two fences, same shape as `shell.exec`:
//!
//! 1. **Scope layer (`required_scope`).** Parses the input's
//!    `url` field and returns `net.fetch:<url>` as the needed
//!    scope. A malformed or missing URL emits
//!    [`deny_scope`]. The turn loop's scope gate checks this
//!    against the held capability set before `execute` runs,
//!    so a call against an out-of-scope URL never touches
//!    the network.
//! 2. **Pre-fetch re-check (`execute`).** `execute` re-parses
//!    the URL and emits `AivyxError::Tool` if it doesn't
//!    parse — the scope gate already verified scope, but the
//!    parse is cheap and guards against any future
//!    validator-drop regression that would let a malformed
//!    URL through.
//!
//! ## Trust-tier gate
//!
//! `net.fetch` is in **both** `CEILING_TRUSTED` and
//! `CEILING_SEMITRUSTED` (see `aivyx-capability`'s ceiling
//! tables). So unlike `shell.exec`, this tool is registered
//! for Local (Trusted) AND Telegram (SemiTrusted) channels —
//! a researcher agent attached to Telegram can fetch URLs.
//! The registration helper `build_web_fetch_for_channel`
//! returns the tool for both tiers and `None` for
//! `Untrusted`/`Kernel`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

use aivyx_capability::Scope;

use crate::{
    AivyxError, StreamEvent, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// Default wall-clock timeout for a single `web.fetch` invocation.
/// 30 seconds matches `shell.exec` and is generous enough for
/// most interactive research fetches. The `timeout_ms` input
/// override is clamped to [`MAX_TIMEOUT_MS`].
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Hard upper bound on a single `web.fetch` timeout. 10 minutes.
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// Hard upper bound on the response body size. 10 MiB. Bodies
/// larger than this surface as a `ToolOutcome::Failed` with a
/// clear detail rather than truncating or OOMing.
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Construction inputs for [`WebFetchTool`]. Splits the
/// fallible client build from the infallible tool construction,
/// matching `shell.exec`'s config→build split.
pub struct WebFetchToolConfig;

impl WebFetchToolConfig {
    pub fn new() -> Self {
        WebFetchToolConfig
    }

    /// Build a ready-to-register [`WebFetchTool`] with a
    /// redirect-free rustls-backed reqwest client. Fails if
    /// the client builder rejects our configuration — that
    /// would be a startup configuration error the operator
    /// needs to see immediately, not at tool-call time.
    pub fn build(self) -> Result<WebFetchTool, AivyxError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // Keep connect-time short so a dead host doesn't
            // eat the whole per-call timeout budget before
            // any bytes flow.
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| {
                AivyxError::Config(format!(
                    "web.fetch reqwest client build failed: {e}"
                ))
            })?;
        Ok(WebFetchTool {
            id: ToolId::new(),
            client: Arc::new(client),
            schema: web_fetch_input_schema_value(),
        })
    }
}

impl Default for WebFetchToolConfig {
    fn default() -> Self {
        WebFetchToolConfig::new()
    }
}

/// Reference HTTP-GET tool. Agents holding
/// `net.fetch:<origin>/<path>` (or `net.fetch` unqualified)
/// can fetch any URL under the granted prefix.
#[derive(Debug)]
pub struct WebFetchTool {
    id: ToolId,
    client: Arc<reqwest::Client>,
    schema: Value,
}

impl WebFetchTool {
    /// Test helper: build a tool with a caller-supplied client.
    /// Production code uses `WebFetchToolConfig::build`; tests
    /// can inject a pre-configured client.
    #[cfg(test)]
    pub(crate) fn from_client(client: reqwest::Client) -> Self {
        WebFetchTool {
            id: ToolId::new(),
            client: Arc::new(client),
            schema: web_fetch_input_schema_value(),
        }
    }
}

/// A scope no real agent should ever hold. Returned by
/// `required_scope` when the input is malformed or the URL
/// doesn't parse. Same shape as `tools::shell`'s deny helper.
fn deny_scope() -> Scope {
    Scope::parse("net.fetch:https://aivyx.invalid/__deny__/invalid-input")
        .expect("deny scope must parse")
}

/// Build the advertised flat input schema for `web.fetch`.
/// Exercises the already-shipped flat-object validator —
/// unlike `shell.exec`, there is no nested object and no
/// Phase 11 Task 3 nested-validator branch.
fn web_fetch_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Absolute http or https URL to GET. \
                                Must be covered by the agent's \
                                net.fetch capability. Redirects are \
                                not followed — a 3xx surfaces as-is."
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_TIMEOUT_MS as i64,
                "description": "Wall-clock timeout in milliseconds. \
                                Default 30000; maximum 600000 \
                                (10 minutes)."
            }
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

/// Pull `timeout_ms`, clamped to the hard ceiling. Missing
/// means [`DEFAULT_TIMEOUT_MS`].
fn input_timeout_ms(input: &Value) -> u64 {
    input
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1, MAX_TIMEOUT_MS)
}

#[async_trait]
impl Tool for WebFetchTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "web.fetch"
    }

    fn description(&self) -> &str {
        "Fetch an HTTP or HTTPS URL via GET and return its \
         status code and UTF-8 body. Redirects are not \
         followed. Response body is streamed to the user as it \
         arrives, subject to a 10 MiB hard cap. The agent must \
         hold a net.fetch capability that covers the requested \
         URL's origin and path prefix."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        let Some(url) = input.get("url").and_then(Value::as_str) else {
            return deny_scope();
        };
        if url.is_empty() {
            return deny_scope();
        }
        // The URL must parse as http/https and have an
        // authority. The capability layer's URL-prefix matcher
        // will re-parse the same string on the matching
        // path — keeping the two parsers aligned matters for
        // "deny the same things both sides would deny."
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return deny_scope();
        }
        Scope::parse(&format!("net.fetch:{url}")).unwrap_or_else(deny_scope)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        // ---- Re-parse input ---------------------------------------
        let url = match input.get("url").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "input must have a non-empty string `url` field"
                        .to_string(),
                });
            }
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("url must start with http:// or https:// (got {url:?})"),
            });
        }
        let timeout_ms = input_timeout_ms(&input);

        // ---- Issue GET ---------------------------------------------
        let request = self
            .client
            .get(&url)
            .timeout(Duration::from_millis(timeout_ms));

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("GET {url} failed: {e}"),
                });
            }
        };

        let status = response.status().as_u16();

        // ---- Stream body through StreamEvent::ToolOutput ----------
        //
        // Collect into a `Vec<u8>` as we go so we can also return
        // the body to the planner. Per-chunk UTF-8 decoding would
        // split multibyte characters at chunk boundaries, so we
        // collect into bytes and only decode/stream the parts that
        // are complete UTF-8 prefixes.
        let mut body_bytes: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("body stream error from {url}: {e}"),
                    });
                }
            };
            if body_bytes.len() + chunk.len() > MAX_BODY_BYTES {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "response body from {url} exceeded {} byte cap",
                        MAX_BODY_BYTES
                    ),
                });
            }
            body_bytes.extend_from_slice(&chunk);

            // Stream the new chunk if it's valid UTF-8. A
            // multi-byte character split across chunks will
            // read as invalid on the first chunk and valid on
            // the next — we keep it simple: if `str::from_utf8`
            // fails on the incremental chunk, we skip streaming
            // that chunk (the full body is decoded below) and
            // let the planner see the aggregated string.
            if let Ok(s) = std::str::from_utf8(&chunk) {
                let _ = ctx
                    .channel
                    .stream_event(StreamEvent::ToolOutput {
                        tool: self.id,
                        tool_name: "web.fetch",
                        chunk: s,
                    })
                    .await;
            }
        }

        let body = match String::from_utf8(body_bytes) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "response body from {url} is not valid UTF-8: {e}"
                    ),
                });
            }
        };

        ToolOutcome::Completed {
            output: json!({
                "url": url,
                "status": status,
                "body": body,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform,
        NullAuditHook, SessionId, TurnId, TurnOutcome,
    };
    use aivyx_capability::CapabilitySet;
    use std::sync::Mutex;

    // ---- Channel fake that captures streamed ToolOutput chunks ----

    struct CapturingChannel {
        session: SessionId,
        token: CancellationToken,
        chunks: Mutex<Vec<String>>,
    }

    impl CapturingChannel {
        fn new() -> Self {
            CapturingChannel {
                session: SessionId::new(),
                token: CancellationToken::new(),
                chunks: Mutex::new(Vec::new()),
            }
        }
        fn chunks(&self) -> Vec<String> {
            self.chunks.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelContext for CapturingChannel {
        fn channel_name(&self) -> &str {
            "test"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(
            &self,
            event: StreamEvent<'_>,
        ) -> Result<(), ChannelError> {
            if let StreamEvent::ToolOutput { chunk, .. } = event {
                self.chunks.lock().unwrap().push(chunk.to_string());
            }
            Ok(())
        }
        async fn finalize(
            &self,
            _outcome: &TurnOutcome,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    fn make_ctx<'a>(
        channel: &'a CapturingChannel,
        audit: &'a dyn crate::AuditHook,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: channel.session,
            turn_id: TurnId::new(),
            channel,
            audit,
            cancellation: &channel.token,
        }
    }

    fn build_tool() -> WebFetchTool {
        WebFetchToolConfig::new()
            .build()
            .expect("web.fetch tool builds with default config")
    }

    // ---- Scope derivation ------------------------------------------

    #[test]
    fn required_scope_uses_full_url_as_net_fetch_qualifier() {
        let tool = build_tool();
        let scope =
            tool.required_scope(&json!({"url": "https://httpbin.org/get"}));
        assert_eq!(scope.base(), "net.fetch");
        assert_eq!(scope.qualifier(), Some("https://httpbin.org/get"));
    }

    #[test]
    fn required_scope_for_missing_url_is_deny_scope() {
        let tool = build_tool();
        let scope = tool.required_scope(&json!({}));
        assert!(scope.qualifier().unwrap().contains("__deny__"));
    }

    #[test]
    fn required_scope_for_empty_url_is_deny_scope() {
        let tool = build_tool();
        let scope = tool.required_scope(&json!({"url": ""}));
        assert!(scope.qualifier().unwrap().contains("__deny__"));
    }

    #[test]
    fn required_scope_for_non_http_scheme_is_deny_scope() {
        // `file://` would otherwise pass the Scope::parse check
        // because `net.fetch` is a known base — but the
        // required_scope gate filters it out up front so an LLM
        // can't use web.fetch as a file-read bypass.
        let tool = build_tool();
        let scope = tool.required_scope(&json!({"url": "file:///etc/passwd"}));
        assert!(scope.qualifier().unwrap().contains("__deny__"));
    }

    #[test]
    fn required_scope_for_ftp_scheme_is_deny_scope() {
        let tool = build_tool();
        let scope =
            tool.required_scope(&json!({"url": "ftp://example.com/file"}));
        assert!(scope.qualifier().unwrap().contains("__deny__"));
    }

    // ---- Capability-layer integration anchor -----------------------

    #[test]
    fn held_origin_grants_needed_subpath() {
        // The interesting property: the tool's required_scope
        // output must actually be granted by a realistic held
        // capability. This is the regression that fires if the
        // scope-base string or the qualifier shape drifts.
        let tool = build_tool();
        let needed = tool.required_scope(&json!({
            "url": "https://httpbin.org/get"
        }));
        let held = CapabilitySet::from_scopes([
            Scope::parse("net.fetch:https://httpbin.org/").unwrap(),
        ]);
        assert!(held.grants(&needed), "origin held must grant subpath needed");
    }

    #[test]
    fn held_origin_does_not_grant_different_host() {
        let tool = build_tool();
        let needed = tool.required_scope(&json!({
            "url": "https://other.example.com/path"
        }));
        let held = CapabilitySet::from_scopes([
            Scope::parse("net.fetch:https://httpbin.org/").unwrap(),
        ]);
        assert!(
            !held.grants(&needed),
            "held httpbin.org must not grant other.example.com"
        );
    }

    // The hostile-suffix regression — critical security property.
    // Phase 12 Q4 resolution: raw-byte prefix is replaced with
    // origin-aware matching, so `example.com` does NOT admit
    // `example.com.evil.com` even though the needed string
    // literally starts with the held string.
    #[test]
    fn held_origin_does_not_grant_hostile_suffix_host() {
        let tool = build_tool();
        let needed = tool.required_scope(&json!({
            "url": "https://example.com.evil.com/login"
        }));
        let held = CapabilitySet::from_scopes([
            Scope::parse("net.fetch:https://example.com/").unwrap(),
        ]);
        assert!(
            !held.grants(&needed),
            "held example.com MUST NOT grant hostile-suffix example.com.evil.com"
        );
    }

    // ---- Schema ----------------------------------------------------

    #[test]
    fn input_schema_matches_flat_web_fetch_contract() {
        let tool = build_tool();
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["url"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["url"]["type"], "string");
        assert_eq!(
            schema["properties"]["timeout_ms"]["type"],
            "integer"
        );
    }

    // ---- Execution: error paths without network --------------------

    #[tokio::test]
    async fn execute_malformed_url_fails_with_tool_error() {
        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(json!({"url": "not-a-url"}), &ctx)
            .await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("http"),
                    "error should mention http scheme requirement: {detail}"
                );
            }
            other => panic!("expected Failed Tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_missing_url_fails_with_tool_error() {
        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool.execute(json!({}), &ctx).await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("url"),
                    "error should mention url field: {detail}"
                );
            }
            other => panic!("expected Failed Tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_unresolvable_host_surfaces_tool_error() {
        // `.invalid` is reserved by RFC 2606 and must never
        // resolve. A real fetch against it produces a connection
        // error; the tool must surface that as a clean
        // `ToolOutcome::Failed` rather than a panic.
        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": "https://aivyx.invalid/test",
                    "timeout_ms": 3000
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("aivyx.invalid"),
                    "error should mention the failing URL: {detail}"
                );
            }
            other => panic!("expected Failed Tool error, got {other:?}"),
        }
    }
}
