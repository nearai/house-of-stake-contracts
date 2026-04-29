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
            ProposalStatus::Created | ProposalStatus::Scheduled | ProposalStatus::Queued => {
                env::panic_str("Voting is not started yet");
            }
            ProposalStatus::Rejected => env::panic_str("Proposal is rejected"),
            ProposalStatus::Expired => env::panic_str("Proposal is expired"),
            ProposalStatus::Slashed => env::panic_str("Proposal is slashed"),
            ProposalStatus::Succeeded
            | ProposalStatus::Defeated
            | ProposalStatus::Timelock
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed => {
                env::panic_str("Voting is finished");
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

        let vote_index = vote as u8;
        let previous_vote = self.votes.get(&(account_id.clone(), proposal_id)).cloned();
        require!(
            previous_vote != Some(vote_index),
            "Already voted for the same option"
        );
        let mut storage_added = self.config.vote_storage_fee;
        if let Some(previous_vote) = previous_vote {
            proposal.votes[previous_vote as usize].remove_vote(account_balance);
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
        proposal.votes[vote_index as usize].add_vote(account_balance);
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
            Promise::new(env::signer_account_id()).transfer(refund);
        }

        events::emit::proposal_vote_action(
            "add_vote",
            &account_id,
            proposal_id,
            vote_index,
            &account_balance,
        );

        if proposal.flow == ProposalFlow::V2
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
            let VAccount::V0(account) = &payload.v_account;
            require!(
                account.account_id == env::predecessor_account_id(),
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
