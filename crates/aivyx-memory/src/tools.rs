//! # Memory tools — the three `Tool` impls from Phase 6 task 3.
//!
//! [`MemoryReadTool`], [`MemoryWriteTool`], and [`MemoryForgetTool`]
//! are the user-facing surface of D1's "memory is a tool, not ambient
//! system" commitment. Each one wraps an `Arc<dyn Memory>` (so the
//! same substrate can be driven by any implementation — redb in
//! production, [`crate::InMemoryMemory`] in tests), owns its
//! pre-built input schema, and routes every call through the
//! capability-typed [`aivyx_core::Tool`] trait that the turn loop
//! already enforces.
//!
//! ## Q5 resolution (for future archaeologists)
//!
//! Phase 6 entered open on "does `Tool::required_scope` need an
//! `input: &Value` parameter?" A careful read of `aivyx-core` at task
//! 3 start revealed that Phase 1's task 3 already folded the R1
//! refinement into the live trait: `fn required_scope(&self, input:
//! &serde_json::Value) -> Scope` has been shipped since commit
//! `33012be`. Phase 4's `FsReadTool` has been using the signature for
//! two phases. No DESIGN.md amendment needed; the five-phase
//! empty-diff streak rolls forward to six. The stale DESIGN.md
//! line-1038 note is a known drift marker and stays as-is per the
//! "evidence-driven amendments" discipline — a future phase that
//! *actually* amends D3 will clean it up in the same diff.
//!
//! ## Q3 resolution — topic filter semantics
//!
//! `memory.read` requires a topic. The `"*"` sentinel from the Phase
//! 6 open-question writeup is **not** implemented in this task; the
//! substrate itself has no cross-topic scan primitive, and the audit
//! story for "one call surfaces arbitrary state" is exactly the kind
//! of thing that deserves deliberate design rather than a reserved
//! string squeezed in at task 3. If Phase 7+ wants it, the call site
//! will be `MemoryReadTool::execute`, not a substrate change. This is
//! in line with option (1) from Phase 6's Q3: require a topic
//! argument, force the planner to name what it recalls.
//!
//! ## Scope derivation (the R1 payoff)
//!
//! Each tool derives a scope that *mentions* the specific topic the
//! call will touch, so that an agent holding `memory.read:topic:notes`
//! cannot use `memory.read` to peek at topic `secrets`. The qualifier
//! shape matches DESIGN.md's D4 taxonomy: `<base>:topic:<topic>`.
//! Malformed input (missing topic, wrong type) routes to a "deny
//! scope" — a scope no agent can possibly hold, which the turn
//! loop's scope gate converts into `ToolOutcome::Denied` before
//! `execute` ever runs. This is exactly the Phase 4 `FsReadTool`
//! pattern; the only shift is that memory uses a topic qualifier
//! instead of a path qualifier.
//!
//! The "deny scope" construction has a subtlety: it must still be a
//! *valid* `Scope` (the `Tool` trait's return type is bare `Scope`,
//! not `Result`), and its `base` must be one of `aivyx-capability`'s
//! `KNOWN_BASES` or `Scope::parse` refuses to build it. The reserved
//! sentinel qualifier `topic:\x00denied` meets both constraints: the
//! base is real (`memory.read`), so the parse succeeds, but no agent
//! will ever be granted a scope with a NUL-containing qualifier, so
//! the gate denies every call that lands on it. Same trick as
//! Phase 4's fs deny scope, adapted to this crate's qualifier shape.
//!
//! ## What does NOT happen here
//!
//! - **No scope checking.** The turn loop's scope gate in
//!   `aivyx-core` is the *only* thing that compares `required_scope`
//!   against the agent's held capabilities. `execute` trusts that
//!   it was invoked with an in-scope call — see the `FsReadTool`
//!   docstring for the same explicit reliance.
//! - **No audit writes.** Each tool builds an `AuditTag::MemoryAccess`
//!   for the turn loop to emit, but this task does not wire
//!   audit-on-execute directly — that's the turn loop's job via
//!   `ToolContext::audit`. The tools **do** emit a MemoryAccess
//!   event at the start of `execute`, so the audit chain records
//!   every memory touch even when the substrate no-ops (e.g., a
//!   read of a never-written topic).
//! - **No topic validation beyond non-empty.** UTF-8 well-formedness
//!   is already guaranteed by the `String` type, and the substrate
//!   itself rejects `EmptyTopic`. Topic length caps, character
//!   restrictions, and reserved prefixes are deferred to Phase 7+
//!   if real-world usage surfaces a need.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, AuditTag, MemoryOperation, Tool, ToolContext, ToolId,
    ToolOutcome, Verification,
};

use crate::{Memory, MemoryError};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Default cap on the number of entries a single `memory.read` call
/// can return. Const, not config — same philosophy as
/// `FsReadTool::MAX_READ_BYTES`. 32 is big enough for any realistic
/// "recent memories" recall while keeping a single tool call from
/// flooding an LLM turn if the agent sets `limit` to something wild
/// (or omits it, in which case this is the default).
pub const DEFAULT_READ_LIMIT: usize = 16;

/// Hard ceiling on `memory.read` limit even when explicitly set.
/// An agent can ask for up to this many; anything larger is clamped.
/// Prevents a single malformed planner step from pulling the entire
/// store into context.
pub const MAX_READ_LIMIT: usize = 64;

/// Phase 7 task 5 — default ceiling on the number of entries a single
/// topic may hold before `memory.write` starts refusing new entries.
/// Not a hard disk-space bound (the redb file can still grow from
/// *many topics*); this is the per-topic GC tripwire that prevents a
/// single topic from becoming an unbounded log. 10_000 is generous
/// enough that realistic "remember X" usage never trips it, but small
/// enough that a runaway loop gets caught long before the substrate
/// chokes.
///
/// The binary reads `AIVYX_MEMORY_MAX_PER_TOPIC` once at startup and
/// passes the resolved cap through `MemoryWriteTool::set_max_per_topic`.
/// Tests override it with the same builder. No ambient global config.
pub const DEFAULT_MAX_PER_TOPIC: usize = 10_000;

