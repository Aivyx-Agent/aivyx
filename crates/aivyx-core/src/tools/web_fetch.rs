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
//! - **Response headers: content-type surfaced.** Phase 12 Q3
//!   originally pinned headers as audit-log-only. Phase 31
//!   Task 2 adds `content_type` to the return payload so the
//!   model can distinguish JSON from HTML from plain text.
//!   Other headers remain audit-only.
//! - **Binary fallback.** Phase 37 replaces the UTF-8-only
//!   gate with a base64 fallback: `String::from_utf8`
//!   success → `body_encoding: "utf-8"`, failure →
//!   `body_encoding: "base64"` with the raw bytes
//!   base64-encoded. `StreamEvent::ToolOutput` streaming is
//!   skipped for binary responses (the chunk type is `&str`).
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

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use dom_smoothie::Readability;
use futures_util::StreamExt;
use serde_json::{json, Value};

use aivyx_capability::{CapabilitySet, Scope};

use crate::egress::EgressPolicy;
use crate::{
    AivyxError, StreamEvent, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// Chapter Rampart — refuse a URL the egress policy blocks (SSRF /
/// private-network guard + opt-in host allow-list). Shared by all three
/// network tools; checked on the initial URL AND every redirect hop. An
/// unset policy (`OnceLock` empty, e.g. in tests) is permissive.
fn egress_refusal(
    tool: ToolId,
    egress: &OnceLock<Arc<EgressPolicy>>,
    url: &str,
) -> Option<ToolOutcome> {
    let reason = egress.get()?.classify(url)?;
    Some(ToolOutcome::Failed(AivyxError::Tool {
        tool,
        detail: format!("refusing to reach {url} — {reason}."),
    }))
}

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

/// Build the shared redirect-free, rustls-backed reqwest client used by every
/// `aivyx-core` web tool (`web.fetch`, `web.extract`). Redirects are off so the
/// scope gate is re-checked per hop by the caller; a short connect timeout keeps
/// a dead host from eating the whole per-call budget. `label` names the tool in
/// the error so a startup misconfig is attributable.
fn build_redirect_free_client(label: &str) -> Result<reqwest::Client, AivyxError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AivyxError::Config(format!("{label} reqwest client build failed: {e}")))
}

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
        let client = build_redirect_free_client("web.fetch")?;
        Ok(WebFetchTool {
            id: ToolId::new(),
            client: Arc::new(client),
            schema: web_fetch_input_schema_value(),
            effective_caps: OnceLock::new(),
            egress: OnceLock::new(),
        })
    }
}

impl Default for WebFetchToolConfig {
    fn default() -> Self {
        WebFetchToolConfig::new()
    }
}

/// Maximum number of redirect hops before the loop gives up.
const MAX_REDIRECT_HOPS: usize = 10;

/// Reference HTTP-GET tool. Agents holding
/// `net.fetch:<origin>/<path>` (or `net.fetch` unqualified)
/// can fetch any URL under the granted prefix.
pub struct WebFetchTool {
    id: ToolId,
    client: Arc<reqwest::Client>,
    schema: Value,
    /// Effective capabilities for per-hop redirect scope checks.
    /// Set once at startup via `set_effective_capabilities`.
    /// When absent, redirects are always denied (safe default).
    effective_caps: OnceLock<CapabilitySet>,
    /// Chapter Rampart — egress policy (unset ⇒ permissive; the binary sets it).
    egress: OnceLock<Arc<EgressPolicy>>,
}

impl std::fmt::Debug for WebFetchTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebFetchTool")
            .field("id", &self.id)
            .field("has_effective_caps", &self.effective_caps.get().is_some())
            .finish()
    }
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
            effective_caps: OnceLock::new(),
            egress: OnceLock::new(),
        }
    }

    /// Chapter Rampart — install the egress policy (SSRF guard + host
    /// allow-list). Called once at startup; unset ⇒ permissive.
    pub fn set_egress_policy(
        &self,
        policy: Arc<EgressPolicy>,
    ) -> Result<(), Arc<EgressPolicy>> {
        self.egress.set(policy)
    }

    /// Install the effective capability set for per-hop redirect
    /// scope checks. Called once at startup after
    /// `assemble_role_envelope` completes. Takes `&self` because
    /// the tool is already inside an `Arc` at the call site.
    /// Returns `Err(caps)` if capabilities were already set.
    pub fn set_effective_capabilities(
        &self,
        caps: CapabilitySet,
    ) -> Result<(), CapabilitySet> {
        self.effective_caps.set(caps)
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
            },
            "follow_redirects": {
                "type": "boolean",
                "description": "Follow 3xx redirects up to 10 hops. \
                                Each hop re-checks the redirect \
                                URL against the agent's net.fetch \
                                scope. Default: false (3xx surfaces \
                                as-is)."
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

// ---------------------------------------------------------------------------
// Shared redirect + body helpers (used by both WebFetchTool and WebPostTool)
// ---------------------------------------------------------------------------

/// Extract a valid absolute HTTP(S) URL from a 3xx response's
/// `Location` header. Returns `None` if the header is missing,
/// unparseable, or uses a non-HTTP scheme. Resolves relative
/// `Location` values against `base_url`.
fn extract_redirect_location(
    response: &reqwest::Response,
    base_url: &str,
) -> Option<String> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)?
        .to_str()
        .ok()?;

    // Handle relative URLs by resolving against the current URL
    let resolved = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_string()
    } else if location.starts_with('/') {
        // Absolute path — resolve against origin
        let url = reqwest::Url::parse(base_url).ok()?;
        let origin = url.origin().unicode_serialization();
        format!("{origin}{location}")
    } else {
        // Relative path — punt, treat as opaque
        return None;
    };

    // Must be http/https
    if !(resolved.starts_with("http://") || resolved.starts_with("https://")) {
        return None;
    }

    Some(resolved)
}

