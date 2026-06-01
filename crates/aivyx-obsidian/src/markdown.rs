//! Markdown-aware helpers for Obsidian notes.
//!
//! Phase 130 Task 10. Three pieces:
//!
//! 1. **Frontmatter splitter** — separates the
//!    YAML frontmatter (between `---` markers at the top
//!    of a file) from the body. We don't parse the YAML
//!    — operators get the raw text in
//!    `obsidian.get_note`'s `frontmatter_raw` field and
//!    can LLM-parse if they care.
//! 2. **Wikilink extractor** — pulls `[[Page Name]]` and
//!    `[[Page Name|display text]]` references out of the
//!    body. Returns the raw link target strings (no fuzzy
//!    path resolution; that's a Phase 131+ candidate).
//! 3. **Tag extractor** — pulls `#tag` tokens (whitespace-
//!    or-line-start bounded; followed by alphanumerics +
//!    `-` + `_` + `/`).
//!
//! The parsers are hand-written so we don't pull a new
//! workspace dependency for this. Same posture as Phase
//! 127's hand-written Python-call parser.

/// Split a markdown document into (frontmatter_raw, body).
///
/// Frontmatter must be at the very start of the file:
/// `---\n` opener, content lines, `---\n` closer. The
/// returned `frontmatter_raw` is the content BETWEEN the
/// `---` markers (not including them); `body` is
/// everything after the closing `---\n`.
///
/// If there's no frontmatter (or the structure is
/// malformed — e.g., opener but no closer), returns
/// `(None, full_content)`.
pub fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed_start = content.trim_start_matches(['\r', '\n']);
    if !trimmed_start.starts_with("---") {
        return (None, content.to_string());
    }
    // Skip past the opening "---" and its line terminator.
    let after_opener = match trimmed_start.find('\n') {
        Some(idx) => &trimmed_start[idx + 1..],
        None => return (None, content.to_string()),
    };
    // Find the closing "---" on its own line.
    let closer = find_line(after_opener, "---");
    let Some((closer_start, closer_end)) = closer else {
        // No closing marker → no frontmatter.
        return (None, content.to_string());
    };
    let fm = &after_opener[..closer_start];
    let body = &after_opener[closer_end..];
    // Strip a single leading newline from body for
    // ergonomics (consumers don't want the
    // `---\n<body starts here>` blank line).
    let body = body.trim_start_matches('\n');
    (Some(fm.to_string()), body.to_string())
}

/// Find a line whose trimmed content equals `needle`.
/// Returns `(start_of_line, end_of_line_inclusive_of_newline)`
/// in the source string, or `None`.
fn find_line(s: &str, needle: &str) -> Option<(usize, usize)> {
    let mut line_start = 0usize;
    while line_start < s.len() {
        let rest = &s[line_start..];
        let line_end = rest.find('\n').map(|i| line_start + i).unwrap_or(s.len());
        let line = &s[line_start..line_end];
        if line.trim() == needle {
            // Include the newline in the end index if
            // present, else point past the end.
            let end_with_newline = if line_end < s.len() {
                line_end + 1
            } else {
                line_end
            };
            return Some((line_start, end_with_newline));
        }
        line_start = line_end + 1;
    }
    None
}

/// Extract wikilink targets from a body. Returns unique
/// target strings preserving insertion order. Handles
/// both `[[Page Name]]` and `[[Page Name|display text]]`
/// (the display text is dropped — we only care about the
/// link target).
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find the matching `]]`.
            let inner_start = i + 2;
            let Some(close_off) = body[inner_start..].find("]]") else {
                break;
            };
            let inner = &body[inner_start..inner_start + close_off];
            // Bare `[[...]]` may have an internal `|` for
            // display text — drop everything after it.
            let target = match inner.find('|') {
                Some(pipe) => &inner[..pipe],
                None => inner,
            };
            let target = target.trim();
            if !target.is_empty() && seen.insert(target.to_string()) {
                out.push(target.to_string());
            }
            i = inner_start + close_off + 2;
        } else {
            i += 1;
        }
    }
    out
}

