//! The concrete reference `Agent` implementation — the Phase 1 turn loop.
//!
//! `ConcreteAgent` composes:
//! - a `CapabilitySet` (the agent's granted scopes)
//! - a `ToolRegistry` (tools it can call)
//! - a `TurnPlanner` (what drives the tool-calling loop — fake in Phase 1)
//! - an `AuditHook` (where audit events go — any `aivyx_audit::AuditWriter`)
//!
//! The loop follows D1's paragraph, in order:
//!
//! 1. Generate `TurnId`. Resolve the channel's trust tier.
//! 2. Compute `effective = caps.intersect(tier.default_ceiling())`.
//! 3. Emit `TurnStarted` audit event.
//! 4. Tool-calling loop. Each pass: check cancellation (→ `Cancelled` if
//!    fired), ask the planner for the next step, and dispatch.
//!    On `ToolCall`: compute required scope via R1, check `effective.grants`,
//!    then either execute and emit a `ToolCall` audit or emit `ScopeDenied`
//!    and pass a `Denied` observation back to the planner.
//!    On `FinalMessage` / `Stop`: terminate with `Completed`.
//! 5. Emit `TurnEnded`, finalize the channel, return.
//!
//! Deferred from Phase 1:
//! - Timeout enforcement (variant exists, loop doesn't check a budget yet —
//!   real deadlines land when a real use case shows up)
//! - `RequiresEscalation` propagation from tools (variant exists, no tool
//!   emits it in Phase 1)
//! - LLM-backed planning (covered by Phase 2's `LlmProvider` + its own
//!   `TurnPlanner` impl)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use aivyx_capability::{CapabilitySet, Scope};

use crate::{
    Agent, AgentId, AivyxError, AuditHook, AuditTag, CancellationToken, ChannelContext, Message,
    StreamEvent, ToolContext, ToolId, ToolOutcome, ToolOutcomeSummary, TurnId, TurnOutcome,
    TurnOutcomeSummary, VerificationSummary,
};
use crate::planner::{NextStep, StepObservation, ToolRegistry, TurnPlanner};

/// Hard upper bound on steps per turn. The LLM-backed planner added in
/// Phase 2 is the first planner that *can* loop indefinitely
/// (`VecPlanner` is bounded by its script length), so the loop now
/// guards against a runaway agent with a fixed budget. 32 is high
/// enough for realistic tool chains and low enough that a misbehaving
/// planner fails loudly rather than burning the host.
pub const MAX_STEPS_PER_TURN: usize = 32;

/// Wall-clock deadline for a single turn. Phase 3 task 4 adds the first
/// code path that emits `TurnOutcome::TimedOut`. A background task
/// spawned by the turn loop cancels the channel's cancellation token
/// when the deadline fires; the planner's mid-stream cancel check then
/// propagates the cancel and the loop translates it into `TimedOut`
/// rather than `Cancelled`.
///
/// 120 seconds is deliberately generous — a multi-step tool chain with
/// several large LLM completions can legitimately take most of a
/// minute, and the point of the budget is to catch *stuck* turns, not
/// to police slow ones. Follows the same "const, not config knob"
/// philosophy as [`MAX_STEPS_PER_TURN`]: a caller who needs a custom
/// budget is almost certainly papering over a real bug.
pub const TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// The reference `Agent` implementation.
///
/// Holds all the collaborators a turn loop needs by `Arc` / interior
/// mutability so that the `Agent::turn(&self, ...)` contract holds:
/// concurrent turns share the agent via `Arc<dyn Agent>`, and each turn
/// produces its own transient state over shared immutable collaborators.
pub struct ConcreteAgent {
    id: AgentId,
    capabilities: CapabilitySet,
    tools: Arc<ToolRegistry>,
    audit: Arc<dyn AuditHook>,
    /// Planner factory: called once per turn. We store a boxed `Fn` so
    /// callers can hand us a fresh planner per turn without us needing to
    /// own a `Mutex<Planner>` (which would serialize concurrent turns).
    planner_factory: Box<dyn Fn() -> Box<dyn TurnPlanner> + Send + Sync>,
    /// Phase 11 Task 2 — role-derived memory topic prefix. When `Some`,
    /// the turn loop injects a `"role_prefix"` key into every tool
    /// input under the same dispatch-layer mechanism that Phase 8
    /// Task 2 uses for `"session"`. Memory tools consume it to
    /// namespace logical topics per role; tools that don't consume
    /// it ignore the extra field. `None` preserves Phase 6–10
    /// behavior byte-for-byte: a bare topic name hits the substrate
    /// unchanged.
    memory_topic_prefix: Option<String>,
    /// Phase 11 Task 4 — role-derived tool allowlist gate (the
    /// belt-and-suspenders dispatch-layer check). When `Some`, the
    /// turn loop rejects any tool call whose name is not in the
    /// set, **before** session/prefix injection and **before** the
    /// capability scope check. The rejection synthesizes a
    /// `tool.allowlist:<tool_name>` scope and routes through
    /// `ToolOutcome::Denied { scope, held }` unchanged, so auditors
    /// grep `scope_requested.base() == "tool.allowlist"` to
    /// distinguish role rejection from capability rejection.
    ///
    /// `None` means "no filter — allow every registered tool,"
    /// preserving Phase 6–10 behavior for agents built without a
    /// role. This is the primary safety net for the planner-layer
    /// filter in `LlmPlannerConfig`: the planner never advertises
    /// filtered tools to the model, so the model never tries to
    /// call them, but a stale-history tool_use block (e.g. from a
    /// resumed conversation) or a non-LLM planner could still
    /// produce an out-of-role call. This check catches that.
    tool_allowlist: Option<std::collections::BTreeSet<String>>,
}

impl ConcreteAgent {
    pub fn new(
        id: AgentId,
        capabilities: CapabilitySet,
        tools: Arc<ToolRegistry>,
        audit: Arc<dyn AuditHook>,
        planner_factory: impl Fn() -> Box<dyn TurnPlanner> + Send + Sync + 'static,
    ) -> Self {
        ConcreteAgent {
            id,
            capabilities,
            tools,
            audit,
            planner_factory: Box::new(planner_factory),
            memory_topic_prefix: None,
            tool_allowlist: None,
        }
    }

    /// Attach a role-derived memory topic prefix. Builder-style so
    /// existing `ConcreteAgent::new` call sites remain byte-identical
    /// when no role is in play. Task 4 of Phase 11 wires this from
    /// `cfg.roles[active_role].memory_topic_prefix` at session
    /// construction time.
    pub fn with_memory_topic_prefix(mut self, prefix: Option<String>) -> Self {
        self.memory_topic_prefix = prefix;
        self
    }

    /// Attach a role-derived tool allowlist. See the
    /// [`Self::tool_allowlist`] field doc for semantics. `None`
    /// means "no filter," preserving legacy behavior. Task 4 of
    /// Phase 11 wires this from
    /// `cfg.roles[active_role].tool_allowlist` at session
    /// construction time.
    pub fn with_tool_allowlist(
        mut self,
        allowlist: Option<std::collections::BTreeSet<String>>,
    ) -> Self {
        self.tool_allowlist = allowlist;
        self
    }
}

#[async_trait]
impl Agent for ConcreteAgent {
    fn id(&self) -> AgentId {
        self.id
    }

    fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    async fn turn(
        &self,
        message: Message,
        channel: &dyn ChannelContext,
    ) -> TurnOutcome {
        let turn_id = TurnId::new();
        let session_id = channel.session_id();
        let tier = channel.trust_tier();
        let effective = self.capabilities.intersect(tier.default_ceiling());
        let cancellation = channel.cancellation_token();
        let start = Instant::now();

        // D1: "trust tier resolution happens first" — fire TurnStarted
        // *before* the loop begins, with the authoritative effective set.
        self.audit.on_event(AuditTag::TurnStarted {
            turn_id,
            session_id,
            channel: channel.platform(),
            trust_tier: tier,
            effective_capabilities: effective.clone(),
        });

        let mut planner = (self.planner_factory)();
        planner.begin_turn(&message).await;

        // Wall-clock deadline task. Spawns in the background, sleeps
        // for TURN_TIMEOUT, and then (a) sets the deadline_fired flag
        // so the loop's outcome translation can distinguish TimedOut
        // from Cancelled, and (b) cancels the channel's token so the
        // planner's in-flight stream (if any) gets interrupted. We
        // hold a handle so the task is aborted cleanly when the turn
        // ends normally — otherwise a fleet of long-running agents
        // would leak timeout tasks until they eventually fired.
        let deadline_fired = Arc::new(AtomicBool::new(false));
        let deadline_task = {
            let deadline_fired = Arc::clone(&deadline_fired);
            let token = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(TURN_TIMEOUT).await;
                deadline_fired.store(true, Ordering::SeqCst);
                token.cancel();
            })
        };

        let mut observed: Vec<StepObservation> = Vec::new();
        let mut tool_calls_made: usize = 0;
        let mut final_message: String = String::new();
        let mut steps: usize = 0;
        let loop_outcome: LoopOutcome;

        loop {
            if cancellation.is_cancelled() {
                loop_outcome = if deadline_fired.load(Ordering::SeqCst) {
                    LoopOutcome::TimedOut
                } else {
                    LoopOutcome::Cancelled
                };
                break;
            }
            if steps >= MAX_STEPS_PER_TURN {
                loop_outcome = LoopOutcome::MaxStepsExceeded;
                break;
            }
            steps += 1;

            let step = planner.next_step(&observed, channel).await;

            // Post-next_step cancellation re-check. The planner may
            // have returned because it detected cancellation inside
            // its own stream consumer (mid-LLM-completion) — in that
            // case we must NOT process its return value as a
            // FinalMessage / Stop, because doing so would emit a
            // Completed outcome when the turn was actually
            // interrupted. We let the next loop iteration's top-of-
            // loop check handle the termination uniformly.
            if cancellation.is_cancelled() {
                loop_outcome = if deadline_fired.load(Ordering::SeqCst) {
                    LoopOutcome::TimedOut
                } else {
                    LoopOutcome::Cancelled
                };
                break;
            }

            match step {
                NextStep::FinalMessage(msg) => {
                    final_message = msg;
                    loop_outcome = LoopOutcome::Completed;
                    break;
                }
                NextStep::Stop => {
                    loop_outcome = LoopOutcome::Completed;
                    break;
                }
                NextStep::ToolCall { tool_id, input } => {
                    tool_calls_made += 1;
                    let (observation, outcome) = self
                        .run_tool_call(
                            turn_id,
                            tool_id,
                            input,
                            channel,
                            &cancellation,
                            &effective,
                        )
                        .await;
                    observed.push(observation);
                    planner.observe_tool_outcome(tool_id, &outcome).await;
                }
            }
        }

        // Abort the deadline task — it's either (a) already fired and
        // cancelled the token, in which case the abort is a no-op, or
        // (b) still sleeping, in which case we want it gone so it
        // doesn't leak. Either way, explicit abort is cheap and
        // intentional.
        deadline_task.abort();

        let duration = start.elapsed();
        let outcome = match loop_outcome {
            LoopOutcome::Completed => TurnOutcome::Completed {
                final_message,
                tool_calls_made,
                duration,
            },
            LoopOutcome::Cancelled => TurnOutcome::Cancelled { tool_calls_made },
            LoopOutcome::TimedOut => TurnOutcome::TimedOut {
                tool_calls_made,
                elapsed: duration,
            },
            LoopOutcome::MaxStepsExceeded => TurnOutcome::Failed(AivyxError::Internal(
                format!("planner exceeded {MAX_STEPS_PER_TURN} steps per turn"),
            )),
        };

        self.audit.on_event(AuditTag::TurnEnded {
            turn_id,
            outcome: TurnOutcomeSummary::from(&outcome),
            tool_calls_made,
            duration,
            usage: planner.turn_usage(),
        });

        // D1: "returning a TurnOutcome to the channel, and yielding control."
        // finalize is synchronous from the loop's perspective — if the
        // channel send fails we downgrade to Failed but still return an
        // outcome, per D3's "every turn completes in some way."
        if let Err(e) = channel.finalize(&outcome).await {
            return TurnOutcome::Failed(crate::AivyxError::Channel(e.to_string()));
        }

        outcome
    }
}

