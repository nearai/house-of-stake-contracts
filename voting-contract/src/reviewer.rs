use crate::proposal::{MajorityType, ProposalFlow, ProposalInfo, ProposalStatus, SnapshotAndState};
use crate::*;
use common::global_state::{GlobalState, VGlobalState};
use common::{TimestampNs, events};
use near_sdk::{Gas, Promise, PromiseOrValue, assert_one_yocto};

pub const GAS_FOR_ON_GET_SNAPSHOT: Gas = Gas::from_tgas(30);

#[near]
impl Contract {
    /// Approves a proposal.
    /// Requires 1 yocto attached.
    /// Reviewers only.
    #[payable]
    pub fn approve_proposal(
        &mut self,
        proposal_id: ProposalId,
        majority_type: Option<MajorityType>,
    ) -> PromiseOrValue<Option<ProposalInfo>> {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        self.internal_advance_queue();

        let mut proposal = self.internal_expect_proposal_updated(proposal_id);
        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
        }
        if proposal.flow == ProposalFlow::FastTrack && majority_type.is_none() {
            env::panic_str("FastTrack proposals require a majority_type");
        }

        events::emit::approve_proposal_action(&env::predecessor_account_id(), proposal_id);

        proposal.reviewer_id = Some(env::predecessor_account_id());
        proposal.approval_time_ns = Some(env::block_timestamp().into());
        proposal.quorum_threshold_bps = self.config.quorum_threshold_bps;
        proposal.quorum_floor = self.config.quorum_floor;

        if proposal.bond_amount > NearToken::ZERO {
            Promise::new(self.config.treasury_account_id.clone())
                .transfer(proposal.bond_amount)
                .detach();
            proposal.bond_amount = NearToken::ZERO;
        }
        match proposal.flow {
            ProposalFlow::Classic => {
                proposal.approval_threshold_bps = self.config.approval_threshold_bps;
            }
            ProposalFlow::FastTrack => {
                proposal.approval_threshold_bps = match majority_type.unwrap() {
                    MajorityType::Simple => self.config.simple_majority_threshold_bps,
                    MajorityType::Strong => self.config.strong_majority_threshold_bps,
                };
                proposal.sandbox_duration_ns = self.config.sandbox_duration_ns;
                proposal.sandbox_threshold_bps = self.config.sandbox_threshold_bps;
            }
        }

