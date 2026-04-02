use crate::proposal::{next_voting_start_ns, Proposal, ProposalStatus, SnapshotAndState, VoteOption};
use crate::*;
use common::{events, near_add, near_sub};
use near_sdk::Promise;

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
            ProposalStatus::Created | ProposalStatus::Scheduled => {
                env::panic_str("Voting is not started yet");
            }
            ProposalStatus::Rejected => {
                env::panic_str("Proposal is rejected");
            }
            ProposalStatus::Expired => {
                env::panic_str("Proposal is expired");
            }
            ProposalStatus::Succeeded
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed => {
                env::panic_str("Voting is finished");
            }
            ProposalStatus::Spam => {
                env::panic_str("Proposal is spam");
            }
        }

        // Validate merkle proof
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
        require!(
            account_id == &env::predecessor_account_id(),
            "Account ID doesn't match the predecessor account ID"
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
            // When changing the vote. Don't need to charge the fee again.
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

        // Note, don't refund 1 yoctoNEAR if changing the vote.
        if attached_deposit > near_add(storage_added, NearToken::from_yoctonear(1)) {
            let refund = near_sub(attached_deposit, storage_added);
            Promise::new(env::predecessor_account_id()).transfer(refund);
        }

        events::emit::proposal_vote_action(
            "add_vote",
            &account_id,
            proposal_id,
            vote_index,
            &account_balance,
        );

        // Schedule voting for the next Monday if sandbox threshold is met
        if proposal.status == ProposalStatus::Sandbox && proposal.sandbox_threshold_met() {
            let now_ns = env::block_timestamp();
            let after_ns = std::cmp::max(now_ns, self.last_voting_end_ns);
            let scheduled_start = next_voting_start_ns(after_ns);
            proposal.voting_start_time_ns = Some(scheduled_start.into());
            proposal.status = ProposalStatus::Scheduled;
            self.last_voting_end_ns = scheduled_start + proposal.voting_duration_ns.0;
        }

        self.votes
            .insert((account_id.clone(), proposal_id), vote_index);
        self.internal_set_proposal(proposal);
    }

    /// Returns the vote of the given account ID and proposal ID.
    pub fn get_vote(&self, account_id: AccountId, proposal_id: ProposalId) -> Option<u8> {
        self.votes.get(&(account_id, proposal_id)).cloned()
    }
}
