use crate::proposal::{Proposal, is_active_status};
use crate::*;
use std::collections::{HashMap, VecDeque};

pub use common::voting::QueueState;

struct QueueAdvanceOutcome {
    active_updates: Vec<Proposal>,
    queue_promotions: Vec<Proposal>,
}

#[near]
impl Contract {
    pub fn advance_queue(&mut self) {
        self.assert_not_paused();
        self.internal_advance_queue();
    }

    /// Returns the active proposal ids and the pending-queue ids (front first).
    pub fn get_queue_state(&self) -> QueueState {
        let outcome = self.compute_queue_advance();

        let exiting: Vec<ProposalId> = outcome
            .active_updates
            .iter()
            .filter(|p| !is_active_status(p.status))
            .map(|p| p.id)
            .collect();

        let mut active_proposals: Vec<ProposalId> = self
            .active_proposals
            .iter()
            .copied()
            .filter(|id| !exiting.contains(id))
            .collect();
        active_proposals.extend(
            outcome
                .queue_promotions
                .iter()
                .filter(|p| is_active_status(p.status))
                .map(|p| p.id),
        );

        let pending_queue: Vec<ProposalId> = self
            .pending_queue
            .iter()
            .copied()
            .skip(outcome.queue_promotions.len())
            .collect();

        QueueState {
            active_proposals,
            pending_queue,
        }
    }
}

impl Contract {
    fn compute_queue_advance(&self) -> QueueAdvanceOutcome {
        let now = env::block_timestamp();
        let mut active_updates = Vec::new();
        let mut queue_promotions = Vec::new();
        let mut virtual_active_count = 0u32;
        let mut freed_slot_times: VecDeque<u64> = VecDeque::new();

        for &id in self.active_proposals.iter() {
            let (proposal, updated) = self
                .internal_get_proposal(id)
                .expect("active proposal missing");
            if is_active_status(proposal.status) {
                virtual_active_count += 1;
            }
            if updated {
                if !is_active_status(proposal.status) {
                    let end_time = proposal.active_end_time_ns();
                    let pos = freed_slot_times
                        .iter()
                        .position(|&t| t >= end_time)
                        .unwrap_or(freed_slot_times.len());
                    freed_slot_times.insert(pos, end_time);
                }
                active_updates.push(proposal);
            }
        }

        for &id in self.pending_queue.iter() {
            if virtual_active_count >= self.config.max_active_proposals {
                break;
            }

            let mut proposal = self.internal_expect_proposal_updated(id);

            // Pre-existing empty slots start at `now`.
            let start_time = freed_slot_times.pop_front().unwrap_or(now).into();

            proposal.activate(start_time);
            proposal.update(now.into());

            if is_active_status(proposal.status) {
                virtual_active_count += 1;
            } else {
                let end = proposal.active_end_time_ns();
                let pos = freed_slot_times
                    .iter()
                    .position(|&t| t >= end)
                    .unwrap_or(freed_slot_times.len());
                freed_slot_times.insert(pos, end);
            }

            queue_promotions.push(proposal);
        }

        QueueAdvanceOutcome {
            active_updates,
            queue_promotions,
        }
    }

    pub(crate) fn internal_advance_queue(&mut self) {
        let outcome = self.compute_queue_advance();

        self.pending_queue
            .drain(0..u32::try_from(outcome.queue_promotions.len()).unwrap());

        for proposal in outcome.queue_promotions {
            self.internal_set_proposal(proposal);
        }
        for proposal in outcome.active_updates {
            self.internal_set_proposal(proposal);
        }
    }

