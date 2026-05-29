//! Operator-facing `aivyx persona` CLI surface — Phase 60.
//!
//! Phase 59 shipped the Persona substrate (storage + chain + propose +
//! apply + assembly). This module ships the operator-facing inspection
//! and revert surface — `show`, `list`, `revert` — closing PRODUCT.md
//! P14 alongside the Web UI Persona pane (Task 5) and the IPC envelopes
//! (Task 4).
//!
//! All three subcommands talk to a running daemon over IPC per the
//! Phase 60 sign-off. The chain lives in encrypted storage; routing
//! through the daemon avoids duplicating the master-key unlock path
//! and ensures the daemon's runtime state stays in sync after revert.
//! If no daemon is running, the CLI errors with a "start daemon
//! first" message.

use std::path::Path;

use aivyx_channel::daemon_client::{
    daemon_is_running, get_effective_persona, get_persona_proposal, list_persona_deltas,
    list_persona_proposals, resolve_persona_proposal, revert_persona_delta,
};
use aivyx_channel::daemon_ipc::{
    default_socket_path, EffectivePersonaSummary, PersonaDeltaSummary,
    PersonaProposalResolution, PersonaProposalSummary,
};

/// Entry point for `aivyx persona show`. Fetches the daemon's current
/// effective Persona via the `GetEffectivePersona` IPC query, then
/// renders it in a labeled banner-style format mirroring `aivyx
/// profile show`.
pub async fn run_persona_show() -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let persona = get_effective_persona(&socket_path)
        .await
        .map_err(|e| format!("failed to fetch persona: {e}"))?;
    print!("{}", render_persona_for_show(&persona));
    Ok(())
}

/// Entry point for `aivyx persona list`. Fetches every approved
/// delta via paginated `ListPersonaDeltas` queries, then renders
/// them in chain order with ids, timestamps, categories, and ops.
/// Phase 113 — `aivyx persona list [--auto-only |
/// --manual-only]` filter discriminator. Mirrors
/// `PersonaListFilter` in the binary; defined here in the
/// module so [`run_persona_list`]'s signature stays in this
/// crate's CLI module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaListPrefix {
    All,
    AutoOnly,
    ManualOnly,
}

/// `pd-auto-` is the `delta_id` prefix Phase 112's
/// `write_auto_accepted_skill` synthesizes for auto-accepted
/// entries. The filter compares the `delta_id` against this
/// prefix to partition the chain into auto vs manual.
pub const AUTO_ACCEPTED_DELTA_ID_PREFIX: &str = "pd-auto-";

pub async fn run_persona_list(filter: PersonaListPrefix) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let mut all: Vec<PersonaDeltaSummary> = Vec::new();
    let mut from_seq = 0u64;
    let page_size = 100u32;
    loop {
        let (entries, total_len) = list_persona_deltas(&socket_path, from_seq, page_size)
            .await
            .map_err(|e| format!("failed to list persona deltas: {e}"))?;
        let returned = entries.len();
        all.extend(entries);
        if returned == 0 || all.len() as u64 >= total_len {
            break;
        }
        from_seq += returned as u64;
    }
    let filtered = filter_delta_list(&all, filter);
    print!("{}", render_delta_list(&filtered));
    Ok(())
}

/// Apply the Phase 113 list filter. `All` returns the input
/// unchanged; `AutoOnly` keeps entries whose `delta_id`
/// starts with [`AUTO_ACCEPTED_DELTA_ID_PREFIX`];
/// `ManualOnly` keeps the complement. Pure function — kept
/// public for unit tests.
pub fn filter_delta_list(
    deltas: &[PersonaDeltaSummary],
    filter: PersonaListPrefix,
) -> Vec<PersonaDeltaSummary> {
    match filter {
        PersonaListPrefix::All => deltas.to_vec(),
        PersonaListPrefix::AutoOnly => deltas
            .iter()
            .filter(|d| d.delta_id.starts_with(AUTO_ACCEPTED_DELTA_ID_PREFIX))
            .cloned()
            .collect(),
        PersonaListPrefix::ManualOnly => deltas
            .iter()
            .filter(|d| !d.delta_id.starts_with(AUTO_ACCEPTED_DELTA_ID_PREFIX))
            .cloned()
            .collect(),
    }
}