        if self.active_proposals.len() < self.config.max_active_proposals {
            proposal.activate(env::block_timestamp().into());
            self.internal_set_proposal(proposal);
            PromiseOrValue::Promise(
                ext_venear::ext(self.config.venear_account_id.clone())
                    .with_unused_gas_weight(1)
                    .get_snapshot()
                    .then(
                        ext_self::ext(env::current_account_id())
                            .with_static_gas(GAS_FOR_ON_GET_SNAPSHOT)
                            .on_get_snapshot(proposal_id),
                    ),
            )
        } else {
            proposal.status = ProposalStatus::Queued;
            self.internal_set_proposal(proposal);
            self.pending_queue.push(proposal_id);
            PromiseOrValue::Value(self.get_proposal(proposal_id))
        }
    }

    /// Rejects the proposal before it has been approved.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the reviewers while the proposal is in the Created status.
    #[payable]
    pub fn reject_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal can only be rejected while in the Created status");
        }

        proposal.reviewer_id = Some(env::predecessor_account_id());
        proposal.status = ProposalStatus::Rejected;

        events::emit::reject_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
    }

    /// Vetoes a proposal.
    /// * Classic flow: only valid during the Timelock period.
    /// * FastTrack flow: valid while the proposal is Scheduled or Voting.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the council members.
    #[payable]
    pub fn veto_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_council();
        self.internal_advance_queue();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        match (proposal.flow, proposal.status) {
            (ProposalFlow::Classic, ProposalStatus::Timelock) => {}
            (ProposalFlow::FastTrack, ProposalStatus::Voting | ProposalStatus::Scheduled) => {}
            (_, _) => env::panic_str("Proposal cannot be vetoed in its current state"),
        }

        proposal.rejecter_id = Some(env::predecessor_account_id());
        proposal.status = ProposalStatus::Vetoed;

        events::emit::veto_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
        self.internal_advance_queue();
    }

    /// Waives the veto right during the timelock period, ending the timelock immediately so the
    /// proposal can advance to the next step.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the council members.
    #[payable]
    pub fn noveto_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_council();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Timelock {
            env::panic_str("Proposal can only be noveto'd during the timelock period");
        }

        proposal.status = if proposal.has_actions() {
            ProposalStatus::Executable
        } else {
            ProposalStatus::Succeeded
        };

        events::emit::noveto_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
        self.internal_advance_queue();
    }

    /// Slashes a proposal that is still in Created status.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the reviewers.
    #[payable]
    pub fn slash_proposal(&mut self, proposal_id: ProposalId) -> PromiseOrValue<()> {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.flow == ProposalFlow::FastTrack,
            "Only FastTrack proposals can be slashed"
        );
        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal can only be slashed while in Created status");
        }

        proposal.status = ProposalStatus::Slashed;

        events::emit::slash_proposal_action(&env::predecessor_account_id(), proposal_id);

        let result = if proposal.bond_amount > NearToken::ZERO {
            let promise = Promise::new(self.config.treasury_account_id.clone())
                .transfer(proposal.bond_amount);
            proposal.bond_amount = NearToken::ZERO;
            PromiseOrValue::Promise(promise)
        } else {
            PromiseOrValue::Value(())
        };

        self.internal_set_proposal(proposal);
        result
    }

    /// Callback that stores the fetched veNEAR snapshot.
    #[private]
    pub fn on_get_snapshot(
        &mut self,
        #[callback] snapshot_and_state: (MerkleTreeSnapshot, VGlobalState),
        proposal_id: ProposalId,
    ) -> ProposalInfo {
        self.assert_not_paused();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Sandbox && proposal.status != ProposalStatus::Voting {
            env::panic_str("Proposal must be in Sandbox or Voting status");
        }
        if proposal.snapshot_and_state.is_some() {
            env::panic_str("Snapshot is already set for this proposal");
        }

        let timestamp: TimestampNs = env::block_timestamp().into();
        let mut global_state: GlobalState = snapshot_and_state.1.into();
        global_state.update(timestamp);
        proposal.snapshot_and_state = Some(SnapshotAndState {
            snapshot: snapshot_and_state.0,
            timestamp_ns: timestamp,
            total_venear: global_state.total_venear_balance.total(),
            venear_growth_config: global_state.venear_growth_config,
        });

        self.internal_set_proposal(proposal);
        self.get_proposal(proposal_id).unwrap()
    }
}

impl Contract {
    pub fn assert_called_by_reviewer(&self) {
        require!(
            self.config
                .reviewer_ids
                .contains(&env::predecessor_account_id()),
            "Only the reviewers can call this method"
        );
    }

    pub fn assert_called_by_council(&self) {
        require!(
            self.config
                .council_ids
                .contains(&env::predecessor_account_id()),
            "Only the council can call this method"
        );
    }
}

#[cfg(test)]
mod tests {
    //! Reviewer/council proposal transitions across allowed/disallowed statuses and access guards.
    use super::*;
    use crate::proposal::ProposalFlow;
    use crate::test_utils::*;
    use near_sdk::json_types::U64;

    // approve_proposal

    #[test]
    #[should_panic(expected = "Only the reviewers can call this method")]
    fn approve_requires_reviewer() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(acc("rando.test.near"), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
    }

    #[test]
    #[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
    fn approve_requires_one_yocto() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(reviewer(), 0, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
    }

