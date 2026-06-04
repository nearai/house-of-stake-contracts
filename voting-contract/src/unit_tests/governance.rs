//! Tests for the governance setters and the two-step ownership transfer.
use crate::proposal::{ProposalFlow, ProposalStatus};
use crate::unit_tests::test_utils::*;
use crate::*;
use common::Bps;

#[test]
fn owner_can_set_venear_account_id() {
    let mut contract = fresh_contract();
    let new_venear = acc("new-venear.test.near");
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_venear_account_id(new_venear.clone());
    assert_eq!(contract.get_config().venear_account_id, new_venear);
}

#[test]
#[should_panic(expected = "Only the owner can call this method")]
fn non_owner_cannot_set_venear_account_id() {
    let mut contract = fresh_contract();
    set_ctx(guardian(), 1, TEST_NOW_NS);
    contract.set_venear_account_id(acc("new-venear.test.near"));
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn set_venear_account_id_requires_one_yocto() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 0, TEST_NOW_NS);
    contract.set_venear_account_id(acc("new-venear.test.near"));
}

#[test]
fn duration_setters_convert_seconds_to_nanoseconds() {
    let mut contract = fresh_contract();
    let cases: &[(&str, u32, u64)] = &[
        ("classic_voting", 3_600, 3_600_000_000_000),
        ("fast_track_voting", 60, 60_000_000_000),
        ("timelock", 86_400, 86_400_000_000_000),
        ("sandbox", 7_200, 7_200_000_000_000),
    ];

    for (label, seconds, expected_ns) in cases {
        set_ctx(owner(), 1, TEST_NOW_NS);
        match *label {
            "classic_voting" => contract.set_classic_voting_duration(*seconds),
            "fast_track_voting" => contract.set_fast_track_voting_duration(*seconds),
            "timelock" => contract.set_timelock_duration(*seconds),
            "sandbox" => contract.set_sandbox_duration(*seconds),
            _ => unreachable!(),
        }
        let cfg = contract.get_config();
        let actual = match *label {
            "classic_voting" => cfg.classic_voting_duration_ns.0,
            "fast_track_voting" => cfg.fast_track_voting_duration_ns.0,
            "timelock" => cfg.timelock_duration_ns.0,
            "sandbox" => cfg.sandbox_duration_ns.0,
            _ => unreachable!(),
        };
        assert_eq!(actual, *expected_ns, "{} mismatch", label);
    }
}

#[test]
fn duration_setter_with_zero_seconds_writes_zero() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_timelock_duration(0);
    assert_eq!(contract.get_config().timelock_duration_ns.0, 0);
}

#[test]
fn duration_setter_with_max_u32_does_not_overflow_u64_multiplication() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_timelock_duration(u32::MAX);
    let expected = u64::from(u32::MAX) * 10u64.pow(9);
    assert_eq!(contract.get_config().timelock_duration_ns.0, expected);
}

#[test]
fn bps_setters_round_trip() {
    let mut contract = fresh_contract();
    let cases: &[(&str, u16)] = &[
        ("quorum", 4_200),
        ("approval", 5_500),
        ("simple_majority", 5_000),
        ("strong_majority", 6_667),
        ("sandbox_threshold", 3_000),
    ];
    for (label, raw_bps) in cases {
        let bps = Bps::new(*raw_bps);
        set_ctx(owner(), 1, TEST_NOW_NS);
        match *label {
            "quorum" => contract.set_quorum_threshold_bps(bps),
            "approval" => contract.set_approval_threshold_bps(bps),
            "simple_majority" => contract.set_simple_majority_threshold_bps(bps),
            "strong_majority" => contract.set_strong_majority_threshold_bps(bps),
            "sandbox_threshold" => contract.set_sandbox_threshold_bps(bps),
            _ => unreachable!(),
        }
        let cfg = contract.get_config();
        let actual = match *label {
            "quorum" => cfg.quorum_threshold_bps,
            "approval" => cfg.approval_threshold_bps,
            "simple_majority" => cfg.simple_majority_threshold_bps,
            "strong_majority" => cfg.strong_majority_threshold_bps,
            "sandbox_threshold" => cfg.sandbox_threshold_bps,
            _ => unreachable!(),
        };
        assert_eq!(actual, bps, "{} mismatch", label);
    }
}

#[test]
fn bps_setters_accept_full_and_zero() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_quorum_threshold_bps(Bps::ZERO);
    assert_eq!(contract.get_config().quorum_threshold_bps, Bps::ZERO);

    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_quorum_threshold_bps(Bps::FULL);
    assert_eq!(contract.get_config().quorum_threshold_bps, Bps::FULL);
}

#[test]
fn ownership_transfer_two_step_flow() {
    let mut contract = fresh_contract();
    let new_owner = acc("next-owner.test.near");

    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.propose_new_owner_account_id(Some(new_owner.clone()));
    assert_eq!(
        contract.get_config().proposed_new_owner_account_id.as_ref(),
        Some(&new_owner)
    );

    set_ctx(new_owner.clone(), 1, TEST_NOW_NS);
    contract.accept_ownership();

    let cfg = contract.get_config();
    assert_eq!(cfg.owner_account_id, new_owner);
    assert_eq!(cfg.proposed_new_owner_account_id, None);
}

#[test]
fn ownership_transfer_can_be_revoked_before_acceptance() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.propose_new_owner_account_id(Some(acc("next-owner.test.near")));
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.propose_new_owner_account_id(None);
    assert_eq!(contract.get_config().proposed_new_owner_account_id, None);
}