/// Check that a redirect target URL is within the agent's
/// effective capabilities for the given scope base. Returns
/// `Ok(())` if granted, or `Err(ToolOutcome)` with a clear
/// denial message if the redirect is out of scope or if no
/// capabilities have been installed.
fn check_redirect_scope(
    tool_id: ToolId,
    redirect_url: &str,
    scope_base: &str,
    effective_caps: &OnceLock<CapabilitySet>,
) -> Result<(), ToolOutcome> {
    let needed = match Scope::parse(&format!("{scope_base}:{redirect_url}")) {
        Some(s) => s,
        None => {
            return Err(ToolOutcome::Completed {
                output: json!({
                    "url": redirect_url,
                    "error": "redirect URL failed scope parse",
                    "body": "",
                    "body_encoding": "utf-8",
                }),
                verified: Verification::NotApplicable,
            });
        }
    };

    let caps = match effective_caps.get() {
        Some(c) => c,
        None => {
            // No capabilities installed — deny redirect as a
            // safe default. The tool still works for non-redirect
            // requests; this path is only hit when
            // follow_redirects=true and the binary didn't wire up
            // set_effective_capabilities (test scenarios, or a
            // future binary refactor that misses the wiring).
            return Err(ToolOutcome::Failed(AivyxError::Tool {
                tool: tool_id,
                detail: format!(
                    "redirect to {redirect_url} denied: no effective \
                     capabilities installed for redirect scope checks"
                ),
            }));
        }
    };

    if !caps.grants(&needed) {
        return Err(ToolOutcome::Completed {
            output: json!({
                "url": redirect_url,
                "error": format!(
                    "redirect to {redirect_url} denied by {scope_base} scope"
                ),
                "body": "",
                "body_encoding": "utf-8",
            }),
            verified: Verification::NotApplicable,
        });
    }

    Ok(())
}