/// Internal loop termination reason before it's translated into a public
/// `TurnOutcome`. Phase 2 added `MaxStepsExceeded`; Phase 3 task 4
/// adds `TimedOut` (the first code path that emits the long-
/// advertised `TurnOutcome::TimedOut`). `Escalated` will join this
/// enum if and when a real tool emits it.
enum LoopOutcome {
    Completed,
    Cancelled,
    TimedOut,
    MaxStepsExceeded,
}

impl ConcreteAgent {
    /// Execute one tool call: resolve the tool, compute its required
    /// scope via R1, scope-check, execute-or-deny, emit the matching
    /// audit event, and return both the observation (for the
    /// `StepObservation` trail) and the full `ToolOutcome` (for the
    /// planner's `observe_tool_outcome` callback). The observation is
    /// what the audit sees; the full outcome is what a smart planner
    /// (e.g. the LLM planner) needs to reason about next.
    async fn run_tool_call(
        &self,
        turn_id: TurnId,
        tool_id: ToolId,
        input: serde_json::Value,
        channel: &dyn ChannelContext,
        cancellation: &CancellationToken,
        effective: &CapabilitySet,
    ) -> (StepObservation, ToolOutcome) {
        let Some(tool) = self.tools.get(tool_id) else {
            // Unknown tool — no scope check possible. This shouldn't happen
            // with a well-behaved planner; treat it as a failed step and
            // synthesize a Failed outcome so the planner sees it too.
            let outcome = ToolOutcome::Failed(AivyxError::NotFound {
                kind: "tool",
                id: tool_id.to_string(),
            });
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Failed,
                },
                outcome,
            );
        };

        // Phase 10 Task 2 — hand-rolled JSON-schema validation.
        //
        // Runs *before* session injection, not after: memory tool
        // schemas set `additionalProperties: false` and do not
        // declare a `session` property, so validating the
        // post-injection input would reject every memory call the
        // moment a Telegram-style channel provides a partition. The
        // session field is a turn-loop internal, not an agent-
        // visible surface, so it sits outside the schema contract.
        //
        // On mismatch the loop short-circuits to
        // `ToolOutcome::Failed` with a human-readable detail, which
        // the planner observes via `observe_tool_outcome` exactly
        // like any other tool failure. We deliberately do NOT route
        // through the deny-scope path: structural malformation is a
        // *planner* bug (or prompt-injection attempt), not a
        // capability question, and routing it through `Denied`
        // would pollute the scope-denial telemetry stream.
        if let Err(err) = crate::schema::validate(tool.input_schema(), &input) {
            let outcome = ToolOutcome::Failed(AivyxError::Tool {
                tool: tool_id,
                detail: format!("input validation failed: {err}"),
            });
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Failed,
                },
                outcome,
            );
        }

        // Phase 11 Task 4 — role-allowlist dispatch-layer gate.
        //
        // If the agent was built with an explicit `tool_allowlist`
        // (from `cfg.roles[active_role].tool_allowlist`), reject
        // any call whose tool name is not in the set. The
        // primary enforcement is at the planner layer
        // (`LlmPlannerConfig` filters the tool catalog before
        // advertising to Anthropic, so the model never sees
        // disallowed tools), but this belt-and-suspenders check
        // catches stale tool_use blocks from resumed conversations
        // and non-LLM planners that don't go through the
        // advertisement filter.
        //
        // Ordering matters: runs *after* schema validation (so
        // malformed input routes to `Failed`, not `Denied`) and
        // *before* session/prefix injection and `required_scope`
        // (so the identity gate "may this role use this tool"
        // fires before the authority gate "what scope does this
        // call need"). Q1 Option A: reuse `ToolOutcome::Denied`
        // with a synthetic `tool.allowlist:<tool_name>` scope.
        // Auditors distinguish role rejection from capability
        // rejection by `scope_requested.base() == "tool.allowlist"`.
        // No new `ToolOutcome` variant, so the production-core
        // byte streak (broken once at Task 3) stays at a single
        // Phase 11 break.
        let tool_name_str = tool.name().to_string();
        if let Some(allowlist) = self.tool_allowlist.as_ref()
            && !allowlist.contains(&tool_name_str)
        {
            // `Scope::parse` must accept this because Phase 11
            // Task 4 added `tool.allowlist` to the aivyx-capability
            // `KNOWN_BASES` allowlist. `expect` is correct: an
            // unparseable synthetic scope is a bug in the
            // capability crate's base list, not a runtime
            // condition we should handle.
            let synthetic = Scope::parse(&format!("tool.allowlist:{tool_name_str}"))
                .expect("tool.allowlist:<name> must parse — see aivyx-capability KNOWN_BASES");
            self.audit.on_event(AuditTag::ScopeDenied {
                turn_id,
                tool_attempted: tool_id,
                scope_requested: synthetic.clone(),
                held_capabilities: effective.clone(),
            });
            let outcome = ToolOutcome::NotInRole {
                tool_name: tool_name_str,
            };
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::NotInRole,
                },
                outcome,
            );
        }

        // Phase 8 Task 2 — session partition injection. Channels that
        // want per-instance memory isolation (Telegram: one chat = one
        // partition) override `ChannelContext::session_partition`. The
        // turn loop threads that partition into the tool's JSON input
        // under a reserved `"session"` key *before* `required_scope`
        // runs, so session-scoped tools (memory.read/write/forget)
        // derive a `session:<partition>` qualifier that the capability
        // check enforces. The LLM never sees this field — it is not
        // in any advertised `input_schema` and is added after the
        // planner emits the call. Non-object inputs (unlikely — all
        // current tools take object inputs) are left untouched.
        let mut input = input;
        if let Some(partition) = channel.session_partition()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert("session".to_string(), serde_json::Value::String(partition));
        }

        // Phase 11 Task 2 — role memory-topic-prefix injection.
        //
        // Same dispatch-layer pattern as session injection above:
        // the prefix is a turn-loop internal, not in any advertised
        // `input_schema`, not visible to the LLM, and not carried
        // into audit or scope qualifiers (those stay keyed on the
        // logical topic the agent actually typed — e.g. `notes` —
        // so an audit chain is identical whether the role was
        // `coder` or `default`).
        //
        // Memory tools read the `"role_prefix"` key via a local
        // helper and prepend it to the logical topic before handing
        // the physical key to the substrate. Tools that don't
        // consume memory ignore the extra field entirely; the
        // validator already ran (above), so the injected key
        // cannot trigger an `additionalProperties: false` rejection.
        if let Some(prefix) = self.memory_topic_prefix.as_ref()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert(
                "role_prefix".to_string(),
                serde_json::Value::String(prefix.clone()),
            );
        }

        let needed: Scope = tool.required_scope(&input);

        if !effective.grants(&needed) {
            // D4: scope denial emits a `ScopeDenied` audit event carrying
            // the held snapshot. The planner observes a `Denied` summary
            // via the observation trail and a full `Denied { scope, held }`
            // outcome via `observe_tool_outcome`.
            self.audit.on_event(AuditTag::ScopeDenied {
                turn_id,
                tool_attempted: tool_id,
                scope_requested: needed.clone(),
                held_capabilities: effective.clone(),
            });
            let outcome = ToolOutcome::Denied {
                scope: needed,
                held: effective.clone(),
            };
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Denied,
                },
                outcome,
            );
        }

        let ctx = ToolContext {
            agent_id: self.id,
            session_id: channel.session_id(),
            turn_id,
            channel,
            audit: self.audit.as_ref(),
            cancellation,
        };

        let input_bytes = serde_json::to_vec(&input).unwrap_or_default();
        let input_hash = sha256_array(&input_bytes);

        // Phase 10 task 3: `ToolCallStarted` was defined in Phase 5
        // but never actually emitted — renderers had a placeholder
        // arm since then. This is the real emission site. Channel
        // errors from the stream are intentionally swallowed: a
        // renderer refusing an event is not a reason to abort the
        // tool call (same rule `llm_planner.rs` applies to
        // streamed text chunks).
        let tool_name = tool.name();
        let _ = channel
            .stream_event(StreamEvent::ToolCallStarted {
                tool: tool_id,
                tool_name,
                input: &input,
            })
            .await;

        let step_start = Instant::now();
        let outcome = tool.execute(input, &ctx).await;
        let step_duration = step_start.elapsed();

        let summary = ToolOutcomeSummary::from(&outcome);

        // Phase 10 task 3: matched `ToolCallFinished` emission. The
        // summary string is a short static label per variant — no
        // allocation, safe to borrow into the event's `&'a str`
        // lifetime. Renderers that want more detail can keep their
        // own state keyed on `tool` across the start/finish pair.
        let outcome_summary_str = tool_outcome_summary_str(&summary);
        let _ = channel
            .stream_event(StreamEvent::ToolCallFinished {
                tool: tool_id,
                tool_name,
                outcome_summary: outcome_summary_str,
            })
            .await;

        self.audit.on_event(AuditTag::ToolCall {
            turn_id,
            tool_id,
            scope_used: needed,
            input_hash,
            outcome: summary.clone(),
            duration: step_duration,
        });

        (StepObservation { tool_id, summary }, outcome)
    }
}

/// Short static label for a `ToolOutcomeSummary`, used as the
/// `outcome_summary` field of `StreamEvent::ToolCallFinished`.
/// Static strings so the event can borrow them for its `&'a str`
/// slot without taking a lifetime on the local function frame.
fn tool_outcome_summary_str(s: &ToolOutcomeSummary) -> &'static str {
    match s {
        ToolOutcomeSummary::Completed {
            verified: VerificationSummary::Verified,
        } => "completed (verified)",
        ToolOutcomeSummary::Completed {
            verified: VerificationSummary::Unverified,
        } => "completed (unverified)",
        ToolOutcomeSummary::Completed {
            verified: VerificationSummary::NotApplicable,
        } => "completed",
        ToolOutcomeSummary::Denied => "denied",
        ToolOutcomeSummary::NotInRole => "not in role",
        ToolOutcomeSummary::RequiresEscalation => "requires escalation",
        ToolOutcomeSummary::Failed => "failed",
    }
}

