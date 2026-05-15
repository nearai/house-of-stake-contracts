use crate::proposal::{ProposalFlow, ProposalId, ProposalStatus};
use crate::*;
use near_sdk::Promise;

#[near]
impl Contract {
    /// Refunds the bond to the proposer. Only valid for `Expired` or `Rejected` proposals.
    pub fn claim_bond(&mut self, proposal_id: ProposalId) -> Promise {
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.flow == ProposalFlow::FastTrack,
            "Bonds only exist on FastTrack proposals"
        );
        require!(
            proposal.proposer_id == env::predecessor_account_id(),
            "Only the proposer can claim the bond"
        );
        require!(proposal.bond_amount > NearToken::ZERO, "No bond to claim");
        require!(
            matches!(
                proposal.status,
                ProposalStatus::Expired | ProposalStatus::Rejected
            ),
            "Bond can only be claimed from expired or rejected proposals"
        );

        let bond = proposal.bond_amount;
        let proposer_id = proposal.proposer_id.clone();
        proposal.bond_amount = NearToken::ZERO;
        self.internal_set_proposal(proposal);

        Promise::new(proposer_id).transfer(bond)
    }
}