/// Construct a `Scope` no agent will ever be granted. Uses the
/// qualifier `"topic:\x00denied"` — a NUL byte inside a qualifier is
/// unreachable through normal `Scope::parse` on user-supplied
/// strings because D4-legal qualifiers never contain NULs, so any
/// agent holding a memory scope will have a NUL-free qualifier and
/// the gate's equality check fails. `base` must be a real base or
/// `Scope::parse` refuses the construction, which is why the
/// function takes the base as an argument.
fn deny_scope(base: &str) -> Scope {
    // `.expect` is fine here: this function is only ever called with
    // one of the three hard-coded base strings below, each of which
    // is in KNOWN_BASES. If someone adds a fourth call site with a
    // typo, the test suite catches it at first run.
    Scope::parse(&format!("{base}:topic:\x00denied"))
        .expect("deny_scope base must be a known scope base")
}

/// Extract a non-empty topic string from a tool-input JSON value.
/// Returns `None` for missing field, wrong type, or empty string.
/// Shared so the three tools' `required_scope` functions derive
/// scopes from exactly the same shape, and `execute` can't disagree
/// with the gate about what "valid" means.
fn topic_from_input(input: &Value) -> Option<&str> {
    let s = input.get("topic")?.as_str()?;
    if s.is_empty() { None } else { Some(s) }
}

/// Extract the optional session partition string from a tool-input
/// JSON value. Phase 8 Task 2 introduced this field to namespace
/// memory per `ChannelContext::session_partition()` — a multi-chat
/// Telegram bot sees a different `session` value per chat, so two
/// chats can write `topic:notes` without seeing each other's entries.
///
/// The field is **never** set by the LLM or by the tool caller's
/// hand-written JSON; the turn loop in `aivyx-core` injects it after
/// the LLM emits the tool call and before `required_scope` runs. See
/// the module doc on `ChannelContext::session_partition` and the
/// turn-loop insertion site for the exact contract.
///
/// A missing field, wrong type, or empty string all map to `None`,
/// which is the single-partition default (`LocalChannel` and any
/// other channel whose `session_partition()` returns `None`).
fn session_from_input(input: &Value) -> Option<&str> {
    let s = input.get("session")?.as_str()?;
    if s.is_empty() { None } else { Some(s) }
}

/// The reserved prefix marker that identifies a session-namespaced
/// physical topic. Uses ASCII `0x01` bytes as separators so no
/// well-formed agent-chosen topic can collide: topics are UTF-8
/// strings and the Phase 6 contract already forbids NUL, but `0x01`
/// is equally unusable as a literal string by a language model and
/// equally easy to reject at the tool-input validation step.
///
/// A physical topic under session `12345` with logical topic `notes`
/// is stored as `\x01s\x0112345\x01notes`. Phase 7 stores open cleanly
/// because they were written under logical topics like `notes` whose
/// literal bytes never start with `\x01`.
const SESSION_PREFIX: &str = "\x01s\x01";

/// Build a session-namespaced physical topic string. When `session`
/// is `None`, returns the logical topic unchanged — this is the
/// single-partition path that preserves every Phase 6/7 on-disk
/// layout exactly. When `session` is `Some`, builds
/// `\x01s\x01<session>\x01<topic>` which can never collide with a
/// logical topic because logical topics cannot contain `\x01`.
fn namespaced_topic(session: Option<&str>, topic: &str) -> String {
    match session {
        None => topic.to_string(),
        Some(s) => format!("{SESSION_PREFIX}{s}\x01{topic}"),
    }
}

/// Reject topics whose literal form starts with the reserved
/// session-namespace prefix. Returning `true` from the caller means
/// "route this input to the deny scope" so the gate never admits a
/// call that would have collided with a physical-topic name. Phase 8
/// Task 2 makes this a hard validation at both `required_scope` and
/// `execute`.
fn topic_uses_reserved_prefix(topic: &str) -> bool {
    topic.starts_with('\x01')
}

/// Build a qualified memory scope.
///
/// Two shapes:
/// - `<base>:topic:<topic>` — Phase 6 single-partition form (when
///   `session` is `None`).
/// - `<base>:topic:<topic>:session:<session>` — Phase 8 Task 2
///   dual-qualifier form (when `session` is `Some`), for
///   multi-chat channels like Telegram.
///
/// Qualifier order is fixed so two logically equivalent scopes
/// always stringify identically — otherwise the audit chain would
/// see non-canonical variants on the same held capability and
/// `Scope::is_granted_by` would flip on qualifier reordering.
fn memory_scope(base: &str, topic: &str, session: Option<&str>) -> Scope {
    let qualified = match session {
        None => format!("{base}:topic:{topic}"),
        Some(s) => format!("{base}:topic:{topic}:session:{s}"),
    };
    Scope::parse(&qualified).unwrap_or_else(|| deny_scope(base))
}

/// Convert a [`MemoryError`] to a [`ToolOutcome::Failed`] with the
/// right `ToolId`. Substrate errors are never successful tool runs.
fn memory_err_to_failed(tool: ToolId, err: MemoryError) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool,
        detail: format!("memory substrate error: {err}"),
    })
}

/// Turn a [`MemoryEntry`] into the JSON shape the tool emits. Kept
/// out of `MemoryEntry`'s own `Serialize` impl because the on-disk
/// at-rest shape (task 1) and the tool output shape can diverge
/// later — the latter is an agent-facing contract, the former is
/// an internal storage detail.
fn entry_to_json(entry: &crate::MemoryEntry) -> Value {
    json!({
        "topic": entry.topic,
        "body": entry.body,
        "seq": entry.seq,
        "created_at_secs": entry.created_at_secs,
    })
}

// ---------------------------------------------------------------------------
// MemoryReadTool
// ---------------------------------------------------------------------------