fn sha256_array(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

// ---------------------------------------------------------------------------
// Tests — the first end-to-end turn-loop run.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use aivyx_capability::TrustTier;

    use crate::{
        AuditTag, ChannelError, ChannelPlatform, NullAuditHook, SessionId, StreamEvent, Tool,
        ToolOutcome, Verification,
    };

    // ---- Test fakes ----

    /// Records every audit event the loop emits so tests can assert on
    /// the exact sequence. Wraps `AuditTag` rather than re-serializing
    /// through `aivyx_audit` because the HMAC-chain property is already
    /// tested in that crate — here we're testing the *loop's* behavior.
    #[derive(Default)]
    struct RecordingAudit {
        events: Mutex<Vec<AuditTag>>,
    }

    impl RecordingAudit {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn snapshot(&self) -> Vec<AuditTag> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AuditHook for RecordingAudit {
        fn on_event(&self, tag: AuditTag) {
            self.events.lock().unwrap().push(tag);
        }
    }

    /// Fake channel. Records stream events and finalize calls so tests
    /// can assert the loop talked to the channel in the right order.
    struct FakeChannel {
        session: SessionId,
        platform: ChannelPlatform,
        tier: TrustTier,
        token: CancellationToken,
        finalized: Mutex<Option<TurnOutcomeSummary>>,
        stream_calls: Mutex<usize>,
    }

    impl FakeChannel {
        fn new(platform: ChannelPlatform, tier: TrustTier) -> Self {
            FakeChannel {
                session: SessionId::new(),
                platform,
                tier,
                token: CancellationToken::new(),
                finalized: Mutex::new(None),
                stream_calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl ChannelContext for FakeChannel {
        fn channel_name(&self) -> &str {
            "fake"
        }
        fn platform(&self) -> ChannelPlatform {
            self.platform
        }
        fn trust_tier(&self) -> TrustTier {
            self.tier
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
            *self.stream_calls.lock().unwrap() += 1;
            Ok(())
        }
        async fn finalize(&self, outcome: &TurnOutcome) -> Result<(), ChannelError> {
            *self.finalized.lock().unwrap() = Some(TurnOutcomeSummary::from(outcome));
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    /// Phase 10 task 3: a channel that records the structured
    /// shape of every StreamEvent it sees. FakeChannel just counts,
    /// which is enough for existing tests; the tool-name emission
    /// tests need to assert on the actual event contents. Keep
    /// it minimal — only the fields the tests actually inspect.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedEvent {
        Text(String),
        Status(String),
        ToolCallStarted { tool_name: String },
        ToolCallFinished { tool_name: String, summary: String },
        Attachment,
        ToolOutput { tool_name: String, chunk: String },
    }

    struct RecordingChannel {
        session: SessionId,
        token: CancellationToken,
        events: Mutex<Vec<RecordedEvent>>,
    }

    impl RecordingChannel {
        fn new() -> Self {
            RecordingChannel {
                session: SessionId::new(),
                token: CancellationToken::new(),
                events: Mutex::new(Vec::new()),
            }
        }
        fn snapshot(&self) -> Vec<RecordedEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelContext for RecordingChannel {
        fn channel_name(&self) -> &str {
            "recording"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> TrustTier {
            TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
            let rec = match event {
                StreamEvent::Text(s) => RecordedEvent::Text(s.to_string()),
                StreamEvent::Status(s) => RecordedEvent::Status(s.to_string()),
                StreamEvent::ToolCallStarted { tool_name, .. } => {
                    RecordedEvent::ToolCallStarted {
                        tool_name: tool_name.to_string(),
                    }
                }
                StreamEvent::ToolCallFinished {
                    tool_name,
                    outcome_summary,
                    ..
                } => RecordedEvent::ToolCallFinished {
                    tool_name: tool_name.to_string(),
                    summary: outcome_summary.to_string(),
                },
                StreamEvent::Attachment { .. } => RecordedEvent::Attachment,
                StreamEvent::ToolOutput {
                    tool_name, chunk, ..
                } => RecordedEvent::ToolOutput {
                    tool_name: tool_name.to_string(),
                    chunk: chunk.to_string(),
                },
            };
            self.events.lock().unwrap().push(rec);
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    /// A fake tool that always completes successfully. `required_scope`
    /// is parameterized so tests can construct tools that need either a
    /// bare or qualified scope.
    struct FakeTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
        scope_fn: Box<dyn Fn(&Value) -> Scope + Send + Sync>,
    }

    impl FakeTool {
        fn new_bare(name: &'static str, scope: &str) -> Self {
            let s = Scope::parse(scope).unwrap();
            FakeTool {
                id: ToolId::new(),
                name,
                schema: json!({}),
                scope_fn: Box::new(move |_| s.clone()),
            }
        }
        fn new_r1(
            name: &'static str,
            f: impl Fn(&Value) -> Scope + Send + Sync + 'static,
        ) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema: json!({}),
                scope_fn: Box::new(f),
            }
        }

        /// Task-2 helper: build a FakeTool whose input_schema is a
        /// real JSON-Schema fragment. `scope_fn` is still invoked
        /// for well-formed inputs; callers that expect validation
        /// to short-circuit before `scope_fn` can plant a panicking
        /// closure there to prove the validator fired first.
        fn new_with_schema(
            name: &'static str,
            schema: Value,
            f: impl Fn(&Value) -> Scope + Send + Sync + 'static,
        ) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema,
                scope_fn: Box::new(f),
            }
        }
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn input_schema(&self) -> &Value {
            &self.schema
        }
        fn required_scope(&self, input: &Value) -> Scope {
            (self.scope_fn)(input)
        }
        async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
    }

    // Phase 12 task 1 test subject: a tool that streams a scripted
    // sequence of chunks via `StreamEvent::ToolOutput` before
    // completing. Exists only to give Task 1 a real driver for the
    // streaming seam without waiting for Task 2's `web.fetch` to
    // land — keeping Task 1 pure infrastructure per the phase draft.
    // Not registered anywhere in production.
    struct ScriptedStreamingTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
        scope: Scope,
        chunks: Vec<String>,
    }

    impl ScriptedStreamingTool {
        fn new(name: &'static str, scope: &str, chunks: Vec<&str>) -> Self {
            ScriptedStreamingTool {
                id: ToolId::new(),
                name,
                schema: json!({}),
                scope: Scope::parse(scope).unwrap(),
                chunks: chunks.into_iter().map(String::from).collect(),
            }
        }
    }

    #[async_trait]
    impl Tool for ScriptedStreamingTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "scripted streaming test tool"
        }
        fn input_schema(&self) -> &Value {
            &self.schema
        }
        fn required_scope(&self, _input: &Value) -> Scope {
            self.scope.clone()
        }
        async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
            for chunk in &self.chunks {
                let _ = ctx
                    .channel
                    .stream_event(StreamEvent::ToolOutput {
                        tool: self.id,
                        tool_name: self.name,
                        chunk: chunk.as_str(),
                    })
                    .await;
            }
            ToolOutcome::Completed {
                output: json!({"streamed": self.chunks.len()}),
                verified: Verification::NotApplicable,
            }
        }
    }

    fn make_agent(
        caps: CapabilitySet,
        tools: Vec<Arc<dyn Tool>>,
        audit: Arc<dyn AuditHook>,
        plan: Vec<NextStep>,
    ) -> ConcreteAgent {
        let registry = Arc::new(ToolRegistry::new(tools));
        let plan_arc = Arc::new(plan);
        ConcreteAgent::new(AgentId::new(), caps, registry, audit, move || {
            Box::new(crate::planner::VecPlanner::new((*plan_arc).clone()))
        })
    }

    // ---- Golden path: one tool call, final message, clean completion ----

    #[tokio::test]
    async fn golden_path_completes_with_correct_audit_trail() {
        let audit = RecordingAudit::new();

        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({"query": "yesterday"}),
            },
            NextStep::FinalMessage("here's what I found".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "what did I work on yesterday?");
        let outcome = agent.turn(message, &channel).await;

        // Return-value shape
        match outcome {
            TurnOutcome::Completed {
                final_message,
                tool_calls_made,
                ..
            } => {
                assert_eq!(final_message, "here's what I found");
                assert_eq!(tool_calls_made, 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Audit trail: exactly TurnStarted → ToolCall → TurnEnded
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(events[1], AuditTag::ToolCall { .. }));
        assert!(matches!(events[2], AuditTag::TurnEnded { .. }));

        // Channel was finalized exactly once with a Completed summary
        assert_eq!(
            *channel.finalized.lock().unwrap(),
            Some(TurnOutcomeSummary::Completed)
        );
    }

    // ---- Phase 12 task 1: streaming tool output ----
    //
    // A tool that emits `StreamEvent::ToolOutput` chunks from inside
    // its `execute` body must see every chunk land on the channel in
    // order, bracketed by the usual `ToolCallStarted` / `ToolCallFinished`
    // markers. Audit chain must still record exactly one `ToolCall`
    // entry per tool call — chunks are a rendering concern, not a
    // forensic one, and the `--verify-only` walker depends on
    // one-entry-per-tool-call staying true.

    #[tokio::test]
    async fn streaming_tool_output_chunks_land_in_order_between_start_and_finish_markers(
    ) {
        let audit = RecordingAudit::new();

        // Tool name stays `test.stream` so the event assertions are
        // specific, but the *scope* has to be a real base from
        // `KNOWN_BASES` — the capability layer's allowlist rejects
        // unknown bases at parse time. `memory.read` is the
        // established test-fixture stand-in for "some bare scope";
        // see `FakeTool::new_bare("memory.read", "memory.read")` in
        // the golden-path test above.
        let tool = Arc::new(ScriptedStreamingTool::new(
            "test.stream",
            "memory.read",
            vec!["chunk-one ", "chunk-two ", "chunk-three"],
        ));
        let tool_id = tool.id();

        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({}),
            },
            NextStep::FinalMessage("done".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = RecordingChannel::new();
        let message = Message::text(channel.session, "stream please");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed, got {other:?}"),
        }

        // The recorded event sequence must be:
        //   ToolCallStarted → ToolOutput × 3 → ToolCallFinished → (any trailing Text)
        // We do not pin the absolute index of the start marker (the
        // turn loop may emit non-tool events around it), but we do
        // pin relative order and the chunk payloads.
        let events = channel.events.lock().unwrap().clone();

        // Collect (index, kind) tuples for the events we care about.
        let mut start_idx = None;
        let mut finish_idx = None;
        let mut output_indices: Vec<(usize, String)> = Vec::new();
        for (i, ev) in events.iter().enumerate() {
            match ev {
                RecordedEvent::ToolCallStarted { tool_name } if tool_name == "test.stream" => {
                    start_idx = Some(i);
                }
                RecordedEvent::ToolCallFinished { tool_name, .. }
                    if tool_name == "test.stream" =>
                {
                    finish_idx = Some(i);
                }
                RecordedEvent::ToolOutput { tool_name, chunk }
                    if tool_name == "test.stream" =>
                {
                    output_indices.push((i, chunk.clone()));
                }
                _ => {}
            }
        }

        let start = start_idx.expect("ToolCallStarted must be recorded");
        let finish = finish_idx.expect("ToolCallFinished must be recorded");
        assert_eq!(
            output_indices.len(),
            3,
            "expected 3 ToolOutput chunks, got {}",
            output_indices.len()
        );
        assert!(
            start < output_indices[0].0,
            "first chunk must come after ToolCallStarted"
        );
        assert!(
            output_indices[2].0 < finish,
            "last chunk must come before ToolCallFinished"
        );
        assert_eq!(output_indices[0].1, "chunk-one ");
        assert_eq!(output_indices[1].1, "chunk-two ");
        assert_eq!(output_indices[2].1, "chunk-three");
        // Chunks are in sequence with no interleaving of other
        // `test.stream` events between them.
        assert_eq!(output_indices[1].0, output_indices[0].0 + 1);
        assert_eq!(output_indices[2].0, output_indices[1].0 + 1);
    }

    #[tokio::test]
    async fn streaming_tool_output_produces_exactly_one_audit_entry_regardless_of_chunk_count(
    ) {
        let audit = RecordingAudit::new();

        // Five chunks — the assertion is that N chunks produce
        // exactly one ToolCall audit entry, not N+1 or N. This
        // invariant is load-bearing for the --verify-only forensic
        // walker and is part of the Phase 12 task 1 acceptance list.
        let tool = Arc::new(ScriptedStreamingTool::new(
            "test.stream",
            "memory.read",
            vec!["a", "b", "c", "d", "e"],
        ));
        let tool_id = tool.id();

        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({}),
            },
            NextStep::FinalMessage("ok".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);
        let channel = RecordingChannel::new();
        let message = Message::text(channel.session, "stream five");
        let _ = agent.turn(message, &channel).await;

        let events = audit.snapshot();
        let tool_calls: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AuditTag::ToolCall { .. }))
            .collect();
        assert_eq!(
            tool_calls.len(),
            1,
            "streaming tool output must produce exactly one ToolCall audit entry, got {} (full trail: {:#?})",
            tool_calls.len(),
            events
        );
        // The overall audit shape is still TurnStarted → ToolCall → TurnEnded.
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(events[1], AuditTag::ToolCall { .. }));
        assert!(matches!(events[2], AuditTag::TurnEnded { .. }));
    }

    // ---- Scope denied: Tier 2 + shell.exec → ScopeDenied in trail ----

    #[tokio::test]
    async fn scope_denied_emits_scope_denied_audit_not_tool_call() {
        let audit = RecordingAudit::new();

        let tool = Arc::new(FakeTool::new_r1("shell.exec", |input| {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Scope::parse(&format!("shell.exec:{cmd}")).unwrap()
        }));
        let tool_id = tool.id();

        // Agent nominally holds shell.exec, but the Tier 2 ceiling strips
        // it — that's the whole point of D5 Scenario 3.
        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({"command": "rm"}),
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Telegram, TrustTier::SemiTrusted);
        let message = Message::text(channel.session, "run rm -rf from Telegram");
        let outcome = agent.turn(message, &channel).await;

        // Denial is not a termination — the turn still Completes, just with
        // tool_calls_made reflecting the attempted call.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed (denial is not termination), got {other:?}"),
        }

        // Audit trail: TurnStarted → ScopeDenied → TurnEnded
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        match &events[1] {
            AuditTag::ScopeDenied {
                scope_requested,
                held_capabilities,
                ..
            } => {
                assert_eq!(scope_requested.base(), "shell.exec");
                assert_eq!(scope_requested.qualifier(), Some("rm"));
                // Tier 2 ceiling has no shell.exec at all, so intersected
                // held set does not grant shell.exec in any form.
                assert!(!held_capabilities.grants(&Scope::parse("shell.exec").unwrap()));
                assert!(!held_capabilities.grants(&Scope::parse("shell.exec:rm").unwrap()));
            }
            other => panic!("expected ScopeDenied at index 1, got {other:?}"),
        }

        // No ToolCall event — the tool never ran.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "no ToolCall event should appear for a denied call"
        );
    }

    // ---- Cancellation before the loop body runs ----

    #[tokio::test]
    async fn cancellation_before_first_step_yields_cancelled() {
        let audit = RecordingAudit::new();
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({}),
        }];
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit.clone(),
            plan,
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        channel.token.cancel(); // cancel BEFORE turn() is called

        let message = Message::text(channel.session, "hi");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Cancelled { tool_calls_made } => assert_eq!(tool_calls_made, 0),
            other => panic!("expected Cancelled, got {other:?}"),
        }

        let events = audit.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(
            events[1],
            AuditTag::TurnEnded {
                outcome: TurnOutcomeSummary::Cancelled,
                tool_calls_made: 0,
                ..
            }
        ));
    }

    // ---- R1 interaction: derived scope satisfied by bare capability ----

    #[tokio::test]
    async fn r1_derived_scope_recorded_in_audit_not_bare_scope() {
        let audit = RecordingAudit::new();

        // R1 tool: derives memory.read:session:abc from input
        let tool = Arc::new(FakeTool::new_r1("memory.read", |input| {
            let sid = input
                .get("session")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            Scope::parse(&format!("memory.read:session:{sid}")).unwrap()
        }));
        let tool_id = tool.id();

        // Agent holds *bare* memory.read — rule 2 (unqualified grants
        // qualified) lets the derived scope pass the check.
        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({"session": "abc"}),
            },
            NextStep::FinalMessage("done".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);
        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "recall session abc");
        let outcome = agent.turn(message, &channel).await;

        assert!(matches!(outcome, TurnOutcome::Completed { .. }));

        let events = audit.snapshot();
        let tool_call = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ToolCall { scope_used, .. } => Some(scope_used.clone()),
                _ => None,
            })
            .expect("expected a ToolCall audit event");

        assert_eq!(tool_call.base(), "memory.read");
        assert_eq!(
            tool_call.qualifier(),
            Some("session:abc"),
            "audit must record the *derived* scope, not the agent's bare capability"
        );
    }

    // ---- Parent-level: D1 Scenario 3 end-to-end ----

    #[tokio::test]
    async fn d1_scenario3_rm_rf_from_telegram_e2e() {
        // A dress rehearsal of the whole stack: channel → loop → scope
        // check → audit emission → finalize. If this test passes, Phase 1
        // has delivered the "structural proof that Phase 0 contract can
        // carry real execution" that the phase is actually about.
        let audit = RecordingAudit::new();

        let shell = Arc::new(FakeTool::new_r1("shell.exec", |input| {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Scope::parse(&format!("shell.exec:{cmd}")).unwrap()
        }));
        let shell_id = shell.id();

        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]),
            vec![shell],
            audit.clone(),
            vec![
                NextStep::ToolCall {
                    tool_id: shell_id,
                    input: json!({"command": "rm"}),
                },
                NextStep::FinalMessage(
                    "I can't run shell commands from Telegram.".to_string(),
                ),
            ],
        );

        let channel = FakeChannel::new(ChannelPlatform::Telegram, TrustTier::SemiTrusted);
        let message = Message::text(channel.session, "run rm -rf /");
        let outcome = agent.turn(message, &channel).await;

        // The turn still completes — denial isn't termination.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made,
                final_message,
                ..
            } => {
                assert_eq!(tool_calls_made, 1);
                assert!(final_message.contains("can't run shell"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Audit trail has the Denied event with the right details.
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AuditTag::TurnStarted { channel: ChannelPlatform::Telegram, trust_tier: TrustTier::SemiTrusted, .. }));
        assert!(matches!(events[1], AuditTag::ScopeDenied { .. }));
        assert!(matches!(events[2], AuditTag::TurnEnded { outcome: TurnOutcomeSummary::Completed, tool_calls_made: 1, .. }));

        // Channel was finalized.
        assert_eq!(
            *channel.finalized.lock().unwrap(),
            Some(TurnOutcomeSummary::Completed)
        );
    }

    // ---- NullAuditHook end-to-end (sanity check that the hook abstraction
    //      does not silently swallow anything the RecordingAudit tests were
    //      catching) ----

    #[tokio::test]
    async fn null_audit_hook_end_to_end_still_produces_correct_outcome() {
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            Arc::new(NullAuditHook),
            vec![
                NextStep::ToolCall {
                    tool_id,
                    input: json!({}),
                },
                NextStep::Stop,
            ],
        );
        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let outcome = agent
            .turn(Message::text(channel.session, "test"), &channel)
            .await;
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // ---- Wall-clock timeout: deadline task fires → TimedOut outcome ----
    //
    // Phase 3 task 4 introduced `TURN_TIMEOUT` and the deadline task.
    // This test proves the loop actually emits `TurnOutcome::TimedOut`
    // (the first code path in the project to do so) when the planner
    // hangs past the deadline. Uses tokio's virtual-time test-util so
    // the test doesn't actually wait 120s wall-clock.

    /// Planner whose `next_step` awaits forever. The only way a turn
    /// using it can terminate is the loop's own cancellation re-check
    /// after the deadline task fires.
    struct HangingPlanner;

    #[async_trait]
    impl TurnPlanner for HangingPlanner {
        async fn begin_turn(&mut self, _message: &Message) {}

        async fn next_step(
            &mut self,
            _observed: &[StepObservation],
            channel: &dyn ChannelContext,
        ) -> NextStep {
            // Mirror the real LLM planner's one_step: race the
            // channel's cancellation against a future that never
            // completes. When the deadline task cancels the token,
            // this branch wins and we surface `Stop` — which the
            // loop then translates to `TimedOut` via its post-
            // next_step cancellation re-check + `deadline_fired` flag.
            let cancel = channel.cancellation_token();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => NextStep::Stop,
                _ = std::future::pending::<()>() => unreachable!(),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wall_clock_timeout_emits_timed_out_outcome() {
        let audit = RecordingAudit::new();
        let registry = Arc::new(ToolRegistry::new(Vec::new()));
        let agent = ConcreteAgent::new(
            AgentId::new(),
            CapabilitySet::empty(),
            registry,
            audit.clone(),
            || Box::new(HangingPlanner),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "hang forever");

        // Race the turn against a virtual-time advance that walks past
        // the deadline. `start_paused = true` pauses the tokio clock
        // at t=0; the advance hops directly to t > TURN_TIMEOUT so the
        // deadline task wakes up immediately in wall-clock terms.
        let turn_fut = agent.turn(message, &channel);
        let advance_fut = async {
            // Yield so the turn task actually starts and spawns the
            // deadline task before we advance time past its sleep.
            tokio::task::yield_now().await;
            tokio::time::advance(TURN_TIMEOUT + Duration::from_secs(1)).await;
        };
        let (outcome, _) = tokio::join!(turn_fut, advance_fut);

        match outcome {
            TurnOutcome::TimedOut {
                tool_calls_made,
                elapsed: _,
            } => {
                // We don't assert on `elapsed` because the loop
                // measures it with `std::time::Instant`, which is not
                // virtualized by `tokio::time::pause()`. In production
                // the field is meaningful; in this test it will be
                // ~microseconds. Asserting outcome variant + no tools
                // called is sufficient to prove the deadline path.
                assert_eq!(tool_calls_made, 0);
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }

        // Audit: TurnStarted → TurnEnded(TimedOut), nothing in between.
        let events = audit.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(
            events[1],
            AuditTag::TurnEnded {
                outcome: TurnOutcomeSummary::TimedOut,
                ..
            }
        ));
    }

    // ---- Max-steps guard: a runaway planner is terminated with Failed ----
    //
    // Phase 2 introduced MAX_STEPS_PER_TURN = 32 so an LLM-backed planner
    // that never emits FinalMessage cannot loop forever. This test proves
    // the guard fires by feeding the loop a script that's longer than the
    // budget — 64 ToolCalls, no FinalMessage — and asserting the loop
    // terminates with Failed(Internal) rather than running to completion.

    #[tokio::test]
    async fn runaway_planner_terminates_with_max_steps_exceeded() {
        let audit = RecordingAudit::new();
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        // Build a script of 64 ToolCalls with no FinalMessage. The
        // budget is MAX_STEPS_PER_TURN; anything past the budget should
        // never run.
        let plan: Vec<NextStep> = (0..64)
            .map(|_| NextStep::ToolCall {
                tool_id,
                input: json!({}),
            })
            .collect();

        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit.clone(),
            plan,
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let outcome = agent
            .turn(Message::text(channel.session, "spam"), &channel)
            .await;

        match outcome {
            TurnOutcome::Failed(AivyxError::Internal(msg)) => {
                assert!(
                    msg.contains("exceeded"),
                    "expected 'exceeded' in error, got {msg:?}"
                );
            }
            other => panic!("expected Failed(Internal), got {other:?}"),
        }

        // The loop should have called exactly MAX_STEPS_PER_TURN tools
        // before bailing — one tool per step, no shortcut.
        let tool_calls: Vec<_> = audit
            .snapshot()
            .into_iter()
            .filter(|e| matches!(e, AuditTag::ToolCall { .. }))
            .collect();
        assert_eq!(tool_calls.len(), MAX_STEPS_PER_TURN);

        // TurnEnded should record the Failed summary.
        let events = audit.snapshot();
        let ended = events
            .iter()
            .find(|e| matches!(e, AuditTag::TurnEnded { .. }))
            .expect("TurnEnded should still be emitted");
        match ended {
            AuditTag::TurnEnded { outcome, .. } => {
                assert_eq!(*outcome, TurnOutcomeSummary::Failed);
            }
            _ => unreachable!(),
        }
    }

    // ---- Phase 10 task 2: JSON-schema validation at the turn loop ----
    //
    // The validator has its own unit tests in `schema.rs`; these two
    // tests prove the *wiring*: the turn loop runs validation before
    // `required_scope`, a failure short-circuits to `Failed` without
    // invoking `scope_fn` or `execute`, and a well-formed input
    // passes through unchanged.

    #[tokio::test]
    async fn malformed_tool_input_is_rejected_before_required_scope() {
        // The FakeTool has a schema requiring a string `path` field.
        // The planner emits an input missing `path`. If validation
        // works, `scope_fn` is never called; it's set to panic so a
        // regression (validator removed or moved after scope check)
        // would panic loudly rather than silently accept.
        let audit = RecordingAudit::new();

        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let tool = Arc::new(FakeTool::new_with_schema("fs.read", schema, |_| {
            panic!("required_scope must not run when validation fails");
        }));
        let tool_id = tool.id();

        // Cap grant exists, so the test isolates the effect of
        // validation from the scope-denial path. If the validator
        // were missing, this call would reach `scope_fn` and panic
        // — which is exactly the regression this test locks in.
        let agent_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({}), // missing required `path`
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "bad call");
        let outcome = agent.turn(message, &channel).await;

        // The turn still Completes — validation failure is a Failed
        // step, not a turn-ending error. Tool_calls_made == 1 because
        // the loop did attempt the call, just not reach execute.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed (validation fail is not termination), got {other:?}"),
        }

        // The audit trail must contain NO ToolCall event and NO
        // ScopeDenied event — validation fails before either runs.
        // It also must not contain a panic trace; if the panic
        // fired we'd never have reached this assertion.
        let events = audit.snapshot();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "validation failure must not emit ToolCall"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "validation failure must route through Failed, not Denied"
        );
    }

    #[tokio::test]
    async fn well_formed_tool_input_passes_validation_and_runs() {
        // Positive case: schema-valid input reaches execute as
        // before, proving validation isn't over-rejecting. This is
        // the counterpart to the negative test above — without it,
        // a buggy validator that rejected *everything* would still
        // pass the negative assertion.
        let audit = RecordingAudit::new();

        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let tool = Arc::new(FakeTool::new_with_schema("fs.read", schema, |_| {
            Scope::parse("fs.read").unwrap()
        }));
        let tool_id = tool.id();

        let agent_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({"path": "notes/today.md"}),
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "good call");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed, got {other:?}"),
        }

        // A valid input reaches execute → ToolCall is recorded.
        let events = audit.snapshot();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "valid input must reach ToolCall"
        );
    }

    // ---- Phase 10 task 3: StreamEvent tool-name emission ----------
    //
    // Before Phase 10 `ToolCallStarted` / `ToolCallFinished` existed
    // in the enum and in the renderers but were never actually
    // emitted by the turn loop — a latent wiring gap since Phase 5.
    // Task 3 closes it and adds `tool_name` so renderers show the
    // human name instead of a short UUID. These tests lock the
    // whole chain in: the loop emits both events around `execute`,
    // in order, carrying the same `tool_name` as `Tool::name()`.

    #[tokio::test]
    async fn tool_call_emits_started_and_finished_events_with_tool_name() {
        let audit = RecordingAudit::new();

        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();
        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({"topic": "notes"}),
        }];
        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = RecordingChannel::new();
        let message = Message::text(channel.session, "check my notes");
        let outcome = agent.turn(message, &channel).await;

        // Turn completes cleanly — this is a golden-path test for
        // the emission, not an error case.
        assert!(matches!(outcome, TurnOutcome::Completed { .. }));

        // Exactly one pair of tool events, ordered started → finished,
        // both carrying the Tool::name() string.
        let events = channel.snapshot();
        let starts: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RecordedEvent::ToolCallStarted { .. }))
            .collect();
        let finishes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RecordedEvent::ToolCallFinished { .. }))
            .collect();
        assert_eq!(starts.len(), 1, "exactly one ToolCallStarted");
        assert_eq!(finishes.len(), 1, "exactly one ToolCallFinished");

        match starts[0] {
            RecordedEvent::ToolCallStarted { tool_name } => {
                assert_eq!(tool_name, "memory.read");
            }
            _ => unreachable!(),
        }
        match finishes[0] {
            RecordedEvent::ToolCallFinished {
                tool_name,
                summary,
            } => {
                assert_eq!(tool_name, "memory.read");
                // FakeTool's execute returns a NotApplicable-verified
                // Completed outcome, which maps to "completed".
                assert_eq!(summary, "completed");
            }
            _ => unreachable!(),
        }

        // Started must precede Finished in the overall event sequence.
        let start_idx = events
            .iter()
            .position(|e| matches!(e, RecordedEvent::ToolCallStarted { .. }))
            .unwrap();
        let finish_idx = events
            .iter()
            .position(|e| matches!(e, RecordedEvent::ToolCallFinished { .. }))
            .unwrap();
        assert!(
            start_idx < finish_idx,
            "ToolCallStarted must precede ToolCallFinished"
        );
    }

    #[tokio::test]
    async fn denied_tool_call_emits_no_stream_events() {
        // Phase 8 Task 2 already guarantees a denied call never
        // reaches `execute`; with Task 3 we now also must NOT emit
        // ToolCallStarted/Finished for a denial, because those
        // events announce to the renderer "the tool is running
        // right now." A denial is a *suppressed* tool call — the
        // human should see nothing, and the scope-denial audit
        // event is what gets recorded.
        let audit = RecordingAudit::new();
        let tool = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let tool_id = tool.id();

        // No caps → denial.
        let agent_caps = CapabilitySet::from_scopes([]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({}),
        }];
        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = RecordingChannel::new();
        let message = Message::text(channel.session, "run");
        let _ = agent.turn(message, &channel).await;

        let events = channel.snapshot();
        assert!(
            !events
                .iter()
                .any(|e| matches!(
                    e,
                    RecordedEvent::ToolCallStarted { .. }
                        | RecordedEvent::ToolCallFinished { .. }
                )),
            "denied tool calls must not emit tool-call stream events: {events:?}"
        );
    }

    // =====================================================================
    // Phase 11 Task 4 — role-allowlist dispatch-layer gate
    // =====================================================================
    //
    // These tests pin the belt-and-suspenders allowlist check in
    // `run_tool_call`. The primary enforcement is at the planner layer
    // (`LlmPlannerConfig::tool_allowlist` filters the advertised catalog),
    // tested separately in `llm_planner.rs`. Here we cover the
    // dispatch-layer safety net: even a planner that emits a call to an
    // out-of-allowlist tool (a non-LLM planner, a resumed conversation's
    // stale tool_use block, etc.) must be rejected before session
    // injection, before `required_scope`, and before the capability
    // check.
    //
    // Phase 28 Task 4 upgraded the routing from `ToolOutcome::Denied`
    // (Phase 11 Q1 Option A) to `ToolOutcome::NotInRole { tool_name }`
    // — forensically distinct in the audit chain. The `ScopeDenied`
    // audit tag still fires with the synthetic `tool.allowlist:<name>`
    // scope for backward compatibility with audit walkers.

    use std::collections::BTreeSet;

    fn allowlist(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn role_allowlist_rejects_out_of_role_tool_at_dispatch_layer() {
        // Setup mirrors the Phase 11 seed roles: `researcher` gets a
        // read-only allowlist, no `shell.exec`. The agent *has* the
        // capability to call shell.exec (so the capability gate
        // would have granted it), but the allowlist gate fires
        // earlier and denies.
        let audit = RecordingAudit::new();
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let shell_id = shell.id();
        let fs_read = Arc::new(FakeTool::new_bare("fs.read", "fs.read"));

        let caps = CapabilitySet::from_scopes([
            Scope::parse("shell.exec").unwrap(),
            Scope::parse("fs.read").unwrap(),
        ]);
        let plan = vec![NextStep::ToolCall {
            tool_id: shell_id,
            input: json!({}),
        }];

        let registry = Arc::new(ToolRegistry::new(vec![
            shell as Arc<dyn Tool>,
            fs_read as Arc<dyn Tool>,
        ]));
        let plan_arc = Arc::new(plan);
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_tool_allowlist(Some(allowlist(&["fs.read", "memory.read"])));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "run rm -rf");
        let _ = agent.turn(message, &channel).await;

        let events = audit.snapshot();
        // Exactly one ScopeDenied event must appear, and the
        // scope's base must be `tool.allowlist` (NOT `shell.exec`) —
        // that's the distinguishing signal for auditors.
        let denial = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ScopeDenied {
                    scope_requested, ..
                } => Some(scope_requested),
                _ => None,
            })
            .expect("role-allowlist rejection must emit ScopeDenied");
        assert_eq!(
            denial.base(),
            "tool.allowlist",
            "scope base must be the synthetic tool.allowlist, not \
             the tool's real capability base"
        );
        assert_eq!(
            denial.qualifier(),
            Some("shell.exec"),
            "qualifier must carry the rejected tool name"
        );

        // Critically: no ToolCall audit event (which would only fire
        // after successful execute). If this assertion fails, the
        // allowlist gate let the call through.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "allowlist rejection must NOT emit a ToolCall event"
        );
    }

    #[tokio::test]
    async fn role_allowlist_accepts_in_role_tool() {
        // Positive control: an agent whose role DOES include the
        // tool in its allowlist reaches the normal
        // execute-and-emit-ToolCall path.
        let audit = RecordingAudit::new();
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let shell_id = shell.id();

        let caps =
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]);
        let plan = vec![
            NextStep::ToolCall {
                tool_id: shell_id,
                input: json!({}),
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let registry =
            Arc::new(ToolRegistry::new(vec![shell as Arc<dyn Tool>]));
        let plan_arc = Arc::new(plan);
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_tool_allowlist(Some(allowlist(&["shell.exec", "fs.read"])));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "build");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => {
                assert_eq!(tool_calls_made, 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        let events = audit.snapshot();
        assert!(
            events.iter().any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "in-role call must produce a ToolCall audit event"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "in-role call must not produce ScopeDenied"
        );
    }

    #[tokio::test]
    async fn role_allowlist_none_preserves_legacy_behavior() {
        // Backwards-compat invariant: an agent built without
        // `with_tool_allowlist` (or with `None`) behaves exactly
        // like a Phase 6–10 agent — every registered tool is
        // callable, subject only to the capability gate. This is
        // the synthesized `default` role's contract from Task 1's
        // backwards-compat bridge.
        let audit = RecordingAudit::new();
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let shell_id = shell.id();

        let caps =
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]);
        let plan = vec![
            NextStep::ToolCall {
                tool_id: shell_id,
                input: json!({}),
            },
            NextStep::FinalMessage("ok".to_string()),
        ];
        let registry =
            Arc::new(ToolRegistry::new(vec![shell as Arc<dyn Tool>]));
        let plan_arc = Arc::new(plan);
        // Note: NO `with_tool_allowlist` call — field stays `None`.
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "hi");
        let outcome = agent.turn(message, &channel).await;
        assert!(matches!(outcome, TurnOutcome::Completed { .. }));
        let events = audit.snapshot();
        assert!(
            events.iter().any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "with no allowlist, shell.exec must execute normally"
        );
    }

    #[tokio::test]
    async fn role_allowlist_fires_before_capability_gate() {
        // Ordering invariant: the allowlist gate runs *before* the
        // capability check, so even if the agent has zero caps
        // (which would produce a `shell.exec` capability denial),
        // the role-rejection path wins and the audit record shows
        // `tool.allowlist:shell.exec`, not `shell.exec`. This is
        // what makes the allowlist the "identity gate" vs. the
        // capability layer's "authority gate."
        let audit = RecordingAudit::new();
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let shell_id = shell.id();

        // Deliberately empty caps.
        let caps = CapabilitySet::from_scopes([]);
        let plan = vec![NextStep::ToolCall {
            tool_id: shell_id,
            input: json!({}),
        }];
        let registry =
            Arc::new(ToolRegistry::new(vec![shell as Arc<dyn Tool>]));
        let plan_arc = Arc::new(plan);
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_tool_allowlist(Some(allowlist(&["fs.read"])));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "try it");
        let _ = agent.turn(message, &channel).await;

        let events = audit.snapshot();
        let denial = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ScopeDenied {
                    scope_requested, ..
                } => Some(scope_requested),
                _ => None,
            })
            .expect("must emit a denial");
        assert_eq!(
            denial.base(),
            "tool.allowlist",
            "allowlist gate must fire before capability gate"
        );
    }

    // =====================================================================
    // Phase 12 Task 3 — cross-role regression
    // =====================================================================
    //
    // The phase-level property: two agents built from the *same* tool
    // registry, differing only in `with_tool_allowlist`, produce the
    // asymmetric "roles actually work for product tools" behavior
    // promised in the Phase 12 entry criteria. A `coder` agent whose
    // allowlist includes `shell.exec` but not `web.fetch` is denied
    // `web.fetch`; a `researcher` agent whose allowlist includes
    // `web.fetch` but not `shell.exec` is denied `shell.exec`. Each
    // agent's in-role tool succeeds.
    //
    // Task 3's draft assumed the test would run through a real
    // TOML config file — but there is no shipped default TOML, so
    // the regression lives here at the agent layer instead (see the
    // Task 3 correction block in PHASE_12.md for the full
    // rationale). The agent-layer property is the load-bearing one
    // regardless of whether a config file is later added: config
    // parsing cares what's *in* the file; this test cares what
    // happens *after* parsing, at dispatch time.
    //
    // The test uses `Arc::clone` to share the same physical tool
    // instances across both agents — proving that the asymmetry is
    // a property of the allowlist view, not of two separate
    // registries that happened to contain different tools.

    /// Helper: drive one turn with a scripted single-tool plan and
    /// return `(final outcome, captured audit events)`. Centralizes
    /// the `ConcreteAgent::new` + `VecPlanner` + `FakeChannel` scaffolding
    /// so the cross-role test below stays about the allowlist seam.
    async fn run_single_tool_turn_with_allowlist(
        tools: Vec<Arc<dyn Tool>>,
        caps: CapabilitySet,
        target: ToolId,
        allowlist: BTreeSet<String>,
    ) -> (TurnOutcome, Vec<AuditTag>) {
        let audit = RecordingAudit::new();
        let registry = Arc::new(ToolRegistry::new(tools));
        let plan = vec![
            NextStep::ToolCall {
                tool_id: target,
                input: json!({}),
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_tool_allowlist(Some(allowlist));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let outcome = agent.turn(message, &channel).await;
        let events = audit.snapshot();
        (outcome, events)
    }

    #[tokio::test]
    async fn cross_role_same_registry_different_allowlists_produce_asymmetric_access() {
        // One physical registry, shared across both agent
        // instances via Arc::clone. This is the invariant that
        // matters: two `--role` invocations of the same binary see
        // the same tool set, and the asymmetry comes from the
        // allowlist applied at agent construction.
        let shell: Arc<dyn Tool> =
            Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"));
        let web: Arc<dyn Tool> =
            Arc::new(FakeTool::new_bare("web.fetch", "net.fetch"));
        let shell_id = shell.id();
        let web_id = web.id();
        let shared_tools: Vec<Arc<dyn Tool>> =
            vec![Arc::clone(&shell), Arc::clone(&web)];

        // Both agents hold the same broad capability set. The
        // allowlist is the ONLY thing that differs between them —
        // so any asymmetric outcome is provably attributable to
        // the allowlist, not to a capability difference.
        let caps = CapabilitySet::from_scopes([
            Scope::parse("shell.exec").unwrap(),
            Scope::parse("net.fetch").unwrap(),
        ]);

        // ---- Scenario 1: coder (shell.exec only) calling web.fetch ----
        let (coder_web_outcome, coder_web_events) =
            run_single_tool_turn_with_allowlist(
                shared_tools.clone(),
                caps.clone(),
                web_id,
                allowlist(&["shell.exec"]),
            )
            .await;
        // Turn completes (the planner's final message still fires)
        // but the tool call was denied — and the denial's audit
        // scope_requested must be `tool.allowlist:web.fetch`, the
        // distinguishing signal for a role-allowlist rejection
        // versus a capability rejection.
        assert!(
            matches!(coder_web_outcome, TurnOutcome::Completed { .. }),
            "coder turn should complete (allowlist denial is a \
             per-tool-call denial, not a turn failure): \
             {coder_web_outcome:?}"
        );
        let coder_denial = coder_web_events
            .iter()
            .find_map(|e| match e {
                AuditTag::ScopeDenied { scope_requested, .. } => {
                    Some(scope_requested)
                }
                _ => None,
            })
            .expect("coder calling web.fetch must emit ScopeDenied");
        assert_eq!(
            coder_denial.base(),
            "tool.allowlist",
            "coder's web.fetch denial must be a role-allowlist denial, \
             not a capability denial"
        );
        assert_eq!(
            coder_denial.qualifier(),
            Some("web.fetch"),
            "qualifier must name the rejected tool"
        );
        assert!(
            !coder_web_events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "coder's denied web.fetch call must NOT emit ToolCall audit event"
        );

        // ---- Scenario 2: researcher (web.fetch only) calling shell.exec ----
        let (researcher_shell_outcome, researcher_shell_events) =
            run_single_tool_turn_with_allowlist(
                shared_tools.clone(),
                caps.clone(),
                shell_id,
                allowlist(&["web.fetch"]),
            )
            .await;
        assert!(
            matches!(researcher_shell_outcome, TurnOutcome::Completed { .. }),
            "researcher turn should complete: {researcher_shell_outcome:?}"
        );
        let researcher_denial = researcher_shell_events
            .iter()
            .find_map(|e| match e {
                AuditTag::ScopeDenied { scope_requested, .. } => {
                    Some(scope_requested)
                }
                _ => None,
            })
            .expect("researcher calling shell.exec must emit ScopeDenied");
        assert_eq!(
            researcher_denial.base(),
            "tool.allowlist",
            "researcher's shell.exec denial must be a role-allowlist denial"
        );
        assert_eq!(
            researcher_denial.qualifier(),
            Some("shell.exec"),
            "qualifier must name the rejected tool"
        );
        assert!(
            !researcher_shell_events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "researcher's denied shell.exec call must NOT emit ToolCall"
        );

        // ---- Scenario 3: coder's in-role call (shell.exec) succeeds ----
        let (coder_shell_outcome, coder_shell_events) =
            run_single_tool_turn_with_allowlist(
                shared_tools.clone(),
                caps.clone(),
                shell_id,
                allowlist(&["shell.exec"]),
            )
            .await;
        match coder_shell_outcome {
            TurnOutcome::Completed { tool_calls_made, .. } => {
                assert_eq!(
                    tool_calls_made, 1,
                    "coder's in-role shell.exec call should execute"
                );
            }
            other => panic!("expected coder Completed, got {other:?}"),
        }
        assert!(
            coder_shell_events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "coder's in-role call must emit a ToolCall audit event"
        );
        assert!(
            !coder_shell_events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "coder's in-role call must NOT emit ScopeDenied"
        );

        // ---- Scenario 4: researcher's in-role call (web.fetch) succeeds ----
        let (researcher_web_outcome, researcher_web_events) =
            run_single_tool_turn_with_allowlist(
                shared_tools.clone(),
                caps.clone(),
                web_id,
                allowlist(&["web.fetch"]),
            )
            .await;
        match researcher_web_outcome {
            TurnOutcome::Completed { tool_calls_made, .. } => {
                assert_eq!(
                    tool_calls_made, 1,
                    "researcher's in-role web.fetch call should execute"
                );
            }
            other => panic!("expected researcher Completed, got {other:?}"),
        }
        assert!(
            researcher_web_events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "researcher's in-role call must emit a ToolCall audit event"
        );
        assert!(
            !researcher_web_events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "researcher's in-role call must NOT emit ScopeDenied"
        );
    }

    #[tokio::test]
    async fn cross_role_allowlist_gate_precedes_real_tool_execution() {
        // Complement to the scenario above: a role-denied call
        // must never reach `Tool::execute` at all. We prove this
        // with a FakeTool whose `execute` would panic — if the
        // allowlist gate is working, the panic never fires
        // because dispatch short-circuits at ScopeDenied.
        //
        // A panic-on-execute fake is a common safety pattern in
        // the agent-layer tests; if some future change threaded
        // the denied call past the allowlist gate, this test
        // would immediately surface it as a test panic rather
        // than as a silently passing "Denied" assertion.
        struct PanicOnExecute {
            id: ToolId,
            name: &'static str,
            schema: Value,
            scope: Scope,
        }
        #[async_trait]
        impl Tool for PanicOnExecute {
            fn id(&self) -> ToolId {
                self.id
            }
            fn name(&self) -> &str {
                self.name
            }
            fn description(&self) -> &str {
                "panic-on-execute fake — must never run if allowlist gate works"
            }
            fn input_schema(&self) -> &Value {
                &self.schema
            }
            fn required_scope(&self, _input: &Value) -> Scope {
                self.scope.clone()
            }
            async fn execute(
                &self,
                _input: Value,
                _ctx: &ToolContext<'_>,
            ) -> ToolOutcome {
                panic!(
                    "allowlist gate failed — {} reached execute despite being \
                     out-of-role",
                    self.name
                );
            }
        }

        let web_panic: Arc<dyn Tool> = Arc::new(PanicOnExecute {
            id: ToolId::new(),
            name: "web.fetch",
            schema: json!({}),
            scope: Scope::parse("net.fetch").unwrap(),
        });
        let web_id = web_panic.id();
        let caps =
            CapabilitySet::from_scopes([Scope::parse("net.fetch").unwrap()]);

        let (outcome, events) = run_single_tool_turn_with_allowlist(
            vec![web_panic],
            caps,
            web_id,
            // `coder` allowlist — web.fetch is NOT included.
            allowlist(&["shell.exec", "fs.read"]),
        )
        .await;

        // If we reach this assertion without a panic, the
        // allowlist gate correctly short-circuited before
        // `execute` fired.
        assert!(matches!(outcome, TurnOutcome::Completed { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "expected a ScopeDenied audit event"
        );
    }

    // =====================================================================
    // Phase 11 Task 4 — `LlmPlannerConfig::tool_allowlist` catalog filter
    // =====================================================================
    //
    // The dispatch-layer gate above is the belt-and-suspenders; this
    // test pins the primary enforcement: the planner, when handed a
    // `Some(allowlist)`, must NOT advertise filtered-out tools to the
    // provider. Covered here at the unit level because the filter
    // applies inside `LlmPlanner::new`, before any turn runs.

    #[test]
    fn llm_planner_filters_tool_catalog_by_role_allowlist() {
        use crate::llm_planner::{LlmPlanner, LlmPlannerConfig};
        // A minimal registry with three tools.
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"))
            as Arc<dyn Tool>;
        let fs_read =
            Arc::new(FakeTool::new_bare("fs.read", "fs.read")) as Arc<dyn Tool>;
        let memory_read =
            Arc::new(FakeTool::new_bare("memory.read", "memory.read"))
                as Arc<dyn Tool>;
        let registry =
            Arc::new(ToolRegistry::new(vec![shell, fs_read, memory_read]));

        // Researcher-style allowlist: no shell.exec.
        let config = LlmPlannerConfig::new("test-model")
            .with_tool_allowlist(Some(allowlist(&["fs.read", "memory.read"])));

        // A provider we never actually call — LlmPlanner::new only
        // reads the registry and config at construction.
        let provider: Arc<dyn aivyx_llm::LlmProvider> =
            Arc::new(FakeProvider);
        let planner = LlmPlanner::new(provider, registry, config);

        let advertised_names: Vec<&str> = planner
            .advertised_tool_names()
            .into_iter()
            .collect();
        assert!(
            advertised_names.contains(&"fs.read"),
            "fs.read must be advertised"
        );
        assert!(
            advertised_names.contains(&"memory.read"),
            "memory.read must be advertised"
        );
        assert!(
            !advertised_names.contains(&"shell.exec"),
            "shell.exec must NOT be advertised to a researcher-role planner"
        );
        assert_eq!(
            advertised_names.len(),
            2,
            "exactly two tools advertised"
        );
    }

    #[test]
    fn llm_planner_none_allowlist_advertises_every_tool() {
        // Backwards-compat: `tool_allowlist: None` preserves the
        // Phase 6–10 "advertise every registered tool" behavior.
        use crate::llm_planner::{LlmPlanner, LlmPlannerConfig};
        let shell = Arc::new(FakeTool::new_bare("shell.exec", "shell.exec"))
            as Arc<dyn Tool>;
        let fs_read =
            Arc::new(FakeTool::new_bare("fs.read", "fs.read")) as Arc<dyn Tool>;
        let registry = Arc::new(ToolRegistry::new(vec![shell, fs_read]));
        // Default config, no allowlist.
        let config = LlmPlannerConfig::new("test-model");
        let provider: Arc<dyn aivyx_llm::LlmProvider> =
            Arc::new(FakeProvider);
        let planner = LlmPlanner::new(provider, registry, config);
        let names = planner.advertised_tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"fs.read"));
    }

    // Minimal fake provider for the filter tests. `LlmPlanner::new`
    // stores the provider but doesn't call it unless a turn runs,
    // so a panicking stub is sufficient.
    struct FakeProvider;

    #[async_trait]
    impl aivyx_llm::LlmProvider for FakeProvider {
        async fn chat_stream(
            &self,
            _request: aivyx_llm::LlmRequest<'_>,
            _cancel: &CancellationToken,
        ) -> Result<Box<dyn aivyx_llm::LlmStream>, aivyx_llm::LlmError> {
            panic!("FakeProvider::chat_stream must not be called in filter tests")
        }
    }

    // =====================================================================
    // Phase 14 Task 3 — sub-agent role-switching end-to-end
    // =====================================================================
    //
    // These tests pin the full `role.switch` dispatch flow against a
    // real `ConcreteAgent::turn` loop, a real `RoleSwitchTool` with a
    // wired child factory, and a `RecordingAudit` capturing the
    // parent→child→parent audit transition. They cover:
    //
    // - happy path: parent holding `role.switch:researcher`
    //   dispatches, child turn runs, audit shows two distinct
    //   `TurnStarted` events with different `effective_capabilities`
    // - structural impossibility: the child's effective capability
    //   set is exactly what the factory built (a narrower one), NOT
    //   whatever scopes the parent held — so the child cannot hold
    //   any scope the parent's factory didn't hand it
    // - negative (scope gate): parent holding only `role.switch:researcher`
    //   calling `role.switch` with `target=scribe` produces a
    //   `ScopeDenied` audit event and no child `TurnStarted`
    // - negative (factory-level): parent holding `role.switch` (broad)
    //   calling `target=unknown` produces a `ToolOutcome::Failed`
    //   via the child factory's `Err` path, and no child `TurnStarted`
    // - output shape: a `Completed` child turn surfaces as a
    //   `ToolOutcome::Completed` whose output JSON has the expected
    //   `status=completed`, `final_message`, `target`, and
    //   `tool_calls_made` fields
    //
    // These tests do NOT need an `LlmPlanner` or a network provider —
    // both the parent and the child run `VecPlanner` with a scripted
    // sequence of `NextStep`s. The role.switch tool's child factory
    // closes over a small "build a child agent with these tools and
    // this plan" function.

    use crate::tools::role_switch::{ChildAgentFactory, RoleSwitchTool};

    /// Helper: build a child `ConcreteAgent` that will run a fixed
    /// `VecPlanner` script and hold a specific `CapabilitySet`. The
    /// returned `Box<dyn Agent>` is what the parent's `role.switch`
    /// tool's child factory hands back at execute time.
    ///
    /// The child uses the same tool registry and audit hook as the
    /// parent — this is the structural shape a real session layer
    /// produces. Audit events from the child's turn land in the same
    /// `RecordingAudit` so the test can assert on the parent→child
    /// event sequence in one snapshot.
    fn build_child_agent(
        tools: Arc<ToolRegistry>,
        audit: Arc<RecordingAudit>,
        caps: CapabilitySet,
        plan: Vec<NextStep>,
    ) -> Box<dyn Agent> {
        let plan_arc = Arc::new(plan);
        let child = ConcreteAgent::new(
            AgentId::new(),
            caps,
            tools,
            audit,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );
        Box::new(child)
    }

    /// Assert helper: drain the captured audit events down to just
    /// the `TurnStarted` events and return their
    /// `effective_capabilities` snapshots in emission order. The
    /// parent→child pattern produces exactly two `TurnStarted`
    /// events (parent's first, then child's inside the tool
    /// execute), and the two snapshots let the test compare
    /// role envelopes directly.
    fn turn_started_cap_snapshots(events: &[AuditTag]) -> Vec<CapabilitySet> {
        events
            .iter()
            .filter_map(|e| match e {
                AuditTag::TurnStarted {
                    effective_capabilities,
                    ..
                } => Some(effective_capabilities.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn role_switch_happy_path_dispatches_child_turn_with_narrowed_caps() {
        // Parent capability set: holds `role.switch:researcher` plus
        // `fs.read` and `fs.write` (the "broad parent" envelope).
        let parent_caps = CapabilitySet::from_scopes([
            Scope::parse("role.switch:researcher").unwrap(),
            Scope::parse("fs.read").unwrap(),
            Scope::parse("fs.write").unwrap(),
        ]);

        // Child capability set: a deliberately narrower subset —
        // `fs.read` only, no `fs.write`, no `role.switch`. This is
        // the attenuated envelope the factory hands the child.
        let child_caps =
            CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        // Build the role.switch tool and install the child factory.
        let role_switch = Arc::new(RoleSwitchTool::new());
        let role_switch_id = role_switch.id();

        // Shared audit + tools so both parent and child emit into
        // the same record. Tools are just the role.switch tool for
        // this test — the child's plan doesn't call any other tool.
        let audit = RecordingAudit::new();
        let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
            Arc::clone(&role_switch) as Arc<dyn Tool>,
        ]));

        // The child factory: closes over the child's cap set, tools,
        // audit, and a fixed plan. Only recognizes "researcher" as
        // a target — anything else produces an Err (exercised by
        // the unknown-target test below).
        let child_tools = Arc::clone(&tools);
        let child_audit = Arc::clone(&audit);
        let child_caps_for_factory = child_caps.clone();
        let factory: Arc<ChildAgentFactory> = Arc::new(move |target: &str| {
            if target != "researcher" {
                return Err(format!("unknown target role {target:?}"));
            }
            // Child's plan: emit a FinalMessage and stop. The
            // child runs one turn and reports a single string.
            let plan = vec![NextStep::FinalMessage(
                "researcher summary: done".to_string(),
            )];
            Ok(build_child_agent(
                Arc::clone(&child_tools),
                Arc::clone(&child_audit),
                child_caps_for_factory.clone(),
                plan,
            ))
        });
        assert!(
            role_switch.set_child_factory(factory).is_ok(),
            "first set_child_factory call must succeed"
        );

        // Parent's plan: call role.switch once, then FinalMessage.
        // The role.switch call runs the child synchronously inside
        // `execute`; when the child completes, the parent's planner
        // observes the Completed tool outcome and proceeds to the
        // FinalMessage step.
        let parent_plan = vec![
            NextStep::ToolCall {
                tool_id: role_switch_id,
                input: json!({
                    "target": "researcher",
                    "task": "read file X and summarize"
                }),
            },
            NextStep::FinalMessage("parent done".to_string()),
        ];
        let plan_arc = Arc::new(parent_plan);
        let parent_agent = ConcreteAgent::new(
            AgentId::new(),
            parent_caps.clone(),
            Arc::clone(&tools),
            Arc::clone(&audit) as Arc<dyn AuditHook>,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let outcome = parent_agent.turn(message, &channel).await;

        // Parent's turn completed cleanly.
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "parent turn should complete: {outcome:?}"
        );

        // Two TurnStarted events: parent's first, child's second.
        let events = audit.snapshot();
        let snapshots = turn_started_cap_snapshots(&events);
        assert_eq!(
            snapshots.len(),
            2,
            "expected exactly 2 TurnStarted events (parent + child), got {}",
            snapshots.len()
        );

        // Parent's effective caps include fs.write; child's do not.
        // This is the structural-impossibility property: the child's
        // envelope was built by the factory, not by the parent's
        // held set being copied across.
        let parent_snapshot = &snapshots[0];
        let child_snapshot = &snapshots[1];
        assert!(
            parent_snapshot.grants(&Scope::parse("fs.write").unwrap()),
            "parent must hold fs.write at turn-start"
        );
        assert!(
            !child_snapshot.grants(&Scope::parse("fs.write").unwrap()),
            "child MUST NOT hold fs.write — that's the whole point of sub-agent attenuation"
        );
        assert!(
            child_snapshot.grants(&Scope::parse("fs.read").unwrap()),
            "child still holds fs.read (it's in the factory's hand-built envelope)"
        );

        // Parent's observed tool outcome (the one the planner sees)
        // shows the child's final message in the output payload.
        // We find it by looking for the ToolCall audit event for
        // the role.switch id and asserting the summary is Completed.
        let tool_call_event = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ToolCall {
                    tool_id, outcome, ..
                } if *tool_id == role_switch_id => Some(outcome),
                _ => None,
            })
            .expect("must emit a ToolCall audit event for role.switch");
        assert!(
            matches!(
                tool_call_event,
                ToolOutcomeSummary::Completed { .. }
            ),
            "role.switch ToolCall audit outcome must be Completed, got {tool_call_event:?}"
        );
    }

    #[tokio::test]
    async fn role_switch_scope_gate_denies_when_parent_lacks_target_scope() {
        // Parent holds `role.switch:researcher` but asks to switch
        // into `scribe`. The derived scope is `role.switch:scribe`,
        // which is NOT granted by `role.switch:researcher` (Rule 3,
        // SimpleGlob equality mismatch). The dispatch gate produces
        // a `ScopeDenied` event and `execute` never runs — so the
        // child factory is never called, no child `TurnStarted`
        // event fires.
        let parent_caps = CapabilitySet::from_scopes([
            Scope::parse("role.switch:researcher").unwrap(),
        ]);

        let role_switch = Arc::new(RoleSwitchTool::new());
        let role_switch_id = role_switch.id();
        let audit = RecordingAudit::new();
        let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
            Arc::clone(&role_switch) as Arc<dyn Tool>,
        ]));

        // Factory that panics if called — proves the scope gate
        // short-circuited before the factory was invoked.
        let factory: Arc<ChildAgentFactory> = Arc::new(|_target: &str| {
            panic!("child factory must not be called when scope gate denies");
        });
        role_switch.set_child_factory(factory).ok();

        let parent_plan = vec![
            NextStep::ToolCall {
                tool_id: role_switch_id,
                input: json!({ "target": "scribe", "task": "x" }),
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(parent_plan);
        let parent_agent = ConcreteAgent::new(
            AgentId::new(),
            parent_caps,
            Arc::clone(&tools),
            Arc::clone(&audit) as Arc<dyn AuditHook>,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let _ = parent_agent.turn(message, &channel).await;

        let events = audit.snapshot();

        // Exactly one TurnStarted (the parent's) — no child turn
        // started because the scope gate fired first.
        let snapshots = turn_started_cap_snapshots(&events);
        assert_eq!(
            snapshots.len(),
            1,
            "scope-denied role.switch must NOT start a child turn"
        );

        // ScopeDenied event for `role.switch:scribe` must appear.
        let denial = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ScopeDenied {
                    scope_requested, ..
                } => Some(scope_requested),
                _ => None,
            })
            .expect("must emit a ScopeDenied event for the denied role.switch call");
        assert_eq!(denial.base(), "role.switch");
        assert_eq!(denial.qualifier(), Some("scribe"));
    }

    #[tokio::test]
    async fn role_switch_factory_error_surfaces_as_failed_tool_outcome() {
        // Parent holds the broad unqualified `role.switch` (Rule 2
        // grants any qualified needed). The scope gate passes, the
        // factory runs, and the factory returns an Err for the
        // unknown target. Expected: `ToolOutcome::Failed`, no child
        // `TurnStarted`.
        let parent_caps =
            CapabilitySet::from_scopes([Scope::parse("role.switch").unwrap()]);

        let role_switch = Arc::new(RoleSwitchTool::new());
        let role_switch_id = role_switch.id();
        let audit = RecordingAudit::new();
        let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
            Arc::clone(&role_switch) as Arc<dyn Tool>,
        ]));

        // Factory that always returns Err.
        let factory: Arc<ChildAgentFactory> =
            Arc::new(|target: &str| Err(format!("unknown target role {target:?}")));
        role_switch.set_child_factory(factory).ok();

        let parent_plan = vec![
            NextStep::ToolCall {
                tool_id: role_switch_id,
                input: json!({ "target": "phantom", "task": "x" }),
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(parent_plan);
        let parent_agent = ConcreteAgent::new(
            AgentId::new(),
            parent_caps,
            Arc::clone(&tools),
            Arc::clone(&audit) as Arc<dyn AuditHook>,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let _ = parent_agent.turn(message, &channel).await;

        let events = audit.snapshot();

        // Only the parent's TurnStarted fires — the child is never
        // constructed because the factory returned Err.
        let snapshots = turn_started_cap_snapshots(&events);
        assert_eq!(snapshots.len(), 1, "factory Err must not start a child turn");

        // The ToolCall audit event for role.switch is `Failed`, and
        // it carries NO `ScopeDenied` (this is a tool-level failure,
        // not a policy denial — the scope gate passed, the factory
        // lookup failed).
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "factory failure must NOT emit a ScopeDenied event"
        );
        let tool_outcome = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ToolCall {
                    tool_id, outcome, ..
                } if *tool_id == role_switch_id => Some(outcome),
                _ => None,
            })
            .expect("must emit a ToolCall audit event for the role.switch attempt");
        assert!(
            matches!(tool_outcome, ToolOutcomeSummary::Failed),
            "factory Err must surface as Failed, got {tool_outcome:?}"
        );
    }

    #[tokio::test]
    async fn role_switch_unconfigured_factory_produces_failed_outcome_not_panic() {
        // A RoleSwitchTool without `set_child_factory` called is a
        // misconfigured session layer. The tool must return
        // `Failed` (not panic) so the error surfaces as a planner-
        // observable tool failure. This test pins that diagnostic
        // path.
        let parent_caps =
            CapabilitySet::from_scopes([Scope::parse("role.switch").unwrap()]);

        let role_switch = Arc::new(RoleSwitchTool::new());
        let role_switch_id = role_switch.id();
        let audit = RecordingAudit::new();
        let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
            Arc::clone(&role_switch) as Arc<dyn Tool>,
        ]));
        // NOTE: deliberately DO NOT call `set_child_factory`.

        let parent_plan = vec![
            NextStep::ToolCall {
                tool_id: role_switch_id,
                input: json!({ "target": "researcher", "task": "x" }),
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(parent_plan);
        let parent_agent = ConcreteAgent::new(
            AgentId::new(),
            parent_caps,
            Arc::clone(&tools),
            Arc::clone(&audit) as Arc<dyn AuditHook>,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let outcome = parent_agent.turn(message, &channel).await;

        // Parent turn still completes — a single Failed tool call
        // is observed by the planner, not a turn-level failure.
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "parent turn should complete even when role.switch fails: {outcome:?}"
        );

        let events = audit.snapshot();
        let tool_outcome = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ToolCall {
                    tool_id, outcome, ..
                } if *tool_id == role_switch_id => Some(outcome),
                _ => None,
            })
            .expect("must emit a ToolCall audit event for the unconfigured call");
        assert!(
            matches!(tool_outcome, ToolOutcomeSummary::Failed),
            "unconfigured factory must surface as Failed, got {tool_outcome:?}"
        );
    }

    #[tokio::test]
    async fn role_switch_child_and_parent_audit_events_use_distinct_turn_ids() {
        // PRODUCT.md P1.4 says each turn must be tagged by the role
        // active at turn-start. The structural realization of that
        // rule is: parent and child emit their OWN `TurnStarted`
        // events, each with its own `TurnId` (because
        // `ConcreteAgent::turn` mints a fresh one at entry). This
        // test pins the distinct-turn-id property.
        let parent_caps = CapabilitySet::from_scopes([
            Scope::parse("role.switch:researcher").unwrap(),
        ]);
        let child_caps =
            CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        let role_switch = Arc::new(RoleSwitchTool::new());
        let role_switch_id = role_switch.id();
        let audit = RecordingAudit::new();
        let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
            Arc::clone(&role_switch) as Arc<dyn Tool>,
        ]));

        let child_tools = Arc::clone(&tools);
        let child_audit = Arc::clone(&audit);
        let child_caps_for_factory = child_caps.clone();
        let factory: Arc<ChildAgentFactory> = Arc::new(move |_target: &str| {
            let plan = vec![NextStep::FinalMessage("child done".to_string())];
            Ok(build_child_agent(
                Arc::clone(&child_tools),
                Arc::clone(&child_audit),
                child_caps_for_factory.clone(),
                plan,
            ))
        });
        role_switch.set_child_factory(factory).ok();

        let parent_plan = vec![
            NextStep::ToolCall {
                tool_id: role_switch_id,
                input: json!({ "target": "researcher", "task": "x" }),
            },
            NextStep::FinalMessage("parent done".to_string()),
        ];
        let plan_arc = Arc::new(parent_plan);
        let parent_agent = ConcreteAgent::new(
            AgentId::new(),
            parent_caps,
            Arc::clone(&tools),
            Arc::clone(&audit) as Arc<dyn AuditHook>,
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "go");
        let _ = parent_agent.turn(message, &channel).await;

        let events = audit.snapshot();
        let turn_ids: Vec<TurnId> = events
            .iter()
            .filter_map(|e| match e {
                AuditTag::TurnStarted { turn_id, .. } => Some(*turn_id),
                _ => None,
            })
            .collect();
        assert_eq!(turn_ids.len(), 2, "expected 2 TurnStarted events, got {turn_ids:?}");
        assert_ne!(
            turn_ids[0], turn_ids[1],
            "parent and child must have distinct TurnIds — that's the P1.4 tagging guarantee"
        );
    }
}