    #[test]
    #[should_panic(expected = "FastTrack proposals require a majority_type")]
    fn approve_fasttrack_without_majority_type_panics() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
    }

    #[test]
    #[should_panic(expected = "Proposal is not in the Created status")]
    fn approve_twice_panics_when_already_active() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, None);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
    }

    #[test]
    fn approve_classic_applies_configured_thresholds() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, None);
        let cfg = default_config();
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(p.quorum_threshold_bps, cfg.quorum_threshold_bps);
        assert_eq!(p.quorum_floor, cfg.quorum_floor);
        assert_eq!(p.approval_threshold_bps, cfg.approval_threshold_bps);
        assert_eq!(p.reviewer_id.as_ref(), Some(&reviewer()));
        assert_eq!(p.approval_time_ns, Some(U64(TEST_NOW_NS)));
    }

    #[test]
    fn approve_fasttrack_simple_majority_uses_simple_threshold() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, Some(MajorityType::Simple));
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(
            p.approval_threshold_bps,
            default_config().simple_majority_threshold_bps
        );
    }

    #[test]
    fn approve_fasttrack_strong_majority_uses_strong_threshold() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, Some(MajorityType::Strong));
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(
            p.approval_threshold_bps,
            default_config().strong_majority_threshold_bps
        );
    }

    #[test]
    fn approve_fasttrack_zeroes_bond_amount() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, Some(MajorityType::Simple));
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(p.bond_amount, NearToken::ZERO);
    }

    #[test]
    fn approve_queues_when_active_slots_full() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let _first = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, _first, None);
        let second = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, second, None);

        assert_eq!(
            contract.get_proposal(second).unwrap().proposal.status,
            ProposalStatus::Queued
        );
        let queue = contract.get_queue_state();
        assert_eq!(queue.pending_queue, vec![second]);
    }

    // reject_proposal

    #[test]
    fn reject_moves_proposal_to_rejected() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(p.status, ProposalStatus::Rejected);
        assert_eq!(p.reviewer_id.as_ref(), Some(&reviewer()));
    }

    #[test]
    #[should_panic(expected = "Only the reviewers can call this method")]
    fn reject_requires_reviewer() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(acc("rando.test.near"), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Proposal can only be rejected while in the Created status")]
    fn reject_panics_for_already_approved_proposal() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, None);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
    }

    // veto_proposal

    #[test]
    fn veto_classic_during_timelock_succeeds() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        // Read at voting_end so update flips Voting -> Timelock.
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(voter(), 0, voting_end);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Timelock
        );

        set_ctx(council(), 1, voting_end);
        contract.veto_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Vetoed
        );
    }

    #[test]
    #[should_panic(expected = "Proposal cannot be vetoed in its current state")]
    fn veto_fasttrack_during_sandbox_panics() {
        // FastTrack veto is allowed only during Scheduled or Voting, not Sandbox.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        set_ctx(council(), 1, TEST_NOW_NS);
        contract.veto_proposal(id);
    }

    #[test]
    fn veto_fasttrack_during_scheduled_succeeds() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Scheduled
        );

        set_ctx(council(), 1, TEST_NOW_NS);
        contract.veto_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Vetoed
        );
    }

    #[test]
    #[should_panic(expected = "Only the council can call this method")]
    fn veto_requires_council() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.veto_proposal(id);
    }

    #[test]
    fn veto_fasttrack_during_voting_succeeds() {
        // Advance to next-Monday voting_start so the proposal is in Voting, then veto.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_start = crate::proposal::next_voting_start_ns(TEST_NOW_NS);
        assert_eq!(
            {
                set_ctx(voter(), 0, voting_start);
                contract.get_proposal(id).unwrap().proposal.status
            },
            ProposalStatus::Voting
        );

        set_ctx(council(), 1, voting_start);
        contract.veto_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Vetoed
        );
    }

    #[test]
    #[should_panic(expected = "Proposal cannot be vetoed in its current state")]
    fn veto_classic_during_voting_panics() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        set_ctx(council(), 1, TEST_NOW_NS);
        contract.veto_proposal(id);
    }

    // noveto_proposal

    #[test]
    fn noveto_signaling_only_proposal_succeeds_immediately() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(voter(), 0, voting_end);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Timelock
        );

        set_ctx(council(), 1, voting_end);
        contract.noveto_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn noveto_with_actions_moves_to_executable() {
        // With actions, noveto transitions Timelock -> Executable, not -> Succeeded.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        set_ctx(
            proposer(),
            NearToken::from_near(200).as_yoctonear(),
            TEST_NOW_NS,
        );
        let id = contract.create_proposal(
            crate::metadata::ProposalMetadata {
                title: Some("noveto-actions".to_string()),
                description: None,
                link: None,
            },
            Some(vec![crate::proposal::ProposalAction::Transfer {
                receiver_id: acc("dest.test.near"),
                amount: NearToken::from_near(1),
            }]),
            ProposalFlow::Classic,
        );
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
        near_sdk::testing_env!(
            VMContextBuilder::new()
                .current_account_id(current_account())
                .predecessor_account_id(current_account())
                .attached_deposit(NearToken::from_yoctonear(0))
                .block_timestamp(TEST_NOW_NS)
                .build()
        );
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(voter(), 0, voting_end);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Timelock
        );

        set_ctx(council(), 1, voting_end);
        contract.noveto_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Executable
        );
    }

    #[test]
    #[should_panic(expected = "Proposal can only be noveto'd during the timelock period")]
    fn noveto_outside_timelock_panics() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        set_ctx(council(), 1, TEST_NOW_NS);
        contract.noveto_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Only the council can call this method")]
    fn noveto_requires_council() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        set_ctx(reviewer(), 1, voting_end);
        contract.noveto_proposal(id);
    }

    // slash_proposal

    #[test]
    fn slash_fasttrack_with_zero_bond_returns_value_variant() {
        // Zero bond_amount makes slash return PromiseOrValue::Value(()), not a transfer promise.
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_bond_amount(NearToken::ZERO);

        // Bond is zero, so creation no longer demands it.
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);

        set_ctx(reviewer(), 1, TEST_NOW_NS);
        match contract.slash_proposal(id) {
            PromiseOrValue::Value(()) => {}
            PromiseOrValue::Promise(_) => panic!("expected Value variant for zero-bond slash"),
        }
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Slashed
        );
    }

    #[test]
    fn slash_fasttrack_in_created_moves_to_slashed() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.slash_proposal(id);
        let p = contract.get_proposal(id).unwrap().proposal;
        assert_eq!(p.status, ProposalStatus::Slashed);
        // bond was forfeited to treasury, contract drops it from the proposal.
        assert_eq!(p.bond_amount, NearToken::ZERO);
    }

    #[test]
    #[should_panic(expected = "Only FastTrack proposals can be slashed")]
    fn slash_rejects_classic_flow() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.slash_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Proposal can only be slashed while in Created status")]
    fn slash_rejects_already_approved_fasttrack() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, None);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.slash_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Only the reviewers can call this method")]
    fn slash_requires_reviewer() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(council(), 1, TEST_NOW_NS);
        let _ = contract.slash_proposal(id);
    }

    // on_get_snapshot — wrong-status panic, distinct from votes::tests coverage.

    #[test]
    #[should_panic(expected = "Proposal must be in Sandbox or Voting status")]
    fn on_get_snapshot_panics_for_created_proposal() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        near_sdk::testing_env!(
            VMContextBuilder::new()
                .current_account_id(current_account())
                .predecessor_account_id(current_account())
                .attached_deposit(NearToken::from_yoctonear(0))
                .block_timestamp(TEST_NOW_NS)
                .build()
        );
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
    }

    // Pause guards: assert_one_yocto() runs before assert_not_paused(), so attach 1 yocto.

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn approve_panics_when_paused() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        contract.paused = true;
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn reject_panics_when_paused() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        contract.paused = true;
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn veto_panics_when_paused() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        contract.paused = true;
        set_ctx(council(), 1, TEST_NOW_NS);
        contract.veto_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn noveto_panics_when_paused() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        contract.paused = true;
        set_ctx(council(), 1, TEST_NOW_NS);
        contract.noveto_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn slash_panics_when_paused() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        contract.paused = true;
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.slash_proposal(id);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn on_get_snapshot_panics_when_paused() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
        contract.paused = true;
        near_sdk::testing_env!(
            VMContextBuilder::new()
                .current_account_id(current_account())
                .predecessor_account_id(current_account())
                .attached_deposit(NearToken::from_yoctonear(0))
                .block_timestamp(TEST_NOW_NS)
                .build()
        );
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
    }
}
