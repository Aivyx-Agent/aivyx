//! `aivyx-gmail auth init` — run the OAuth authorization-code
//! flow.
//!
//! Phase 123 Task 3 — operator-facing.
//!
//! ## Flow
//!
//! 1. Load OAuth config from
//!    `~/.aivyx-pa/tool-processes/gmail/config.toml`.
//! 2. Parse the redirect_uri to extract the loopback port +
//!    callback path.
//! 3. Bind a `TcpListener` on `127.0.0.1:<port>`. If the port
//!    is taken, surface a clear error (operator can edit
//!    `redirect_uri` to pick a free port).
//! 4. Generate a CSRF `state` token (UUID v4).
//! 5. Build the Google consent URL and print it. Operator
//!    pastes it in their browser; Google redirects to the
//!    loopback URI with `?code=X&state=Y`.
//! 6. Read one HTTP request from the listener; extract the
//!    `code` and `state` query params.
//! 7. Verify `state` matches what we generated.
//! 8. Respond to the browser with a tiny "you can close this"
//!    HTML page so the operator gets visible confirmation.
//! 9. Exchange the code for tokens
//!    ([`crate::oauth::exchange_code`]).
//! 10. Save tokens
//!     ([`crate::oauth::storage::save_tokens`]).
//!
//! ## What this module deliberately doesn't do
//!
//! - **No browser-launching.** The CLI prints the URL and lets
//!   the operator click. Avoids a cross-platform browser-open
//!   dep + the failure modes of "your default browser is
//!   misconfigured." The URL is the substrate; the click is
//!   the operator's.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use reqwest::Client;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

use crate::oauth::{
    exchange_code, save_tokens, ExchangeError, OAuthConfig, StorageError,
    GOOGLE_AUTH_ENDPOINT, GOOGLE_TOKEN_ENDPOINT,
};

#[derive(Debug, Error)]
pub enum InitError {
    #[error("redirect_uri {0:?} could not be parsed: must look like `http://127.0.0.1:<port>/<path>`")]
    BadRedirectUri(String),
    #[error("redirect_uri host must be 127.0.0.1 or localhost; got {0:?}")]
    NonLoopbackRedirect(String),
    #[error("failed to bind loopback listener on port {port}: {source}")]
    BindFailed { port: u16, source: io::Error },
    #[error("loopback listener I/O failed: {0}")]
    ListenerIo(io::Error),
    #[error("OAuth callback timed out after {0:?} — operator never completed the consent flow")]
    CallbackTimeout(Duration),
    #[error("OAuth callback missing required query param: {0}")]
    CallbackMissingParam(&'static str),
    #[error("OAuth callback state mismatch — possible CSRF; expected {expected:?}, got {got:?}")]
    StateMismatch { expected: String, got: String },
    #[error("Google returned an error in the callback: {error}{}", description.as_ref().map(|d| format!(" — {d}")).unwrap_or_default())]
    GoogleCallbackError {
        error: String,
        description: Option<String>,
    },
    #[error("token exchange failed: {0}")]
    Exchange(#[from] ExchangeError),
    #[error("token storage failed: {0}")]
    Storage(#[from] StorageError),
}

/// Build the Google OAuth 2.0 authorization URL.
///
/// Pure function so the test suite can pin the constructed URL
/// exactly without spinning up an HTTP listener.
///
/// Includes `access_type=offline` + `prompt=consent` so Google
/// always returns a refresh_token — without these, a re-init
/// after a previous init might not get a fresh refresh token.
pub fn build_consent_url(config: &OAuthConfig, state: &str) -> String {
    let scope = config.scopes_space_delimited();
    let mut url = String::with_capacity(512);
    url.push_str(GOOGLE_AUTH_ENDPOINT);
    url.push('?');
    append_query_param(&mut url, "client_id", &config.client_id);
    url.push('&');
    append_query_param(&mut url, "redirect_uri", &config.redirect_uri);
    url.push('&');
    append_query_param(&mut url, "response_type", "code");
    url.push('&');
    append_query_param(&mut url, "scope", &scope);
    url.push('&');
    append_query_param(&mut url, "access_type", "offline");
    url.push('&');
    append_query_param(&mut url, "prompt", "consent");
    url.push('&');
    append_query_param(&mut url, "state", state);
    url
}

/// Percent-encode `value` and append `key=value` to `dest`.
/// Single-purpose minimal implementation — avoids pulling
/// `url` / `percent-encoding` as a new dep just for this.
fn append_query_param(dest: &mut String, key: &str, value: &str) {
    dest.push_str(key);
    dest.push('=');
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                dest.push(b as char);
            }
            _ => {
                dest.push_str(&format!("%{b:02X}"));
            }
        }
    }
}