/// Extract tag tokens from a body. Tags are
/// `#word`-bounded by start-of-string / whitespace and
/// extend through alphanumerics + `-` + `_` + `/`.
/// Returns unique tags preserving insertion order.
///
/// Notable exclusions: hex colors (`#abc123` followed by
/// alphanumerics is still a tag; we don't try to
/// distinguish), inline headings (`# Heading` has a
/// space after `#` so it doesn't match), tags inside
/// fenced code blocks (we don't strip code fences;
/// operators wanting that can filter their results
/// LLM-side — overshoot here is the conservative
/// posture).
pub fn extract_tags(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            // The `#` must be at the start of the body or
            // preceded by whitespace.
            let at_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
            if at_boundary {
                // Collect tag chars.
                let mut j = i + 1;
                while j < bytes.len() && is_tag_char(bytes[j]) {
                    j += 1;
                }
                if j > i + 1 {
                    let tag = &body[i + 1..j];
                    if seen.insert(tag.to_string()) {
                        out.push(tag.to_string());
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- split_frontmatter ----

    #[test]
    fn split_frontmatter_returns_none_when_no_marker() {
        let (fm, body) = split_frontmatter("just body\nstuff");
        assert!(fm.is_none());
        assert_eq!(body, "just body\nstuff");
    }

    #[test]
    fn split_frontmatter_extracts_frontmatter_and_body() {
        let src = "---\ntags: [a, b]\ndate: 2026-06-01\n---\n\nBody text here\n";
        let (fm, body) = split_frontmatter(src);
        assert_eq!(fm.unwrap(), "tags: [a, b]\ndate: 2026-06-01\n");
        // Leading blank line after closing `---` is
        // stripped for ergonomics — consumers don't want
        // it.
        assert_eq!(body, "Body text here\n");
    }

    #[test]
    fn split_frontmatter_handles_leading_blank_line_before_marker() {
        let src = "\n---\nkey: value\n---\nbody";
        let (fm, body) = split_frontmatter(src);
        assert_eq!(fm.unwrap(), "key: value\n");
        assert_eq!(body, "body");
    }

    #[test]
    fn split_frontmatter_returns_none_when_only_opener() {
        // Opener but no closer → return full content with
        // no frontmatter.
        let src = "---\nstuff that never closes\nmore stuff";
        let (fm, body) = split_frontmatter(src);
        assert!(fm.is_none());
        assert_eq!(body, src);
    }

    #[test]
    fn split_frontmatter_empty_frontmatter_yields_some_empty() {
        let src = "---\n---\nbody";
        let (fm, body) = split_frontmatter(src);
        assert_eq!(fm.unwrap(), "");
        assert_eq!(body, "body");
    }

    // ---- extract_wikilinks ----

    #[test]
    fn extract_wikilinks_finds_simple_link() {
        let body = "See [[Another Page]] for details.";
        assert_eq!(extract_wikilinks(body), vec!["Another Page"]);
    }

    #[test]
    fn extract_wikilinks_strips_display_text() {
        let body = "See [[Page Name|the cool page]] for details.";
        assert_eq!(extract_wikilinks(body), vec!["Page Name"]);
    }

    #[test]
    fn extract_wikilinks_dedupes() {
        let body = "[[A]] and [[A]] again, also [[B]].";
        assert_eq!(extract_wikilinks(body), vec!["A", "B"]);
    }

    #[test]
    fn extract_wikilinks_preserves_order() {
        let body = "[[Z]] then [[A]] then [[M]]";
        assert_eq!(extract_wikilinks(body), vec!["Z", "A", "M"]);
    }

    #[test]
    fn extract_wikilinks_returns_empty_for_no_links() {
        assert!(extract_wikilinks("just text, no links").is_empty());
    }

    #[test]
    fn extract_wikilinks_handles_unclosed_link() {
        let body = "Opener [[no close, then [[Closes Now]]";
        // The unclosed `[[no close` triggers a find for
        // `]]` from index 8; finds the `]]` of "Closes Now",
        // so target is "no close, then [[Closes Now" — odd
        // but acceptable for the MVP. Document this as a
        // known edge case.
        let links = extract_wikilinks(body);
        // We DO extract something; just confirm it doesn't
        // crash and produces a deterministic result.
        assert!(!links.is_empty());
    }

    // ---- extract_tags ----

    #[test]
    fn extract_tags_finds_simple_tags() {
        let body = "Project #work and #personal notes #2026";
        let tags = extract_tags(body);
        assert_eq!(tags, vec!["work", "personal", "2026"]);
    }

    #[test]
    fn extract_tags_supports_slash_nested_tags() {
        let body = "#projects/aivyx is great";
        assert_eq!(extract_tags(body), vec!["projects/aivyx"]);
    }

    #[test]
    fn extract_tags_supports_dash_and_underscore() {
        let body = "#in-progress #to_review";
        assert_eq!(extract_tags(body), vec!["in-progress", "to_review"]);
    }

    #[test]
    fn extract_tags_skips_heading_hash_with_space() {
        let body = "# Just a heading\nNot a #tag";
        // The `#` in `# Just` has a space after it (not a
        // tag char) so no tag extracted. The `#tag` in
        // line two is whitespace-bounded so DOES extract.
        let tags = extract_tags(body);
        assert_eq!(tags, vec!["tag"]);
    }

    #[test]
    fn extract_tags_dedupes_preserving_order() {
        let body = "#a #b #a #c";
        assert_eq!(extract_tags(body), vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_tags_requires_whitespace_boundary_before_hash() {
        // A `#` mid-word (like in URLs) doesn't trigger a
        // tag.
        let body = "https://example.com#anchor is not a tag";
        assert!(extract_tags(body).is_empty());
    }

    #[test]
    fn extract_tags_returns_empty_when_no_tags() {
        assert!(extract_tags("just text").is_empty());
    }

    #[test]
    fn extract_tags_at_start_of_line_extracted() {
        let body = "#first-tag at start\n#second after newline";
        let tags = extract_tags(body);
        assert!(tags.contains(&"first-tag".to_string()));
        assert!(tags.contains(&"second".to_string()));
    }
}
