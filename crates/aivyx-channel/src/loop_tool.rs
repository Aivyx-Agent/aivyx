//! Phase 173 — the autonomous-loop backlog agent tools.
//!
//! Two channel-tier tools (siblings of `mission.*`) that let the
//! loop agent interact with the Phase 173
//! [`crate::loop_backlog`] substrate from inside a
//! `TriggerSource::Loop` turn:
//!
//! - **`loop.next`** — read the next pending story (the Ralph
//!   task-selection step). Returns the story's id + title +
//!   body, or a clean `backlog_empty` signal.
//! - **`loop.complete`** — mark a story `Done` (after the agent
//!   has run gates + committed, per the canonical loop prompt).
//!
//! Both follow the `OnceLock`-store pattern of `mission_tool`:
//! registered with an empty slot, filled by the binary's
//! startup path via `set_backlog` after `RedbStorage::open`.
//! The agent already has `git` + `shell` in the thirteen-tool
//! core, so these two are the *only* new agent-facing surface
//! the loop needs — the substrate core is untouched.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::loop_backlog::{PersistentLoopBacklog, StoryStatus};

/// Shared handle to the backlog the tools read/write. The binary
/// builds one `PersistentLoopBacklog` and clones the `Arc` into
/// both tools + the loop driver so they all see one chain.
pub type SharedBacklog = Arc<PersistentLoopBacklog>;

/// Phase 175 — the reserved memory topic the loop progress log
/// lives under. `loop.note` appends here; the driver reads the
/// last N entries and injects them into each fresh iteration's
/// prompt. Owned in one place so the writer (the tool) and the
/// reader (the driver) can never disagree on the topic.
pub const LOOP_PROGRESS_TOPIC: &str = "loop:progress";

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// loop.next
// ---------------------------------------------------------------------------

pub struct LoopNextTool {
    id: ToolId,
    schema: Value,
    backlog: OnceLock<SharedBacklog>,
}

impl std::fmt::Debug for LoopNextTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopNextTool")
            .field("id", &self.id)
            .field("has_backlog", &self.backlog.get().is_some())
            .finish()
    }
}

impl Default for LoopNextTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopNextTool {
    pub fn new() -> Self {
        LoopNextTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            backlog: OnceLock::new(),
        }
    }

    pub fn set_backlog(&self, backlog: SharedBacklog) -> Result<(), SharedBacklog> {
        self.backlog.set(backlog)
    }
}

#[async_trait]
impl Tool for LoopNextTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "loop.next"
    }

    fn description(&self) -> &str {
        "Return the next pending story from the autonomous-loop backlog \
         (lowest priority number first, then insertion order). Output is a \
         JSON object: `{ \"story\": { \"id\", \"priority\", \"title\", \
         \"body\" }, \"remaining\": N }` when a story is pending, or \
         `{ \"backlog_empty\": true, \"remaining\": 0 }` when the backlog \
         is complete. Call this first each iteration; if the backlog is \
         empty, stop."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("loop.next").expect("known base")
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(backlog) = self.backlog.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "loop.next invoked without a backlog; the session \
                         layer must call LoopNextTool::set_backlog(...) after \
                         opening storage"
                    .to_string(),
            });
        };

        match backlog.next_pending() {
            Some(story) => ToolOutcome::Completed {
                output: json!({
                    "story": {
                        "id": story.id,
                        "priority": story.priority,
                        "title": story.title,
                        "body": story.body,
                    },
                    "remaining": backlog.remaining_count(),
                }),
                verified: Verification::NotApplicable,
            },
            None => ToolOutcome::Completed {
                output: json!({ "backlog_empty": true, "remaining": 0 }),
                verified: Verification::NotApplicable,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// loop.complete
// ---------------------------------------------------------------------------

pub struct LoopCompleteTool {
    id: ToolId,
    schema: Value,
    backlog: OnceLock<SharedBacklog>,
}

impl std::fmt::Debug for LoopCompleteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopCompleteTool")
            .field("id", &self.id)
            .field("has_backlog", &self.backlog.get().is_some())
            .finish()
    }
}