/// Parse the redirect_uri to extract the loopback port. Only
/// accepts `http://127.0.0.1:<port>/<path>` (and `localhost`
/// equivalent) — Google's OAuth requires loopback redirect URIs
/// to use these hosts.
pub fn parse_loopback_port(redirect_uri: &str) -> Result<u16, InitError> {
    let after_scheme = redirect_uri
        .strip_prefix("http://")
        .ok_or_else(|| InitError::BadRedirectUri(redirect_uri.to_string()))?;
    // Split on first `/` to separate authority from path.
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let (host, port_str) = authority
        .rsplit_once(':')
        .ok_or_else(|| InitError::BadRedirectUri(redirect_uri.to_string()))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err(InitError::NonLoopbackRedirect(host.to_string()));
    }
    port_str
        .parse::<u16>()
        .map_err(|_| InitError::BadRedirectUri(redirect_uri.to_string()))
}

/// Result of receiving one OAuth callback on the loopback
/// listener. Either a successful `code` + `state` capture or
/// the Google-side error (e.g. `access_denied` if the user
/// denied consent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackCapture {
    pub code: String,
    pub state: String,
}

/// Listen for ONE HTTP request on `listener`, parse its query
/// string for OAuth callback params, write a tiny success
/// response, and return the captured values.
///
/// `total_timeout` bounds the wait — surfaces as
/// [`InitError::CallbackTimeout`] if the operator never
/// completes the consent flow.
pub async fn await_callback(
    listener: TcpListener,
    total_timeout: Duration,
) -> Result<CallbackCapture, InitError> {
    let fut = async {
        let (mut socket, _addr) =
            listener.accept().await.map_err(InitError::ListenerIo)?;
        let (read_half, mut write_half) = socket.split();
        let mut reader = BufReader::new(read_half);

        // Read the request line to extract the query string.
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .await
            .map_err(InitError::ListenerIo)?;

        // Drain headers + body so the client's send buffer can
        // flush; otherwise the browser may hang on response.
        let mut content_length: usize = 0;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await.map_err(InitError::ListenerIo)?;
            if n == 0 || line == "\r\n" {
                break;
            }
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = rest.trim().parse().unwrap_or(0);
            }
        }
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body).await;
        }

        // Parse the query string.
        let query_string = extract_query_string(&request_line)
            .ok_or(InitError::CallbackMissingParam("(query string)"))?;
        let params = parse_query_params(query_string);

        // Google-side error?
        if let Some(err) = params.iter().find(|(k, _)| k == "error").map(|(_, v)| v.clone()) {
            let desc = params
                .iter()
                .find(|(k, _)| k == "error_description")
                .map(|(_, v)| v.clone());
            write_browser_response(
                &mut write_half,
                "OAuth flow failed",
                &format!("Google returned error: {err}"),
            )
            .await
            .map_err(InitError::ListenerIo)?;
            return Err(InitError::GoogleCallbackError {
                error: err,
                description: desc,
            });
        }

        let code = params
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone())
            .ok_or(InitError::CallbackMissingParam("code"))?;
        let state = params
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.clone())
            .ok_or(InitError::CallbackMissingParam("state"))?;

        // Confirm to the browser before returning.
        write_browser_response(
            &mut write_half,
            "Aivyx Gmail — authorization received",
            "You can close this tab and return to your terminal.",
        )
        .await
        .map_err(InitError::ListenerIo)?;

        Ok(CallbackCapture { code, state })
    };

    timeout(total_timeout, fut)
        .await
        .map_err(|_| InitError::CallbackTimeout(total_timeout))?
}

