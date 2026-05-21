use crate::proposal::{
    Proposal, ProposalFlow, ProposalStatus, SnapshotAndState, VoteOption, next_voting_start_ns,
};
use crate::reviewer::GAS_FOR_ON_GET_SNAPSHOT;
use crate::*;
use common::{events, near_add, near_sub};
use near_sdk::{Gas, Promise};

const GAS_FOR_CHAINED_VOTE: Gas = Gas::from_tgas(50);

/// Vote inputs accepted by `take_snapshot_and_vote` to optionally cast a vote in the same call.
#[derive(Clone)]
#[near(serializers=[json])]
pub struct VotePayload {
    pub vote: VoteOption,
    pub merkle_proof: MerkleProof,
    pub v_account: VAccount,
}

#[near]
impl Contract {
    /// Cast a vote for the given proposal and the given voting option.
    /// The caller has to provide a merkle proof and the account state from the snapshot.
    /// The caller should match the account ID in the account state.
    /// Requires a deposit to cover the storage fee or at least 1 yoctoNEAR if changing the vote.
    #[payable]
    pub fn vote(
        &mut self,
        proposal_id: ProposalId,
        vote: VoteOption,
        merkle_proof: MerkleProof,
        v_account: VAccount,
    ) {
        self.assert_not_paused();
        self.internal_advance_queue();
        let attached_deposit = env::attached_deposit();
        require!(!attached_deposit.is_zero(), "Requires attached deposit");

        let mut proposal: Proposal = self.internal_expect_proposal_updated(proposal_id);

        match proposal.status {
            ProposalStatus::Voting => {}
            ProposalStatus::Sandbox => {
                if vote != VoteOption::For {
                    env::panic_str("Only 'For' votes are allowed during the sandbox period");
                }
            }
            _ => {
                env::panic_str("Proposal is not in the voting phase");
            }
        }

        require!(
            proposal.snapshot_and_state.is_some(),
            "Snapshot has not been taken yet — call take_snapshot_and_vote first"
        );

        {
            let SnapshotAndState { snapshot, .. } = proposal.snapshot_and_state.as_ref().unwrap();
            require!(
                merkle_proof.is_valid(snapshot.root.into(), snapshot.length, &v_account),
                "Invalid merkle proof"
            );
        }

        let timestamp_ns = proposal.snapshot_and_state.as_ref().unwrap().timestamp_ns;
        let account: Account = v_account.into();
        let account_id = &account.account_id;
        let predecessor_account_id = &env::predecessor_account_id();
        require!(
            account_id == predecessor_account_id
                || predecessor_account_id == &env::current_account_id(),
            "Account ID doesn't match the predecessor account ID or self-call."
        );
        let account_balance = account.total_balance(
            timestamp_ns,
            &proposal
                .snapshot_and_state
                .as_ref()
                .unwrap()
                .venear_growth_config,
        );
        require!(!account_balance.is_zero(), "Account has no veNEAR balance");

        let vote_index = u8::from(vote);
        let previous_vote = self.votes.get(&(account_id.clone(), proposal_id)).cloned();
        require!(
            previous_vote != Some(vote_index),
            "Already voted for the same option"
        );
        let mut storage_added = self.config.vote_storage_fee;
        if let Some(previous_vote) = previous_vote {
            proposal.votes[usize::from(previous_vote)].remove_vote(account_balance);
            proposal.total_votes.remove_vote(account_balance);
            storage_added = NearToken::from_yoctonear(0);

            events::emit::proposal_vote_action(
                "remove_vote",
                &account_id,
                proposal_id,
                previous_vote,
                &account_balance,
            );
        }
        proposal.votes[usize::from(vote_index)].add_vote(account_balance);
        proposal.total_votes.add_vote(account_balance);

        require!(
            attached_deposit >= storage_added,
            format!(
                "Requires deposit of {}",
                storage_added.exact_amount_display()
            )
        );

        if attached_deposit > near_add(storage_added, NearToken::from_yoctonear(1)) {
            let refund = near_sub(attached_deposit, storage_added);
            Promise::new(env::signer_account_id())
                .transfer(refund)
                .detach();
        }

        events::emit::proposal_vote_action(
            "add_vote",
            &account_id,
            proposal_id,
            vote_index,
            &account_balance,
        );

        if proposal.flow == ProposalFlow::FastTrack
            && proposal.status == ProposalStatus::Sandbox
            && proposal.sandbox_threshold_met()
        {
            let scheduled_start = next_voting_start_ns(env::block_timestamp());
            proposal.voting_start_time_ns = Some(scheduled_start.into());
            proposal.status = ProposalStatus::Scheduled;
        }

        self.votes
            .insert((account_id.clone(), proposal_id), vote_index);
        self.internal_set_proposal(proposal);
    }

