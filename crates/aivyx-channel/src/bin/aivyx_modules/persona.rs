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
    daemon_is_running, get_effective_persona, list_persona_deltas, revert_persona_delta,
};
use aivyx_channel::daemon_ipc::{
    default_socket_path, EffectivePersonaSummary, PersonaDeltaSummary,
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
}