/// `memory.read` — recall up to `limit` most recent entries for a
/// given topic.
///
/// Input shape:
/// ```json
/// { "topic": "notes", "limit": 10 }
/// ```
/// `limit` is optional; missing means [`DEFAULT_READ_LIMIT`], and any
/// value larger than [`MAX_READ_LIMIT`] is clamped down. Output is
/// `{"entries": [...]}` with `entries` newest-first.
pub struct MemoryReadTool {
    id: ToolId,
    memory: Arc<dyn Memory>,
    schema: Value,
}

impl std::fmt::Debug for MemoryReadTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryReadTool")
            .field("id", &self.id)
            .field("memory", &"Arc<dyn Memory>")
            .finish()
    }
}

impl MemoryReadTool {
    /// Construct a new `memory.read` tool over the given substrate.
    /// Infallible — the substrate handle is already live, there is
    /// nothing to canonicalize or check.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        MemoryReadTool {
            id: ToolId::new(),
            memory,
            schema: read_input_schema_value(),
        }
    }
}

fn read_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topic": {
                "type": "string",
                "description": "Non-empty topic tag to recall. \
                                Memory is strictly topic-scoped; a \
                                reader cannot see topics other than \
                                this one."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_READ_LIMIT,
                "description": "Maximum number of recent entries to return. \
                                Defaults to 16, capped at 64."
            }
        },
        "required": ["topic"],
        "additionalProperties": false,
    })
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "memory.read"
    }

    fn description(&self) -> &str {
        "Recall recent memory entries stored under a given topic. \
         Returns up to `limit` entries (default 16, max 64), newest first. \
         The agent must hold `memory.read:topic:<topic>` — scopes are \
         per-topic, so reading one topic does not grant access to others."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match topic_from_input(input) {
            Some(topic) if !topic_uses_reserved_prefix(topic) => {
                memory_scope("memory.read", topic, session_from_input(input))
            }
            // Missing topic, wrong type, empty topic, or a topic that
            // literally starts with the reserved `\x01` session-
            // namespace prefix — the last case would let an agent
            // side-door into another chat's physical storage key, so
            // deny at the gate.
            _ => deny_scope("memory.read"),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let topic = match topic_from_input(&input) {
            Some(t) if !topic_uses_reserved_prefix(t) => t.to_string(),
            _ => {
                // Same invariant violation argument as FsReadTool: the
                // scope gate would have denied a missing-topic call
                // because `required_scope` returned the deny scope.
                // Reaching `execute` without a topic means either the
                // gate was bypassed or the scope-gate logic has a
                // bug. Either way, fail loudly in audit.
                return ToolOutcome::Failed(AivyxError::Internal(
                    "memory.read: reached execute with malformed input \
                     (topic missing, non-string, or reserved prefix) \
                     after scope gate admitted the call"
                        .to_string(),
                ));
            }
        };
        // `session` is the namespacing source from the channel's
        // `session_partition()`, injected by the turn loop. `None`
        // preserves Phase 6 single-partition behavior.
        let session = session_from_input(&input).map(str::to_string);

        let requested = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_READ_LIMIT);
        let limit = requested.clamp(1, MAX_READ_LIMIT);

        // Audit records the *logical* scope and the *logical* topic the
        // agent asked for. The physical topic is an internal storage
        // detail and must never leak into the audit chain — otherwise
        // `verify_from_disk` would have to know about namespacing to
        // round-trip a chain, breaking D1's "audit verifies without
        // live substrate" rule.
        ctx.audit.on_event(AuditTag::MemoryAccess {
            turn_id: ctx.turn_id,
            operation: MemoryOperation::Read,
            scope: memory_scope("memory.read", &topic, session.as_deref()),
            query_or_key: topic.clone(),
        });

        let physical = namespaced_topic(session.as_deref(), &topic);
        let mut entries = match self.memory.get_recent(&physical, limit).await {
            Ok(v) => v,
            Err(e) => return memory_err_to_failed(self.id, e),
        };
        // Restore the logical topic on the way out — the agent asked
        // for `notes`, not `\x01s\x0112345\x01notes`. Same reasoning
        // as the audit event above.
        for entry in entries.iter_mut() {
            entry.topic = topic.clone();
        }

        let json_entries: Vec<Value> = entries.iter().map(entry_to_json).collect();

        ToolOutcome::Completed {
            output: json!({
                "topic": topic,
                "entries": json_entries,
                "count": json_entries.len(),
            }),
            // Read is inherently a query — "verification" is
            // meaningless for something that didn't mutate state.
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryWriteTool
// ---------------------------------------------------------------------------

/// `memory.write` — store a single entry under a topic.
///
/// Input shape:
/// ```json
/// { "topic": "notes", "body": "the user's favorite color is purple" }
/// ```
/// Output is `{"seq": <u64>}`, the substrate's assigned monotonic
/// sequence number. The tool verifies the write by calling
/// `get_recent(topic, 1)` and checking the top entry matches what it
/// just wrote — this is what `Verification::Verified` exists for per
/// D1's "tool success ≠ intent completed" rule.
pub struct MemoryWriteTool {
    id: ToolId,
    memory: Arc<dyn Memory>,
    schema: Value,
    /// Phase 7 task 5 — per-topic GC tripwire. When a topic already
    /// holds at least this many entries, `execute` refuses further
    /// writes with a `ToolOutcome::Failed` carrying a
    /// recovery-actionable error detail. Defaults to
    /// [`DEFAULT_MAX_PER_TOPIC`]; binaries and tests can override via
    /// [`MemoryWriteTool::set_max_per_topic`].
    max_per_topic: usize,
}

impl std::fmt::Debug for MemoryWriteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryWriteTool")
            .field("id", &self.id)
            .field("memory", &"Arc<dyn Memory>")
            .field("max_per_topic", &self.max_per_topic)
            .finish()
    }
}