/// Entry point for `aivyx persona revert <delta_id>`. Sends a
/// `RevertPersonaDelta` IPC message to the daemon, which appends a
/// `Revert` op delta to the chain and recomputes the shared runtime
/// state. Per Q5(a) at Phase 60 sign-off, no operator approval gate
/// — the operator is the proposer.
pub async fn run_persona_revert(target_delta_id: &str) -> Result<(), String> {
    if target_delta_id.is_empty() {
        return Err("`aivyx persona revert` requires a delta id".into());
    }
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    match revert_persona_delta(&socket_path, target_delta_id).await {
        Ok(seq) => {
            eprintln!(
                "aivyx persona revert: ok — appended revert delta at chain seq {seq} \
                 (target: {target_delta_id})"
            );
            Ok(())
        }
        Err(e) => Err(format!("revert failed: {e}")),
    }
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx persona: no daemon running on socket {} — \
         start the daemon first with `aivyx daemon run` (or just `aivyx`)",
        socket_path.display(),
    ))
}

/// Render the effective Persona for `show`. Pure function — split
/// out so unit tests can drive it against fixtures without IPC.
fn render_persona_for_show(persona: &EffectivePersonaSummary) -> String {
    let mut out = String::new();
    out.push_str("Persona\n");
    out.push_str("=======\n\n");

    if !persona.is_non_empty {
        out.push_str(
            "Persona is empty — no reflection-approved deltas have shaped this assistant yet.\n",
        );
        return out;
    }

    // Refined scalars.
    if let Some(name) = &persona.assistant_name {
        out.push_str(&format!("  assistant_name (refined)    = {name:?}\n"));
    }
    if let Some(op) = &persona.operator_profile {
        out.push_str(&format!("  operator_profile (refined)  = {op:?}\n"));
    }
    if let Some(style) = &persona.communication_style {
        out.push_str(&format!("  communication_style (refined) = {style:?}\n"));
    }

    // List categories.
    render_list_field(&mut out, "primary_use_cases", &persona.primary_use_cases);
    render_list_field(
        &mut out,
        "behavioral_preferences",
        &persona.behavioral_preferences,
    );
    render_list_field(
        &mut out,
        "behavioral_constraints",
        &persona.behavioral_constraints,
    );
    render_list_field(&mut out, "learned_context", &persona.learned_context);
    render_list_field(
        &mut out,
        "communication_adaptations",
        &persona.communication_adaptations,
    );
    render_list_field(&mut out, "character_traits", &persona.character_traits);
    render_list_field(
        &mut out,
        "relationship_milestones",
        &persona.relationship_milestones,
    );
    out
}

fn render_list_field(out: &mut String, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("  {label}:\n"));
    for item in items {
        out.push_str(&format!("    - {item}\n"));
    }
}

/// Render the persona delta log for `list`. Each entry shows its seq,
/// id, category, op, and approval timestamp.
fn render_delta_list(deltas: &[PersonaDeltaSummary]) -> String {
    let mut out = String::new();
    out.push_str("Persona delta log\n");
    out.push_str("=================\n\n");
    if deltas.is_empty() {
        out.push_str("No persona deltas have been approved yet.\n");
        return out;
    }
    for d in deltas {
        let op_str = serde_json::to_string(&d.op).unwrap_or_else(|_| "{}".to_string());
        out.push_str(&format!(
            "[#{seq}] {id}  category={category}  approved_at={ts}ms\n  op = {op}\n",
            seq = d.seq,
            id = d.delta_id,
            category = d.category,
            ts = d.approved_at_unix_ms,
            op = op_str,
        ));
    }
    out
}

// ---------------------------------------------------------------
// Phase 70 — `aivyx persona proposals` subcommand handlers.
// ---------------------------------------------------------------

/// Entry point for `aivyx persona proposals list [--status ...]`.
pub async fn run_persona_proposals_list(status: &str) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (proposals, total_len) =
        list_persona_proposals(&socket_path, status, 200)
            .await
            .map_err(|e| format!("failed to list persona proposals: {e}"))?;
    print!(
        "{}",
        render_proposal_list(status, &proposals, total_len)
    );
    Ok(())
}

/// Entry point for `aivyx persona proposals show <id>`.
pub async fn run_persona_proposals_show(proposal_id: &str) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let proposal = get_persona_proposal(&socket_path, proposal_id)
        .await
        .map_err(|e| format!("failed to fetch persona proposal: {e}"))?;
    match proposal {
        Some(p) => {
            print!("{}", render_proposal_detail(&p));
            Ok(())
        }
        None => Err(format!("no proposal found with id `{proposal_id}`")),
    }
}