/// Collect the response body, streaming UTF-8 chunks through
/// `StreamEvent::ToolOutput` and applying the base64 fallback
/// for binary content. Returns `(body_string, encoding)` on
/// success or `ToolOutcome::Failed` on error.
async fn collect_body(
    response: reqwest::Response,
    tool_id: ToolId,
    tool_name: &str,
    url: &str,
    ctx: &ToolContext<'_>,
) -> Result<(String, &'static str), ToolOutcome> {
    let mut body_bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(b) => b,
            Err(e) => {
                return Err(ToolOutcome::Failed(AivyxError::Tool {
                    tool: tool_id,
                    detail: format!("body stream error from {url}: {e}"),
                }));
            }
        };
        if body_bytes.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ToolOutcome::Failed(AivyxError::Tool {
                tool: tool_id,
                detail: format!(
                    "response body from {url} exceeded {} byte cap",
                    MAX_BODY_BYTES
                ),
            }));
        }
        body_bytes.extend_from_slice(&chunk);

        if let Ok(s) = std::str::from_utf8(&chunk) {
            let _ = ctx
                .channel
                .stream_event(StreamEvent::ToolOutput {
                    tool: tool_id,
                    tool_name,
                    chunk: s,
                })
                .await;
        }
    }

    let (body, encoding) = match String::from_utf8(body_bytes) {
        Ok(s) => (s, "utf-8"),
        Err(e) => {
            let raw = e.into_bytes();
            let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
            (encoded, "base64")
        }
    };

    Ok((body, encoding))
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
         status code and body. UTF-8 bodies are streamed to \
         the user as they arrive; non-UTF-8 (binary) bodies \
         are returned base64-encoded with body_encoding set \
         to \"base64\". Set follow_redirects to true to \
         follow 3xx redirects (up to 10 hops); each hop \
         re-checks the redirect URL against the agent's \
         net.fetch scope. Subject to a 10 MiB hard cap. \
         The agent must hold a net.fetch capability that \
         covers the requested URL's origin and path prefix."
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
        let follow_redirects = input
            .get("follow_redirects")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // ---- Issue GET (with optional redirect loop) ---------------
        let mut current_url = url;
        let mut hops: usize = 0;

        let response = loop {
            // Chapter Rampart — SSRF / egress guard on the initial URL and
            // every redirect hop (a public URL can 3xx to 169.254.169.254).
            if let Some(o) = egress_refusal(self.id, &self.egress, &current_url) {
                return o;
            }
            let request = self
                .client
                .get(&current_url)
                .timeout(Duration::from_millis(timeout_ms));

            let resp = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("GET {current_url} failed: {e}"),
                    });
                }
            };

            // Check for redirect
            if follow_redirects && resp.status().is_redirection() {
                hops += 1;
                if hops > MAX_REDIRECT_HOPS {
                    return ToolOutcome::Completed {
                        output: json!({
                            "url": current_url,
                            "status": resp.status().as_u16(),
                            "error": format!(
                                "redirect chain exceeded {MAX_REDIRECT_HOPS} hops"
                            ),
                            "body": "",
                            "body_encoding": "utf-8",
                        }),
                        verified: Verification::NotApplicable,
                    };
                }

                let location = match extract_redirect_location(&resp, &current_url) {
                    Some(loc) => loc,
                    None => break resp, // No valid Location — return 3xx as-is
                };

                // Re-derive scope from redirect URL and check
                // against effective capabilities.
                if let Err(outcome) = check_redirect_scope(
                    self.id,
                    &location,
                    "net.fetch",
                    &self.effective_caps,
                ) {
                    return outcome;
                }

                current_url = location;
                continue;
            }

            break resp;
        };

        let final_url = current_url;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        // ---- Stream body through StreamEvent::ToolOutput ----------
        let (body, body_encoding) = match collect_body(
            response,
            self.id,
            "web.fetch",
            &final_url,
            ctx,
        )
        .await
        {
            Ok(pair) => pair,
            Err(outcome) => return outcome,
        };

        ToolOutcome::Completed {
            output: json!({
                "url": final_url,
                "status": status,
                "content_type": content_type,
                "body": body,
                "body_encoding": body_encoding,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// WebExtractTool — Chapter Forge (FG.1)
//
// "Read a page", not "fetch a page": GET a URL with the same hardened,
// redirect-free client as web.fetch, then run a readability pass (dom_smoothie,
// MIT) to return the article's title + clean text instead of raw HTML. Reuses
// the `net.fetch` scope — extraction *is* an outbound GET, no new capability.
// ---------------------------------------------------------------------------

/// Construction inputs for [`WebExtractTool`].
pub struct WebExtractToolConfig;

impl WebExtractToolConfig {
    pub fn new() -> Self {
        WebExtractToolConfig
    }

    /// Build a ready-to-register [`WebExtractTool`] sharing `web.fetch`'s
    /// redirect-free rustls client (same SSRF posture).
    pub fn build(self) -> Result<WebExtractTool, AivyxError> {
        Ok(WebExtractTool {
            id: ToolId::new(),
            client: Arc::new(build_redirect_free_client("web.extract")?),
            schema: web_extract_input_schema_value(),
            egress: OnceLock::new(),
        })
    }
}

impl Default for WebExtractToolConfig {
    fn default() -> Self {
        WebExtractToolConfig::new()
    }
}

fn web_extract_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Absolute http or https URL of an article/page to read. \
                                Must be covered by the agent's net.fetch capability. \
                                Returns the extracted article text, not raw HTML — \
                                use web.fetch for raw bytes."
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_TIMEOUT_MS as i64,
                "description": "Wall-clock timeout in milliseconds. Default 30000; \
                                maximum 600000 (10 minutes)."
            }
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

/// `web.extract` — fetch a URL and return its readable article text.
pub struct WebExtractTool {
    id: ToolId,
    client: Arc<reqwest::Client>,
    schema: Value,
    /// Chapter Rampart — egress policy (unset ⇒ permissive).
    egress: OnceLock<Arc<EgressPolicy>>,
}

impl std::fmt::Debug for WebExtractTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebExtractTool").finish()
    }
}

impl WebExtractTool {
    /// Chapter Rampart — install the egress policy (unset ⇒ permissive).
    pub fn set_egress_policy(
        &self,
        policy: Arc<EgressPolicy>,
    ) -> Result<(), Arc<EgressPolicy>> {
        self.egress.set(policy)
    }

    /// Pure readability pass over an HTML string. Split out so it is directly
    /// unit-testable without the network. Returns `(title, byline, text)`.
    pub(crate) fn extract_html(
        html: &str,
        url: &str,
    ) -> Result<(String, Option<String>, String), String> {
        let mut read = Readability::new(html, Some(url), None)
            .map_err(|e| format!("readability init failed: {e}"))?;
        let article = read.parse().map_err(|e| format!("extraction failed: {e}"))?;
        let text = article.text_content.trim().to_string();
        if text.is_empty() {
            return Err("no readable content found (not an article?)".to_string());
        }
        Ok((article.title, article.byline, text))
    }
}