fn extract_query_string(request_line: &str) -> Option<&str> {
    // Request line: `GET /path?key=val HTTP/1.1\r\n`
    let path_and_query = request_line.split_whitespace().nth(1)?;
    let q_idx = path_and_query.find('?')?;
    Some(&path_and_query[q_idx + 1..])
}

fn parse_query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'%' => {
                let hi = bytes.next();
                let lo = bytes.next();
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    if let (Some(h), Some(l)) =
                        (hex_value(hi), hex_value(lo))
                    {
                        out.push((h * 16 + l) as char);
                        continue;
                    }
                }
                out.push('%');
            }
            b'+' => out.push(' '),
            _ => out.push(b as char),
        }
    }
    out
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn write_browser_response<W: AsyncWriteExt + Unpin>(
    sink: &mut W,
    title: &str,
    body_text: &str,
) -> io::Result<()> {
    let html = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:2rem;\">\
         <h1>{title}</h1><p>{body_text}</p></body></html>",
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        html.len(),
        html,
    );
    sink.write_all(response.as_bytes()).await?;
    sink.flush().await
}

/// End-to-end `auth init` runner. The CLI dispatcher in
/// [`crate::auth_cli::cli`] calls this after parsing the
/// subcommand. Returns `Ok(())` on a successful token save;
/// any error in the chain surfaces unchanged with operator-
/// actionable context.
///
/// `callback_timeout` defaults to 5 minutes when called from
/// the CLI; the longer window accommodates a slow consent flow
/// (operator typing 2FA codes, re-authing, etc).
pub async fn run_auth_init(
    config: &OAuthConfig,
    token_path: &std::path::Path,
    callback_timeout: Duration,
    state_token: &str,
) -> Result<(), InitError> {
    let port = parse_loopback_port(&config.redirect_uri)?;
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|source| InitError::BindFailed { port, source })?;

    let consent_url = build_consent_url(config, state_token);
    println!(
        "\n\
         Aivyx Gmail — OAuth consent required\n\
         ------------------------------------\n\
         Open this URL in your browser:\n\n  {consent_url}\n\n\
         After granting consent, Google will redirect you to\n\
         {} — the loopback listener is waiting.\n",
        config.redirect_uri,
    );

    let capture = await_callback(listener, callback_timeout).await?;
    if capture.state != state_token {
        return Err(InitError::StateMismatch {
            expected: state_token.to_string(),
            got: capture.state,
        });
    }

    let client = Client::new();
    let tokens =
        exchange_code(&client, GOOGLE_TOKEN_ENDPOINT, config, &capture.code).await?;
    save_tokens(token_path, &tokens).await?;
    println!(
        "Tokens saved to {token_path:?}.\n\
         Granted scope: {}\n\
         Run `aivyx-gmail auth status` to inspect.",
        tokens.granted_scope,
    );
    Ok(())
}

