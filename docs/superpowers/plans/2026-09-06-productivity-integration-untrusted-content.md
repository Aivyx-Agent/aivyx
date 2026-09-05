# Close the Productivity-Integration Untrusted-Content Coverage Gap — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flag 28 productivity-integration tools' output as untrusted
(`output_is_untrusted() -> true`) across 9 crates so Chapter Bulwark's
fencing and Chapter Picket's injection scan — both already wired into a
single, centralized call site — cover them, closing a real, currently
open prompt-injection surface.

**Architecture:** Each of the 28 tools gets exactly one addition to its
existing `impl Tool for X` block: a one-line trait method override plus a
tool-specific comment explaining why that tool's content is externally
authored. No turn-loop, no shared-helper, no new-abstraction changes
anywhere — the existing `Agent::run_tool_call` call site
(`crates/aivyx-core/src/agent.rs:1410`) already applies both mechanisms
uniformly to any tool returning `true` here.

**Tech Stack:** Rust, existing `aivyx_core::Tool` trait, `cargo test`.

## Global Constraints

- Full design: `docs/superpowers/specs/2026-09-06-productivity-integration-untrusted-content-design.md`.
- Do NOT modify any tool listed in the design spec's "left at default
  `false`" list (every `*Send`, `*Draft`, `*Create*`, `*Update*`,
  `*Delete*`, `*Archive*`, `*Activate*`, `*Deactivate*`, `*Execute*`
  tool across all 9 crates, plus every `aivyx-toolkit` tool other than
  `WebSearch`). None of those tools appear in this plan's tasks.
- No changes anywhere in `crates/aivyx-core/src/agent.rs` or any other
  turn-loop code.
- `docs/THREAT_MODEL.md`'s fix (Task 9) corrects exactly two claims,
  verbatim as specified — no other edits to that document.

---

## Task 1: aivyx-gmail (2 tools)

**Files:**
- Modify: `crates/aivyx-gmail/src/tools/search.rs` (`GmailSearch`)
- Modify: `crates/aivyx-gmail/src/tools/read.rs` (`GmailRead`)

**Interfaces:** Consumes: nothing. Produces: nothing (independent of
every other task in this plan).

- [ ] **Step 1: Add the override to `GmailSearch`**

In `crates/aivyx-gmail/src/tools/search.rs`, find:

```rust
    fn name(&self) -> &str {
        "gmail.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "gmail.search"
    }

    // Chapter Picket follow-up (Finding 3) — search results include
    // message snippets and subjects, externally authored email
    // content that may carry a prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test asserting the override, in the same file**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn gmail_search_output_is_untrusted_for_bulwark() {
        let client = std::sync::Arc::new(crate::GmailClient::new(
            reqwest::Client::new(),
            crate::OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            crate::TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        let tool = GmailSearch::new(client);
        assert!(tool.output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `GmailRead`**

In `crates/aivyx-gmail/src/tools/read.rs`, find:

```rust
    fn name(&self) -> &str {
        "gmail.read"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "gmail.read"
    }

    // Chapter Picket follow-up (Finding 3) — the email body and
    // headers are externally authored content that may carry a
    // prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test asserting the override, in the same file**

Find:

```rust
mod tests {
    use super::*;
    use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};
```

Replace with:

```rust
mod tests {
    use super::*;
    use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};

    #[test]
    fn gmail_read_output_is_untrusted_for_bulwark() {
        let client = std::sync::Arc::new(crate::GmailClient::new(
            reqwest::Client::new(),
            crate::OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            crate::TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        let tool = GmailRead::new(client);
        assert!(tool.output_is_untrusted());
    }
```

- [ ] **Step 5: Run the crate's tests**

Run: `cargo test -p aivyx-gmail`
Expected: all tests pass, including the two new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-gmail/src/tools/search.rs crates/aivyx-gmail/src/tools/read.rs
git commit -m "fix(aivyx-gmail): flag search/read output as untrusted (Chapter Picket Finding 3)

