//! RFC 5322 MIME message construction + Gmail-API base64url
//! encoding.
//!
//! Phase 123 Tasks 6 and 7 — `gmail.draft` and `gmail.send`
//! both POST a `raw` field to the Gmail API carrying a
//! base64url-encoded RFC 5322 message. This module is the
//! shared substrate so the two tools don't reimplement MIME.
//!
//! ## Header-injection defense
//!
//! Header values that contain CR or LF would let a hostile
//! input attach arbitrary additional headers (or even a fake
//! body). The validator at the top of every header-supplied
//! field rejects CR + LF; tests pin every rejection so a
//! regression surfaces immediately.
//!
//! ## Encodings
//!
//! - **Subject** — RFC 2047 base64 encoded-word
//!   (`=?UTF-8?B?<b64>?=`) when non-ASCII; raw when ASCII-
//!   only.
//! - **Body** — `Content-Transfer-Encoding: base64`,
//!   line-wrapped at 76 chars per RFC 2045. Universal-safe
//!   for any UTF-8 payload; loses readability in raw-source
//!   inspectors compared to quoted-printable, but keeps the
//!   substrate simple.
//! - **From / To** — raw header values. Operators can
//!   include display names; we don't parse them; the
//!   CR/LF rejection is the defense.

use base64::prelude::{Engine, BASE64_STANDARD, BASE64_URL_SAFE};
use thiserror::Error;

/// Wrap-width for base64-encoded body chunks. RFC 2045 caps
/// MIME lines at 76 chars including any line-folding
/// whitespace; we choose exactly 76 so wrapped output round-
/// trips through Gmail's MIME parser cleanly.
const BODY_LINE_WIDTH: usize = 76;