#[test]
#[should_panic(expected = "Only the proposed new owner can call this method")]
fn accept_ownership_rejects_unproposed_caller() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.propose_new_owner_account_id(Some(acc("next-owner.test.near")));
    set_ctx(acc("imposter.test.near"), 1, TEST_NOW_NS);
    contract.accept_ownership();
}

#[test]
#[should_panic(expected = "Only the proposed new owner can call this method")]
fn accept_ownership_when_none_proposed_panics() {
    let mut contract = fresh_contract();
    set_ctx(acc("rando.test.near"), 1, TEST_NOW_NS);
    contract.accept_ownership();
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn accept_ownership_requires_one_yocto() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    let new_owner = acc("next-owner.test.near");
    contract.propose_new_owner_account_id(Some(new_owner.clone()));

    set_ctx(new_owner, 0, TEST_NOW_NS);
    contract.accept_ownership();
}

#[test]
#[should_panic(expected = "Only the owner can call this method")]
fn non_owner_cannot_propose_new_owner() {
    // The two-step flow's custom logic warrants its own role-rejection test.
    let mut contract = fresh_contract();
    set_ctx(guardian(), 1, TEST_NOW_NS);
    contract.propose_new_owner_account_id(Some(acc("next-owner.test.near")));
}

#[test]
fn set_max_active_proposals_persists() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_max_active_proposals(5);
    assert_eq!(contract.get_config().max_active_proposals, 5);
}

#[test]
#[should_panic(expected = "max_active_proposals must be greater than 0")]
fn set_max_active_proposals_rejects_zero() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_max_active_proposals(0);
}

#[test]
fn set_max_active_proposals_one_is_minimum_valid() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_max_active_proposals(1);
    assert_eq!(contract.get_config().max_active_proposals, 1);
}

#[test]
fn set_reviewer_ids_replaces_full_list() {
    let mut contract = fresh_contract();
    let new_ids = vec![acc("rev-a.test.near"), acc("rev-b.test.near")];
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_reviewer_ids(new_ids.clone());
    assert_eq!(contract.get_config().reviewer_ids, new_ids);
}

#[test]
fn set_base_proposal_fee_round_trips() {
    let mut contract = fresh_contract();
    let new_fee = NearToken::from_near(7);
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_base_proposal_fee(new_fee);
    assert_eq!(contract.get_config().base_proposal_fee, new_fee);
}

#[test]
fn set_guardians_replaces_full_list() {
    let mut contract = fresh_contract();
    let new_ids = vec![acc("g-a.test.near"), acc("g-b.test.near")];
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_guardians(new_ids.clone());
    assert_eq!(contract.get_config().guardians, new_ids);
}

#[test]
fn set_council_ids_replaces_full_list() {
    let mut contract = fresh_contract();
    let new_ids = vec![acc("c-a.test.near"), acc("c-b.test.near")];
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_council_ids(new_ids.clone());
    assert_eq!(contract.get_config().council_ids, new_ids);
}

#[test]
fn set_bond_amount_round_trips() {
    let mut contract = fresh_contract();
    let new_bond = NearToken::from_near(42);
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_bond_amount(new_bond);
    assert_eq!(contract.get_config().bond_amount, new_bond);
}

#[test]
fn set_treasury_account_id_round_trips() {
    let mut contract = fresh_contract();
    let new_treasury = acc("new-treasury.test.near");
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_treasury_account_id(new_treasury.clone());
    assert_eq!(contract.get_config().treasury_account_id, new_treasury);
}

#[test]
fn set_quorum_floor_round_trips() {
    let mut contract = fresh_contract();
    let new_floor = NearToken::from_near(123);
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_quorum_floor(new_floor);
    assert_eq!(contract.get_config().quorum_floor, new_floor);
}

#[test]
fn set_classic_proposal_expiration_round_trips() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_classic_proposal_expiration(12_345);
    assert_eq!(
        contract.get_config().classic_proposal_expiration_ns.0,
        12_345u64 * 1_000_000_000
    );
}

#[test]
fn set_fast_track_proposal_expiration_round_trips() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_fast_track_proposal_expiration(6_789);
    assert_eq!(
        contract.get_config().fast_track_proposal_expiration_ns.0,
        6_789u64 * 1_000_000_000
    );
}

#[test]
fn set_max_active_proposals_promotes_queued_when_cap_grows() {
    let fixture = snapshot_with_voters(
        &[VoterSpec::new(for_voter(), NearToken::from_near(100))],
        NearToken::from_near(1_000),
    );

    let mut contract = fresh_contract();
    // Shrink active cap to 1 so the second approval is forced to Queued.
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_max_active_proposals(1);

    let id_a = create_proposal(&mut contract, ProposalFlow::Classic);
    approve_proposal(&mut contract, id_a, Some(&fixture));
    let id_b = create_proposal(&mut contract, ProposalFlow::Classic);
    approve_proposal(&mut contract, id_b, None);

    assert_eq!(
        contract.get_proposal(id_a).unwrap().proposal.status,
        ProposalStatus::Voting
    );
    assert_eq!(
        contract.get_proposal(id_b).unwrap().proposal.status,
        ProposalStatus::Queued
    );

    // Lifting the cap promotes the queued proposal via internal_advance_queue.
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.set_max_active_proposals(2);

    assert_eq!(
        contract.get_proposal(id_b).unwrap().proposal.status,
        ProposalStatus::Voting
    );
}