#[async_trait]
impl Tool for WebExtractTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "web.extract"
    }

    fn description(&self) -> &str {
        "Fetch a web page and return its readable article text (title + clean body), \
         not raw HTML. Use this to *read* a page; use web.fetch for raw bytes. \
         Input: { url, timeout_ms? }. Needs the net.fetch capability for the URL."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        let Some(url) = input.get("url").and_then(Value::as_str) else {
            return deny_scope();
        };
        if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            return deny_scope();
        }
        Scope::parse(&format!("net.fetch:{url}")).unwrap_or_else(deny_scope)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let url = match input.get("url").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "input must have a non-empty string `url` field".to_string(),
                });
            }
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("url must start with http:// or https:// (got {url:?})"),
            });
        }
        // Chapter Rampart — SSRF / egress guard.
        if let Some(o) = egress_refusal(self.id, &self.egress, &url) {
            return o;
        }
        let timeout_ms = input_timeout_ms(&input);

        let response = match self
            .client
            .get(&url)
            .timeout(Duration::from_millis(timeout_ms))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("GET {url} failed: {e}"),
                });
            }
        };

        let status = response.status().as_u16();
        if response.status().is_redirection() {
            return ToolOutcome::Completed {
                output: json!({
                    "url": url,
                    "status": status,
                    "extractable": false,
                    "error": "got a redirect; web.extract does not follow redirects — \
                              re-issue against the redirect target, or use web.fetch",
                }),
                verified: Verification::NotApplicable,
            };
        }

        let (body, body_encoding) =
            match collect_body(response, self.id, "web.extract", &url, ctx).await {
                Ok(pair) => pair,
                Err(outcome) => return outcome,
            };
        if body_encoding != "utf-8" {
            return ToolOutcome::Completed {
                output: json!({
                    "url": url,
                    "status": status,
                    "extractable": false,
                    "error": "response body is not UTF-8 text (binary/non-HTML) — use web.fetch",
                }),
                verified: Verification::NotApplicable,
            };
        }

        match Self::extract_html(&body, &url) {
            Ok((title, byline, text)) => {
                let word_count = text.split_whitespace().count();
                ToolOutcome::Completed {
                    output: json!({
                        "url": url,
                        "status": status,
                        "extractable": true,
                        "title": title,
                        "byline": byline,
                        "text": text,
                        "word_count": word_count,
                    }),
                    verified: Verification::NotApplicable,
                }
            }
            Err(e) => ToolOutcome::Completed {
                output: json!({
                    "url": url,
                    "status": status,
                    "extractable": false,
                    "error": format!("{e}; the page may not be an article — try web.fetch"),
                }),
                verified: Verification::NotApplicable,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// WebPostTool — Phase 37 Task 3
// ---------------------------------------------------------------------------

/// Construction inputs for [`WebPostTool`].
pub struct WebPostToolConfig;

impl WebPostToolConfig {
    pub fn new() -> Self {
        WebPostToolConfig
    }

    pub fn build(self) -> Result<WebPostTool, AivyxError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| {
                AivyxError::Config(format!(
                    "web.post reqwest client build failed: {e}"
                ))
            })?;
        Ok(WebPostTool {
            id: ToolId::new(),
            client: Arc::new(client),
            schema: web_post_input_schema_value(),
            effective_caps: OnceLock::new(),
            egress: OnceLock::new(),
        })
    }
}

impl Default for WebPostToolConfig {
    fn default() -> Self {
        WebPostToolConfig::new()
    }
}

impl WebPostTool {
    /// Chapter Rampart — install the egress policy (unset ⇒ permissive).
    pub fn set_egress_policy(
        &self,
        policy: Arc<EgressPolicy>,
    ) -> Result<(), Arc<EgressPolicy>> {
        self.egress.set(policy)
    }

    /// Install the effective capability set for per-hop redirect
    /// scope checks. Same pattern as `WebFetchTool`.
    pub fn set_effective_capabilities(
        &self,
        caps: CapabilitySet,
    ) -> Result<(), CapabilitySet> {
        self.effective_caps.set(caps)
    }
}

/// HTTP write-verb tool. Agents holding `net.post:<origin>/<path>`
/// (or `net.post` unqualified) can POST/PUT/PATCH/DELETE against
/// any URL under the granted prefix.
///
/// Separate from `WebFetchTool` because:
/// - Different scope base: `net.post` (write) vs `net.fetch` (read).
/// - Different trust tier: `net.post` is Trusted-only;
///   `net.fetch` is SemiTrusted-accessible.
/// - Different input shape: needs `body` and `content_type` fields.
pub struct WebPostTool {
    id: ToolId,
    client: Arc<reqwest::Client>,
    schema: Value,
    /// Effective capabilities for per-hop redirect scope checks.
    effective_caps: OnceLock<CapabilitySet>,
    /// Chapter Rampart — egress policy (unset ⇒ permissive).
    egress: OnceLock<Arc<EgressPolicy>>,
}