    /// Returns the vote of the given account ID and proposal ID.
    pub fn get_vote(&self, account_id: AccountId, proposal_id: ProposalId) -> Option<u8> {
        self.votes.get(&(account_id, proposal_id)).cloned()
    }

    /// Fetches a fresh veNEAR snapshot for a proposal that's already in Sandbox/Voting without
    /// a snapshot, and optionally casts a vote in the same transaction.
    #[payable]
    pub fn take_snapshot_and_vote(
        &mut self,
        proposal_id: ProposalId,
        vote: Option<VotePayload>,
    ) -> Promise {
        self.assert_not_paused();
        self.internal_advance_queue();

        let proposal = self.internal_expect_proposal_updated(proposal_id);
        if proposal.status != ProposalStatus::Sandbox && proposal.status != ProposalStatus::Voting {
            env::panic_str("Proposal must be in Sandbox or Voting status to take a snapshot");
        }
        if vote.is_none() && proposal.snapshot_and_state.is_some() {
            env::panic_str("Snapshot is already set for this proposal");
        }

        let mut promise: Option<Promise> = None;
        if proposal.snapshot_and_state.is_none() {
            promise = Some(
                ext_venear::ext(self.config.venear_account_id.clone())
                    .with_unused_gas_weight(1)
                    .get_snapshot()
                    .then(
                        ext_self::ext(env::current_account_id())
                            .with_static_gas(GAS_FOR_ON_GET_SNAPSHOT)
                            .on_get_snapshot(proposal_id),
                    ),
            );
        }

        if let Some(payload) = vote {
            let caller_account: Account = payload.v_account.clone().into();
            let caller_account_id = &caller_account.account_id;
            require!(
                caller_account_id == &env::predecessor_account_id(),
                "v_account does not match the caller"
            );
            let action = ext_self::ext(env::current_account_id())
                .with_attached_deposit(env::attached_deposit())
                .with_static_gas(GAS_FOR_CHAINED_VOTE)
                .vote(
                    proposal_id,
                    payload.vote,
                    payload.merkle_proof,
                    payload.v_account,
                );
            promise = Some(match promise {
                Some(p) => p.then(action),
                None => action,
            });
        }

        promise.unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;
    use merkle_tree::MerkleProof;
    use near_sdk::NearToken;

    /// Single-voter fixture + fresh contract, created and approved with snapshot.
    fn setup(
        flow: ProposalFlow,
        near_balance: NearToken,
        total: NearToken,
    ) -> (Contract, SnapshotFixture, ProposalId) {
        let fixture = snapshot_with_voters(&[VoterSpec::new(voter(), near_balance)], total);
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, flow);
        approve_proposal(&mut contract, pid, Some(&fixture));
        (contract, fixture, pid)
    }

