//! `claim_bond` guards in order: flow, proposer, bond > 0, status, transfer.
use crate::proposal::{ProposalFlow, ProposalStatus};
use crate::unit_tests::test_utils::*;
use crate::*;

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
    // approve_proposal zeroes the bond, so the claim trips bond > 0 before status.
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
    // Bond stored at creation matches config and survives rejection until claimed.
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
    // New proposals carry the custom bond; rejection preserves it, claim zeroes it.
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
    // Expiration happens via claim_bond's implicit update(), with no prior status read.
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
    // Zero configured bond means a Rejected proposal still trips bond > 0 first.
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_bond_amount(NearToken::ZERO);

    let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
    set_ctx(reviewer(), 1, TEST_NOW_NS);
    contract.reject_proposal(id);

    set_ctx(proposer(), 0, TEST_NOW_NS);
    let _ = contract.claim_bond(id);
}