/// Entry point for `aivyx persona proposals approve <id>`. CLI v1
/// applies the agent's proposed op verbatim; operators who want
/// to edit the op before approving use the Web UI Proposals pane.
pub async fn run_persona_proposals_approve(
    proposal_id: &str,
) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let success = resolve_persona_proposal(
        &socket_path,
        proposal_id,
        PersonaProposalResolution::Approve,
    )
    .await
    .map_err(|e| format!("approve failed: {e}"))?;
    eprintln!(
        "aivyx persona proposals approve: ok — proposal `{proposal_id}` \
         applied as persona delta at chain seq {seq}",
        seq = success
            .applied_seq
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
    );
    Ok(())
}

/// Entry point for `aivyx persona proposals reject <id> [--reason ...]`.
pub async fn run_persona_proposals_reject(
    proposal_id: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    resolve_persona_proposal(
        &socket_path,
        proposal_id,
        PersonaProposalResolution::Reject {
            reason: reason.map(str::to_string),
        },
    )
    .await
    .map_err(|e| format!("reject failed: {e}"))?;
    eprintln!(
        "aivyx persona proposals reject: ok — proposal `{proposal_id}` rejected"
    );
    Ok(())
}

/// Render a proposal list for `proposals list`. Pure function so
/// unit tests can drive against fixtures without IPC.
///
/// Phase 94 — linked supersession pairs (mutually-referenced
/// `supersedes_proposal_id`) render together with a
/// `└─ supersedes:` / `└─ superseded by:` indicator under
/// each half. Standalone proposals render exactly as
/// pre-Phase-94. The Phase 92 guarantee that each half
/// remains independently `Revert`-able is preserved — the
/// CLI subcommands `approve` / `reject` still take a single
/// proposal id.
fn render_proposal_list(
    status: &str,
    proposals: &[PersonaProposalSummary],
    total_len: u64,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Persona proposals (status filter: {status})\n",
    ));
    out.push_str("==========================================\n\n");
    if proposals.is_empty() {
        out.push_str(&format!(
            "No proposals matching status filter `{status}`.\n"
        ));
        return out;
    }
    for rendering in
        aivyx_channel::proposal_grouping::group_supersession_pairs(proposals)
    {
        match rendering {
            aivyx_channel::proposal_grouping::ProposalRendering::Linked {
                remove_side,
                append_side,
            } => {
                out.push_str(&render_one_proposal_row(remove_side));
                out.push_str(&format!(
                    "  └─ superseded by: {}\n",
                    append_side.id,
                ));
                out.push_str(&render_one_proposal_row(append_side));
                out.push_str(&format!(
                    "  └─ supersedes: {}\n",
                    remove_side.id,
                ));
            }
            aivyx_channel::proposal_grouping::ProposalRendering::Unlinked(
                p,
            ) => {
                out.push_str(&render_one_proposal_row(p));
            }
        }
    }
    if total_len as usize > proposals.len() {
        out.push_str(&format!(
            "\n(showing first {} of {} proposals matching the filter)\n",
            proposals.len(),
            total_len,
        ));
    }
    out
}

/// Render one proposal row in the standard pre-Phase-94
/// shape. Pulled out so both the linked-pair and unlinked
/// paths emit byte-identical row content; only the
/// surrounding `└─` indicator differs.
fn render_one_proposal_row(p: &PersonaProposalSummary) -> String {
    let op_str =
        serde_json::to_string(&p.proposed_op).unwrap_or_else(|_| "{}".into());
    format!(
        "[{status}] {id}  category={category}  proposed_at={ts}ms\n  op = {op}\n",
        status = p.status,
        id = p.id,
        category = p.category,
        ts = p.proposed_at_unix_ms,
        op = op_str,
    )
}

