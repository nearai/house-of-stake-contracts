use crate::*;
use common::Bps;
use near_sdk::assert_one_yocto;

#[near]
impl Contract {
    /// Updates the account ID of the veNEAR contract.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_venear_account_id(&mut self, venear_account_id: AccountId) {
        assert_one_yocto();
        self.assert_owner();
        self.config.venear_account_id = venear_account_id;
    }

    /// Updates the list of account IDs that can review proposals.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_reviewer_ids(&mut self, reviewer_ids: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.reviewer_ids = reviewer_ids;
    }

    /// Updates the Classic-flow voting duration in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_classic_voting_duration(&mut self, voting_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.classic_voting_duration_ns =
            (u64::from(voting_duration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the FastTrack-flow voting duration in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_fast_track_voting_duration(&mut self, voting_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.fast_track_voting_duration_ns =
            (u64::from(voting_duration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the base fee required to create a proposal.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_base_proposal_fee(&mut self, base_proposal_fee: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.base_proposal_fee = base_proposal_fee;
    }

    /// Proposes the new owner account ID.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn propose_new_owner_account_id(&mut self, new_owner_account_id: Option<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.proposed_new_owner_account_id = new_owner_account_id;
    }

    /// Accepts the new owner account ID.
    /// Can only be called by the new owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn accept_ownership(&mut self) {
        assert_one_yocto();
        let predecessor = env::predecessor_account_id();
        require!(
            self.config.proposed_new_owner_account_id.as_ref() == Some(&predecessor),
            "Only the proposed new owner can call this method"
        );
        self.config.owner_account_id = predecessor;
        self.config.proposed_new_owner_account_id = None;
    }

    /// Sets the list of account IDs that can pause the contract.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_guardians(&mut self, guardians: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.guardians = guardians;
    }

    /// Updates the list of council member account IDs who can veto proposals during timelock.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_council_ids(&mut self, council_ids: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.council_ids = council_ids;
    }

    /// Updates the timelock duration in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_timelock_duration(&mut self, timelock_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.timelock_duration_ns = (u64::from(timelock_duration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the Classic proposal expiration duration in seconds.
    /// Set to 0 to disable proposal expiration.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_proposal_expiration(&mut self, proposal_expiration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.proposal_expiration_ns =
            (u64::from(proposal_expiration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the FastTrack proposal expiration duration in seconds.
    /// Set to 0 to disable proposal expiration.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_fast_track_proposal_expiration(&mut self, proposal_expiration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.fast_track_proposal_expiration_ns =
            (u64::from(proposal_expiration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the quorum threshold in basis points (e.g. 3500 = 35%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_quorum_threshold_bps(&mut self, quorum_threshold_bps: Bps) {
        assert_one_yocto();
        self.assert_owner();
        self.config.quorum_threshold_bps = quorum_threshold_bps;
    }

    /// Updates the quorum floor (absolute minimum veNEAR required for quorum).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_quorum_floor(&mut self, quorum_floor: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.quorum_floor = quorum_floor;
    }

    /// Updates the classic-flow approval threshold in basis points (e.g. 5000 = 50%,
    /// 6667 = ~66.67%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_approval_threshold_bps(&mut self, approval_threshold_bps: Bps) {
        assert_one_yocto();
        self.assert_owner();
        self.config.approval_threshold_bps = approval_threshold_bps;
    }

    /// Updates the FastTrack bond amount required to create a proposal.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_bond_amount(&mut self, bond_amount: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.bond_amount = bond_amount;
    }

    /// Updates the treasury account ID that receives forfeited FastTrack bonds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_treasury_account_id(&mut self, treasury_account_id: AccountId) {
        assert_one_yocto();
        self.assert_owner();
        self.config.treasury_account_id = treasury_account_id;
    }

    /// Updates the FastTrack simple majority threshold in basis points (e.g. 5000 = 50%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_simple_majority_threshold_bps(&mut self, simple_majority_threshold_bps: Bps) {
        assert_one_yocto();
        self.assert_owner();
        self.config.simple_majority_threshold_bps = simple_majority_threshold_bps;
    }

    /// Updates the FastTrack strong (super) majority threshold in basis points (e.g. 6667 ≈ 66.67%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_strong_majority_threshold_bps(&mut self, strong_majority_threshold_bps: Bps) {
        assert_one_yocto();
        self.assert_owner();
        self.config.strong_majority_threshold_bps = strong_majority_threshold_bps;
    }

    /// Updates the FastTrack sandbox duration in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_sandbox_duration(&mut self, sandbox_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.sandbox_duration_ns = (u64::from(sandbox_duration_sec) * 10u64.pow(9)).into();
    }

    /// Updates the FastTrack sandbox threshold in basis points (e.g. 3000 = 30%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_sandbox_threshold_bps(&mut self, sandbox_threshold_bps: Bps) {
        assert_one_yocto();
        self.assert_owner();
        self.config.sandbox_threshold_bps = sandbox_threshold_bps;
    }

    /// Updates the maximum number of simultaneously active (Sandbox/Scheduled/Voting) proposals.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_max_active_proposals(&mut self, max_active_proposals: u32) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            max_active_proposals > 0,
            "max_active_proposals must be greater than 0"
        );
        self.config.max_active_proposals = max_active_proposals;
        self.internal_advance_queue();
    }
}

impl Contract {
    pub fn assert_owner(&self) {
        require!(
            env::predecessor_account_id() == self.config.owner_account_id,
            "Only the owner can call this method"
        );
    }

    /// Asserts that the caller is one of the guardians or the owner.
    pub fn assert_guardian(&self) {
        let predecessor = env::predecessor_account_id();
        require!(
            self.config.guardians.contains(&predecessor)
                || predecessor == self.config.owner_account_id,
            "Only the guardian can call this method"
        );
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the governance setters and the two-step ownership transfer.
    //!
    //! The setters share the same shape (assert_one_yocto + assert_owner +
    //! mutate one config field), so this suite tests one representative of
    //! each shape exhaustively, plus the few setters with custom logic:
    //! `accept_ownership` (two-step), `set_max_active_proposals` (>0 guard
    //! and queue advance side-effect).
    use super::*;
    use crate::proposal::{ProposalFlow, ProposalStatus};
    use crate::test_utils::*;

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
        // propose_new_owner_account_id has its own custom logic (the two-step
        // flow) and warrants a distinct role-rejection test, unlike the
        // plain-shape setters covered by `non_owner_cannot_set_venear_account_id`.
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
    fn set_proposal_expiration_round_trips() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_proposal_expiration(12_345);
        assert_eq!(
            contract.get_config().proposal_expiration_ns.0,
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
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(100)),
            ],
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

        // Lifting the cap to 2 must promote the queued proposal during the
        // setter's `internal_advance_queue` tail call.
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_max_active_proposals(2);

        assert_eq!(
            contract.get_proposal(id_b).unwrap().proposal.status,
            ProposalStatus::Voting
        );
    }

}