#[derive(Debug, Error)]
pub enum MimeError {
    #[error("`{field}` is empty")]
    Empty { field: &'static str },
    #[error("`{field}` contains CR or LF — header-injection rejected")]
    HeaderInjection { field: &'static str },
    #[error("`to` must contain `@`")]
    InvalidTo,
    #[error("`from` must contain `@`")]
    InvalidFrom,
    #[error("`in_reply_to_message_id` must look like `<id@host>` or `id@host`; got {0:?}")]
    InvalidMessageId(String),
}

/// Operator-supplied parameters for one outbound message.
///
/// Used by both `gmail.draft` (Task 6) and `gmail.send` (Task
/// 7). The latter additionally honors [`from`] for send-as
/// aliases; the former ignores it (drafts always use the
/// authenticated user's primary address).
#[derive(Debug, Clone)]
pub struct MimeMessageParams {
    pub to: String,
    pub subject: String,
    pub body_text: String,
    /// RFC 5322 Message-ID of the message being replied to,
    /// for `In-Reply-To` + `References` header threading.
    /// May be supplied with or without angle brackets;
    /// normalized to `<id@host>` form on build.
    pub in_reply_to_message_id: Option<String>,
    /// Optional `From:` override. Only honored by
    /// `gmail.send` for send-as aliases the operator's
    /// Gmail account has authorized. `None` → Google fills
    /// in the primary address.
    pub from: Option<String>,
}

/// Build the RFC 5322 message body. Returns the raw text form
/// (CRLF-delimited per RFC; what `encode_for_api` then
/// base64url-encodes).
///
/// Validation:
/// - `to`, `subject`, `body_text` must be non-empty.
/// - `to`, `from`, `subject`, `in_reply_to_message_id` must
///   not contain CR or LF.
/// - `to` and `from` must contain `@`.
/// - `in_reply_to_message_id`, if present, must contain `@`.
pub fn build_raw(params: &MimeMessageParams) -> Result<String, MimeError> {
    validate(params)?;

    let mut out = String::new();

    if let Some(from) = params.from.as_deref() {
        out.push_str("From: ");
        out.push_str(from);
        out.push_str("\r\n");
    }
    out.push_str("To: ");
    out.push_str(&params.to);
    out.push_str("\r\n");
    out.push_str("Subject: ");
    out.push_str(&encode_subject(&params.subject));
    out.push_str("\r\n");

    if let Some(raw_id) = params.in_reply_to_message_id.as_deref() {
        let normalized = normalize_message_id(raw_id);
        out.push_str("In-Reply-To: ");
        out.push_str(&normalized);
        out.push_str("\r\n");
        out.push_str("References: ");
        out.push_str(&normalized);
        out.push_str("\r\n");
    }

    out.push_str("MIME-Version: 1.0\r\n");
    out.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    out.push_str("Content-Transfer-Encoding: base64\r\n");
    out.push_str("\r\n");
    out.push_str(&encode_body_base64_wrapped(&params.body_text));
    Ok(out)
}

/// Convenience — build the MIME message + base64url-encode it
/// for the Gmail API's `raw` field. Both `gmail.draft` and
/// `gmail.send` call this directly.
pub fn build_for_api(params: &MimeMessageParams) -> Result<String, MimeError> {
    let raw = build_raw(params)?;
    Ok(BASE64_URL_SAFE.encode(raw.as_bytes()))
}

fn validate(params: &MimeMessageParams) -> Result<(), MimeError> {
    check_non_empty("to", &params.to)?;
    check_non_empty("subject", &params.subject)?;
    check_non_empty("body_text", &params.body_text)?;
    check_no_crlf("to", &params.to)?;
    check_no_crlf("subject", &params.subject)?;
    if let Some(from) = &params.from {
        check_non_empty("from", from)?;
        check_no_crlf("from", from)?;
        if !from.contains('@') {
            return Err(MimeError::InvalidFrom);
        }
    }
    if !params.to.contains('@') {
        return Err(MimeError::InvalidTo);
    }
    if let Some(id) = &params.in_reply_to_message_id {
        check_no_crlf("in_reply_to_message_id", id)?;
        let stripped = id.trim_start_matches('<').trim_end_matches('>').trim();
        if stripped.is_empty() || !stripped.contains('@') {
            return Err(MimeError::InvalidMessageId(id.clone()));
        }
    }
    Ok(())
}

fn check_non_empty(field: &'static str, value: &str) -> Result<(), MimeError> {
    if value.trim().is_empty() {
        return Err(MimeError::Empty { field });
    }
    Ok(())
}

fn check_no_crlf(field: &'static str, value: &str) -> Result<(), MimeError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(MimeError::HeaderInjection { field });
    }
    Ok(())
}

/// Wrap a Message-ID in `<>` if not already wrapped. Trims
/// whitespace first; preserves the inner identifier
/// verbatim.
fn normalize_message_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

/// RFC 2047 encoded-word format when non-ASCII; raw when
/// ASCII. Phase 123 ships one encoded-word per Subject —
/// long non-ASCII subjects may exceed RFC 2047's 75-char
/// limit per encoded-word; in practice Gmail accepts the
/// over-long form. A future phase can add proper line-
/// folding if real use surfaces a parser that rejects it.
fn encode_subject(subject: &str) -> String {
    if subject.is_ascii() {
        return subject.to_string();
    }
    let encoded = BASE64_STANDARD.encode(subject.as_bytes());
    format!("=?UTF-8?B?{encoded}?=")
}

/// Base64-encode the body and wrap at 76 chars per RFC 2045.
fn encode_body_base64_wrapped(body: &str) -> String {
    let encoded = BASE64_STANDARD.encode(body.as_bytes());
    if encoded.len() <= BODY_LINE_WIDTH {
        return format!("{encoded}\r\n");
    }
    let mut out = String::with_capacity(encoded.len() + encoded.len() / BODY_LINE_WIDTH * 2);
    let bytes = encoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let end = (i + BODY_LINE_WIDTH).min(bytes.len());
        // Safe: base64 is pure ASCII.
        out.push_str(std::str::from_utf8(&bytes[i..end]).unwrap());
        out.push_str("\r\n");
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MimeMessageParams {
        MimeMessageParams {
            to: "bob@example.com".to_string(),
            subject: "hello".to_string(),
            body_text: "hi bob".to_string(),
            in_reply_to_message_id: None,
            from: None,
        }
    }

    #[test]
    fn build_raw_minimal_message_has_expected_headers_and_body() {
        let raw = build_raw(&sample()).expect("build");
        assert!(raw.contains("To: bob@example.com\r\n"));
        assert!(raw.contains("Subject: hello\r\n"));
        assert!(raw.contains("MIME-Version: 1.0\r\n"));
        assert!(raw.contains("Content-Type: text/plain; charset=UTF-8\r\n"));
        assert!(raw.contains("Content-Transfer-Encoding: base64\r\n"));
        // Body is base64 of "hi bob" = "aGkgYm9i"
        assert!(raw.contains("aGkgYm9i"));
        // No From: header when from is None — Google fills it in.
        assert!(!raw.contains("From:"));
    }

    #[test]
    fn build_raw_with_from_emits_from_header() {
        let mut p = sample();
        p.from = Some("alias@example.com".to_string());
        let raw = build_raw(&p).expect("build");
        assert!(raw.contains("From: alias@example.com\r\n"));
    }

    #[test]
    fn build_raw_subject_ascii_is_unencoded() {
        let raw = build_raw(&sample()).expect("build");
        assert!(raw.contains("Subject: hello\r\n"));
        assert!(!raw.contains("=?UTF-8?B?"));
    }

    #[test]
    fn build_raw_subject_non_ascii_uses_rfc2047_b_encoded_word() {
        let mut p = sample();
        p.subject = "héllo 🌍".to_string();
        let raw = build_raw(&p).expect("build");
        // Encoded-word wrapper present.
        assert!(raw.contains("=?UTF-8?B?"), "{raw}");
        // Raw subject NOT present in the header line — it was
        // replaced by the encoded form.
        assert!(!raw.contains("héllo 🌍\r\n"), "{raw}");
    }

    #[test]
    fn build_raw_with_in_reply_to_adds_threading_headers() {
        let mut p = sample();
        p.in_reply_to_message_id = Some("<msg-id-x@host>".to_string());
        let raw = build_raw(&p).expect("build");
        assert!(raw.contains("In-Reply-To: <msg-id-x@host>\r\n"));
        assert!(raw.contains("References: <msg-id-x@host>\r\n"));
    }

    #[test]
    fn build_raw_wraps_bare_message_id_in_angle_brackets() {
        let mut p = sample();
        p.in_reply_to_message_id = Some("msg-id-bare@host".to_string());
        let raw = build_raw(&p).expect("build");
        assert!(raw.contains("In-Reply-To: <msg-id-bare@host>\r\n"), "{raw}");
        assert!(raw.contains("References: <msg-id-bare@host>\r\n"), "{raw}");
    }

    #[test]
    fn build_raw_rejects_empty_to() {
        let mut p = sample();
        p.to = "".to_string();
        match build_raw(&p) {
            Err(MimeError::Empty { field }) => assert_eq!(field, "to"),
            other => panic!("expected Empty(to); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_empty_subject() {
        let mut p = sample();
        p.subject = "   ".to_string();
        match build_raw(&p) {
            Err(MimeError::Empty { field }) => assert_eq!(field, "subject"),
            other => panic!("expected Empty(subject); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_empty_body() {
        let mut p = sample();
        p.body_text = "".to_string();
        match build_raw(&p) {
            Err(MimeError::Empty { field }) => assert_eq!(field, "body_text"),
            other => panic!("expected Empty(body_text); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_header_injection_in_to() {
        let mut p = sample();
        p.to = "bob@example.com\r\nBcc: eve@example.com".to_string();
        match build_raw(&p) {
            Err(MimeError::HeaderInjection { field }) => assert_eq!(field, "to"),
            other => panic!("expected HeaderInjection(to); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_header_injection_in_subject() {
        let mut p = sample();
        p.subject = "hi\nBcc: eve@example.com".to_string();
        match build_raw(&p) {
            Err(MimeError::HeaderInjection { field }) => assert_eq!(field, "subject"),
            other => panic!("expected HeaderInjection(subject); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_header_injection_in_from() {
        let mut p = sample();
        p.from = Some("alias@example.com\r\nBcc: eve@example.com".to_string());
        match build_raw(&p) {
            Err(MimeError::HeaderInjection { field }) => assert_eq!(field, "from"),
            other => panic!("expected HeaderInjection(from); got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_header_injection_in_message_id() {
        let mut p = sample();
        p.in_reply_to_message_id = Some("id@host\r\nBcc: eve".to_string());
        match build_raw(&p) {
            Err(MimeError::HeaderInjection { field }) => {
                assert_eq!(field, "in_reply_to_message_id");
            }
            other => panic!("expected HeaderInjection; got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_to_without_at_sign() {
        let mut p = sample();
        p.to = "bob".to_string();
        match build_raw(&p) {
            Err(MimeError::InvalidTo) => {}
            other => panic!("expected InvalidTo; got {other:?}"),
        }
    }

    #[test]
    fn build_raw_rejects_message_id_without_at_sign() {
        let mut p = sample();
        p.in_reply_to_message_id = Some("<bare-id>".to_string());
        match build_raw(&p) {
            Err(MimeError::InvalidMessageId(_)) => {}
            other => panic!("expected InvalidMessageId; got {other:?}"),
        }
    }

    #[test]
    fn build_raw_wraps_long_body_at_76_chars() {
        let mut p = sample();
        // 200-byte body → base64 ~272 chars → wraps to 4 lines.
        p.body_text = "x".repeat(200);
        let raw = build_raw(&p).expect("build");
        // Take the body section (after the blank-line separator).
        let body_section = raw.split("\r\n\r\n").nth(1).expect("body section");
        for line in body_section.lines() {
            if line.is_empty() {
                continue;
            }
            assert!(
                line.len() <= BODY_LINE_WIDTH,
                "wrapped line exceeds {BODY_LINE_WIDTH}: {} ({line})",
                line.len()
            );
        }
    }

    #[test]
    fn build_for_api_returns_base64url_of_raw() {
        let s = build_for_api(&sample()).expect("build");
        // Base64url alphabet — no `+` or `/` characters.
        assert!(!s.contains('+'), "{s}");
        assert!(!s.contains('/'), "{s}");
        // Decoding it must round-trip back to the raw form.
        let decoded = BASE64_URL_SAFE.decode(&s).expect("decode");
        let text = String::from_utf8(decoded).expect("utf8");
        assert!(text.contains("To: bob@example.com"));
        assert!(text.contains("Subject: hello"));
    }

    #[test]
    fn normalize_message_id_idempotent_on_already_wrapped() {
        assert_eq!(normalize_message_id("<id@host>"), "<id@host>");
        assert_eq!(normalize_message_id("  <id@host>  "), "<id@host>");
        assert_eq!(normalize_message_id("id@host"), "<id@host>");
    }
}