/// Render full proposal detail for `proposals show <id>`.
fn render_proposal_detail(p: &PersonaProposalSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!("Proposal {id}\n", id = p.id));
    out.push_str("=========================\n");
    out.push_str(&format!("  status      = {}\n", p.status));
    out.push_str(&format!("  category    = {}\n", p.category));
    out.push_str(&format!(
        "  proposed_at = {}ms\n",
        p.proposed_at_unix_ms,
    ));
    out.push_str(&format!(
        "  source ses  = {}\n",
        p.source_reflection_session_id,
    ));
    let op_str = serde_json::to_string_pretty(&p.proposed_op)
        .unwrap_or_else(|_| "{}".into());
    out.push_str(&format!(
        "\n  proposed op:\n{}\n",
        indent_block(&op_str, "    "),
    ));
    // Phase 118 — when the category is one of the operator-
    // staged refinement kinds (ProfileHint or
    // RoleDefinitionSuggestion), the AppendList value is a
    // JSON-serialized payload. Render it human-readably so
    // the operator doesn't have to parse JSON-in-JSON to
    // decide whether to approve.
    if let Some(rendered) = render_phase_118_payload(&p.category, &p.proposed_op) {
        out.push_str(&format!("\n  rendered draft:\n{rendered}"));
    }
    if let Some(reason) = &p.proposed_reason {
        out.push_str(&format!("\n  agent reason: {reason}\n"));
    }
    if let Some(applied_op) = &p.applied_op {
        let applied_str = serde_json::to_string_pretty(applied_op)
            .unwrap_or_else(|_| "{}".into());
        if applied_str != op_str {
            out.push_str(&format!(
                "\n  applied op (operator-edited):\n{}\n",
                indent_block(&applied_str, "    "),
            ));
        }
    }
    if let Some(seq) = p.applied_seq {
        out.push_str(&format!("\n  applied at persona chain seq: {seq}\n"));
    }
    if let Some(reason) = &p.rejected_reason {
        out.push_str(&format!("\n  operator reject reason: {reason}\n"));
    }
    if let Some(ts) = p.resolved_at_unix_ms {
        out.push_str(&format!("\n  resolved_at = {ts}ms\n"));
    }
    out
}

