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

#[cfg(test)]
mod tests {
    //! `claim_bond` walks five guards in this order: flow, proposer, bond > 0,
    //! status, then transfer. Each test below drives state into the guard it
    //! exercises strictly through public `Contract` methods so the test set
    //! covers the real on-chain code path, not the struct in isolation.
    use super::*;
    use crate::test_utils::*;

    #[test]
    fn claim_bond_expired_happy_path_zeroes_bond() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_fast_track_proposal_expiration(3_600);
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        let after = TEST_NOW_NS + contract.get_config().fast_track_proposal_expiration_ns.0;

        set_ctx(proposer(), 0, after);
        let _ = contract.claim_bond(id);

        let info = contract.get_proposal(id).unwrap();
        assert_eq!(info.proposal.bond_amount, NearToken::ZERO);
        assert_eq!(info.proposal.status, ProposalStatus::Expired);
    }

    #[test]
    fn claim_bond_rejected_happy_path_zeroes_bond() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);

        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);

        let info = contract.get_proposal(id).unwrap();
        assert_eq!(info.proposal.bond_amount, NearToken::ZERO);
        assert_eq!(info.proposal.status, ProposalStatus::Rejected);
    }

    #[test]
    #[should_panic(expected = "Bond can only be claimed from expired or rejected proposals")]
    fn claim_bond_in_created_status_panics() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }

    #[test]
    #[should_panic(expected = "No bond to claim")]
    fn claim_bond_after_approval_hits_no_bond_guard() {
        // approve_proposal forwards the bond to treasury and sets bond_amount
        // to zero. A claim attempt against the now-Sandbox proposal therefore
        // trips the `bond_amount > 0` guard before reaching the status check.
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, None);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.status,
            ProposalStatus::Sandbox
        );

        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }

    #[test]
    #[should_panic(expected = "Bonds only exist on FastTrack proposals")]
    fn claim_bond_rejects_classic_flow() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);

        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }

    #[test]
    #[should_panic(expected = "Only the proposer can claim the bond")]
    fn claim_bond_only_proposer_may_claim() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
        set_ctx(acc("intruder.test.near"), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }

    #[test]
    #[should_panic(expected = "No bond to claim")]
    fn claim_bond_double_claim_panics() {
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }

    #[test]
    fn claim_bond_rejected_returns_full_configured_bond_amount() {
        // Verify the bond stored at creation time mirrors the configured amount
        // and is preserved through rejection so claim_bond returns it intact.
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.bond_amount,
            default_config().bond_amount
        );

        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.bond_amount,
            default_config().bond_amount
        );

        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.bond_amount,
            NearToken::ZERO
        );
    }

    #[test]
    fn claim_bond_honors_custom_configured_bond_amount() {
        // After set_bond_amount(50 NEAR), new FastTrack proposals carry that
        // amount; rejection preserves it and claim_bond zeroes it.
        let mut contract = fresh_contract();
        let custom = NearToken::from_near(50);
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_bond_amount(custom);

        set_ctx(
            proposer(),
            NearToken::from_near(200).as_yoctonear(),
            TEST_NOW_NS,
        );
        let id = contract.create_proposal(
            proposal_metadata("custom-bond"),
            None,
            ProposalFlow::FastTrack,
        );
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.bond_amount,
            custom
        );

        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);
        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
        assert_eq!(
            contract.get_proposal(id).unwrap().proposal.bond_amount,
            NearToken::ZERO
        );
    }

    #[test]
    fn claim_bond_expired_via_implicit_update_on_get() {
        // Drive expiration purely via `internal_expect_proposal_updated`'s
        // implicit `update()` call rather than an intervening status read.
        // The status transition Created -> Expired must happen inside
        // claim_bond itself, then the Expired arm passes.
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_fast_track_proposal_expiration(60);
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);

        let cfg_expiration = contract.get_config().fast_track_proposal_expiration_ns.0;
        set_ctx(proposer(), 0, TEST_NOW_NS + cfg_expiration);
        let _ = contract.claim_bond(id);

        let info = contract.get_proposal(id).unwrap();
        assert_eq!(info.proposal.status, ProposalStatus::Expired);
        assert_eq!(info.proposal.bond_amount, NearToken::ZERO);
    }

    #[test]
    #[should_panic(expected = "No bond to claim")]
    fn claim_bond_after_creation_with_zero_configured_bond_panics() {
        // Owner zeroes the bond requirement; a freshly-rejected FastTrack
        // proposal therefore carries bond=0 and hits the bond>0 guard before
        // the status check, even with a perfectly-claim-eligible Rejected
        // status.
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_bond_amount(NearToken::ZERO);

        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        contract.reject_proposal(id);

        set_ctx(proposer(), 0, TEST_NOW_NS);
        let _ = contract.claim_bond(id);
    }
}