impl Default for LoopCompleteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopCompleteTool {
    pub fn new() -> Self {
        LoopCompleteTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "story_id": {
                        "type": "string",
                        "description": "The id of the story to mark Done \
                                        (from loop.next)."
                    }
                },
                "required": ["story_id"]
            }),
            backlog: OnceLock::new(),
        }
    }

    pub fn set_backlog(&self, backlog: SharedBacklog) -> Result<(), SharedBacklog> {
        self.backlog.set(backlog)
    }
}

#[async_trait]
impl Tool for LoopCompleteTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "loop.complete"
    }

    fn description(&self) -> &str {
        "Mark an autonomous-loop backlog story as Done. Input is a JSON \
         object with a `story_id` field (from loop.next). Call this ONLY \
         after the story's quality gates pass (tests/typecheck) and the \
         work is committed. Returns `{ \"completed\": \"<id>\", \
         \"remaining\": N }`. Marking an unknown or already-resolved story \
         fails."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("loop.complete").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(backlog) = self.backlog.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "loop.complete: no backlog configured".to_string(),
            });
        };

        let story_id = input
            .get("story_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if story_id.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "loop.complete requires a non-empty `story_id` field"
                    .to_string(),
            });
        }

        // Guard: refuse to "complete" a story that isn't pending,
        // surfacing a clear reason rather than a chain error.
        match backlog.get(&story_id) {
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("loop.complete: unknown story `{story_id}`"),
                });
            }
            Some(story) if !matches!(story.status, StoryStatus::Pending) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "loop.complete: story `{story_id}` is not pending \
                         (already resolved)"
                    ),
                });
            }
            Some(_) => {}
        }

        match backlog.mark_done(story_id.clone(), now_unix_ms()).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({
                    "completed": story_id,
                    "remaining": backlog.remaining_count(),
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("loop.complete failed: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// loop.note (Phase 175)
// ---------------------------------------------------------------------------

/// `loop.note` — append a one-line learning to the loop progress
/// log (the reserved [`LOOP_PROGRESS_TOPIC`] memory topic the
/// driver injects into each fresh iteration). Owns the topic so
/// the agent can't mis-route the note; gated under the
/// `loop.note` scope (narrower than `memory.write` — it can only
/// write this one reserved topic).
pub struct LoopNoteTool {
    id: ToolId,
    schema: Value,
    memory: OnceLock<Arc<dyn aivyx_memory::Memory>>,
}

impl std::fmt::Debug for LoopNoteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopNoteTool")
            .field("id", &self.id)
            .field("has_memory", &self.memory.get().is_some())
            .finish()
    }
}

impl Default for LoopNoteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopNoteTool {
    pub fn new() -> Self {
        LoopNoteTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "A short one-line learning for \
                                        future iterations (a gotcha, a \
                                        convention, a path)."
                    }
                },
                "required": ["text"]
            }),
            memory: OnceLock::new(),
        }
    }

    pub fn set_memory(
        &self,
        memory: Arc<dyn aivyx_memory::Memory>,
    ) -> Result<(), Arc<dyn aivyx_memory::Memory>> {
        self.memory.set(memory)
    }
}