fn indent_block(s: &str, indent: &str) -> String {
    s.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Phase 118 — when the proposal's category is one of the
/// operator-staged refinement kinds, the `AppendList.value`
/// field carries a JSON-serialized
/// [`aivyx_core::skill_proposer::ProfileFieldHint`] or
/// [`aivyx_core::skill_proposer::RoleDraft`] payload. This
/// renders the payload in operator-readable form so
/// `aivyx persona proposals show <id>` doesn't make the
/// operator parse JSON-in-JSON.
///
/// Returns `None` when the category isn't Phase 118, when the
/// op shape isn't `AppendList`, or when the inner blob
/// doesn't parse — the proposed_op JSON dump above still
/// shows the raw form so nothing is hidden.
fn render_phase_118_payload(
    category: &str,
    proposed_op: &serde_json::Value,
) -> Option<String> {
    // Op must be an `AppendList { value: <json-string> }`.
    let kind = proposed_op.get("kind")?.as_str()?;
    if kind != "AppendList" {
        return None;
    }
    let inner_blob = proposed_op.get("value")?.as_str()?;
    match category {
        "ProfileHint" => render_profile_hint_payload(inner_blob),
        "RoleDefinitionSuggestion" => render_role_draft_payload(inner_blob),
        _ => None,
    }
}

fn render_profile_hint_payload(blob: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(blob).ok()?;
    let field = parsed.get("field")?.as_str()?;
    let suggested_value = parsed.get("suggested_value")?.as_str()?;
    let rationale = parsed.get("rationale")?.as_str()?;
    let mut out = String::new();
    out.push_str(&format!("    field            = {field}\n"));
    out.push_str(&format!("    suggested_value  = {suggested_value:?}\n"));
    out.push_str("    rationale        =\n");
    for line in rationale.lines() {
        out.push_str(&format!("      {line}\n"));
    }
    out.push_str(
        "\n  To apply: edit aivyx.toml [profile] and update the\n",
    );
    out.push_str(
        "  field above. Phase 118 does NOT auto-mutate aivyx.toml.\n",
    );
    Some(out)
}

fn render_role_draft_payload(blob: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(blob).ok()?;
    let name = parsed.get("name")?.as_str()?;
    let parent = parsed.get("parent").and_then(|v| v.as_str());
    let system_prompt_addendum =
        parsed.get("system_prompt_addendum")?.as_str()?;
    let tool_allowlist_additions: Vec<String> = parsed
        .get("tool_allowlist_additions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let rationale = parsed.get("rationale")?.as_str()?;
    let mut out = String::new();
    out.push_str(&format!("    name             = {name}\n"));
    match parent {
        Some(p) => {
            out.push_str(&format!("    parent           = {p}\n"));
        }
        None => {
            out.push_str("    parent           = (none — top-level role)\n");
        }
    }
    out.push_str("    system_prompt_addendum:\n");
    for line in system_prompt_addendum.lines() {
        out.push_str(&format!("      {line}\n"));
    }
    if tool_allowlist_additions.is_empty() {
        out.push_str("    tool_allowlist_additions: (none)\n");
    } else {
        out.push_str("    tool_allowlist_additions:\n");
        for tool in &tool_allowlist_additions {
            out.push_str(&format!("      - {tool}\n"));
        }
    }
    out.push_str("    rationale        =\n");
    for line in rationale.lines() {
        out.push_str(&format!("      {line}\n"));
    }
    out.push_str(
        "\n  To apply: edit aivyx.toml and add a [roles.<name>]\n",
    );
    out.push_str(
        "  section using the addendum + tool_allowlist above\n",
    );
    out.push_str(
        "  on top of any inherited parent role. Phase 118 does NOT\n",
    );
    out.push_str("  auto-mutate aivyx.toml.\n");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_persona() -> EffectivePersonaSummary {
        EffectivePersonaSummary::default()
    }

    fn populated_persona() -> EffectivePersonaSummary {
        EffectivePersonaSummary {
            assistant_name: Some("Codex".into()),
            communication_style: Some("terse, conclusion-first".into()),
            behavioral_preferences: vec!["always cite sources".into()],
            learned_context: vec!["operator uses Vim".into()],
            is_non_empty: true,
            ..Default::default()
        }
    }

    #[test]
    fn show_empty_persona_renders_explanatory_message() {
        let out = render_persona_for_show(&empty_persona());
        assert!(out.contains("Persona is empty"));
        assert!(out.contains("no reflection-approved deltas"));
    }

    #[test]
    fn show_populated_persona_renders_labeled_fields() {
        let out = render_persona_for_show(&populated_persona());
        assert!(out.contains("assistant_name (refined)"));
        assert!(out.contains("Codex"));
        assert!(out.contains("communication_style (refined)"));
        assert!(out.contains("behavioral_preferences:"));
        assert!(out.contains("- always cite sources"));
        assert!(out.contains("learned_context:"));
        assert!(out.contains("- operator uses Vim"));
        // Empty fields don't appear at all.
        assert!(!out.contains("character_traits"));
        assert!(!out.contains("relationship_milestones"));
    }

    #[test]
    fn list_empty_chain_renders_explanatory_message() {
        let out = render_delta_list(&[]);
        assert!(out.contains("No persona deltas have been approved yet"));
    }

    #[test]
    fn list_one_delta_renders_seq_id_category_op() {
        let delta = PersonaDeltaSummary {
            seq: 5,
            delta_id: "pd-abc".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            approved_at_unix_ms: 1_715_000_060_000,
            proposal_id: "rp-1".into(),
            category: "BehavioralPreferences".into(),
            op: serde_json::json!({ "kind": "AppendList", "value": "prefer terse" }),
            mac_hex: "0".repeat(64),
        };
        let out = render_delta_list(&[delta]);
        assert!(out.contains("[#5]"));
        assert!(out.contains("pd-abc"));
        assert!(out.contains("category=BehavioralPreferences"));
        assert!(out.contains("approved_at=1715000060000ms"));
        assert!(out.contains("\"kind\":\"AppendList\""));
        assert!(out.contains("prefer terse"));
    }

    // ---- Phase 70 — proposal render helpers ----

    fn pending_fixture() -> PersonaProposalSummary {
        PersonaProposalSummary {
            id: "pp-abc".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-1".into(),
            status: "Pending".into(),
            category: "BehavioralPreferences".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": "prefer terse",
            }),
            proposed_reason: Some("operator confirmed terse 3x".into()),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: None,
        }
    }

    #[test]
    fn proposals_list_empty_renders_filter_aware_message() {
        let out = render_proposal_list("pending", &[], 0);
        assert!(out.contains("status filter: pending"));
        assert!(
            out.contains("No proposals matching status filter `pending`")
        );
    }

    #[test]
    fn proposals_list_shows_each_proposal_with_status_category_op() {
        let out = render_proposal_list("pending", &[pending_fixture()], 1);
        assert!(out.contains("[Pending] pp-abc"));
        assert!(out.contains("category=BehavioralPreferences"));
        assert!(out.contains("proposed_at=1715000000000ms"));
        assert!(out.contains("\"kind\":\"AppendList\""));
        assert!(out.contains("prefer terse"));
    }

    /// Phase 94 — a linked supersession pair (mutually-
    /// referenced `supersedes_proposal_id`) renders with the
    /// `└─ supersedes:` and `└─ superseded by:` indicators
    /// under each half. The RemoveList side comes first,
    /// regardless of input order, and points at the
    /// AppendList side.
    #[test]
    fn proposals_list_renders_linked_pair_with_indicator() {
        let remove_side = PersonaProposalSummary {
            id: "supersede-remove:consolidate-pair:auth+jwt".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-2".into(),
            status: "Pending".into(),
            category: "LearnedContext".into(),
            proposed_op: serde_json::json!({
                "kind": "RemoveList",
                "value": "consolidate-pair:auth+jwt",
            }),
            proposed_reason: Some("superseded".into()),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: Some(
                "consolidate-pair:auth+sessions".into(),
            ),
        };
        let append_side = PersonaProposalSummary {
            id: "consolidate-pair:auth+sessions".into(),
            proposed_at_unix_ms: 1_715_000_000_001,
            source_reflection_session_id: "ses-2".into(),
            status: "Pending".into(),
            category: "LearnedContext".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": "consolidate-pair:auth+sessions",
            }),
            proposed_reason: Some("supersedes auth+jwt".into()),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: Some(
                "supersede-remove:consolidate-pair:auth+jwt".into(),
            ),
        };
        // Input order: AppendList first; grouping helper
        // should still emit RemoveList side first.
        let out = render_proposal_list(
            "pending",
            &[append_side, remove_side],
            2,
        );
        assert!(out.contains(
            "[Pending] supersede-remove:consolidate-pair:auth+jwt"
        ));
        assert!(out.contains(
            "└─ superseded by: consolidate-pair:auth+sessions"
        ));
        assert!(out.contains(
            "[Pending] consolidate-pair:auth+sessions"
        ));
        assert!(out.contains(
            "└─ supersedes: supersede-remove:consolidate-pair:auth+jwt"
        ));
        // The RemoveList row appears before the AppendList row.
        let remove_pos = out
            .find("supersede-remove:consolidate-pair:auth+jwt")
            .unwrap();
        let append_pos = out
            .find("[Pending] consolidate-pair:auth+sessions")
            .unwrap();
        assert!(
            remove_pos < append_pos,
            "RemoveList row must render before AppendList row"
        );
    }

    /// Phase 94 — an orphan-link proposal (its
    /// `supersedes_proposal_id` points at a partner not in
    /// the input list, e.g., the partner was rejected and
    /// is filtered out by the status filter) degrades to
    /// the standard unlinked row. No `└─` indicator is
    /// emitted.
    #[test]
    fn proposals_list_orphan_link_degrades_to_unlinked() {
        let orphan = PersonaProposalSummary {
            id: "consolidate-pair:deploy+rollback".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-3".into(),
            status: "Pending".into(),
            category: "LearnedContext".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": "consolidate-pair:deploy+rollback",
            }),
            proposed_reason: Some("strengthened".into()),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: Some(
                "supersede-remove:consolidate-pair:deploy+ship".into(),
            ),
        };
        let out = render_proposal_list("pending", &[orphan], 1);
        assert!(out.contains(
            "[Pending] consolidate-pair:deploy+rollback"
        ));
        // Orphan: no link indicator since the partner is
        // absent.
        assert!(
            !out.contains("└─ supersedes:"),
            "orphan must not render a supersedes indicator"
        );
        assert!(
            !out.contains("└─ superseded by:"),
            "orphan must not render a superseded-by indicator"
        );
    }

    /// Phase 94 — pre-Phase-94 regression: a plain unlinked
    /// proposal (no `supersedes_proposal_id`) renders
    /// byte-identical to before. The indicator never
    /// appears on unlinked rows.
    #[test]
    fn proposals_list_unlinked_row_unchanged() {
        let out = render_proposal_list(
            "pending", &[pending_fixture()], 1,
        );
        assert!(!out.contains("└─"));
    }

    #[test]
    fn proposals_list_paginated_note_appears_when_total_exceeds_returned() {
        let out = render_proposal_list("pending", &[pending_fixture()], 5);
        assert!(out.contains("showing first 1 of 5 proposals"));
    }

    #[test]
    fn proposals_show_includes_proposed_op_reason_and_status() {
        let out = render_proposal_detail(&pending_fixture());
        assert!(out.contains("Proposal pp-abc"));
        assert!(out.contains("status      = Pending"));
        assert!(out.contains("category    = BehavioralPreferences"));
        assert!(out.contains("source ses  = ses-1"));
        assert!(out.contains("proposed op:"));
        assert!(out.contains("\"AppendList\""));
        assert!(out.contains("agent reason: operator confirmed terse 3x"));
        // Not yet resolved / not edited → these lines absent.
        assert!(!out.contains("applied op"));
        assert!(!out.contains("resolved_at"));
        assert!(!out.contains("operator reject reason"));
    }

    #[test]
    fn proposals_show_with_operator_edit_renders_both_ops() {
        let mut p = pending_fixture();
        p.status = "Approved".into();
        p.applied_op = Some(serde_json::json!({
            "kind": "AppendList",
            "value": "operator-edited preference",
        }));
        p.applied_seq = Some(7);
        p.resolved_at_unix_ms = Some(1_715_000_060_000);
        let out = render_proposal_detail(&p);
        assert!(out.contains("proposed op:"));
        assert!(out.contains("applied op (operator-edited):"));
        assert!(out.contains("operator-edited preference"));
        assert!(out.contains("applied at persona chain seq: 7"));
        assert!(out.contains("resolved_at = 1715000060000ms"));
    }

    #[test]
    fn proposals_show_with_rejection_renders_operator_reason() {
        let mut p = pending_fixture();
        p.status = "Rejected".into();
        p.rejected_reason = Some("too aggressive".into());
        p.resolved_at_unix_ms = Some(1_715_000_060_000);
        let out = render_proposal_detail(&p);
        assert!(out.contains("operator reject reason: too aggressive"));
        assert!(out.contains("resolved_at = 1715000060000ms"));
    }
}

