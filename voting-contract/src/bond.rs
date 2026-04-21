use crate::proposal::{ProposalFlow, ProposalId, ProposalStatus};
use crate::*;
use near_sdk::Promise;

#[near]
impl Contract {
    /// Allows the proposer of a v2 proposal to reclaim their bond if the proposal expired
    /// before it was approved. Any other terminal state forfeits the bond.
    pub fn claim_bond(&mut self, proposal_id: ProposalId) {
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.flow == ProposalFlow::V2,
            "Bonds only exist on v2 proposals"
        );
        require!(
            proposal.proposer_id == env::predecessor_account_id(),
            "Only the proposer can claim the bond"
        );
        require!(proposal.bond_amount.as_yoctonear() > 0, "No bond to claim");
        require!(
            proposal.status == ProposalStatus::Expired,
            "Bond can only be claimed from expired proposals"
        );

        let bond = proposal.bond_amount;
        let proposer_id = proposal.proposer_id.clone();
        proposal.bond_amount = NearToken::from_yoctonear(0);
        self.internal_set_proposal(proposal);

        Promise::new(proposer_id).transfer(bond);
    }
}
