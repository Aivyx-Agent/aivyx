//! Phase 94 — client-side grouping for Phase 92 linked
//! supersession proposals.
//!
//! Phase 92 introduced the cross-referenced
//! `supersedes_proposal_id` field on `ProposedPersonaDelta`:
//! when shared-endpoint supersession fires, the
//! consolidation pass files **two linked proposals** — a
//! `RemoveList` half retiring the old endpoint and an
//! `AppendList` half proposing the new one — each carrying
//! the other's id. v1 surfaced the linkage textually only
//! (via the `reason` field on each half).
//!
//! Phase 94 keeps the IPC contract unchanged and lets both
//! the CLI and the Web UI render the linked pair as **one
//! grouped unit** by running this pure helper over the flat
//! `Vec<PersonaProposal>` the IPC already returns. The
//! grouping is structural — two proposals group together iff
//! their `supersedes_proposal_id` fields reference each other
//! mutually. Dangling references (the partner was rejected
//! or removed) degrade gracefully to `Unlinked`; self-
//! references and one-way links are defended.

use std::collections::{HashMap, HashSet};

use crate::persona::PersonaDeltaOp;
use crate::persona_proposal::PersonaProposal;

/// One row in the grouped rendering output.
///
/// `Linked { remove_side, append_side }` is the Phase 92
/// supersession pair: the `RemoveList` half retiring the old
/// `consolidate-pair:` facet + the `AppendList` half
/// proposing the new one, both carrying the cross-referenced
/// `supersedes_proposal_id`. The renderer treats the two as
/// one operator-facing decision (one primary "approve both"
/// action plus a split menu for the partial cases — Phase 92
/// guarantees each half remains independently
/// `Revert`-able).
///
/// `Unlinked` is everything else: standalone recall-feedback
/// proposals (Phase 77), standalone consolidate-pair
/// proposals from cycles where supersession didn't fire,
/// and any proposal whose link partner is missing from the
/// input (the partner was resolved/removed; the surviving
/// half renders flat).
#[derive(Debug)]
pub enum ProposalRendering<'a> {
    Linked {
        remove_side: &'a PersonaProposal,
        append_side: &'a PersonaProposal,
    },
    Unlinked(&'a PersonaProposal),
}