GmailSearch and GmailRead return externally-authored email content
that could carry a prompt-injection payload; neither was fenced by
Bulwark or scanned by Picket before this change. GmailSend/GmailDraft
are unaffected -- their output is an internally-generated confirmation."
```

---

## Task 2: aivyx-calendar (4 tools)

**Files:**
- Modify: `crates/aivyx-calendar/src/tools/get_event.rs` (`CalendarGetEvent`)
- Modify: `crates/aivyx-calendar/src/tools/list_events.rs` (`CalendarListEvents`)
- Modify: `crates/aivyx-calendar/src/tools/upcoming.rs` (`CalendarUpcoming`)
- Modify: `crates/aivyx-calendar/src/tools/list_calendars.rs` (`CalendarListCalendars`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `CalendarGetEvent`**

In `crates/aivyx-calendar/src/tools/get_event.rs`, find:

```rust
    fn name(&self) -> &str {
        "calendar.get_event"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "calendar.get_event"
    }

    // Chapter Picket follow-up (Finding 3) — an event's title,
    // description, and attendee-supplied fields are externally
    // authored (e.g. from an invite sent by someone else) and may
    // carry a prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn calendar_get_event_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `CalendarListEvents`**

In `crates/aivyx-calendar/src/tools/list_events.rs`, find:

```rust
    fn name(&self) -> &str {
        "calendar.list_events"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "calendar.list_events"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // calendar.get_event: a list of externally authored event
    // content.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::super::event_summary;
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::super::event_summary;
    use super::*;

    #[test]
    fn calendar_list_events_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `CalendarUpcoming`**

In `crates/aivyx-calendar/src/tools/upcoming.rs`, find:

```rust
    fn name(&self) -> &str {
        "calendar.upcoming"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "calendar.upcoming"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // calendar.get_event: a list of externally authored event
    // content.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a `make_tool()` helper and a test in the same file**

`upcoming.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
    use chrono::TimeZone;
```

Replace with:

```rust
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_tool() -> CalendarUpcoming {
        use crate::oauth::TokenSet;
        use crate::{CalendarClient, OAuthConfig};
        use std::sync::Arc;
        let client = Arc::new(CalendarClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        CalendarUpcoming::new(client)
    }

    #[test]
    fn calendar_upcoming_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Add the override to `CalendarListCalendars`**

In `crates/aivyx-calendar/src/tools/list_calendars.rs`, find:

```rust
    fn name(&self) -> &str {
        "calendar.list_calendars"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "calendar.list_calendars"
    }

    // Chapter Picket follow-up (Finding 3) — calendar names/summaries
    // can be set by whoever shares a calendar with the operator;
    // externally authored metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 8: Add a `make_tool()` helper and a test in the same file**

`list_calendars.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> CalendarListCalendars {
        use crate::oauth::TokenSet;
        use crate::{CalendarClient, OAuthConfig};
        use std::sync::Arc;
        let client = Arc::new(CalendarClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        CalendarListCalendars::new(client)
    }

    #[test]
    fn calendar_list_calendars_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

**Note:** Step 2's and Step 4's insertions both add a `mod tests` block
member ahead of where each file's own `make_tool()` already lives
further down — Rust does not require declaration order within a
module, so this compiles regardless of where `make_tool` is defined
relative to its callers.

- [ ] **Step 9: Run the crate's tests**

Run: `cargo test -p aivyx-calendar`
Expected: all tests pass, including the four new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-calendar/src/tools/get_event.rs crates/aivyx-calendar/src/tools/list_events.rs crates/aivyx-calendar/src/tools/upcoming.rs crates/aivyx-calendar/src/tools/list_calendars.rs
git commit -m "fix(aivyx-calendar): flag get_event/list_events/upcoming/list_calendars output as untrusted (Chapter Picket Finding 3)

All four return externally-authored event/calendar content (titles,
descriptions, attendee fields, calendar names set by whoever shared
them) that could carry a prompt-injection payload. CreateEvent/
UpdateEvent/DeleteEvent are unaffected -- their output is an
internally-generated confirmation."
```

---

## Task 3: aivyx-drive (8 tools)

**Files:**
- Modify: `crates/aivyx-drive/src/tools/list_drives.rs` (`DriveListDrives`)
- Modify: `crates/aivyx-drive/src/tools/list_folder.rs` (`DriveListFolder`)
- Modify: `crates/aivyx-drive/src/tools/search.rs` (`DriveSearch`)
- Modify: `crates/aivyx-drive/src/tools/get_metadata.rs` (`DriveGetMetadata`)
- Modify: `crates/aivyx-drive/src/tools/download_file.rs` (`DriveDownloadFile`)
- Modify: `crates/aivyx-drive/src/tools/recent_changes.rs` (`DriveRecentChanges`)
- Modify: `crates/aivyx-drive/src/tools/recent_files.rs` (`DriveRecentFiles`)
- Modify: `crates/aivyx-drive/src/tools/recent_activity.rs` (`DriveRecentActivity`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `DriveListDrives`**

In `crates/aivyx-drive/src/tools/list_drives.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.list_drives"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.list_drives"
    }

    // Chapter Picket follow-up (Finding 3) — shared-drive names can
    // be set by any collaborator; externally authored metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a `make_tool()` helper and a test in the same file**

`list_drives.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> DriveListDrives {
        use crate::{DriveClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(DriveClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        DriveListDrives::new(client)
    }

    #[test]
    fn drive_list_drives_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `DriveListFolder`**

In `crates/aivyx-drive/src/tools/list_folder.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.list_folder"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.list_folder"
    }

    // Chapter Picket follow-up (Finding 3) — file/folder names in
    // Drive are externally authored and may carry a prompt-injection
    // payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn drive_list_folder_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `DriveSearch`**

In `crates/aivyx-drive/src/tools/search.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.search"
    }

    // Chapter Picket follow-up (Finding 3) — search results (file
    // names/snippets) are externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn drive_search_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Add the override to `DriveGetMetadata`**

In `crates/aivyx-drive/src/tools/get_metadata.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.get_metadata"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.get_metadata"
    }

    // Chapter Picket follow-up (Finding 3) — file metadata (name,
    // owner, description) is externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 8: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn drive_get_metadata_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 9: Add the override to `DriveDownloadFile`**

In `crates/aivyx-drive/src/tools/download_file.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.download_file"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.download_file"
    }

    // Chapter Picket follow-up (Finding 3) — the actual file content
    // is externally authored, the same rationale as fs.read.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 10: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn drive_download_file_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 11: Add the override to `DriveRecentChanges`**

In `crates/aivyx-drive/src/tools/recent_changes.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.recent_changes"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.recent_changes"
    }

    // Chapter Picket follow-up (Finding 3) — a list of recently
    // changed files' externally authored names/metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 12: Add a `make_tool()` helper and a test in the same file**

`recent_changes.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> DriveRecentChanges {
        use crate::{DriveClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(DriveClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        DriveRecentChanges::new(client)
    }

    #[test]
    fn drive_recent_changes_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 13: Add the override to `DriveRecentFiles`**

In `crates/aivyx-drive/src/tools/recent_files.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.recent_files"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.recent_files"
    }

    // Chapter Picket follow-up (Finding 3) — a list of recently
    // touched files' externally authored names/metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 14: Add a `make_tool()` helper and a test in the same file**

`recent_files.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> DriveRecentFiles {
        use crate::{DriveClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(DriveClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        DriveRecentFiles::new(client)
    }

    #[test]
    fn drive_recent_files_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 15: Add the override to `DriveRecentActivity`**

In `crates/aivyx-drive/src/tools/recent_activity.rs`, find:

```rust
    fn name(&self) -> &str {
        "drive.recent_activity"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "drive.recent_activity"
    }

    // Chapter Picket follow-up (Finding 3) — a list of recent
    // activity referencing externally authored file names/metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 16: Add a test reusing the file's existing `make_test_client()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn drive_recent_activity_output_is_untrusted_for_bulwark() {
        let tool = DriveRecentActivity::new(make_test_client());
        assert!(tool.output_is_untrusted());
    }
```

- [ ] **Step 17: Run the crate's tests**

Run: `cargo test -p aivyx-drive`
Expected: all tests pass, including the eight new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 18: Commit**

```bash
git add crates/aivyx-drive/src/tools/list_drives.rs crates/aivyx-drive/src/tools/list_folder.rs crates/aivyx-drive/src/tools/search.rs crates/aivyx-drive/src/tools/get_metadata.rs crates/aivyx-drive/src/tools/download_file.rs crates/aivyx-drive/src/tools/recent_changes.rs crates/aivyx-drive/src/tools/recent_files.rs crates/aivyx-drive/src/tools/recent_activity.rs
git commit -m "fix(aivyx-drive): flag 8 read-shaped tools' output as untrusted (Chapter Picket Finding 3)

list_drives/list_folder/search/get_metadata/download_file/
recent_changes/recent_files/recent_activity all return externally
authored file/folder names, metadata, or content that could carry a
prompt-injection payload. create_folder/upload_file/delete_file are
unaffected -- their output is an internally-generated confirmation."
```

---

## Task 4: aivyx-contacts (3 tools)

**Files:**
- Modify: `crates/aivyx-contacts/src/tools/get.rs` (`ContactsGet`)
- Modify: `crates/aivyx-contacts/src/tools/search.rs` (`ContactsSearch`)
- Modify: `crates/aivyx-contacts/src/tools/list.rs` (`ContactsList`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `ContactsGet`**

In `crates/aivyx-contacts/src/tools/get.rs`, find:

```rust
    fn name(&self) -> &str {
        "contacts.get"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "contacts.get"
    }

    // Chapter Picket follow-up (Finding 3) — contact fields (name,
    // notes, organization) are externally authored and may carry a
    // prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a `make_tool()` helper and a test in the same file**

`get.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> ContactsGet {
        use crate::{ContactsClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(ContactsClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        ContactsGet::new(client)
    }

    #[test]
    fn contacts_get_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `ContactsSearch`**

In `crates/aivyx-contacts/src/tools/search.rs`, find:

```rust
    fn name(&self) -> &str {
        "contacts.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "contacts.search"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // contacts.get: externally authored contact fields.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a `make_tool()` helper and a test in the same file**

`search.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> ContactsSearch {
        use crate::{ContactsClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(ContactsClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        ContactsSearch::new(client)
    }

    #[test]
    fn contacts_search_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `ContactsList`**

In `crates/aivyx-contacts/src/tools/list.rs`, find:

```rust
    fn name(&self) -> &str {
        "contacts.list"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "contacts.list"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // contacts.get: externally authored contact fields.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a `make_tool()` helper and a test in the same file**

`list.rs` has no existing tool-construction test. Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    fn make_tool() -> ContactsList {
        use crate::{ContactsClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(ContactsClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        ContactsList::new(client)
    }

    #[test]
    fn contacts_list_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Run the crate's tests**

Run: `cargo test -p aivyx-contacts`
Expected: all tests pass, including the three new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-contacts/src/tools/get.rs crates/aivyx-contacts/src/tools/search.rs crates/aivyx-contacts/src/tools/list.rs
git commit -m "fix(aivyx-contacts): flag get/search/list output as untrusted (Chapter Picket Finding 3)

All three return externally-authored contact fields (name, notes,
organization) that could carry a prompt-injection payload.
create/update/delete are unaffected -- their output is an
internally-generated confirmation."
```

---

## Task 5: aivyx-notion (3 tools)

**Files:**
- Modify: `crates/aivyx-notion/src/tools/search.rs` (`NotionSearch`)
- Modify: `crates/aivyx-notion/src/tools/get_page.rs` (`NotionGetPage`)
- Modify: `crates/aivyx-notion/src/tools/list_database.rs` (`NotionListDatabase`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `NotionSearch`**

In `crates/aivyx-notion/src/tools/search.rs`, find:

```rust
    fn name(&self) -> &str {
        "notion.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "notion.search"
    }

    // Chapter Picket follow-up (Finding 3) — search results surface
    // page content editable by any workspace collaborator;
    // externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn notion_search_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `NotionGetPage`**

In `crates/aivyx-notion/src/tools/get_page.rs`, find:

```rust
    fn name(&self) -> &str {
        "notion.get_page"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "notion.get_page"
    }

    // Chapter Picket follow-up (Finding 3) — page content is
    // editable by any workspace collaborator; externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn notion_get_page_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `NotionListDatabase`**

In `crates/aivyx-notion/src/tools/list_database.rs`, find:

```rust
    fn name(&self) -> &str {
        "notion.list_database"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "notion.list_database"
    }

    // Chapter Picket follow-up (Finding 3) — database entries are
    // editable by any workspace collaborator; externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn notion_list_database_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Run the crate's tests**

Run: `cargo test -p aivyx-notion`
Expected: all tests pass, including the three new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-notion/src/tools/search.rs crates/aivyx-notion/src/tools/get_page.rs crates/aivyx-notion/src/tools/list_database.rs
git commit -m "fix(aivyx-notion): flag search/get_page/list_database output as untrusted (Chapter Picket Finding 3)

All three return page/database content editable by any workspace
collaborator, which could carry a prompt-injection payload.
create_page/update_page_properties/append_blocks/archive_page are
unaffected -- their output is an internally-generated confirmation."
```

---

## Task 6: aivyx-obsidian (3 tools)

**Files:**
- Modify: `crates/aivyx-obsidian/src/tools/get_note.rs` (`ObsidianGetNote`)
- Modify: `crates/aivyx-obsidian/src/tools/search.rs` (`ObsidianSearch`)
- Modify: `crates/aivyx-obsidian/src/tools/list_folder.rs` (`ObsidianListFolder`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `ObsidianGetNote`**

In `crates/aivyx-obsidian/src/tools/get_note.rs`, find:

```rust
    fn name(&self) -> &str {
        "obsidian.get_note"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "obsidian.get_note"
    }

    // Chapter Picket follow-up (Finding 3) — note content could have
    // been written by anyone with vault access or synced from
    // elsewhere; externally authored, the same rationale as fs.read.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn obsidian_get_note_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `ObsidianSearch`**

In `crates/aivyx-obsidian/src/tools/search.rs`, find:

```rust
    fn name(&self) -> &str {
        "obsidian.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "obsidian.search"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // obsidian.get_note: externally authored vault content.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn obsidian_search_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `ObsidianListFolder`**

In `crates/aivyx-obsidian/src/tools/list_folder.rs`, find:

```rust
    fn name(&self) -> &str {
        "obsidian.list_folder"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "obsidian.list_folder"
    }

    // Chapter Picket follow-up (Finding 3) — filenames in the vault
    // are externally authored, the same rationale as fs.read.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn obsidian_list_folder_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Run the crate's tests**

Run: `cargo test -p aivyx-obsidian`
Expected: all tests pass, including the three new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-obsidian/src/tools/get_note.rs crates/aivyx-obsidian/src/tools/search.rs crates/aivyx-obsidian/src/tools/list_folder.rs
git commit -m "fix(aivyx-obsidian): flag get_note/search/list_folder output as untrusted (Chapter Picket Finding 3)

All three return vault note content/filenames that could have been
written by anyone with vault access, the same rationale as fs.read.
create_note/update_note/delete_note are unaffected -- their output is
an internally-generated confirmation."
```

---

## Task 7: aivyx-n8n (4 tools)

**Files:**
- Modify: `crates/aivyx-n8n/src/tools/get_workflow.rs` (`N8nGetWorkflow`)
- Modify: `crates/aivyx-n8n/src/tools/list_workflows.rs` (`N8nListWorkflows`)
- Modify: `crates/aivyx-n8n/src/tools/get_execution.rs` (`N8nGetExecution`)
- Modify: `crates/aivyx-n8n/src/tools/list_executions.rs` (`N8nListExecutions`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `N8nGetWorkflow`**

In `crates/aivyx-n8n/src/tools/get_workflow.rs`, find:

```rust
    fn name(&self) -> &str {
        "n8n.get_workflow"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "n8n.get_workflow"
    }

    // Chapter Picket follow-up (Finding 3) — a workflow definition
    // could have been created by any team member with n8n access;
    // externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn n8n_get_workflow_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 3: Add the override to `N8nListWorkflows`**

In `crates/aivyx-n8n/src/tools/list_workflows.rs`, find:

```rust
    fn name(&self) -> &str {
        "n8n.list_workflows"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "n8n.list_workflows"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // n8n.get_workflow: externally authored workflow definitions.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 4: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn n8n_list_workflows_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 5: Add the override to `N8nGetExecution`**

In `crates/aivyx-n8n/src/tools/get_execution.rs`, find:

```rust
    fn name(&self) -> &str {
        "n8n.get_execution"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "n8n.get_execution"
    }

    // Chapter Picket follow-up (Finding 3) — execution results can
    // include arbitrary output from external systems/webhooks the
    // workflow touched; externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 6: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn n8n_get_execution_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 7: Add the override to `N8nListExecutions`**

In `crates/aivyx-n8n/src/tools/list_executions.rs`, find:

```rust
    fn name(&self) -> &str {
        "n8n.list_executions"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "n8n.list_executions"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // n8n.get_execution: externally authored execution results.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 8: Add a test reusing the file's existing `make_tool()` helper**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn n8n_list_executions_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
```

- [ ] **Step 9: Run the crate's tests**

Run: `cargo test -p aivyx-n8n`
Expected: all tests pass, including the four new
`*_output_is_untrusted_for_bulwark` tests.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-n8n/src/tools/get_workflow.rs crates/aivyx-n8n/src/tools/list_workflows.rs crates/aivyx-n8n/src/tools/get_execution.rs crates/aivyx-n8n/src/tools/list_executions.rs
git commit -m "fix(aivyx-n8n): flag get_workflow/list_workflows/get_execution/list_executions output as untrusted (Chapter Picket Finding 3)

All four return externally-authored workflow definitions or execution
results (which can include arbitrary output from external systems the
workflow touched) that could carry a prompt-injection payload.
create/update/activate/deactivate/delete/execute are unaffected --
their output is an internally-generated confirmation."
```

---

## Task 8: aivyx-toolkit (1 tool)

**Files:**
- Modify: `crates/aivyx-toolkit/src/tools/web_search.rs` (`WebSearch`)

**Interfaces:** Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Add the override to `WebSearch`**

In `crates/aivyx-toolkit/src/tools/web_search.rs`, find the tool's
`name()` method. First confirm its exact surrounding text:

Run: `grep -n -A3 'fn name(&self) -> &str' crates/aivyx-toolkit/src/tools/web_search.rs`
Expected: shows `fn name(&self) -> &str {` followed by `"web.search"` and
a closing brace.

Find:

```rust
    fn name(&self) -> &str {
        "web.search"
    }
```

Replace with:

```rust
    fn name(&self) -> &str {
        "web.search"
    }

    // Chapter Picket follow-up (Finding 3) — real web search results
    // are externally authored, the same rationale as web.fetch.
    fn output_is_untrusted(&self) -> bool {
        true
    }
```

- [ ] **Step 2: Add a test in the same file**

Find:

```rust
mod tests {
    use super::*;
```

Replace with:

```rust
mod tests {
    use super::*;

    #[test]
    fn web_search_output_is_untrusted_for_bulwark() {
        let tool = WebSearch::new(Client::new(), "BSA-test".to_string());
        assert!(tool.output_is_untrusted());
    }
```

- [ ] **Step 3: Run the crate's tests**

Run: `cargo test -p aivyx-toolkit`
Expected: all tests pass, including the new
`web_search_output_is_untrusted_for_bulwark` test.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-toolkit/src/tools/web_search.rs
git commit -m "fix(aivyx-toolkit): flag web.search output as untrusted (Chapter Picket Finding 3)

WebSearch is a genuine HTTP-backed external search tool with the same
untrusted-content shape as web.fetch, but had no output_is_untrusted
override -- a ninth real gap found alongside the 8 productivity
integrations during this phase's grounding."
```

---

## Task 9: Correct docs/THREAT_MODEL.md's stale §5.3

**Files:**
- Modify: `docs/THREAT_MODEL.md`

**Interfaces:** Consumes: nothing (independent of Tasks 1-8; can run in
any order relative to them). Produces: nothing.

- [ ] **Step 1: Correct the two stale claims**

Find this exact block in `docs/THREAT_MODEL.md`:

```markdown
**Updated post-Phase-180** (this section's own "last reviewed" line at
the top of the document still reads "Phase 180 exit" and was not bumped
for this edit — a real staleness this correction closes, not a new
phase). Chapter Bulwark added real prompt-injection resistance since
Phase 180: fetched, parsed, and tool-process content is fenced as
untrusted data at every ingress (web fetches, file reads, MCP tool
output, operator-provided context files), so the model sees that
content marked as data, not as instructions it should follow. This is
a structural mitigation, not a pattern-matching scanner — Aivyx still
has no content-level scanner for prompt-injection *payloads* the way
Hermes Agent's Tirith does; Bulwark's fencing works by changing how
untrusted content is presented to the model, not by detecting and
blocking malicious patterns within it.
```

Replace with:

```markdown
**Updated post-Phase-180, corrected again for Phase 197's own
follow-up work** (this section's own "last reviewed" line at the top
of the document still reads "Phase 180 exit" — out of scope for this
correction; only the two factual claims below, which this phase's own
work directly contradicts, are being fixed here). Chapter Bulwark
added real prompt-injection resistance since Phase 180: fetched,
parsed, and tool-process content is fenced as untrusted data at every
ingress (web fetches, file reads, MCP tool output, operator-provided
context files, the productivity integrations' externally-authored
content — Gmail, Calendar, Drive, Contacts, Notion, Obsidian, n8n —
and the toolkit's web search), so the model sees that content marked
as data, not as instructions it should follow. Since Chapter Picket
(Phases 194-196), this is no longer only a structural mitigation:
`aivyx-injection-guard` is a real content-level scanner for
prompt-injection *payloads*, layered on top of Bulwark's fencing — it
actively scans the same untrusted content for known injection
phrasings and escalates the turn on a match, the same category of
defense Hermes Agent's Tirith provides.
```

- [ ] **Step 2: Confirm no other mentions of the old claim remain**

Run: `grep -n "still has no content-level scanner" docs/THREAT_MODEL.md`
Expected: no output (empty).

- [ ] **Step 3: Commit**

```bash
git add docs/THREAT_MODEL.md
git commit -m "docs: correct THREAT_MODEL.md's stale Chapter Picket + coverage claims

Section 5.3 said Aivyx 'still has no content-level scanner for
prompt-injection payloads' -- false since Chapter Picket shipped
aivyx-injection-guard in Phases 194-196 and this section was never
updated. Its coverage list also predated this phase's own new
productivity-integration coverage. Both corrected."
```

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** Tasks 1-8 implement every row of the design
  spec's 28-tool table, one crate per task, matching the spec's exact
  file:line locations. Task 9 implements the spec's Section 4 fix
  verbatim. Nothing in the spec's "left at default `false`" list is
  touched anywhere in this plan.
- **No placeholders:** every step's edit is a real, exact `find` /
  `replace with` pair copied from the actual current file content
  (verified via direct reads of all 28 tool files' `impl Tool for`
  blocks and all 28 files' `mod tests` opening lines before writing
  this plan); every test is real, compilable code using each file's
  own already-proven client-construction pattern (either an existing
  `make_tool()`/`make_test_client()` helper, or — for the 10 files
  that lacked one — a new helper copied from a working sibling
  pattern in the same crate).
- **Type/interface consistency:** every `make_tool()` return type
  matches its tool's real struct name; every new test's assertion
  (`output_is_untrusted()`) matches the exact method name the design
  spec and Task 1-8 overrides both use.