    #[test]
    fn vote_happy_path_voting() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.votes[0].total_votes, 1);
        assert_eq!(
            proposal.votes[0].total_venear,
            voting_power(NearToken::from_near(100))
        );
        assert_eq!(proposal.total_votes.total_votes, 1);
        assert_eq!(contract.get_vote(voter(), pid), Some(0));
    }

    #[test]
    fn vote_sandbox_for_allowed_below_threshold() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::FastTrack,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.status, ProposalStatus::Sandbox);
        assert_eq!(proposal.votes[0].total_votes, 1);
    }

    #[test]
    #[should_panic(expected = "Only 'For' votes are allowed during the sandbox period")]
    fn vote_sandbox_against_panics() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::FastTrack,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::Against);
    }

    #[test]
    #[should_panic(expected = "Proposal is not in the voting phase")]
    fn vote_rejected_in_created_status() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);
    }

    #[test]
    #[should_panic(expected = "Snapshot has not been taken yet")]
    fn vote_requires_snapshot() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, None);

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);
    }

    #[test]
    #[should_panic(expected = "Invalid merkle proof")]
    fn vote_invalid_proof() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());
        let tampered = MerkleProof {
            index: proof.index.wrapping_add(1),
            path: proof.path,
        };

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, tampered, v_account);
    }

    #[test]
    #[should_panic(expected = "Account ID doesn't match the predecessor account ID")]
    fn vote_predecessor_mismatch() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(acc("attacker.test.near"), vote_deposit_yocto(), TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof, v_account);
    }

    #[test]
    #[should_panic(expected = "Account has no veNEAR balance")]
    fn vote_zero_balance_rejected() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_yoctonear(0),
            NearToken::from_near(10_000),
        );

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);
    }

    #[test]
    #[should_panic(expected = "Already voted for the same option")]
    fn vote_double_same_option_panics() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof.clone(), v_account.clone());
        set_ctx(voter(), 1, TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof, v_account);
    }

    #[test]
    fn vote_change_option_moves_balance() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof.clone(), v_account.clone());
        set_ctx(voter(), 1, TEST_NOW_NS);
        contract.vote(pid, VoteOption::Against, proof, v_account);

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.votes[0].total_votes, 0);
        assert_eq!(proposal.votes[0].total_venear, NearToken::from_yoctonear(0));
        assert_eq!(proposal.votes[1].total_votes, 1);
        assert_eq!(
            proposal.votes[1].total_venear,
            voting_power(NearToken::from_near(100))
        );
        assert_eq!(proposal.total_votes.total_votes, 1);
        assert_eq!(contract.get_vote(voter(), pid), Some(1));
    }

    #[test]
    #[should_panic(expected = "Requires attached deposit")]
    fn vote_requires_attached_deposit() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), 0, TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof, v_account);
    }

    #[test]
    #[should_panic(expected = "Requires deposit of")]
    fn vote_insufficient_deposit_for_first_vote() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), 1, TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof, v_account);
    }

    #[test]
    fn vote_fasttrack_sandbox_threshold_flips_to_scheduled() {
        // Voter holds 400 NEAR out of 1 000 = 40%, above the configured 30% sandbox bps.
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::FastTrack,
            NearToken::from_near(400),
            NearToken::from_near(1_000),
        );

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.status, ProposalStatus::Scheduled);
        assert_eq!(
            proposal.voting_start_time_ns.unwrap().0,
            crate::proposal::next_voting_start_ns(TEST_NOW_NS)
        );
    }

    #[test]
    #[should_panic(expected = "Snapshot is already set for this proposal")]
    fn take_snapshot_panics_if_snapshot_present_without_vote() {
        let (mut contract, _fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );

        set_ctx(voter(), 1, TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(pid, None);
    }

    #[test]
    #[should_panic(expected = "Proposal must be in Sandbox or Voting status to take a snapshot")]
    fn take_snapshot_panics_in_wrong_status() {
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);

        set_ctx(voter(), 1, TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(pid, None);
    }

    #[test]
    #[should_panic(expected = "v_account does not match the caller")]
    fn take_snapshot_vote_payload_predecessor_mismatch() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, None);
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(acc("attacker.test.near"), vote_deposit_yocto(), TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(
            pid,
            Some(VotePayload {
                vote: VoteOption::For,
                merkle_proof: proof,
                v_account,
            }),
        );
    }

    #[test]
    fn on_get_snapshot_sets_snapshot_and_state() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, Some(&fixture));

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        let stored = proposal.snapshot_and_state.expect("snapshot stored");
        assert_eq!(stored.snapshot.length, fixture.snapshot.length);
        assert_eq!(stored.snapshot.root, fixture.snapshot.root);
    }

    #[test]
    #[should_panic(expected = "Snapshot is already set for this proposal")]
    fn on_get_snapshot_panics_if_already_set() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );

        // Re-trigger the snapshot callback directly to hit the duplicate-set guard.
        near_sdk::testing_env!(
            VMContextBuilder::new()
                .current_account_id(current_account())
                .predecessor_account_id(current_account())
                .attached_deposit(NearToken::from_yoctonear(0))
                .block_timestamp(TEST_NOW_NS)
                .build()
        );
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), pid);
    }

    #[test]
    #[should_panic(expected = "Proposal is not in the voting phase")]
    fn vote_rejected_in_queued_status() {
        // max_active = 1, so the second approval lands Queued; vote() must refuse.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(1);

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        approve_proposal(&mut contract, b, None);
        let b_raw: Proposal = contract.proposals.get(b).cloned().unwrap().into();
        assert_eq!(b_raw.status, ProposalStatus::Queued);

        cast_vote(&mut contract, &fixture, voter(), b, VoteOption::For);
    }

    #[test]
    #[should_panic(expected = "Proposal is not in the voting phase")]
    fn vote_rejected_in_scheduled_status() {
        // FastTrack over sandbox threshold flips to Scheduled; voting then is rejected.
        let voter_a = voter();
        let filler = acc("filler1.test.near");
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(voter_a.clone(), NearToken::from_near(400)),
                VoterSpec::new(filler.clone(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, pid, Some(&fixture));

        cast_vote(&mut contract, &fixture, voter_a, pid, VoteOption::For);
        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.status, ProposalStatus::Scheduled);

        cast_vote(&mut contract, &fixture, filler, pid, VoteOption::For);
    }

    #[test]
    #[should_panic(expected = "Proposal is not in the voting phase")]
    fn vote_rejected_in_terminal_status() {
        // No votes cast; advance past voting end so update() transitions Voting -> Defeated.
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let voting_duration_ns = default_config().classic_voting_duration_ns.0;
        let after_end = TEST_NOW_NS + voting_duration_ns + 1;

        cast_vote_at(
            &mut contract,
            &fixture,
            voter(),
            pid,
            VoteOption::For,
            after_end,
        );
    }

    #[test]
    fn vote_multiple_voters_aggregate() {
        let voter_a = voter();
        let voter_b = acc("voter-b.test.near");
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(voter_a.clone(), NearToken::from_near(100)),
                VoterSpec::new(voter_b.clone(), NearToken::from_near(40)),
            ],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, Some(&fixture));

        cast_vote(
            &mut contract,
            &fixture,
            voter_a.clone(),
            pid,
            VoteOption::For,
        );
        cast_vote(
            &mut contract,
            &fixture,
            voter_b.clone(),
            pid,
            VoteOption::Against,
        );

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.votes[0].total_votes, 1);
        assert_eq!(
            proposal.votes[0].total_venear,
            voting_power(NearToken::from_near(100))
        );
        assert_eq!(proposal.votes[1].total_votes, 1);
        assert_eq!(
            proposal.votes[1].total_venear,
            voting_power(NearToken::from_near(40))
        );
        assert_eq!(proposal.total_votes.total_votes, 2);
        assert_eq!(
            proposal.total_votes.total_venear,
            voting_power(NearToken::from_near(140))
        );
        assert_eq!(contract.get_vote(voter_a, pid), Some(0));
        assert_eq!(contract.get_vote(voter_b, pid), Some(1));
    }

    #[test]
    #[should_panic(expected = "Invalid merkle proof")]
    fn vote_rejects_tampered_v_account() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        // Inflate the balance after the proof was generated: leaf hash no longer matches.
        let tampered = match v_account {
            VAccount::V1(mut a) => {
                a.balance.near_balance = NearToken::from_near(1_000_000);
                VAccount::V1(a)
            }
            VAccount::V0(_) => unreachable!("fixture builds V1"),
        };

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        contract.vote(pid, VoteOption::For, proof, tampered);
    }

    #[test]
    fn vote_accepts_self_call_predecessor() {
        // Self-call predecessor (the chained take_snapshot_and_vote path) votes
        // as the v_account's owner, not as current_account_id.
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        near_sdk::testing_env!(
            VMContextBuilder::new()
                .current_account_id(current_account())
                .predecessor_account_id(current_account())
                .attached_deposit(NearToken::from_yoctonear(vote_deposit_yocto()))
                .block_timestamp(TEST_NOW_NS)
                .build()
        );
        contract.vote(pid, VoteOption::For, proof, v_account);

        assert_eq!(contract.get_vote(voter(), pid), Some(0));
        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(
            proposal.votes[0].total_venear,
            voting_power(NearToken::from_near(100))
        );
    }

    #[test]
    fn get_vote_returns_none_before_voting() {
        let (contract, _fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        assert_eq!(contract.get_vote(voter(), pid), None);
        assert_eq!(contract.get_vote(acc("never-voted.test.near"), pid), None);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn vote_panics_when_paused() {
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        contract.paused = true;

        cast_vote(&mut contract, &fixture, voter(), pid, VoteOption::For);
    }

    #[test]
    fn vote_refunds_excess_deposit_above_storage_fee() {
        // 11 millinear attached over the 10 millinear fee triggers the refund branch.
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(
            voter(),
            NearToken::from_millinear(11).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(pid, VoteOption::For, proof, v_account);

        let proposal: Proposal = contract.proposals.get(pid).cloned().unwrap().into();
        assert_eq!(proposal.votes[0].total_votes, 1);
    }

    #[test]
    fn take_snapshot_and_vote_without_vote_returns_snapshot_promise() {
        // No snapshot yet: the call must build and return the fetch promise.
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, None);

        set_ctx(voter(), 1, TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(pid, None);
    }

    #[test]
    fn take_snapshot_and_vote_with_vote_payload_only_chains_action() {
        // Snapshot present: fetch is skipped, only the chained vote action runs.
        let (mut contract, fixture, pid) = setup(
            ProposalFlow::Classic,
            NearToken::from_near(100),
            NearToken::from_near(10_000),
        );
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(
            pid,
            Some(VotePayload {
                vote: VoteOption::For,
                merkle_proof: proof,
                v_account,
            }),
        );
    }

    #[test]
    fn take_snapshot_and_vote_with_both_chains_snapshot_then_vote() {
        // No snapshot + a vote payload: fetch promise built, then vote chained onto it.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(voter(), NearToken::from_near(100))],
            NearToken::from_near(10_000),
        );
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, None);
        let (proof, v_account) = fixture.proof_for(&voter());

        set_ctx(voter(), vote_deposit_yocto(), TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(
            pid,
            Some(VotePayload {
                vote: VoteOption::For,
                merkle_proof: proof,
                v_account,
            }),
        );
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn take_snapshot_and_vote_panics_when_paused() {
        let mut contract = fresh_contract();
        let pid = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, pid, None);
        contract.paused = true;

        set_ctx(voter(), 1, TEST_NOW_NS);
        let _ = contract.take_snapshot_and_vote(pid, None);
    }
}