impl MemoryWriteTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        MemoryWriteTool {
            id: ToolId::new(),
            memory,
            schema: write_input_schema_value(),
            max_per_topic: DEFAULT_MAX_PER_TOPIC,
        }
    }

    /// Override the per-topic write cap. Returns `self` for the
    /// builder-style chaining the binary uses when
    /// `AIVYX_MEMORY_MAX_PER_TOPIC` is set, and that tests use to drive
    /// the tripwire with a manageable number of seeded entries.
    ///
    /// A cap of `0` is legal and means "refuse every write" — useful
    /// for testing the refusal path but nothing else. The tool does
    /// not silently reinterpret `0` as "unlimited"; if you want that,
    /// use `usize::MAX`.
    pub fn set_max_per_topic(mut self, cap: usize) -> Self {
        self.max_per_topic = cap;
        self
    }
}

fn write_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topic": {
                "type": "string",
                "description": "Non-empty topic tag to store the entry under. \
                                Agents must hold `memory.write:topic:<topic>`."
            },
            "body": {
                "type": "string",
                "description": "Freeform UTF-8 body of what to remember."
            }
        },
        "required": ["topic", "body"],
        "additionalProperties": false,
    })
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "memory.write"
    }

    fn description(&self) -> &str {
        "Store a new memory entry under a topic. Returns the assigned \
         monotonic sequence number. The tool re-reads the topic after \
         writing and returns `Verified` only if the new entry is at \
         the head of the recall order."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match topic_from_input(input) {
            Some(topic) if !topic_uses_reserved_prefix(topic) => {
                memory_scope("memory.write", topic, session_from_input(input))
            }
            _ => deny_scope("memory.write"),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let topic = match topic_from_input(&input) {
            Some(t) if !topic_uses_reserved_prefix(t) => t.to_string(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Internal(
                    "memory.write: reached execute with malformed input \
                     (topic missing, non-string, or reserved prefix) \
                     after scope gate admitted the call"
                        .to_string(),
                ));
            }
        };
        let session = session_from_input(&input).map(str::to_string);
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "input must have a string `body` field".to_string(),
                });
            }
        };

        ctx.audit.on_event(AuditTag::MemoryAccess {
            turn_id: ctx.turn_id,
            operation: MemoryOperation::Write,
            scope: memory_scope("memory.write", &topic, session.as_deref()),
            query_or_key: topic.clone(),
        });

        // Phase 7 task 5 — GC tripwire. Count the topic's existing
        // entries via a bounded `get_recent` call. `RedbMemory::get_recent`
        // does a full `scan_prefix` on the topic regardless of limit
        // (decode cost is what scales with `limit`), so passing
        // `max_per_topic` gives us the smallest decoded prefix that
        // can still answer "is the topic at or above cap?" The audit
        // event above has already been emitted — a refused write is
        // still a write *intent*, and recording it is the whole point
        // of the audit chain. Only after the tripwire fires do we
        // surface the refusal to the caller.
        //
        // Phase 8 Task 2 — the tripwire counts entries in the
        // *physical* (namespaced) topic, not the logical one.
        // Otherwise chat A filling its `notes` bucket would cap chat
        // B's unrelated `notes` bucket — the whole point of session
        // partitioning is that these are independent quotas.
        let physical = namespaced_topic(session.as_deref(), &topic);
        let existing = match self
            .memory
            .get_recent(&physical, self.max_per_topic)
            .await
        {
            Ok(v) => v,
            Err(e) => return memory_err_to_failed(self.id, e),
        };
        if existing.len() >= self.max_per_topic {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "topic '{topic}' has reached the per-topic size cap \
                     ({} entries); call memory.forget for this topic or \
                     write under a different topic",
                    self.max_per_topic
                ),
            });
        }

        let seq = match self.memory.put(&physical, &body).await {
            Ok(s) => s,
            Err(e) => return memory_err_to_failed(self.id, e),
        };

        // Verification fence. Re-read the topic's newest entry and
        // confirm it is what we just wrote. This catches substrate
        // drift: if a put-then-read races with a concurrent
        // `forget`, or if RedbMemory's seq counter somehow gets
        // out of sync with the store, we want `Verified` to be a
        // lie only when the write truly landed. D1 calls this the
        // "verify what you did" rule.
        let verified = match self.memory.get_recent(&physical, 1).await {
            Ok(latest) => match latest.first() {
                Some(top) if top.seq == seq && top.body == body => Verification::Verified,
                _ => Verification::Unverified,
            },
            // A read failure after a successful write is *still* a
            // successful write from the substrate's point of view;
            // report it as Unverified rather than demoting the whole
            // call to Failed.
            Err(_) => Verification::Unverified,
        };

        ToolOutcome::Completed {
            output: json!({
                "topic": topic,
                "seq": seq,
            }),
            verified,
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryForgetTool
// ---------------------------------------------------------------------------

/// `memory.forget` — delete every entry under a topic.
///
/// Input shape:
/// ```json
/// { "topic": "notes" }
/// ```
/// Output is `{"deleted": <usize>}`. Verification re-reads the topic
/// and confirms it's empty.
pub struct MemoryForgetTool {
    id: ToolId,
    memory: Arc<dyn Memory>,
    schema: Value,
}

impl std::fmt::Debug for MemoryForgetTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryForgetTool")
            .field("id", &self.id)
            .field("memory", &"Arc<dyn Memory>")
            .finish()
    }
}

impl MemoryForgetTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        MemoryForgetTool {
            id: ToolId::new(),
            memory,
            schema: forget_input_schema_value(),
        }
    }
}