impl std::fmt::Debug for WebPostTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPostTool")
            .field("id", &self.id)
            .field("has_effective_caps", &self.effective_caps.get().is_some())
            .finish()
    }
}

/// Allowed HTTP methods for `WebPostTool`. Case-insensitive on
/// input, normalized to uppercase.
const ALLOWED_METHODS: &[&str] = &["POST", "PUT", "PATCH", "DELETE"];

fn web_post_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Absolute http or https URL to send the request to. \
                                Must be covered by the agent's net.post capability."
            },
            "method": {
                "type": "string",
                "enum": ["POST", "PUT", "PATCH", "DELETE"],
                "description": "HTTP method. Default: POST."
            },
            "body": {
                "description": "Request body. A string is sent as-is; a JSON object \
                                or array is serialized to JSON. Omit for an empty body."
            },
            "content_type": {
                "type": "string",
                "description": "Content-Type header value. Default: application/json \
                                when body is a JSON object/array, text/plain when \
                                body is a string, omitted when body is absent."
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_TIMEOUT_MS as i64,
                "description": "Wall-clock timeout in milliseconds. \
                                Default 30000; maximum 600000 (10 minutes)."
            },
            "follow_redirects": {
                "type": "boolean",
                "description": "Follow 3xx redirects up to 10 hops. \
                                Each hop re-checks the redirect URL \
                                against the agent's net.post scope. \
                                Redirect hops always use GET. \
                                Default: false."
            }
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

/// A scope no real agent should ever hold. Returned by
/// `required_scope` when the input is malformed.
fn deny_post_scope() -> Scope {
    Scope::parse("net.post:https://aivyx.invalid/__deny__/invalid-input")
        .expect("deny scope must parse")
}