/// Generate a fresh CSRF state token. UUID v4 — random enough
/// for loopback-bound OAuth flows; not cryptographically perfect
/// but the loopback redirect URI binding already provides the
/// load-bearing constraint (an attacker can't redirect Google
/// to a non-loopback URI without the operator's OAuth app
/// configuration cooperating).
pub fn generate_state_token() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::OAuthConfig;
    use std::time::Duration;

    fn sample_config() -> OAuthConfig {
        // Phase 129 OAuth lift: OAuthConfig::new yields
        // empty scopes; chain .with_scopes() with the
        // gmail-specific default set so the consent-URL
        // construction has a real scope param to build
        // with.
        OAuthConfig::new(
            "id.apps.googleusercontent.com",
            "GOCSPX-secret",
            "http://127.0.0.1:8088/cb",
        )
        .with_scopes(crate::DEFAULT_GMAIL_SCOPES.iter().copied())
    }

    #[test]
    fn build_consent_url_includes_all_required_params() {
        let url = build_consent_url(&sample_config(), "STATE-X");
        assert!(url.starts_with(GOOGLE_AUTH_ENDPOINT));
        assert!(url.contains("client_id=id.apps.googleusercontent.com"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=STATE-X"));
        // redirect_uri is percent-encoded.
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8088%2Fcb"));
        // Scopes are space-delimited inside the encoded scope param.
        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com"));
    }

    #[test]
    fn parse_loopback_port_accepts_127_0_0_1() {
        let p =
            parse_loopback_port("http://127.0.0.1:8088/cb").expect("parse");
        assert_eq!(p, 8088);
    }

    #[test]
    fn parse_loopback_port_accepts_localhost() {
        let p = parse_loopback_port("http://localhost:54321/oauth").expect("parse");
        assert_eq!(p, 54321);
    }

    #[test]
    fn parse_loopback_port_rejects_non_loopback() {
        match parse_loopback_port("http://example.com:80/cb") {
            Err(InitError::NonLoopbackRedirect(host)) => {
                assert_eq!(host, "example.com");
            }
            other => panic!("expected NonLoopbackRedirect; got {other:?}"),
        }
    }

    #[test]
    fn parse_loopback_port_rejects_https() {
        match parse_loopback_port("https://127.0.0.1:443/cb") {
            Err(InitError::BadRedirectUri(_)) => {}
            other => panic!("expected BadRedirectUri; got {other:?}"),
        }
    }

    #[test]
    fn parse_loopback_port_rejects_missing_port() {
        match parse_loopback_port("http://127.0.0.1/cb") {
            Err(InitError::BadRedirectUri(_)) => {}
            other => panic!("expected BadRedirectUri; got {other:?}"),
        }
    }

    #[test]
    fn percent_decode_handles_typical_escapes() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("a+b"), "a b");
    }

    #[test]
    fn parse_query_params_extracts_all_pairs() {
        let p = parse_query_params("code=ABC&state=XYZ&scope=read%20write");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], ("code".to_string(), "ABC".to_string()));
        assert_eq!(p[1], ("state".to_string(), "XYZ".to_string()));
        assert_eq!(p[2], ("scope".to_string(), "read write".to_string()));
    }

    #[test]
    fn extract_query_string_from_get_request() {
        let line = "GET /cb?code=X&state=Y HTTP/1.1\r\n";
        assert_eq!(extract_query_string(line), Some("code=X&state=Y"));
    }

    #[test]
    fn extract_query_string_returns_none_without_question_mark() {
        let line = "GET /cb HTTP/1.1\r\n";
        assert!(extract_query_string(line).is_none());
    }

    #[test]
    fn generate_state_token_returns_unique_values() {
        let a = generate_state_token();
        let b = generate_state_token();
        assert_ne!(a, b);
        assert!(a.len() >= 16); // UUID v4 is 36 chars w/ hyphens.
    }

    #[tokio::test]
    async fn await_callback_captures_code_and_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Send a fake browser GET to the listener.
        tokio::spawn(async move {
            // Give the listener a tick to await.accept.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut stream =
                tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
            let req = "GET /cb?code=AUTH-X&state=STATE-X HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       \r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            // Read the response so the listener's write completes.
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
        });

        let capture = await_callback(listener, Duration::from_secs(2))
            .await
            .expect("capture");
        assert_eq!(capture.code, "AUTH-X");
        assert_eq!(capture.state, "STATE-X");
    }

    #[tokio::test]
    async fn await_callback_surfaces_google_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut stream =
                tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
            let req = "GET /cb?error=access_denied&error_description=user%20said%20no HTTP/1.1\r\n\
                       Host: 127.0.0.1\r\n\
                       \r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
        });

        match await_callback(listener, Duration::from_secs(2)).await {
            Err(InitError::GoogleCallbackError { error, description }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("user said no"));
            }
            other => panic!("expected GoogleCallbackError; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_callback_times_out_when_nothing_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        match await_callback(listener, Duration::from_millis(100)).await {
            Err(InitError::CallbackTimeout(d)) => {
                assert_eq!(d, Duration::from_millis(100));
            }
            other => panic!("expected CallbackTimeout; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_callback_rejects_missing_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut stream =
                tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
            // state present, code missing
            let req = "GET /cb?state=XYZ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
        });

        match await_callback(listener, Duration::from_secs(2)).await {
            Err(InitError::CallbackMissingParam(field)) => assert_eq!(field, "code"),
            other => panic!("expected CallbackMissingParam; got {other:?}"),
        }
    }
}