    pub(crate) fn get_proposals_virtual_updates(&self) -> HashMap<ProposalId, Proposal> {
        self.compute_queue_advance()
            .queue_promotions
            .into_iter()
            .map(|p| (p.id, p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    //! Contract-level checks for the queue surface. Every assertion that
    //! checks active-set membership also checks the corresponding stored
    //! status (via `assert_active_with_status` / `assert_queued_at`).
    use super::*;
    use crate::metadata::ProposalMetadata;
    use crate::proposal::ProposalStatus::*;
    use crate::proposal::{Proposal, ProposalFlow, ProposalStatus};
    use crate::test_utils::*;
    use common::voting::VoteOption;
    use near_sdk::json_types::U64;

    // ---------------------------------------------------------------
    // Basics
    // ---------------------------------------------------------------

    #[test]
    fn get_queue_state_on_empty_contract_returns_empty() {
        let contract = fresh_contract();
        let state = contract.get_queue_state();
        assert!(state.active_proposals.is_empty());
        assert!(state.pending_queue.is_empty());
    }

    #[test]
    fn advance_queue_is_a_noop_when_nothing_is_pending() {
        let mut contract = fresh_contract();
        set_ctx(proposer(), 0, TEST_NOW_NS);
        contract.advance_queue();
        let state = contract.get_queue_state();
        assert!(state.active_proposals.is_empty());
        assert!(state.pending_queue.is_empty());
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn advance_queue_when_paused_panics() {
        let mut contract = fresh_contract();
        contract.paused = true;
        set_ctx(proposer(), 0, TEST_NOW_NS);
        contract.advance_queue();
    }

    #[test]
    fn approval_with_full_slots_appends_to_pending_queue() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let first = create_proposal(&mut contract, ProposalFlow::Classic);
        let second = create_proposal(&mut contract, ProposalFlow::Classic);
        let third = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, first, None);
        approve_proposal(&mut contract, second, None);
        approve_proposal(&mut contract, third, None);

        assert_active_with_status(&contract, first, ProposalStatus::Voting);
        assert_queued_at(&contract, second, 0);
        assert_queued_at(&contract, third, 1);
    }

    #[test]
    fn active_proposal_drops_off_queue_state_once_voting_ends() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(100))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        assert_active_with_status(&contract, id, ProposalStatus::Voting);

        // Advance past voting_end without quorum -> Defeated, slot frees.
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(voter(), 0, voting_end);
        let state = contract.get_queue_state();
        assert!(!state.active_proposals.contains(&id));
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn get_queue_state_promotes_queued_proposal_virtually_when_slot_frees() {
        // Set max_active = 1, approve two proposals (second is Queued), then
        // advance past the first proposal's voting_end. get_queue_state must
        // virtually promote the second proposal even before advance_queue
        // commits the change to storage.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(100))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let first = create_proposal(&mut contract, ProposalFlow::Classic);
        let second = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, first, Some(&fixture));
        approve_proposal(&mut contract, second, None);
        assert_active_with_status(&contract, first, ProposalStatus::Voting);
        assert_queued_at(&contract, second, 0);

        // Advance to first proposal's voting_end -> Defeated, slot freed.
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(voter(), 0, voting_end);
        // get_proposal() runs the virtual update too — verify the promoted
        // proposal's STATUS is Voting, not just its active-set membership.
        assert_eq!(
            contract.get_proposal(first).unwrap().proposal.status,
            ProposalStatus::Defeated
        );
        assert_eq!(
            contract.get_proposal(second).unwrap().proposal.status,
            ProposalStatus::Voting
        );
        let state = contract.get_queue_state();
        assert!(!state.active_proposals.contains(&first));
        assert!(state.active_proposals.contains(&second));
        assert!(state.pending_queue.is_empty());
    }