#[cfg(test)]
mod phase_113_filter_tests {
    use super::*;

    fn delta_with_id(id: &str) -> PersonaDeltaSummary {
        PersonaDeltaSummary {
            seq: 0,
            delta_id: id.into(),
            proposed_at_unix_ms: 0,
            approved_at_unix_ms: 0,
            proposal_id: "p".into(),
            category: "LearnedSkill".into(),
            op: serde_json::json!({"kind": "AppendList", "value": "{}"}),
            mac_hex: "".into(),
        }
    }

    #[test]
    fn filter_all_passes_everything_through() {
        let input = vec![
            delta_with_id("pd-manual-001"),
            delta_with_id("pd-auto-abc"),
            delta_with_id("pd-manual-002"),
        ];
        let out = filter_delta_list(&input, PersonaListPrefix::All);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn filter_auto_only_keeps_pd_auto_prefix_entries() {
        let input = vec![
            delta_with_id("pd-manual-001"),
            delta_with_id("pd-auto-abc"),
            delta_with_id("pd-auto-def"),
            delta_with_id("pd-manual-002"),
        ];
        let out = filter_delta_list(&input, PersonaListPrefix::AutoOnly);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.delta_id.starts_with("pd-auto-")));
    }

    #[test]
    fn filter_manual_only_keeps_complement_of_pd_auto_prefix() {
        let input = vec![
            delta_with_id("pd-manual-001"),
            delta_with_id("pd-auto-abc"),
            delta_with_id("pd-approved-from-proposal"),
        ];
        let out = filter_delta_list(&input, PersonaListPrefix::ManualOnly);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| !d.delta_id.starts_with("pd-auto-")));
    }

    #[test]
    fn filter_on_empty_input_is_empty_regardless_of_filter() {
        let empty: Vec<PersonaDeltaSummary> = Vec::new();
        assert!(filter_delta_list(&empty, PersonaListPrefix::All).is_empty());
        assert!(filter_delta_list(&empty, PersonaListPrefix::AutoOnly).is_empty());
        assert!(filter_delta_list(&empty, PersonaListPrefix::ManualOnly).is_empty());
    }

    #[test]
    fn auto_accepted_prefix_constant_matches_phase_112_synthesizer() {
        // The constant must match what `write_auto_accepted_skill`
        // synthesizes in aivyx-channel/src/skill_auto_proposer.rs.
        // If a future phase changes either side, this test surfaces
        // the divergence.
        assert_eq!(AUTO_ACCEPTED_DELTA_ID_PREFIX, "pd-auto-");
    }

    // ----- Phase 118 — proposal-detail rendering for the new categories -----

    fn profile_hint_proposal() -> PersonaProposalSummary {
        let payload = serde_json::json!({
            "field": "CommunicationStyle",
            "suggested_value": "terse and bullet-formatted",
            "rationale": "operator consistently uses bullets in their own messages",
        });
        PersonaProposalSummary {
            id: "pp-phase118-hint".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-118".into(),
            status: "Pending".into(),
            category: "ProfileHint".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": payload.to_string(),
            }),
            proposed_reason: Some(
                "Phase 118 — observed recurring style preference".into(),
            ),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: None,
        }
    }

    fn role_definition_suggestion_proposal() -> PersonaProposalSummary {
        let payload = serde_json::json!({
            "name": "research-deploy",
            "parent": "research",
            "system_prompt_addendum": "After research, summarize deploy diff for approval.",
            "tool_allowlist_additions": ["git.commit", "shell.deploy"],
            "rationale": "operator's research-then-deploy shape repeats five+ times this week",
        });
        PersonaProposalSummary {
            id: "pp-phase118-role".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-118".into(),
            status: "Pending".into(),
            category: "RoleDefinitionSuggestion".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": payload.to_string(),
            }),
            proposed_reason: Some(
                "Phase 118 — recurring shape past existing role envelope".into(),
            ),
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: None,
        }
    }

    #[test]
    fn proposal_detail_renders_profile_hint_payload_humanreadably() {
        let proposal = profile_hint_proposal();
        let out = render_proposal_detail(&proposal);
        // Header carries the category.
        assert!(out.contains("category    = ProfileHint"));
        // Rendered draft block carries the field, value, rationale.
        assert!(out.contains("rendered draft:"));
        assert!(out.contains("field            = CommunicationStyle"));
        assert!(out.contains("bullet-formatted"));
        assert!(out.contains("rationale"));
        assert!(out.contains("uses bullets"));
        // Operator action instruction explains the workflow
        // (Phase 118 does NOT auto-mutate aivyx.toml).
        assert!(out.contains("To apply"));
        assert!(out.contains("aivyx.toml"));
        assert!(out.contains("does NOT auto-mutate"));
    }

    #[test]
    fn proposal_detail_renders_role_draft_payload_humanreadably() {
        let proposal = role_definition_suggestion_proposal();
        let out = render_proposal_detail(&proposal);
        assert!(out.contains("category    = RoleDefinitionSuggestion"));
        assert!(out.contains("rendered draft:"));
        assert!(out.contains("name             = research-deploy"));
        assert!(out.contains("parent           = research"));
        assert!(out.contains("system_prompt_addendum"));
        assert!(out.contains("summarize deploy diff"));
        assert!(out.contains("tool_allowlist_additions"));
        assert!(out.contains("- git.commit"));
        assert!(out.contains("- shell.deploy"));
        assert!(out.contains("rationale"));
        assert!(out.contains("repeats"));
        assert!(out.contains("[roles.<name>]"));
        assert!(out.contains("does NOT"));
    }

    #[test]
    fn proposal_detail_renders_role_draft_with_no_parent_as_top_level() {
        let mut proposal = role_definition_suggestion_proposal();
        // Override the payload's parent to null.
        let payload = serde_json::json!({
            "name": "operator-mode",
            "parent": null,
            "system_prompt_addendum": "operator-direct mode",
            "tool_allowlist_additions": [],
            "rationale": "top-level role distinct from anything existing",
        });
        proposal.proposed_op = serde_json::json!({
            "kind": "AppendList",
            "value": payload.to_string(),
        });
        let out = render_proposal_detail(&proposal);
        assert!(out.contains("name             = operator-mode"));
        assert!(out.contains("parent           = (none — top-level role)"));
        // Empty allowlist → explicit "(none)" rather than blank.
        assert!(out.contains("tool_allowlist_additions: (none)"));
    }

    #[test]
    fn proposal_detail_renders_non_phase_118_proposal_without_rendered_draft() {
        // Phase 117-and-earlier categories don't get the
        // rendered-draft block — the standard JSON dump above
        // the helper already shows everything.
        let proposal = PersonaProposalSummary {
            id: "pp-legacy".into(),
            proposed_at_unix_ms: 1_715_000_000_000,
            source_reflection_session_id: "ses-legacy".into(),
            status: "Pending".into(),
            category: "BehavioralPreferences".into(),
            proposed_op: serde_json::json!({
                "kind": "AppendList",
                "value": "prefer terse replies",
            }),
            proposed_reason: None,
            applied_op: None,
            applied_seq: None,
            rejected_reason: None,
            resolved_at_unix_ms: None,
            supersedes_proposal_id: None,
        };
        let out = render_proposal_detail(&proposal);
        assert!(out.contains("category    = BehavioralPreferences"));
        // No rendered-draft block for legacy categories.
        assert!(!out.contains("rendered draft:"));
    }

    #[test]
    fn proposal_detail_falls_back_when_phase_118_payload_is_malformed() {
        // Defensive: a malformed inner JSON blob shouldn't
        // crash the renderer. The proposed_op JSON dump above
        // still shows the raw value so nothing is hidden.
        let mut proposal = profile_hint_proposal();
        proposal.proposed_op = serde_json::json!({
            "kind": "AppendList",
            "value": "this is not valid json",
        });
        let out = render_proposal_detail(&proposal);
        assert!(out.contains("category    = ProfileHint"));
        // Rendered-draft helper returned None; section absent.
        assert!(!out.contains("rendered draft:"));
        // Raw op block is still there.
        assert!(out.contains("proposed op:"));
    }
}