#[async_trait]
impl Tool for WebPostTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "web.post"
    }

    fn description(&self) -> &str {
        "Send an HTTP request with a write verb (POST, PUT, \
         PATCH, DELETE) to a URL and return its status code \
         and body. UTF-8 bodies are returned directly; binary \
         bodies are base64-encoded. Set follow_redirects to \
         true to follow 3xx redirects (up to 10 hops, using \
         GET); each hop re-checks scope. Subject to a 10 MiB \
         response cap. The agent must hold a net.post \
         capability that covers the requested URL's origin \
         and path prefix."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        let Some(url) = input.get("url").and_then(Value::as_str) else {
            return deny_post_scope();
        };
        if url.is_empty() {
            return deny_post_scope();
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return deny_post_scope();
        }
        Scope::parse(&format!("net.post:{url}")).unwrap_or_else(deny_post_scope)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        // ---- Parse input -------------------------------------------------
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
        // Chapter Rampart — SSRF / egress guard (outbound POST to a private
        // address is the exfil case this most protects against).
        if let Some(o) = egress_refusal(self.id, &self.egress, &url) {
            return o;
        }

        let method_str = input
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("POST");
        let method_upper = method_str.to_uppercase();
        if !ALLOWED_METHODS.contains(&method_upper.as_str()) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "unsupported HTTP method {method_str:?}; \
                     allowed: POST, PUT, PATCH, DELETE"
                ),
            });
        }

        let timeout_ms = input_timeout_ms(&input);
        let follow_redirects = input
            .get("follow_redirects")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // ---- Build request body ------------------------------------------
        let (body_bytes, content_type) = match input.get("body") {
            None => (Vec::new(), None),
            Some(Value::String(s)) => {
                let ct = input
                    .get("content_type")
                    .and_then(Value::as_str)
                    .unwrap_or("text/plain")
                    .to_string();
                (s.as_bytes().to_vec(), Some(ct))
            }
            Some(val) => {
                // JSON object or array — serialize to JSON bytes
                let ct = input
                    .get("content_type")
                    .and_then(Value::as_str)
                    .unwrap_or("application/json")
                    .to_string();
                let serialized = serde_json::to_vec(val).unwrap_or_default();
                (serialized, Some(ct))
            }
        };

        // ---- Issue initial request (POST/PUT/PATCH/DELETE) ----------------
        let method = match method_upper.as_str() {
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "PATCH" => reqwest::Method::PATCH,
            "DELETE" => reqwest::Method::DELETE,
            _ => unreachable!("validated above"),
        };

        let mut req_builder = self
            .client
            .request(method, &url)
            .timeout(Duration::from_millis(timeout_ms))
            .body(body_bytes);

        if let Some(ref ct) = content_type {
            req_builder = req_builder.header(reqwest::header::CONTENT_TYPE, ct.as_str());
        }

        let mut response = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("{method_upper} {url} failed: {e}"),
                });
            }
        };

        // ---- Redirect loop (subsequent hops always use GET) --------------
        let mut current_url = url;
        let mut hops: usize = 0;

        while follow_redirects && response.status().is_redirection() {
            hops += 1;
            if hops > MAX_REDIRECT_HOPS {
                return ToolOutcome::Completed {
                    output: json!({
                        "url": current_url,
                        "method": method_upper,
                        "status": response.status().as_u16(),
                        "error": format!(
                            "redirect chain exceeded {MAX_REDIRECT_HOPS} hops"
                        ),
                        "body": "",
                        "body_encoding": "utf-8",
                    }),
                    verified: Verification::NotApplicable,
                };
            }

            let location = match extract_redirect_location(&response, &current_url) {
                Some(loc) => loc,
                None => break, // No valid Location — return 3xx as-is
            };

            if let Err(outcome) = check_redirect_scope(
                self.id,
                &location,
                "net.post",
                &self.effective_caps,
            ) {
                return outcome;
            }

            current_url = location;

            // Chapter Rampart — egress guard on the redirect target too.
            if let Some(o) = egress_refusal(self.id, &self.egress, &current_url) {
                return o;
            }

            // Redirect hops always use GET (POST-redirect-GET per
            // HTTP 303 semantics; we apply this uniformly).
            let hop_request = self
                .client
                .get(&current_url)
                .timeout(Duration::from_millis(timeout_ms));

            response = match hop_request.send().await {
                Ok(r) => r,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("GET {current_url} (redirect hop) failed: {e}"),
                    });
                }
            };
        }

        let final_url = current_url;
        let status = response.status().as_u16();
        let resp_content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        // ---- Collect response body ----------------------------------------
        let (body, body_encoding) = match collect_body(
            response,
            self.id,
            "web.post",
            &final_url,
            ctx,
        )
        .await
        {
            Ok(pair) => pair,
            Err(outcome) => return outcome,
        };

        ToolOutcome::Completed {
            output: json!({
                "url": final_url,
                "method": method_upper,
                "status": status,
                "content_type": resp_content_type,
                "body": body,
                "body_encoding": body_encoding,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod web_extract_tests {
    use super::*;

    const ARTICLE: &str = r#"<!DOCTYPE html><html><head><title>The Headline</title></head>
        <body>
          <nav>Home About Contact</nav>
          <article>
            <h1>The Headline</h1>
            <p>This is the first substantial paragraph of the article body, long
               enough that the readability scorer treats it as real content and
               not boilerplate navigation chrome.</p>
            <p>A second paragraph continues the article with more sentences so the
               extractor has enough text to confidently select this region.</p>
          </article>
          <footer>Copyright 2026</footer>
        </body></html>"#;

    #[test]
    fn extract_pulls_title_and_body_drops_chrome() {
        let (title, _byline, text) =
            WebExtractTool::extract_html(ARTICLE, "https://example.com/post").expect("extract");
        assert!(title.contains("Headline"), "title: {title:?}");
        assert!(text.contains("first substantial paragraph"), "text: {text:?}");
        assert!(text.contains("second paragraph"), "text: {text:?}");
        // Navigation/footer chrome should be dropped by the readability pass.
        assert!(!text.contains("Home About Contact"), "nav leaked: {text:?}");
    }

    #[test]
    fn extract_errors_on_empty_or_contentless_html() {
        assert!(WebExtractTool::extract_html("<html><body></body></html>", "https://x.test").is_err());
    }

    #[test]
    fn required_scope_is_net_fetch_for_valid_url() {
        let tool = WebExtractToolConfig::new().build().expect("build");
        let scope = tool.required_scope(&json!({"url": "https://example.com/a"}));
        assert_eq!(scope.base(), "net.fetch");
    }

    #[test]
    fn required_scope_denies_non_http_or_missing_url() {
        // deny_scope() keeps base net.fetch but an unmatchable __deny__ qualifier,
        // so compare against the sentinel, not the base.
        let tool = WebExtractToolConfig::new().build().expect("build");
        let deny = format!("{:?}", deny_scope());
        assert_eq!(format!("{:?}", tool.required_scope(&json!({"url": "ftp://x"}))), deny);
        assert_eq!(format!("{:?}", tool.required_scope(&json!({}))), deny);
        // sanity: a real https URL is NOT the deny sentinel
        assert_ne!(format!("{:?}", tool.required_scope(&json!({"url": "https://ok.test/a"}))), deny);
    }
}

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
    async fn execute_egress_guard_refuses_metadata_and_localhost() {
        // The guard short-circuits before any network call, so this needs no
        // server. Covers the cloud-metadata + localhost SSRF cases.
        let tool = build_tool();
        let _ = tool.set_egress_policy(std::sync::Arc::new(
            crate::egress::EgressPolicy::default(),
        ));
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://localhost:7843/api",
            "http://127.0.0.1/",
        ] {
            let ctx = make_ctx(&channel, &audit);
            match tool.execute(json!({ "url": url }), &ctx).await {
                ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                    assert!(detail.contains("refusing to reach"), "url {url}: {detail}");
                }
                other => panic!("expected egress refusal for {url}, got {other:?}"),
            }
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

    /// Starts a mock HTTP server returning a fixed response.
    async fn mock_server(
        status: &'static str,
        headers: &'static str,
        body: &'static [u8],
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        tokio::spawn(async move {
            for _ in 0..5 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;

                    let mut response = format!(
                        "HTTP/1.1 {status}\r\n\
                         Content-Length: {}\r\n\
                         {headers}\
                         Connection: close\r\n\
                         \r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body);
                    let _ = stream.write_all(&response).await;
                });
            }
        });

        base_url
    }

    #[tokio::test]
    async fn execute_utf8_body_returns_utf8_encoding() {
        let body = b"Hello, world!";
        let url = mock_server(
            "200 OK",
            "Content-Type: text/plain\r\n",
            body,
        )
        .await;

        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(json!({"url": format!("{url}/test")}), &ctx)
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["body"], "Hello, world!");
                assert_eq!(output["body_encoding"], "utf-8");
                assert_eq!(output["status"], 200);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_binary_body_returns_base64_encoding() {
        // Invalid UTF-8 bytes
        let body: &[u8] = &[0xFF, 0xFE, 0x00, 0x01, 0x89, 0x50, 0x4E, 0x47];
        let url = mock_server(
            "200 OK",
            "Content-Type: application/octet-stream\r\n",
            body,
        )
        .await;

        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(json!({"url": format!("{url}/binary")}), &ctx)
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["body_encoding"], "base64");
                // Decode and verify round-trip
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(output["body"].as_str().unwrap())
                    .unwrap();
                assert_eq!(decoded, body);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Binary responses should NOT have been streamed
        assert!(
            channel.chunks().is_empty(),
            "binary body should not produce ToolOutput stream chunks"
        );
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

    // ---- Mock server that echoes request details ----------------------

    /// Starts a mock HTTP server that echoes request method, headers,
    /// and body back as a JSON response.
    async fn echo_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        tokio::spawn(async move {
            for _ in 0..10 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);

                    // Parse method from first line
                    let method = request
                        .split_whitespace()
                        .next()
                        .unwrap_or("UNKNOWN")
                        .to_string();

                    // Extract body after \r\n\r\n
                    let req_body = request
                        .split_once("\r\n\r\n")
                        .map(|(_, b)| b.to_string())
                        .unwrap_or_default();

                    let echo = json!({
                        "echo_method": method,
                        "echo_body": req_body,
                    })
                    .to_string();

                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: application/json\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\
                         \r\n\
                         {echo}",
                        echo.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        base_url
    }

    fn build_post_tool() -> WebPostTool {
        WebPostToolConfig::new()
            .build()
            .expect("web.post tool builds with default config")
    }

    // ---- WebPostTool: scope derivation --------------------------------

    #[test]
    fn post_required_scope_uses_net_post_base() {
        let tool = build_post_tool();
        let scope = tool.required_scope(&json!({
            "url": "https://api.example.com/data"
        }));
        assert_eq!(scope.base(), "net.post");
        assert_eq!(
            scope.qualifier(),
            Some("https://api.example.com/data")
        );
    }

    #[test]
    fn post_required_scope_for_missing_url_is_deny_scope() {
        let tool = build_post_tool();
        let scope = tool.required_scope(&json!({}));
        assert!(scope.qualifier().unwrap().contains("__deny__"));
        assert_eq!(scope.base(), "net.post");
    }

    // ---- WebPostTool: execution ----------------------------------------

    #[tokio::test]
    async fn post_sends_json_body() {
        let url = echo_server().await;
        let tool = build_post_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/api"),
                    "body": {"key": "value"}
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["status"], 200);
                assert_eq!(output["method"], "POST");
                // The echo server returns the request body
                let echo_body: Value = serde_json::from_str(
                    output["body"].as_str().unwrap(),
                )
                .unwrap();
                assert_eq!(echo_body["echo_method"], "POST");
                // Verify the JSON body was sent
                let sent: Value = serde_json::from_str(
                    echo_body["echo_body"].as_str().unwrap(),
                )
                .unwrap();
                assert_eq!(sent["key"], "value");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_put_method_is_honored() {
        let url = echo_server().await;
        let tool = build_post_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/resource"),
                    "method": "PUT",
                    "body": "updated"
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["method"], "PUT");
                let echo_body: Value = serde_json::from_str(
                    output["body"].as_str().unwrap(),
                )
                .unwrap();
                assert_eq!(echo_body["echo_method"], "PUT");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_delete_method_is_honored() {
        let url = echo_server().await;
        let tool = build_post_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/resource/42"),
                    "method": "DELETE"
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["method"], "DELETE");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_invalid_method_fails() {
        let tool = build_post_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": "http://localhost:1/test",
                    "method": "GET"
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("unsupported"),
                    "expected method error: {detail}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_missing_url_fails() {
        let tool = build_post_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool.execute(json!({}), &ctx).await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("url"),
                    "expected url error: {detail}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ---- Redirect tests -------------------------------------------------

    /// Mock server that returns a 301 redirect to a given Location,
    /// then serves a 200 OK with the given body at the redirect target.
    /// The redirect is triggered by any path ending in `/redirect`;
    /// all other paths return the final body.
    async fn redirect_server(
        status_code: u16,
        final_body: &'static str,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let base_for_task = base_url.clone();
        tokio::spawn(async move {
            for _ in 0..15 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let base = base_for_task.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);

                    // Parse path from first line
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/");

                    let response = if path.ends_with("/redirect") {
                        format!(
                            "HTTP/1.1 {status_code} Redirect\r\n\
                             Location: {base}/final\r\n\
                             Content-Length: 0\r\n\
                             Connection: close\r\n\
                             \r\n"
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: text/plain\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\
                             \r\n\
                             {final_body}",
                            final_body.len()
                        )
                    };
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        base_url
    }

    /// Mock server that always redirects (for testing the hop cap).
    async fn infinite_redirect_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let base_for_task = base_url.clone();
        tokio::spawn(async move {
            for i in 0..20 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let base = base_for_task.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;

                    let response = format!(
                        "HTTP/1.1 301 Moved\r\n\
                         Location: {base}/hop{i}\r\n\
                         Content-Length: 0\r\n\
                         Connection: close\r\n\
                         \r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        base_url
    }

    fn build_tool_with_caps(scopes: &[&str]) -> WebFetchTool {
        let tool = build_tool();
        let caps = CapabilitySet::from_scopes(
            scopes.iter().map(|s| Scope::parse(s).unwrap()),
        );
        tool.set_effective_capabilities(caps).unwrap();
        tool
    }

    #[tokio::test]
    async fn redirect_301_followed_within_scope() {
        let url = redirect_server(301, "arrived").await;
        let tool = build_tool_with_caps(&[
            &format!("net.fetch:{url}/"),
        ]);
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/redirect"),
                    "follow_redirects": true
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["status"], 200);
                assert_eq!(output["url"], format!("{url}/final"));
                assert!(
                    output["body"].as_str().unwrap().contains("arrived"),
                    "should have followed redirect to final page"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_302_followed_within_scope() {
        let url = redirect_server(302, "found-it").await;
        let tool = build_tool_with_caps(&[
            &format!("net.fetch:{url}/"),
        ]);
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/redirect"),
                    "follow_redirects": true
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["status"], 200);
                assert!(
                    output["body"].as_str().unwrap().contains("found-it"),
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_to_out_of_scope_url_denied() {
        let url = redirect_server(301, "secret").await;
        // Grant scope only for the initial URL's path, not for /final
        let tool = build_tool_with_caps(&[
            "net.fetch:http://other-host.invalid/",
        ]);
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/redirect"),
                    "follow_redirects": true
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert!(
                    output["error"]
                        .as_str()
                        .unwrap()
                        .contains("denied"),
                    "redirect to out-of-scope URL should be denied: {output}"
                );
            }
            other => panic!("expected Completed with denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_chain_capped_at_max_hops() {
        let url = infinite_redirect_server().await;
        let tool = build_tool_with_caps(&[
            &format!("net.fetch:{url}/"),
        ]);
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/start"),
                    "follow_redirects": true
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert!(
                    output["error"]
                        .as_str()
                        .unwrap()
                        .contains("exceeded"),
                    "should hit hop cap: {output}"
                );
            }
            other => panic!("expected Completed with hop-cap error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_without_follow_flag_returns_3xx_as_is() {
        let url = redirect_server(301, "should-not-reach").await;
        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/redirect"),
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(
                    output["status"], 301,
                    "without follow_redirects, 301 should surface as-is"
                );
            }
            other => panic!("expected Completed with 301, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_without_capabilities_installed_fails() {
        let url = redirect_server(301, "nope").await;
        // Don't call set_effective_capabilities
        let tool = build_tool();
        let channel = CapturingChannel::new();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);

        let out = tool
            .execute(
                json!({
                    "url": format!("{url}/redirect"),
                    "follow_redirects": true
                }),
                &ctx,
            )
            .await;

        match out {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("no effective capabilities"),
                    "should explain caps not installed: {detail}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
