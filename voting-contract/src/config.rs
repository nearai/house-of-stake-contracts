use crate::*;
use near_sdk::json_types::U64;

/// The configuration of the voting contract.
#[derive(Debug, Clone)]
#[near(serializers=[borsh, json])]
pub struct Config {
    /// The account ID of the veNEAR contract.
    pub venear_account_id: AccountId,

    /// The account IDs that can approve proposals.
    pub reviewer_ids: Vec<AccountId>,

    /// The account IDs of the council members who can veto proposals.
    pub council_ids: Vec<AccountId>,

    /// The account ID that can upgrade the current contract and modify the config.
    pub owner_account_id: AccountId,

    /// The maximum duration of the voting period in nanoseconds.
    pub voting_duration_ns: U64,

    /// The duration of the timelock period in nanoseconds after voting ends.
    pub timelock_duration_ns: U64,

    /// The base fee in addition to the storage fee required to create a proposal.
    pub base_proposal_fee: NearToken,

    /// The bond amount required to create a v2 proposal.
    pub bond_amount: NearToken,

    /// Storage fee required to store a vote for an active proposal. It can be refunded once the
    /// proposal is finalized.
    pub vote_storage_fee: NearToken,

    /// The list of account IDs that can pause the contract.
    pub guardians: Vec<AccountId>,

    /// The maximum time in nanoseconds a proposal can stay in Created status before expiring.
    /// 0 means no expiration.
    pub proposal_expiration_ns: U64,

    /// Proposed new owner account ID. The account has to accept ownership.
    pub proposed_new_owner_account_id: Option<AccountId>,

    /// Quorum threshold in basis points (e.g. 3500 = 35% of total veNEAR supply).
    pub quorum_threshold_bps: u16,

    /// Absolute minimum veNEAR required for quorum, regardless of BPS calculation.
    pub quorum_floor: NearToken,

    /// Approval threshold in basis points for the classic flow (e.g. 5000 = 50%).
    /// Applied as: for_votes / (for_votes + against_votes) >= approval_threshold_bps / 10000.
    pub approval_threshold_bps: u16,

    /// Simple majority threshold in basis points for v2 proposals (e.g. 5000 = 50%).
    pub simple_majority_threshold_bps: u16,

    /// Strong (super) majority threshold in basis points for v2 proposals (e.g. 6667 ≈ 66.67%).
    pub strong_majority_threshold_bps: u16,

    /// The duration of the sandbox pre-voting period in nanoseconds for v2 proposals.
    pub sandbox_duration_ns: U64,

    /// The "For" votes threshold to graduate a v2 proposal from Sandbox to Scheduled.
    pub sandbox_threshold_bps: u16,

    /// Maximum number of proposals allowed in Sandbox/Scheduled/Voting simultaneously.
    /// Extra approved proposals park in the pending queue until a slot frees.
    pub max_active_proposals: u32,
}

#[near]
impl Contract {
    /// Returns the current contract configuration.
    pub fn get_config(&self) -> &Config {
        &self.config
    }
}