#[async_trait]
impl Tool for LoopNoteTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "loop.note"
    }

    fn description(&self) -> &str {
        "Append a one-line learning to the autonomous-loop progress log. \
         Input is a JSON object with a `text` field. The note is stored \
         durably and surfaced to FUTURE loop iterations (which start with \
         fresh context), so record anything the next iteration should know \
         — a gotcha, a codebase convention, where tests live. Keep it to \
         one short line."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("loop.note").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(memory) = self.memory.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "loop.note: no memory substrate configured".to_string(),
            });
        };

        let text = input
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if text.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "loop.note requires a non-empty `text` field"
                    .to_string(),
            });
        }

        // Phase 177 — de-dup against the most-recent progress note:
        // an agent that re-states the same learning on consecutive
        // iterations shouldn't fill the injected progress block with
        // repeats. Most-recent-only (a full-history scan would be
        // O(n) per note); catches the common back-to-back case.
        if let Ok(recent) =
            memory.get_recent(LOOP_PROGRESS_TOPIC, 1).await
        {
            if recent.first().map(|e| e.body.trim()) == Some(text.as_str())
            {
                return ToolOutcome::Completed {
                    output: json!({ "noted": false, "deduped": true }),
                    verified: Verification::NotApplicable,
                };
            }
        }

        match memory.put(LOOP_PROGRESS_TOPIC, &text).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "noted": true }),
                verified: Verification::NotApplicable,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("loop.note failed to record: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{
        AgentId, CancellationToken, ChannelContext, ChannelError,
        ChannelPlatform, SessionId, StreamEvent, TurnId, TurnOutcome,
    };
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use std::path::PathBuf;

    // -- Minimal Trusted-tier test channel + null audit (same
    //    pattern as memory_gc_tool) so execute() has a context. --

    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }

    #[async_trait]
    impl ChannelContext for NoopChannel {
        fn session_id(&self) -> SessionId {
            self.session
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn channel_name(&self) -> &str {
            "test"
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        async fn stream_event(
            &self,
            _event: StreamEvent<'_>,
        ) -> Result<(), ChannelError> {
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

    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _tag: aivyx_core::AuditTag) {}
    }

    fn make_ctx<'a>(
        ch: &'a NoopChannel,
        audit: &'a dyn aivyx_core::AuditHook,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: ch,
            audit,
            cancellation: &ch.token,
        }
    }

    fn ctx_parts() -> (NoopChannel, NoopAudit) {
        (
            NoopChannel {
                session: SessionId::new(),
                token: CancellationToken::new(),
            },
            NoopAudit,
        )
    }

    #[test]
    fn names_and_scopes() {
        let next = LoopNextTool::new();
        assert_eq!(next.name(), "loop.next");
        assert_eq!(next.required_scope(&json!({})).as_str(), "loop.next");
        let done = LoopCompleteTool::new();
        assert_eq!(done.name(), "loop.complete");
        assert_eq!(
            done.required_scope(&json!({})).as_str(),
            "loop.complete"
        );
        assert!(done.input_schema()["required"]
            .as_array()
            .unwrap()
            .contains(&json!("story_id")));
    }

    async fn backlog() -> (SharedBacklog, PathBuf) {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = PathBuf::from(base)
            .join(format!("aivyx-loop-tool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([3u8; 32]),
        )
        .await
        .unwrap();
        let bl = Arc::new(
            PersistentLoopBacklog::open(
                store.domain(KeyDomain::LoopBacklog),
                b"k".to_vec(),
            )
            .await
            .unwrap(),
        );
        (bl, dir)
    }

    #[tokio::test]
    async fn next_then_complete_round_trip() {
        let (bl, dir) = backlog().await;
        bl.add_story("s-1".into(), 1, 1, "Story one".into(), "body".into())
            .await
            .unwrap();

        let next = LoopNextTool::new();
        assert!(next.set_backlog(Arc::clone(&bl)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        let out = next.execute(json!({}), &ctx).await;
        let id = match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["story"]["title"], "Story one");
                assert_eq!(output["remaining"], 1);
                output["story"]["id"].as_str().unwrap().to_string()
            }
            other => panic!("expected Completed, got {other:?}"),
        };
        assert_eq!(id, "s-1");

        let done = LoopCompleteTool::new();
        assert!(done.set_backlog(Arc::clone(&bl)).is_ok());
        let out = done.execute(json!({ "story_id": "s-1" }), &ctx).await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["completed"], "s-1");
                assert_eq!(output["remaining"], 0);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Backlog now empty.
        let out = next.execute(json!({}), &ctx).await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["backlog_empty"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn complete_unknown_story_fails() {
        let (bl, dir) = backlog().await;
        let done = LoopCompleteTool::new();
        assert!(done.set_backlog(Arc::clone(&bl)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        let out = done.execute(json!({ "story_id": "nope" }), &ctx).await;
        assert!(matches!(out, ToolOutcome::Failed(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn complete_already_done_fails() {
        let (bl, dir) = backlog().await;
        bl.add_story("s".into(), 1, 1, "t".into(), "".into())
            .await
            .unwrap();
        bl.mark_done("s".into(), 2).await.unwrap();
        let done = LoopCompleteTool::new();
        assert!(done.set_backlog(Arc::clone(&bl)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        let out = done.execute(json!({ "story_id": "s" }), &ctx).await;
        assert!(matches!(out, ToolOutcome::Failed(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tools_without_backlog_fail_cleanly() {
        let next = LoopNextTool::new();
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        assert!(matches!(
            next.execute(json!({}), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }

    // ---- loop.note (Phase 175) --------------------------------

    #[test]
    fn loop_note_name_and_scope() {
        let t = LoopNoteTool::new();
        assert_eq!(t.name(), "loop.note");
        assert_eq!(t.required_scope(&json!({})).as_str(), "loop.note");
        assert!(t.input_schema()["required"]
            .as_array()
            .unwrap()
            .contains(&json!("text")));
    }

    #[tokio::test]
    async fn loop_note_writes_reserved_topic() {
        use aivyx_memory::{InMemoryMemory, Memory};
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let t = LoopNoteTool::new();
        assert!(t.set_memory(Arc::clone(&mem)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);

        let out = t
            .execute(json!({ "text": "tests live in tests/" }), &ctx)
            .await;
        assert!(matches!(out, ToolOutcome::Completed { .. }));

        // Landed under the reserved topic, retrievable newest-first.
        let recent = mem.get_recent(LOOP_PROGRESS_TOPIC, 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].body, "tests live in tests/");
    }

    #[tokio::test]
    async fn loop_note_rejects_empty_and_missing_memory() {
        use aivyx_memory::{InMemoryMemory, Memory};
        // Empty text → failure (no write).
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let t = LoopNoteTool::new();
        assert!(t.set_memory(Arc::clone(&mem)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        assert!(matches!(
            t.execute(json!({ "text": "   " }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
        assert!(mem
            .get_recent(LOOP_PROGRESS_TOPIC, 10)
            .await
            .unwrap()
            .is_empty());

        // No memory configured → clean failure.
        let bare = LoopNoteTool::new();
        assert!(matches!(
            bare.execute(json!({ "text": "x" }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn loop_note_dedups_consecutive_identical_notes() {
        use aivyx_memory::{InMemoryMemory, Memory};
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let t = LoopNoteTool::new();
        assert!(t.set_memory(Arc::clone(&mem)).is_ok());
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);

        // First write lands.
        let out = t
            .execute(json!({ "text": "use --release" }), &ctx)
            .await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["noted"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Exact repeat (with surrounding whitespace) is deduped.
        let out = t
            .execute(json!({ "text": "  use --release  " }), &ctx)
            .await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["noted"], false);
                assert_eq!(output["deduped"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        // Still only one entry stored.
        assert_eq!(
            mem.get_recent(LOOP_PROGRESS_TOPIC, 10).await.unwrap().len(),
            1
        );

        // A DIFFERENT note still writes (and resets the
        // most-recent for future dedup).
        let out = t
            .execute(json!({ "text": "tests in tests/" }), &ctx)
            .await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["noted"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        assert_eq!(
            mem.get_recent(LOOP_PROGRESS_TOPIC, 10).await.unwrap().len(),
            2
        );

        // And a non-consecutive repeat of the FIRST note writes
        // (most-recent-only dedup — the current most-recent is the
        // different note).
        let out = t
            .execute(json!({ "text": "use --release" }), &ctx)
            .await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["noted"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        assert_eq!(
            mem.get_recent(LOOP_PROGRESS_TOPIC, 10).await.unwrap().len(),
            3
        );
    }
}
