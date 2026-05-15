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
pub async fn run_persona_list() -> Result<(), String> {
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
    print!("{}", render_delta_list(&all));
    Ok(())
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
    for p in proposals {
        let op_str =
            serde_json::to_string(&p.proposed_op).unwrap_or_else(|_| "{}".into());
        out.push_str(&format!(
            "[{status}] {id}  category={category}  proposed_at={ts}ms\n  op = {op}\n",
            status = p.status,
            id = p.id,
            category = p.category,
            ts = p.proposed_at_unix_ms,
            op = op_str,
        ));
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
