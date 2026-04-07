use crate::proposal::{ProposalId, ProposalStatus};
use crate::*;
use near_sdk::{assert_one_yocto, Promise};

#[near]
impl Contract {
    /// Allows the proposer to reclaim their bond from a terminal proposal (non-slashed).
    /// Bond is 0 for approved proposals (already returned during approval).
    /// Primary use: expired proposals. Also works as safety-net fallback.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn claim_bond(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.proposer_id == env::predecessor_account_id(),
            "Only the proposer can claim the bond"
        );
        require!(proposal.bond_amount.as_yoctonear() > 0, "No bond to claim");

        // Only claimable from terminal non-slashed states
        match proposal.status {
            ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Rejected
            | ProposalStatus::Failed
            | ProposalStatus::Succeeded => {}
            ProposalStatus::Slashed => {
                env::panic_str("Bond is forfeited for slashed proposals");
            }
            _ => {
                env::panic_str(
                    "Bond can only be claimed from terminal proposals (Expired, Defeated, Rejected, Failed, Succeeded)",
                );
            }
        }

        let bond = proposal.bond_amount;
        let proposer_id = proposal.proposer_id.clone();
        proposal.bond_amount = NearToken::from_yoctonear(0);
        self.internal_set_proposal(proposal);

        Promise::new(proposer_id).transfer(bond);
    }
}