    #[test]
    fn advance_queue_commits_virtual_promotion_to_storage() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(100))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);
        let first = create_proposal(&mut contract, ProposalFlow::Classic);
        let second = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, first, Some(&fixture));
        approve_proposal(&mut contract, second, None);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(proposer(), 0, voting_end);
        contract.advance_queue();

        // After commit, read RAW stored state (bypassing the implicit update
        // path) to verify the promotion landed in storage with the correct
        // status, not just as a transient virtual override.
        let raw_first: Proposal = contract.proposals.get(first).cloned().unwrap().into();
        let raw_second: Proposal = contract.proposals.get(second).cloned().unwrap().into();
        assert_eq!(raw_first.status, ProposalStatus::Defeated);
        assert_eq!(raw_second.status, ProposalStatus::Voting);
        // Queue promotion must NOT auto-fetch a snapshot; the first voter
        // through the door is responsible for calling take_snapshot_and_vote.
        assert!(raw_second.snapshot_and_state.is_none());
        assert!(!contract.active_proposals.contains(&first));
        assert!(contract.active_proposals.contains(&second));
        assert!(contract.pending_queue.is_empty());
    }

    // ---------------------------------------------------------------
    // Default cap
    // ---------------------------------------------------------------

    #[test]
    fn default_cap_allows_three_concurrent_active_proposals() {
        // Default config sets max_active_proposals = 3, so three back-to-back
        // approvals must all enter the active set with status Voting.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        assert_eq!(contract.get_config().max_active_proposals, 3);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, Some(&fixture));
        approve_proposal(&mut contract, c, Some(&fixture));

        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert_active_with_status(&contract, c, ProposalStatus::Voting);
        assert!(contract.get_queue_state().pending_queue.is_empty());
    }

    // ---------------------------------------------------------------
    // Backdating
    // ---------------------------------------------------------------

    #[test]
    fn promoted_proposal_backdates_voting_start_to_freed_slot_end_time() {
        // max_active = 1 isolates the test from default-cap noise:
        // exactly one slot frees, exactly one promotion happens.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, None);
        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_queued_at(&contract, b, 0);

        let cfg = default_config();
        let a_voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        // Advance one minute past A's voting_end so the slot is unambiguously
        // free, but B's hypothetical voting window (a_voting_end + duration)
        // is still in the future.
        let advance_to = a_voting_end + 60 * 1_000_000_000;
        set_ctx(proposer(), 0, advance_to);
        contract.advance_queue();

        let b_raw: Proposal = contract.proposals.get(b).cloned().unwrap().into();
        assert_eq!(
            b_raw.voting_start_time_ns,
            Some(U64(a_voting_end)),
            "promoted proposal must backdate start to the freed slot's end_time, not to `now`"
        );
        assert_eq!(b_raw.status, ProposalStatus::Voting);
        assert!(
            a_voting_end + cfg.classic_voting_duration_ns.0 > advance_to,
            "fixture invariant: B's voting_end must remain in the future"
        );
    }

    #[test]
    fn backdating_cascade_through_already_stale_windows() {
        // Setup: max_active = 1, queue B and C after A. Advance to 2.5 ×
        // voting_duration past creation. A's window [0, dur] is past
        // (Defeated). B inherits start = dur, B's window [dur, 2*dur] is
        // also past (also Defeated). C inherits start = 2*dur, C's window
        // [2*dur, 3*dur] straddles `now` -> Voting.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let voting_duration = default_config().classic_voting_duration_ns.0;
        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, None);
        approve_proposal(&mut contract, c, None);
        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_queued_at(&contract, b, 0);
        assert_queued_at(&contract, c, 1);

        let advance_to = TEST_NOW_NS + voting_duration * 5 / 2;
        set_ctx(proposer(), 0, advance_to);
        contract.advance_queue();

        let a_raw: Proposal = contract.proposals.get(a).cloned().unwrap().into();
        let b_raw: Proposal = contract.proposals.get(b).cloned().unwrap().into();
        let c_raw: Proposal = contract.proposals.get(c).cloned().unwrap().into();

        assert_eq!(a_raw.status, ProposalStatus::Defeated);
        assert_eq!(b_raw.status, ProposalStatus::Defeated);
        assert_eq!(c_raw.status, ProposalStatus::Voting);

        assert_eq!(
            b_raw.voting_start_time_ns,
            Some(U64(TEST_NOW_NS + voting_duration))
        );
        assert_eq!(
            c_raw.voting_start_time_ns,
            Some(U64(TEST_NOW_NS + 2 * voting_duration))
        );

        // Pending queue is fully drained.
        assert!(contract.pending_queue.is_empty());
        // Only C is active.
        assert!(!contract.active_proposals.contains(&a));
        assert!(!contract.active_proposals.contains(&b));
        assert!(contract.active_proposals.contains(&c));
    }

    // ---------------------------------------------------------------
    // FIFO and partial promotion
    // ---------------------------------------------------------------

    #[test]
    fn slot_frees_one_at_a_time_promotes_head_only() {
        // max_active = 2: A and B Voting. Stagger their creation so only one
        // slot frees first. The pending queue head must promote, the tail
        // stays queued.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(2);

        let cfg = default_config();

        let a = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, a, Some(&fixture));
        // Stagger by 1 hour so B's voting_end is strictly later than A's, with
        // enough margin to land `between` cleanly inside [a_end, b_end].
        let stagger_ns: u64 = 3_600 * 1_000_000_000;
        let b_creation = TEST_NOW_NS + stagger_ns;
        set_ctx(
            proposer(),
            NearToken::from_near(100).as_yoctonear(),
            b_creation,
        );
        let b = contract.create_proposal(
            ProposalMetadata {
                title: Some("b".to_string()),
                description: None,
                link: None,
            },
            None,
            ProposalFlow::Classic,
        );
        set_ctx(reviewer(), 1, b_creation);
        let _ = contract.approve_proposal(b, None);
        set_ctx(current_account(), 0, b_creation);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), b);

        let c = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, c, None);
        let d = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, d, None);

        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert_queued_at(&contract, c, 0);
        assert_queued_at(&contract, d, 1);

        // Advance to a point where ONLY A's voting_end has passed but B's
        // hasn't. A is Defeated, slot frees, exactly one queued promotes.
        let a_voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        let between = a_voting_end + 60 * 1_000_000_000;
        let b_voting_end = b_creation + cfg.classic_voting_duration_ns.0;
        assert!(
            between < b_voting_end,
            "fixture invariant broken: between={} b_end={}",
            between,
            b_voting_end
        );

        set_ctx(proposer(), 0, between);
        contract.advance_queue();

        assert_eq!(
            contract.get_proposal(a).unwrap().proposal.status,
            ProposalStatus::Defeated
        );
        assert_eq!(
            contract.get_proposal(b).unwrap().proposal.status,
            ProposalStatus::Voting
        );
        assert_eq!(
            contract.get_proposal(c).unwrap().proposal.status,
            ProposalStatus::Voting
        );
        // D stays queued and shifts to head.
        assert_queued_at(&contract, d, 0);
    }

    // ---------------------------------------------------------------
    // Mixed flow types
    // ---------------------------------------------------------------

    #[test]
    fn mixed_classic_and_fasttrack_share_active_slots() {
        // Default cap = 3, mixed flows: one Classic Voting, one FastTrack
        // Sandbox, one FastTrack-with-threshold-met -> Scheduled.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        let classic = create_proposal(&mut contract, ProposalFlow::Classic);
        let sandbox_id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        let scheduled_id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, classic, Some(&fixture));
        approve_proposal(&mut contract, sandbox_id, Some(&fixture));
        approve_proposal(&mut contract, scheduled_id, Some(&fixture));
        // Cast a For vote on `scheduled_id` to clear the sandbox threshold
        // and graduate it to Scheduled. The 400 NEAR weight clears 10%.
        let (proof, v_account) = fixture.proof_for(&for_voter());
        set_ctx(
            for_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(scheduled_id, VoteOption::For, proof, v_account);

        assert_active_with_status(&contract, classic, ProposalStatus::Voting);
        assert_active_with_status(&contract, sandbox_id, ProposalStatus::Sandbox);
        assert_active_with_status(&contract, scheduled_id, ProposalStatus::Scheduled);
        assert_eq!(contract.get_queue_state().active_proposals.len(), 3);
    }

    // ---------------------------------------------------------------
    // max_active_proposals changes
    // ---------------------------------------------------------------

    #[test]
    fn reducing_max_active_does_not_evict_existing_active_proposals() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, Some(&fixture));
        approve_proposal(&mut contract, c, Some(&fixture));

        // Shrink cap below current load.
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert_active_with_status(&contract, c, ProposalStatus::Voting);

        // Subsequent approvals must queue, since virtual_active_count = 3 >= cap = 1.
        let d = create_proposal(&mut contract, ProposalFlow::Classic);
        let e = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, d, None);
        approve_proposal(&mut contract, e, None);
        assert_queued_at(&contract, d, 0);
        assert_queued_at(&contract, e, 1);

        // All three actives time out as Defeated, freeing 3 slots; cap = 1
        // means only one queued proposal promotes. The other stays at pos 0.
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(proposer(), 0, voting_end);
        contract.advance_queue();

        assert_active_with_status(&contract, d, ProposalStatus::Voting);
        assert_queued_at(&contract, e, 0);
        assert_eq!(contract.active_proposals.len(), 1);
    }

    #[test]
    fn lifting_max_active_promotes_multiple_queued_at_once() {
        // max_active = 1, A active, B/C/D queued. Lift cap to 3 -> B and C
        // promote (filling the 2 newly-available slots), D stays queued.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        let d = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, None);
        approve_proposal(&mut contract, c, None);
        approve_proposal(&mut contract, d, None);
        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_queued_at(&contract, b, 0);
        assert_queued_at(&contract, c, 1);
        assert_queued_at(&contract, d, 2);

        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(3);

        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert_active_with_status(&contract, c, ProposalStatus::Voting);
        assert_queued_at(&contract, d, 0);
    }

    // ---------------------------------------------------------------
    // Slot-freeing exits: veto / noveto / sandbox-timeout
    // ---------------------------------------------------------------

    #[test]
    fn fasttrack_sandbox_timeout_frees_slot_for_queued_promotion() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        // 50 NEAR For-power is below the 10% sandbox threshold of a 1 000-NEAR
        // supply, so A never graduates and times out as Defeated.
        let a_fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(50))],
            NearToken::from_near(1_000),
        );
        let a = create_proposal(&mut contract, ProposalFlow::FastTrack);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&a_fixture));
        approve_proposal(&mut contract, b, None);
        assert_active_with_status(&contract, a, ProposalStatus::Sandbox);
        assert_queued_at(&contract, b, 0);

        // Sandbox window expires.
        let sandbox_end = TEST_NOW_NS + default_config().sandbox_duration_ns.0;
        set_ctx(proposer(), 0, sandbox_end);
        contract.advance_queue();

        assert_eq!(
            contract.get_proposal(a).unwrap().proposal.status,
            ProposalStatus::Defeated
        );
        // B has been promoted with backdated start = a.sandbox_end.
        let b_raw: Proposal = contract.proposals.get(b).cloned().unwrap().into();
        assert_eq!(b_raw.voting_start_time_ns, Some(U64(sandbox_end)));
        // sandbox_end + classic_voting_duration is in the future of `now`, so B is Voting.
        assert_eq!(b_raw.status, ProposalStatus::Voting);
    }

    #[test]
    fn council_veto_during_timelock_frees_slot_for_queued_promotion() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, a, Some(&fixture));
        let (proof, v_account) = fixture.proof_for(&for_voter());
        set_ctx(
            for_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(a, VoteOption::For, proof, v_account);

        let b = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, b, None);
        assert_queued_at(&contract, b, 0);

        let cfg = default_config();
        let voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        // Move A into Timelock then veto it.
        assert_eq!(
            {
                set_ctx(voter(), 0, voting_end);
                contract.get_proposal(a).unwrap().proposal.status
            },
            ProposalStatus::Timelock
        );

        set_ctx(council(), 1, voting_end);
        contract.veto_proposal(a);

        // veto_proposal triggers internal_advance_queue, so B promotes.
        assert_eq!(
            contract.get_proposal(a).unwrap().proposal.status,
            ProposalStatus::Vetoed
        );
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert!(contract.pending_queue.is_empty());
    }

    #[test]
    fn council_noveto_during_timelock_frees_slot_for_queued_promotion() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, a, Some(&fixture));
        let (proof, v_account) = fixture.proof_for(&for_voter());
        set_ctx(
            for_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(a, VoteOption::For, proof, v_account);

        let b = create_proposal(&mut contract, ProposalFlow::Classic);

        approve_proposal(&mut contract, b, None);
        assert_queued_at(&contract, b, 0);

        let cfg = default_config();
        let voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        // Move A into Timelock then noveto it (slot would otherwise stay held
        // until voting_end + timelock_duration).
        assert_eq!(
            {
                set_ctx(voter(), 0, voting_end);
                contract.get_proposal(a).unwrap().proposal.status
            },
            ProposalStatus::Timelock
        );

        set_ctx(council(), 1, voting_end);
        contract.noveto_proposal(a);

        // noveto_proposal triggers internal_advance_queue, so B promotes.
        // A has no actions, so its terminal status is Succeeded.
        assert_eq!(
            contract.get_proposal(a).unwrap().proposal.status,
            ProposalStatus::Succeeded
        );
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert!(contract.pending_queue.is_empty());
    }

    // ---------------------------------------------------------------
    // Lifecycle walkthrough
    // ---------------------------------------------------------------

    #[test]
    fn virtual_queue_walkthrough_across_all_lifecycle_paths() {
        // Single walkthrough of slot-freeing across THREE different active-proposal
        // lifecycle paths, with seven queued proposals waiting to take freed slots.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        // Default cap is 3 — exactly room for A, B, C.
        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        cast_vote_at(
            &mut contract,
            &fixture,
            for_voter(),
            a,
            VoteOption::For,
            TEST_NOW_NS,
        );

        let b = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, b, Some(&fixture));

        let c = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, c, Some(&fixture));
        cast_vote_at(
            &mut contract,
            &fixture,
            for_voter(),
            c,
            VoteOption::For,
            TEST_NOW_NS,
        );

        let mut queued = Vec::with_capacity(7);
        for i in 0..7 {
            let flow = if i % 2 == 0 {
                ProposalFlow::Classic
            } else {
                ProposalFlow::FastTrack
            };
            let id = create_proposal(&mut contract, flow);
            approve_proposal(&mut contract, id, None);
            queued.push(id);
        }
        let cfg = default_config();
        let t_c_voting_start = crate::proposal::next_voting_start_ns(TEST_NOW_NS);
        let t_b_sandbox_end = TEST_NOW_NS + cfg.sandbox_duration_ns.0;
        let t_c_voting_end = t_c_voting_start + cfg.fast_track_voting_duration_ns.0;
        let t_a_voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        let t_a_timelock_end = t_a_voting_end + cfg.timelock_duration_ns.0;
        // Pin the only non-derived fact: TEST_NOW_NS = 2026-06-01 00:00 UTC
        // = Monday 01:00 CET, so next_voting_start_ns rounds to the FOLLOWING
        // Monday 00:00 CET = 2026-06-08. All other times above are pure
        // arithmetic on `cfg.*_duration_ns` and TEST_NOW_NS, so re-asserting
        // them in any other form would be tautological with their definition.
        assert_eq!(t_c_voting_start, date_ns(2026, 6, 8));

        // T0 — A Voting, B Sandbox, C Scheduled (vote pushed past threshold).
        assert_eq!(
            all_statuses_at(&contract, TEST_NOW_NS),
            vec![
                Voting, Sandbox, Scheduled, Queued, Queued, Queued, Queued, Queued, Queued, Queued
            ],
        );

        // C Scheduled → Voting at next-Monday start (slot still held).
        assert_eq!(
            all_statuses_at(&contract, t_c_voting_start),
            vec![
                Voting, Sandbox, Voting, Queued, Queued, Queued, Queued, Queued, Queued, Queued
            ],
        );

        // CHECKPOINT 1 — B Defeated, Q1 (Classic) promoted into B's slot,
        // backdated to B's sandbox_end.
        assert_eq!(
            all_statuses_at(&contract, t_b_sandbox_end),
            vec![
                Voting, Defeated, Voting, Voting, Queued, Queued, Queued, Queued, Queued, Queued
            ],
        );
        assert_eq!(
            contract
                .get_proposal(queued[0])
                .unwrap()
                .proposal
                .voting_start_time_ns,
            Some(U64(t_b_sandbox_end)),
            "Q1 backdates voting_start to B's sandbox_end"
        );

        // CHECKPOINT 2 — C Succeeded, Q2 (FastTrack) promoted into C's slot,
        // backdated to C's voting_end.
        assert_eq!(
            all_statuses_at(&contract, t_c_voting_end),
            vec![
                Voting, Defeated, Succeeded, Voting, Sandbox, Queued, Queued, Queued, Queued,
                Queued
            ],
        );
        assert_eq!(
            contract
                .get_proposal(queued[1])
                .unwrap()
                .proposal
                .sandbox_start_time_ns,
            Some(U64(t_c_voting_end)),
            "Q2 backdates sandbox_start to C's voting_end"
        );

        // A Voting → Timelock (still active; slot held).
        assert_eq!(
            all_statuses_at(&contract, t_a_voting_end),
            vec![
                Timelock, Defeated, Succeeded, Voting, Sandbox, Queued, Queued, Queued, Queued,
                Queued
            ],
        );

        // CHECKPOINT 3 — A Succeeded.
        assert_eq!(
            all_statuses_at(&contract, t_a_timelock_end),
            vec![
                Succeeded, Defeated, Succeeded, Defeated, Defeated, Voting, Defeated, Voting,
                Sandbox, Queued
            ],
        );
    }

    // ---------------------------------------------------------------
    // Ordering invariants
    // ---------------------------------------------------------------

    #[test]
    fn active_proposals_iterate_in_approval_order() {
        // Sequential approvals at default cap: get_queue_state must list
        // active_proposals in the order they were approved.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
                VoterSpec::new(against_voter(), NearToken::from_near(100)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, Some(&fixture));
        approve_proposal(&mut contract, c, Some(&fixture));

        let state = contract.get_queue_state();
        assert_eq!(
            state.active_proposals,
            vec![a, b, c],
            "active_proposals must iterate in approval order"
        );
    }

    #[test]
    fn active_proposals_ordering_is_set_membership_only_after_removals() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
                VoterSpec::new(against_voter(), NearToken::from_near(100)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(2);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        let d = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, Some(&fixture));
        approve_proposal(&mut contract, c, None);
        approve_proposal(&mut contract, d, None);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0 + 1;
        set_ctx(voter(), 0, voting_end);
        let mut before_active = contract.get_queue_state().active_proposals;
        before_active.sort();

        set_ctx(proposer(), 0, voting_end);
        contract.advance_queue();

        let mut after_active = contract.get_queue_state().active_proposals;
        after_active.sort();

        let mut expected = vec![c, d];
        expected.sort();
        assert_eq!(before_active, expected, "virtual must promote C and D");
        assert_eq!(after_active, expected, "committed must contain C and D");
        let _ = a;
        let _ = b;
    }

    // ---------------------------------------------------------------
    // Virtual-vs-committed equivalence
    // ---------------------------------------------------------------

    // Each test builds two contracts: a read-only one (virtual path) and
    // one that commits via `advance_queue`. Both views are compared on
    // active-set membership, pending-queue order, and per-proposal status.

    #[test]
    fn virtual_matches_committed_in_backdating_cascade() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
                VoterSpec::new(against_voter(), NearToken::from_near(100)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );

        let virtual_c = {
            let mut c = fresh_contract();
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            c
        };
        let mut committed_c = {
            let mut c = fresh_contract();
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, None);
            c
        };

        let voting_duration = default_config().classic_voting_duration_ns.0;
        let advance_to = TEST_NOW_NS + voting_duration * 5 / 2;

        set_ctx(voter(), 0, advance_to);
        let v_state = virtual_c.get_queue_state();
        let v_statuses: Vec<_> = (0..virtual_c.get_num_proposals())
            .map(|i| virtual_c.get_proposal(i).unwrap().proposal.status)
            .collect();

        set_ctx(proposer(), 0, advance_to);
        committed_c.advance_queue();
        set_ctx(voter(), 0, advance_to);
        let c_state = committed_c.get_queue_state();
        let c_statuses: Vec<_> = (0..committed_c.get_num_proposals())
            .map(|i| committed_c.get_proposal(i).unwrap().proposal.status)
            .collect();

        let mut v_active = v_state.active_proposals.clone();
        let mut c_active = c_state.active_proposals.clone();
        v_active.sort();
        c_active.sort();
        assert_eq!(v_active, c_active);
        assert_eq!(v_state.pending_queue, c_state.pending_queue);
        assert_eq!(v_statuses, c_statuses);
    }

    #[test]
    fn virtual_matches_committed_when_active_proposals_transition_without_queue_promotion() {
        // Three active proposals at default cap; no queue. Advance past the
        // voting_end so all three transition Voting -> Defeated via
        // `active_updates` (not queue_promotions). The invariant must hold
        // for the active_updates branch too.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
                VoterSpec::new(against_voter(), NearToken::from_near(100)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );

        let virtual_c = {
            let mut c = fresh_contract();
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            c
        };
        let mut committed_c = {
            let mut c = fresh_contract();
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            let id = create_proposal(&mut c, ProposalFlow::Classic);
            approve_proposal(&mut c, id, Some(&fixture));
            c
        };

        // No votes cast -> below quorum -> all three Defeated at voting_end.
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;

        set_ctx(voter(), 0, voting_end);
        let v_state = virtual_c.get_queue_state();
        let v_statuses: Vec<_> = (0..virtual_c.get_num_proposals())
            .map(|i| virtual_c.get_proposal(i).unwrap().proposal.status)
            .collect();

        set_ctx(proposer(), 0, voting_end);
        committed_c.advance_queue();
        set_ctx(voter(), 0, voting_end);
        let c_state = committed_c.get_queue_state();
        let c_statuses: Vec<_> = (0..committed_c.get_num_proposals())
            .map(|i| committed_c.get_proposal(i).unwrap().proposal.status)
            .collect();

        let mut v_active = v_state.active_proposals.clone();
        let mut c_active = c_state.active_proposals.clone();
        v_active.sort();
        c_active.sort();
        assert_eq!(v_active, c_active);
        assert_eq!(v_state.pending_queue, c_state.pending_queue);
        assert_eq!(v_statuses, c_statuses);
    }
}