fn forget_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topic": {
                "type": "string",
                "description": "Non-empty topic tag to forget. Every \
                                entry under this topic is deleted. \
                                Agents must hold \
                                `memory.forget:topic:<topic>`."
            }
        },
        "required": ["topic"],
        "additionalProperties": false,
    })
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "memory.forget"
    }

    fn description(&self) -> &str {
        "Delete every memory entry under a topic. Returns the number \
         of entries removed. Verification re-reads the topic and \
         confirms it is empty."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match topic_from_input(input) {
            Some(topic) if !topic_uses_reserved_prefix(topic) => {
                memory_scope("memory.forget", topic, session_from_input(input))
            }
            _ => deny_scope("memory.forget"),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let topic = match topic_from_input(&input) {
            Some(t) if !topic_uses_reserved_prefix(t) => t.to_string(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Internal(
                    "memory.forget: reached execute with malformed input \
                     (topic missing, non-string, or reserved prefix) \
                     after scope gate admitted the call"
                        .to_string(),
                ));
            }
        };
        let session = session_from_input(&input).map(str::to_string);

        ctx.audit.on_event(AuditTag::MemoryAccess {
            turn_id: ctx.turn_id,
            operation: MemoryOperation::Forget,
            scope: memory_scope("memory.forget", &topic, session.as_deref()),
            query_or_key: topic.clone(),
        });

        let physical = namespaced_topic(session.as_deref(), &topic);
        let deleted = match self.memory.forget(&physical).await {
            Ok(n) => n,
            Err(e) => return memory_err_to_failed(self.id, e),
        };

        // Verification: read 1 from the topic, confirm we get nothing.
        // A concurrent writer could in principle add an entry between
        // forget and get_recent; we still report Verified because the
        // forget call itself did land — the new entry is a fresh
        // write, not a survivor. But if the substrate returns an
        // error on the read, we cannot prove the forget worked, so
        // report Unverified.
        let verified = match self.memory.get_recent(&physical, 1).await {
            Ok(v) if v.is_empty() => Verification::Verified,
            Ok(_) => Verification::Unverified,
            Err(_) => Verification::Unverified,
        };

        ToolOutcome::Completed {
            output: json!({
                "topic": topic,
                "deleted": deleted,
            }),
            verified,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// Unit tests exercise the three tools against `InMemoryMemory` so they
// stay at microsecond speed. The integration test in task 5 will run
// the same code paths against `RedbMemory` over a real scratch store.
//
// Test coverage matches the PHASE_6.md task 3 contract line-by-line:
//   - scope-checked happy path (write → read → correct entry)
//   - scope derivation is input-dependent (R1 payoff)
//   - scope denial (malformed input → deny scope)
//   - empty result (read of unknown topic → {"entries": [], "count": 0})
//   - forget clears a topic and the verification reports Verified
//   - write reports Verified after put+re-read
//   - substrate error surfaces as ToolOutcome::Failed

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryMemory;
    use aivyx_core::{
        AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform,
        NullAuditHook, SessionId, StreamEvent, TurnId, TurnOutcome,
    };

    // ---- Test helpers ----------------------------------------------

    /// A minimal `ChannelContext` that ignores every event. Mirrors
    /// the `NoopChannel` in `aivyx-core::tools::fs`'s own test module
    /// — the Phase 4 reference harness — adapted here because the
    /// memory tools never actually call into the channel (unlike
    /// `fs.read`, they don't stream progress). Kept verbose rather
    /// than hidden behind a macro because the `#[async_trait]`
    /// impl is the whole point of the fake and hiding it would
    /// obscure the assertions.
    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }

    #[async_trait]
    impl ChannelContext for NoopChannel {
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
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    /// Build a fresh `ToolContext` from a channel + audit + cancel.
    /// Takes them by reference because the resulting context borrows
    /// them for its lifetime.
    fn make_ctx<'a>(
        channel: &'a NoopChannel,
        audit: &'a dyn aivyx_core::AuditHook,
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

    fn fresh_channel() -> NoopChannel {
        NoopChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        }
    }

    fn fresh_memory() -> Arc<dyn Memory> {
        Arc::new(InMemoryMemory::new())
    }

    // ---- Scope derivation (R1 payoff) ------------------------------

    #[test]
    fn read_scope_is_derived_from_topic() {
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"topic": "notes"}));
        assert_eq!(scope.base(), "memory.read");
        assert_eq!(scope.qualifier(), Some("topic:notes"));
    }

    #[test]
    fn write_scope_is_derived_from_topic() {
        let tool = MemoryWriteTool::new(fresh_memory());
        let scope =
            tool.required_scope(&json!({"topic": "secrets", "body": "hunter2"}));
        assert_eq!(scope.base(), "memory.write");
        assert_eq!(scope.qualifier(), Some("topic:secrets"));
    }

    #[test]
    fn forget_scope_is_derived_from_topic() {
        let tool = MemoryForgetTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"topic": "notes"}));
        assert_eq!(scope.base(), "memory.forget");
        assert_eq!(scope.qualifier(), Some("topic:notes"));
    }

    #[test]
    fn read_scope_from_two_different_topics_is_not_equal() {
        // The whole point of deriving the scope from the input: an
        // agent with `memory.read:topic:notes` must not be able to
        // use `memory.read` to peek at topic `secrets`. Scope
        // equality drives that, so this test proves the derivation
        // returns genuinely distinct scopes.
        let tool = MemoryReadTool::new(fresh_memory());
        let a = tool.required_scope(&json!({"topic": "notes"}));
        let b = tool.required_scope(&json!({"topic": "secrets"}));
        assert_ne!(a, b);
    }

    // ---- Deny-scope fallback for malformed input -------------------

    #[test]
    fn read_scope_for_missing_topic_is_deny_scope() {
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({}));
        // Deny-scope qualifier contains a NUL — any real granted
        // scope has a NUL-free qualifier, so equality fails.
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    #[test]
    fn read_scope_for_wrong_type_topic_is_deny_scope() {
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"topic": 42}));
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    #[test]
    fn read_scope_for_empty_topic_is_deny_scope() {
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"topic": ""}));
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    #[test]
    fn write_scope_for_missing_topic_is_deny_scope() {
        let tool = MemoryWriteTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"body": "hi"}));
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    #[test]
    fn forget_scope_for_missing_topic_is_deny_scope() {
        let tool = MemoryForgetTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({}));
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    // ---- Happy path execution --------------------------------------

    #[tokio::test]
    async fn write_then_read_round_trips_through_the_tool_surface() {
        let mem = fresh_memory();
        let writer = MemoryWriteTool::new(mem.clone());
        let reader = MemoryReadTool::new(mem.clone());
        let chan = fresh_channel();
        let audit = NullAuditHook;

        let ctx = make_ctx(&chan, &audit);
        let write_outcome = writer
            .execute(
                json!({"topic": "notes", "body": "purple"}),
                &ctx,
            )
            .await;
        match write_outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "notes");
                assert_eq!(output["seq"], 0);
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("write should complete, got {other:?}"),
        }

        let ctx = make_ctx(&chan, &audit);
        let read_outcome = reader
            .execute(json!({"topic": "notes"}), &ctx)
            .await;
        match read_outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "notes");
                assert_eq!(output["count"], 1);
                assert_eq!(verified, Verification::NotApplicable);
                let entries = output["entries"].as_array().unwrap();
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0]["body"], "purple");
                assert_eq!(entries[0]["seq"], 0);
            }
            other => panic!("read should complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_of_unknown_topic_returns_empty_list_not_error() {
        let reader = MemoryReadTool::new(fresh_memory());
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = reader
            .execute(json!({"topic": "never-written"}), &ctx)
            .await;
        match outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["count"], 0);
                assert!(output["entries"].as_array().unwrap().is_empty());
                assert_eq!(verified, Verification::NotApplicable);
            }
            other => panic!("unknown-topic read must still Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_limit_defaults_and_is_capped() {
        // Seed strictly more than MAX_READ_LIMIT so the clamp
        // assertion has enough supply to actually saturate. With 50
        // seeded entries and a 10_000-limit request, the clamp would
        // fire but `get_recent` would still only return 50 — the
        // assertion would pass for the wrong reason (not enough data
        // to notice the clamp).
        let mem = fresh_memory();
        for i in 0..(MAX_READ_LIMIT + 16) {
            mem.put("notes", &format!("entry {i}")).await.unwrap();
        }
        let reader = MemoryReadTool::new(mem);
        let chan = fresh_channel();
        let audit = NullAuditHook;

        // Default limit (no `limit` field) -> DEFAULT_READ_LIMIT.
        let ctx = make_ctx(&chan, &audit);
        let out = reader.execute(json!({"topic": "notes"}), &ctx).await;
        if let ToolOutcome::Completed { output, .. } = out {
            assert_eq!(output["count"], DEFAULT_READ_LIMIT);
        } else {
            panic!("default-limit read should Complete");
        }

        // Explicit limit below cap.
        let ctx = make_ctx(&chan, &audit);
        let out = reader
            .execute(json!({"topic": "notes", "limit": 5}), &ctx)
            .await;
        if let ToolOutcome::Completed { output, .. } = out {
            assert_eq!(output["count"], 5);
        } else {
            panic!("explicit-limit read should Complete");
        }

        // Explicit limit above MAX_READ_LIMIT — clamped.
        let ctx = make_ctx(&chan, &audit);
        let out = reader
            .execute(json!({"topic": "notes", "limit": 10_000}), &ctx)
            .await;
        if let ToolOutcome::Completed { output, .. } = out {
            assert_eq!(output["count"], MAX_READ_LIMIT);
        } else {
            panic!("clamped-limit read should Complete");
        }
    }

    #[tokio::test]
    async fn forget_clears_topic_and_reports_verified() {
        let mem = fresh_memory();
        mem.put("notes", "x").await.unwrap();
        mem.put("notes", "y").await.unwrap();
        mem.put("todos", "z").await.unwrap();

        let forget = MemoryForgetTool::new(mem.clone());
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = forget.execute(json!({"topic": "notes"}), &ctx).await;
        match outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["deleted"], 2);
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("forget should Complete, got {other:?}"),
        }
        // And the other topic survived.
        assert_eq!(mem.get_recent("todos", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn forget_of_unknown_topic_reports_zero_and_verified() {
        let forget = MemoryForgetTool::new(fresh_memory());
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = forget.execute(json!({"topic": "nothing"}), &ctx).await;
        match outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["deleted"], 0);
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("unknown-topic forget should Complete, got {other:?}"),
        }
    }

    // ---- Malformed input paths through execute ---------------------

    #[tokio::test]
    async fn write_missing_body_fails_loudly() {
        let writer = MemoryWriteTool::new(fresh_memory());
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        // Topic is present (so the scope gate would admit the call),
        // but body is missing. Execute should return Failed.
        let outcome = writer.execute(json!({"topic": "notes"}), &ctx).await;
        assert!(matches!(
            outcome,
            ToolOutcome::Failed(AivyxError::Tool { .. })
        ));
    }

    #[tokio::test]
    async fn read_missing_topic_in_execute_is_internal_error() {
        // This path is unreachable through the real turn loop — the
        // scope gate would deny the call. The test proves that *if*
        // the gate is ever bypassed, execute still fails closed with
        // an Internal error (not a silent success), matching
        // FsReadTool's same invariant.
        let reader = MemoryReadTool::new(fresh_memory());
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = reader.execute(json!({}), &ctx).await;
        assert!(matches!(
            outcome,
            ToolOutcome::Failed(AivyxError::Internal(_))
        ));
    }

    // ---- Tool metadata sanity --------------------------------------

    #[test]
    fn tool_names_are_the_expected_d4_strings() {
        // D4 scope taxonomy lists these exact base strings. Tool
        // names should match so registry lookups are obvious.
        assert_eq!(MemoryReadTool::new(fresh_memory()).name(), "memory.read");
        assert_eq!(MemoryWriteTool::new(fresh_memory()).name(), "memory.write");
        assert_eq!(
            MemoryForgetTool::new(fresh_memory()).name(),
            "memory.forget"
        );
    }

    // ---- Phase 7 task 5: per-topic GC tripwire ---------------------

    #[tokio::test]
    async fn write_at_cap_still_succeeds() {
        // With `max_per_topic = 3` and two existing entries, the next
        // write is still legal: `existing.len() == 2 < 3`, so the
        // tripwire does not fire. This is the boundary case — one
        // more write takes the topic *exactly to* the cap, and the
        // next one after that is the first one that must fail.
        let mem = fresh_memory();
        mem.put("notes", "first").await.unwrap();
        mem.put("notes", "second").await.unwrap();

        let writer = MemoryWriteTool::new(mem.clone()).set_max_per_topic(3);
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = writer
            .execute(json!({"topic": "notes", "body": "third"}), &ctx)
            .await;
        match outcome {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "notes");
                assert_eq!(output["seq"], 2);
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("at-cap write should Complete, got {other:?}"),
        }
        // And the topic is now exactly at cap — the next write must
        // refuse.
        assert_eq!(mem.get_recent("notes", 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn write_over_cap_fails_with_tool_error_naming_topic_and_cap() {
        let mem = fresh_memory();
        for i in 0..3 {
            mem.put("notes", &format!("entry {i}")).await.unwrap();
        }

        let writer = MemoryWriteTool::new(mem.clone()).set_max_per_topic(3);
        let chan = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&chan, &audit);

        let outcome = writer
            .execute(
                json!({"topic": "notes", "body": "one too many"}),
                &ctx,
            )
            .await;
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { tool, detail }) => {
                assert_eq!(tool, writer.id());
                // Recovery hint must name the topic (so the agent knows
                // *which* forget call to make) and the cap (so the
                // agent has grounds to tell the user this was a
                // configured tripwire, not substrate failure).
                assert!(
                    detail.contains("notes"),
                    "detail should name topic, got {detail}"
                );
                assert!(
                    detail.contains('3'),
                    "detail should mention the cap, got {detail}"
                );
                assert!(
                    detail.contains("memory.forget"),
                    "detail should suggest memory.forget, got {detail}"
                );
            }
            other => panic!("over-cap write must Fail with Tool error, got {other:?}"),
        }
        // The refused write must not have landed.
        assert_eq!(mem.get_recent("notes", 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn forget_clears_the_cap_so_next_write_succeeds() {
        // A full topic + forget + retry is the agent's documented
        // recovery path. This test walks it end-to-end through the
        // tool surface, to prove the detail message's advice actually
        // works.
        let mem = fresh_memory();
        for i in 0..3 {
            mem.put("notes", &format!("entry {i}")).await.unwrap();
        }

        let writer = MemoryWriteTool::new(mem.clone()).set_max_per_topic(3);
        let forget = MemoryForgetTool::new(mem.clone());
        let chan = fresh_channel();
        let audit = NullAuditHook;

        // Sanity: the next write fails.
        let ctx = make_ctx(&chan, &audit);
        let first = writer
            .execute(json!({"topic": "notes", "body": "blocked"}), &ctx)
            .await;
        assert!(matches!(first, ToolOutcome::Failed(_)));

        // Recovery: forget the topic.
        let ctx = make_ctx(&chan, &audit);
        let cleared = forget.execute(json!({"topic": "notes"}), &ctx).await;
        match cleared {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["deleted"], 3);
            }
            other => panic!("forget should Complete, got {other:?}"),
        }

        // Retry: the same write now succeeds.
        let ctx = make_ctx(&chan, &audit);
        let retry = writer
            .execute(json!({"topic": "notes", "body": "blocked"}), &ctx)
            .await;
        match retry {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "notes");
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("retry after forget should Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cap_is_per_topic_not_global() {
        // Fill topic `notes` to cap, then prove that `todos` — a
        // different topic — still accepts writes under the same tool
        // instance. This pins the cap as per-*qualifier*, matching
        // the scope model where `memory.write:topic:notes` and
        // `memory.write:topic:todos` are distinct capabilities.
        let mem = fresh_memory();
        for i in 0..3 {
            mem.put("notes", &format!("note {i}")).await.unwrap();
        }

        let writer = MemoryWriteTool::new(mem.clone()).set_max_per_topic(3);
        let chan = fresh_channel();
        let audit = NullAuditHook;

        let ctx = make_ctx(&chan, &audit);
        let blocked = writer
            .execute(json!({"topic": "notes", "body": "nope"}), &ctx)
            .await;
        assert!(matches!(blocked, ToolOutcome::Failed(_)));

        let ctx = make_ctx(&chan, &audit);
        let ok = writer
            .execute(json!({"topic": "todos", "body": "unrelated"}), &ctx)
            .await;
        match ok {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "todos");
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("other-topic write should Complete, got {other:?}"),
        }
    }

    #[test]
    fn tool_ids_are_unique_per_construction() {
        let a = MemoryReadTool::new(fresh_memory()).id();
        let b = MemoryReadTool::new(fresh_memory()).id();
        assert_ne!(a, b, "ToolId::new must mint fresh UUIDs");
    }

    // ---- Phase 8 task 2: session-scoped qualifier derivation ------

    #[test]
    fn read_scope_with_session_yields_both_qualifiers() {
        // With the turn loop's injected `session` field, the derived
        // scope gains a trailing `:session:<id>` qualifier. This is
        // what lets a capability bundle for chat A
        // (`memory.read:topic:notes:session:A`) fail to grant a read
        // attempt from chat B.
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({
            "topic": "notes",
            "session": "12345",
        }));
        assert_eq!(scope.base(), "memory.read");
        assert_eq!(scope.qualifier(), Some("topic:notes:session:12345"));
    }

    #[test]
    fn read_scopes_for_same_topic_different_sessions_are_distinct() {
        let tool = MemoryReadTool::new(fresh_memory());
        let a = tool.required_scope(&json!({"topic": "notes", "session": "A"}));
        let b = tool.required_scope(&json!({"topic": "notes", "session": "B"}));
        assert_ne!(
            a, b,
            "session qualifier must distinguish otherwise-identical reads"
        );
    }

    #[test]
    fn write_scope_with_session_is_session_qualified() {
        let tool = MemoryWriteTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({
            "topic": "secrets",
            "body": "x",
            "session": "chatA",
        }));
        assert_eq!(scope.qualifier(), Some("topic:secrets:session:chatA"));
    }

    #[test]
    fn forget_scope_with_session_is_session_qualified() {
        let tool = MemoryForgetTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({
            "topic": "notes",
            "session": "chatB",
        }));
        assert_eq!(scope.qualifier(), Some("topic:notes:session:chatB"));
    }

    #[test]
    fn empty_session_is_treated_as_none_for_scope_derivation() {
        // The injection point sends `""` only if a channel override
        // returns `Some("")`, which would be a channel bug — but we
        // still want the tool to degrade to unsession-qualified rather
        // than emitting a scope like `memory.read:topic:notes:session:`
        // that no capability bundle would ever grant.
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({"topic": "notes", "session": ""}));
        assert_eq!(scope.qualifier(), Some("topic:notes"));
    }

    #[test]
    fn reserved_prefix_topic_is_denied_even_with_session() {
        // A topic that literally starts with `\x01` would let an
        // agent side-door into another chat's physical key. Denied
        // regardless of whether `session` is present.
        let tool = MemoryReadTool::new(fresh_memory());
        let scope = tool.required_scope(&json!({
            "topic": "\x01s\x01other\x01notes",
            "session": "mine",
        }));
        assert!(scope.qualifier().unwrap().contains('\x00'));
    }

    // ---- Phase 8 task 2: physical isolation at the substrate ------

    #[tokio::test]
    async fn writes_in_different_sessions_do_not_share_storage() {
        // This is the whole point of Option B. Two sessions writing to
        // the same *logical* topic `notes` must land under different
        // physical keys so that a read from session A cannot observe
        // session B's body.
        let mem = fresh_memory();
        let writer = MemoryWriteTool::new(mem.clone());
        let reader = MemoryReadTool::new(mem.clone());
        let chan = fresh_channel();
        let audit = NullAuditHook;

        // Session A writes "purple" to `notes`.
        let ctx = make_ctx(&chan, &audit);
        let _ = writer
            .execute(
                json!({"topic": "notes", "body": "purple", "session": "A"}),
                &ctx,
            )
            .await;

        // Session B writes "green" to `notes`.
        let ctx = make_ctx(&chan, &audit);
        let _ = writer
            .execute(
                json!({"topic": "notes", "body": "green", "session": "B"}),
                &ctx,
            )
            .await;

        // Session A reads back — sees only purple.
        let ctx = make_ctx(&chan, &audit);
        let a_out = reader
            .execute(json!({"topic": "notes", "session": "A"}), &ctx)
            .await;
        let ToolOutcome::Completed { output, .. } = a_out else {
            panic!("session A read should Complete");
        };
        let entries_a = output["entries"].as_array().unwrap();
        assert_eq!(entries_a.len(), 1);
        assert_eq!(entries_a[0]["body"], "purple");
        // And the agent-visible topic is the logical name, not the
        // physical namespaced one.
        assert_eq!(entries_a[0]["topic"], "notes");
        assert_eq!(output["topic"], "notes");

        // Session B reads back — sees only green.
        let ctx = make_ctx(&chan, &audit);
        let b_out = reader
            .execute(json!({"topic": "notes", "session": "B"}), &ctx)
            .await;
        let ToolOutcome::Completed { output, .. } = b_out else {
            panic!("session B read should Complete");
        };
        let entries_b = output["entries"].as_array().unwrap();
        assert_eq!(entries_b.len(), 1);
        assert_eq!(entries_b[0]["body"], "green");
        assert_eq!(entries_b[0]["topic"], "notes");
    }

    #[tokio::test]
    async fn forget_in_one_session_does_not_clear_another() {
        let mem = fresh_memory();
        let writer = MemoryWriteTool::new(mem.clone());
        let forget = MemoryForgetTool::new(mem.clone());
        let reader = MemoryReadTool::new(mem.clone());
        let chan = fresh_channel();
        let audit = NullAuditHook;

        let ctx = make_ctx(&chan, &audit);
        let _ = writer
            .execute(
                json!({"topic": "notes", "body": "keep me", "session": "A"}),
                &ctx,
            )
            .await;
        let ctx = make_ctx(&chan, &audit);
        let _ = writer
            .execute(
                json!({"topic": "notes", "body": "me too", "session": "B"}),
                &ctx,
            )
            .await;

        // B forgets — A's entry must survive.
        let ctx = make_ctx(&chan, &audit);
        let cleared = forget
            .execute(json!({"topic": "notes", "session": "B"}), &ctx)
            .await;
        match cleared {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["deleted"], 1);
            }
            other => panic!("forget B should Complete, got {other:?}"),
        }

        let ctx = make_ctx(&chan, &audit);
        let a_out = reader
            .execute(json!({"topic": "notes", "session": "A"}), &ctx)
            .await;
        if let ToolOutcome::Completed { output, .. } = a_out {
            let entries = output["entries"].as_array().unwrap();
            assert_eq!(entries.len(), 1, "session A should be untouched");
            assert_eq!(entries[0]["body"], "keep me");
        } else {
            panic!("session A read should still Complete");
        }
    }

    #[tokio::test]
    async fn per_topic_cap_is_per_session() {
        // Filling session A's `notes` to cap must not refuse session
        // B's unrelated `notes` writes.
        let mem = fresh_memory();
        let writer = MemoryWriteTool::new(mem.clone()).set_max_per_topic(2);
        let chan = fresh_channel();
        let audit = NullAuditHook;

        // A fills to cap.
        for i in 0..2 {
            let ctx = make_ctx(&chan, &audit);
            let out = writer
                .execute(
                    json!({
                        "topic": "notes",
                        "body": format!("a{i}"),
                        "session": "A",
                    }),
                    &ctx,
                )
                .await;
            assert!(matches!(out, ToolOutcome::Completed { .. }));
        }

        // A's next write is refused.
        let ctx = make_ctx(&chan, &audit);
        let blocked = writer
            .execute(
                json!({"topic": "notes", "body": "over", "session": "A"}),
                &ctx,
            )
            .await;
        assert!(matches!(blocked, ToolOutcome::Failed(_)));

        // But B's first write still succeeds — different bucket.
        let ctx = make_ctx(&chan, &audit);
        let ok = writer
            .execute(
                json!({"topic": "notes", "body": "b0", "session": "B"}),
                &ctx,
            )
            .await;
        match ok {
            ToolOutcome::Completed { output, verified } => {
                assert_eq!(output["topic"], "notes");
                assert_eq!(verified, Verification::Verified);
            }
            other => panic!("B's first write should Complete, got {other:?}"),
        }
    }
}
