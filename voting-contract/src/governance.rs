use crate::*;
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

    /// Updates the maximum duration of the voting period in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_voting_duration(&mut self, voting_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.voting_duration_ns = (voting_duration_sec as u64 * 10u64.pow(9)).into();
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
        self.config.timelock_duration_ns = (timelock_duration_sec as u64 * 10u64.pow(9)).into();
    }

    /// Updates the proposal expiration duration in seconds.
    /// Set to 0 to disable proposal expiration.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_proposal_expiration(&mut self, proposal_expiration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.proposal_expiration_ns =
            (proposal_expiration_sec as u64 * 10u64.pow(9)).into();
    }

    /// Updates the quorum threshold in basis points (e.g. 3500 = 35%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_quorum_threshold_bps(&mut self, quorum_threshold_bps: u16) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            quorum_threshold_bps <= 10_000,
            "Quorum threshold must be <= 10000 bps"
        );
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
    pub fn set_approval_threshold_bps(&mut self, approval_threshold_bps: u16) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            approval_threshold_bps <= 10_000,
            "Approval threshold must be <= 10000 bps"
        );
        self.config.approval_threshold_bps = approval_threshold_bps;
    }

    /// Updates the v2 bond amount required to create a proposal.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_bond_amount(&mut self, bond_amount: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.bond_amount = bond_amount;
    }

    /// Updates the v2 simple majority threshold in basis points (e.g. 5000 = 50%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_simple_majority_threshold_bps(&mut self, simple_majority_threshold_bps: u16) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            simple_majority_threshold_bps <= 10_000,
            "Simple majority threshold must be <= 10000 bps"
        );
        self.config.simple_majority_threshold_bps = simple_majority_threshold_bps;
    }

    /// Updates the v2 strong (super) majority threshold in basis points (e.g. 6667 ≈ 66.67%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_strong_majority_threshold_bps(&mut self, strong_majority_threshold_bps: u16) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            strong_majority_threshold_bps <= 10_000,
            "Strong majority threshold must be <= 10000 bps"
        );
        self.config.strong_majority_threshold_bps = strong_majority_threshold_bps;
    }

    /// Updates the v2 sandbox duration in seconds.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_sandbox_duration(&mut self, sandbox_duration_sec: u32) {
        assert_one_yocto();
        self.assert_owner();
        self.config.sandbox_duration_ns = (sandbox_duration_sec as u64 * 10u64.pow(9)).into();
    }

    /// Updates the v2 sandbox threshold in basis points (e.g. 3000 = 30%).
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn set_sandbox_threshold_bps(&mut self, sandbox_threshold_bps: u16) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            sandbox_threshold_bps <= 10_000,
            "Sandbox threshold must be <= 10000 bps"
        );
        self.config.sandbox_threshold_bps = sandbox_threshold_bps;
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