/// Group Phase 92 linked supersession proposals.
///
/// Pure structural pass: builds an id → proposal map, then
/// walks the input list in order emitting one
/// `ProposalRendering` per logical unit (one per linked
/// pair; one per unlinked proposal). Stability:
///
/// - Input ordering is preserved at the *pair* level —
///   each pair appears at the position of its
///   first-encountered half in the input.
/// - Within a `Linked` rendering, `remove_side` is always
///   the half whose op is `RemoveList` and `append_side`
///   is always the half whose op is `AppendList`,
///   regardless of which half came first in input order.
///
/// Defensive cases (all degrade to `Unlinked` rather than
/// panic):
///
/// - **Self-reference** (a proposal's
///   `supersedes_proposal_id` equals its own id) → emit
///   `Unlinked` for that proposal.
/// - **Dangling reference** (the partner id is not in the
///   input list) → emit `Unlinked` for the surviving half.
/// - **Asymmetric link** (A points at B but B doesn't
///   point back at A) → emit `Unlinked` for both. Only
///   mutual references group.
/// - **Same-op pair** (both halves point at each other but
///   both are `AppendList`, or both `RemoveList`, or
///   neither is a list op) → emit `Unlinked` for both.
///   The grouping invariant requires exactly one of each.
pub fn group_supersession_pairs(
    proposals: &[PersonaProposal],
) -> Vec<ProposalRendering<'_>> {
    let by_id: HashMap<&str, &PersonaProposal> = proposals
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();
    let mut emitted: HashSet<&str> = HashSet::new();
    let mut out: Vec<ProposalRendering<'_>> =
        Vec::with_capacity(proposals.len());

    for p in proposals {
        if emitted.contains(p.id.as_str()) {
            continue;
        }
        let Some(partner_id) =
            p.proposed_op.supersedes_proposal_id.as_deref()
        else {
            out.push(ProposalRendering::Unlinked(p));
            emitted.insert(p.id.as_str());
            continue;
        };
        if partner_id == p.id {
            // Self-reference defended.
            out.push(ProposalRendering::Unlinked(p));
            emitted.insert(p.id.as_str());
            continue;
        }
        let Some(partner) = by_id.get(partner_id).copied() else {
            // Dangling reference — partner not in input.
            out.push(ProposalRendering::Unlinked(p));
            emitted.insert(p.id.as_str());
            continue;
        };
        let partner_back =
            partner.proposed_op.supersedes_proposal_id.as_deref();
        if partner_back != Some(p.id.as_str()) {
            // Asymmetric link — only one side points at the
            // other. The other side will be visited in its
            // own turn and also fall through to `Unlinked`.
            out.push(ProposalRendering::Unlinked(p));
            emitted.insert(p.id.as_str());
            continue;
        }
        // Mutual reference confirmed. Determine RemoveList
        // vs AppendList; require exactly one of each.
        let (remove_side, append_side) = match (
            &p.proposed_op.op,
            &partner.proposed_op.op,
        ) {
            (
                PersonaDeltaOp::RemoveList { .. },
                PersonaDeltaOp::AppendList { .. },
            ) => (p, partner),
            (
                PersonaDeltaOp::AppendList { .. },
                PersonaDeltaOp::RemoveList { .. },
            ) => (partner, p),
            _ => {
                // Same-op pair (both AppendList, both
                // RemoveList, or neither a list op) — the
                // grouping invariant requires exactly one
                // of each. Defend by emitting both as
                // Unlinked.
                out.push(ProposalRendering::Unlinked(p));
                emitted.insert(p.id.as_str());
                continue;
            }
        };
        out.push(ProposalRendering::Linked {
            remove_side,
            append_side,
        });
        emitted.insert(remove_side.id.as_str());
        emitted.insert(append_side.id.as_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{
        PersonaDeltaCategory, PersonaDeltaOp, ProposedPersonaDelta,
    };
    use crate::persona_proposal::{
        PersonaProposal, ProposalStatus,
    };

    /// Build a proposal with the named id + op + optional
    /// `supersedes_proposal_id`. All other fields use stable
    /// defaults so the test fixtures stay terse.
    fn proposal(
        id: &str,
        op: PersonaDeltaOp,
        supersedes: Option<&str>,
    ) -> PersonaProposal {
        PersonaProposal {
            id: id.into(),
            proposed_at_unix_ms: 1_000_000,
            source_reflection_session_id: "test-session".into(),
            proposed_op: ProposedPersonaDelta {
                op,
                category: PersonaDeltaCategory::LearnedContext,
                reason: Some("test".into()),
                supersedes_proposal_id: supersedes.map(String::from),
            },
            status: ProposalStatus::Pending,
        }
    }

    fn remove_list(id: &str, supersedes: Option<&str>) -> PersonaProposal {
        proposal(
            id,
            PersonaDeltaOp::RemoveList {
                value: format!("remove {id}"),
            },
            supersedes,
        )
    }

    fn append_list(id: &str, supersedes: Option<&str>) -> PersonaProposal {
        proposal(
            id,
            PersonaDeltaOp::AppendList {
                value: format!("append {id}"),
            },
            supersedes,
        )
    }

    /// Mixed fixture (Q4a): two linked-pair proposals + one
    /// orphan-link proposal + one unlinked recall-fb
    /// proposal + one unlinked consolidate-pair proposal →
    /// exactly one `Linked`, three `Unlinked`. The orphan
    /// degrades gracefully (its partner isn't in the list).
    #[test]
    fn mixed_fixture_one_linked_three_unlinked() {
        let proposals = vec![
            // Pair: RemoveList superseded by AppendList.
            remove_list(
                "supersede-remove:consolidate-pair:auth+jwt",
                Some("consolidate-pair:auth+sessions"),
            ),
            append_list(
                "consolidate-pair:auth+sessions",
                Some("supersede-remove:consolidate-pair:auth+jwt"),
            ),
            // Orphan: points at a proposal not in the list.
            append_list(
                "consolidate-pair:deploy+rollback",
                Some("supersede-remove:consolidate-pair:deploy+ship"),
            ),
            // Plain unlinked recall-feedback proposal.
            append_list("recall-fb:notes", None),
            // Plain unlinked consolidate-pair proposal
            // (no supersession this cycle).
            append_list("consolidate-pair:auth+oauth", None),
        ];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 4);
        let mut linked = 0;
        let mut unlinked = 0;
        for r in &rendered {
            match r {
                ProposalRendering::Linked {
                    remove_side,
                    append_side,
                } => {
                    linked += 1;
                    assert_eq!(
                        remove_side.id,
                        "supersede-remove:consolidate-pair:auth+jwt"
                    );
                    assert_eq!(
                        append_side.id,
                        "consolidate-pair:auth+sessions"
                    );
                }
                ProposalRendering::Unlinked(_) => {
                    unlinked += 1;
                }
            }
        }
        assert_eq!(linked, 1);
        assert_eq!(unlinked, 3);
    }

    /// The `Linked` rendering always puts the `RemoveList`
    /// side in `remove_side` and the `AppendList` side in
    /// `append_side`, regardless of input order.
    #[test]
    fn linked_pair_sorts_remove_before_append_by_op() {
        // Input order: AppendList first.
        let proposals_a = vec![
            append_list("A", Some("R")),
            remove_list("R", Some("A")),
        ];
        let r_a = group_supersession_pairs(&proposals_a);
        assert_eq!(r_a.len(), 1);
        match &r_a[0] {
            ProposalRendering::Linked {
                remove_side,
                append_side,
            } => {
                assert_eq!(remove_side.id, "R");
                assert_eq!(append_side.id, "A");
            }
            other => panic!("expected Linked, got {other:?}"),
        }

        // Input order: RemoveList first. Same result.
        let proposals_b = vec![
            remove_list("R", Some("A")),
            append_list("A", Some("R")),
        ];
        let r_b = group_supersession_pairs(&proposals_b);
        assert_eq!(r_b.len(), 1);
        match &r_b[0] {
            ProposalRendering::Linked {
                remove_side,
                append_side,
            } => {
                assert_eq!(remove_side.id, "R");
                assert_eq!(append_side.id, "A");
            }
            other => panic!("expected Linked, got {other:?}"),
        }
    }

    /// Self-reference (a proposal pointing at itself) is
    /// defended — emit `Unlinked` rather than treating it
    /// as a self-pair.
    #[test]
    fn self_reference_degrades_to_unlinked() {
        let proposals =
            vec![append_list("self-ref", Some("self-ref"))];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 1);
        assert!(matches!(
            &rendered[0],
            ProposalRendering::Unlinked(_)
        ));
    }

    /// Asymmetric link — A points at B but B doesn't point
    /// back. Both render as `Unlinked` (the half-link is
    /// not strong enough to group). Defends against
    /// malformed data from operator edits or partial
    /// chain replays.
    #[test]
    fn asymmetric_link_emits_both_unlinked() {
        let proposals = vec![
            // A points at B.
            append_list("A", Some("B")),
            // B doesn't point back.
            remove_list("B", None),
        ];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 2);
        for r in &rendered {
            assert!(matches!(r, ProposalRendering::Unlinked(_)));
        }
    }

    /// Same-op pair (both AppendList, mutually linked) is
    /// defended — the grouping invariant requires exactly
    /// one `RemoveList` + one `AppendList`. Mutual link
    /// between two AppendLists is treated as a structural
    /// error and both render `Unlinked`.
    #[test]
    fn same_op_pair_emits_both_unlinked() {
        let proposals = vec![
            append_list("A1", Some("A2")),
            append_list("A2", Some("A1")),
        ];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 2);
        for r in &rendered {
            assert!(matches!(r, ProposalRendering::Unlinked(_)));
        }
    }

    /// Dangling reference (a proposal points at a partner
    /// id that's not in the input list — partner was
    /// rejected/removed) degrades to `Unlinked` for the
    /// surviving half. The pair-text reason on the
    /// surviving proposal still mentions the resolved
    /// partner; Phase 94 just renders this half flat.
    #[test]
    fn dangling_partner_id_degrades_to_unlinked() {
        let proposals = vec![append_list(
            "consolidate-pair:deploy+rollback",
            Some("supersede-remove:consolidate-pair:deploy+ship"),
        )];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 1);
        match &rendered[0] {
            ProposalRendering::Unlinked(p) => {
                assert_eq!(
                    p.id,
                    "consolidate-pair:deploy+rollback"
                );
            }
            other => panic!("expected Unlinked, got {other:?}"),
        }
    }

    /// Empty input → empty output. Trivial but pinned.
    #[test]
    fn empty_input_is_empty_output() {
        let proposals: Vec<PersonaProposal> = vec![];
        assert!(group_supersession_pairs(&proposals).is_empty());
    }

    /// Input order determines pair position. The pair
    /// emits at the position of its first-encountered
    /// half: with input `[A_link, B_unrelated, R_link]`
    /// the output is `[Linked, Unlinked]` and the linked
    /// pair takes the position of `A_link` (index 0).
    #[test]
    fn pair_emits_at_position_of_first_half() {
        let proposals = vec![
            append_list("A", Some("R")), // index 0 — first half
            append_list("X", None),       // index 1 — unrelated
            remove_list("R", Some("A")), // index 2 — second half
        ];
        let rendered = group_supersession_pairs(&proposals);
        assert_eq!(rendered.len(), 2);
        // Position 0: the pair (emitted at A's position).
        assert!(matches!(
            &rendered[0],
            ProposalRendering::Linked { .. }
        ));
        // Position 1: the unrelated proposal.
        match &rendered[1] {
            ProposalRendering::Unlinked(p) => {
                assert_eq!(p.id, "X");
            }
            other => panic!("expected Unlinked X, got {other:?}"),
        }
    }
}
