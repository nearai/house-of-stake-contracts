use crate::metadata::ProposalMetadata;
use crate::*;
use common::{TimestampNs, events, near_add, near_sub};
use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::{Gas, Promise};

pub type ProposalId = u32;

/// A single action that the voting contract can execute on behalf of a passed proposal.
#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub enum ProposalAction {
    FunctionCall {
        receiver_id: AccountId,
        method_name: String,
        args: Base64VecU8,
        deposit: NearToken,
        gas: Gas,
    },
    Transfer {
        receiver_id: AccountId,
        amount: NearToken,
    },
}

/// The fixed voting options for proposals.
#[derive(Clone, Copy, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum VoteOption {
    For,
    Against,
    Abstain,
}

/// The old proposal structure (V1) that includes the `rejected` field.
#[derive(Clone)]
#[near(serializers=[borsh])]
pub struct ProposalV1 {
    pub id: ProposalId,
    pub creation_time_ns: U64,
    pub proposer_id: AccountId,
    pub reviewer_id: Option<AccountId>,
    pub voting_start_time_ns: Option<U64>,
    pub voting_duration_ns: U64,
    pub rejected: bool,
    pub snapshot_and_state: Option<SnapshotAndState>,
    pub votes: Vec<VoteStats>,
    pub total_votes: VoteStats,
    pub status: ProposalStatus,
}

#[derive(Clone)]
#[near(serializers=[borsh])]
pub enum VProposal {
    V1(ProposalV1),
    Current(Proposal),
}

impl From<Proposal> for VProposal {
    fn from(current: Proposal) -> Self {
        Self::Current(current)
    }
}

impl From<VProposal> for Proposal {
    fn from(value: VProposal) -> Self {
        match value {
            VProposal::V1(v1) => Proposal {
                id: v1.id,
                creation_time_ns: v1.creation_time_ns,
                proposer_id: v1.proposer_id,
                reviewer_id: v1.reviewer_id,
                rejecter_id: None,
                voting_start_time_ns: v1.voting_start_time_ns,
                voting_duration_ns: v1.voting_duration_ns,
                timelock_duration_ns: U64(0),
                expiration_ns: U64(0),
                snapshot_and_state: v1.snapshot_and_state,
                votes: v1.votes,
                total_votes: v1.total_votes,
                status: v1.status,
                quorum_threshold_bps: 0,
                quorum_floor: NearToken::from_yoctonear(0),
                approval_threshold_bps: 0,
                actions: None,
            },
            VProposal::Current(current) => current,
        }
    }
}

/// The proposal structure that contains all the information about a proposal.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct Proposal {
    /// The unique identifier of the proposal, generated automatically.
    pub id: ProposalId,
    /// The timestamp in nanoseconds when the proposal was created, generated automatically.
    pub creation_time_ns: U64,
    /// The account ID of the proposer.
    pub proposer_id: AccountId,
    /// The account ID of the reviewer, who approved the proposal.
    pub reviewer_id: Option<AccountId>,
    /// The account ID of the council member who rejected (vetoed) the proposal.
    pub rejecter_id: Option<AccountId>,
    /// The timestamp when the voting starts.
    pub voting_start_time_ns: Option<U64>,
    /// The voting duration in nanoseconds, generated from the config.
    pub voting_duration_ns: U64,
    /// The duration of the timelock period in nanoseconds, stored per-proposal from config.
    pub timelock_duration_ns: U64,
    /// The deadline in nanoseconds by which the proposal must be approved. 0 means no expiration.
    pub expiration_ns: U64,
    /// The snapshot of the contract state and global state. Fetched when the proposal is approved.
    pub snapshot_and_state: Option<SnapshotAndState>,
    /// Aggregated votes per voting option.
    pub votes: Vec<VoteStats>,
    /// The total aggregated voting information across all voting options.
    pub total_votes: VoteStats,
    /// The status of the proposal. It's optional and can be computed from the proposal itself.
    pub status: ProposalStatus,
    /// Quorum threshold in basis points.
    pub quorum_threshold_bps: u16,
    /// Absolute minimum veNEAR required for quorum.
    pub quorum_floor: NearToken,
    /// Approval threshold in basis points.
    pub approval_threshold_bps: u16,
    /// Optional list of on-chain actions to execute when the proposal succeeds.
    pub actions: Option<Vec<ProposalAction>>,
}

/// The proposal information structure that contains the proposal and its metadata.
#[derive(Clone)]
#[near(serializers=[json])]
pub struct ProposalInfo {
    #[serde(flatten)]
    pub proposal: Proposal,
    #[serde(flatten)]
    pub metadata: ProposalMetadata,
}

/// The status of the proposal
#[derive(Clone, Copy, Debug, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum ProposalStatus {
    /// The proposal was created and is waiting for the approver to approve it.
    Created,
    /// The proposal was rejected by the council during the timelock period.
    Rejected,
    /// The proposal is in the voting phase.
    Voting,
    /// The proposal voting has finished, quorum was met and approval threshold was met.
    Succeeded,
    /// The voting has ended and the proposal is in the timelock period awaiting potential council veto.
    Timelock,
    /// The proposal expired before being approved by a reviewer.
    Expired,
    /// The proposal voting has finished, but quorum was not met or approval threshold was not met.
    Defeated,
    /// The proposal passed and has actions ready for on-chain execution.
    Executable,
    /// The proposal actions are being executed (dispatched, awaiting callback).
    InProgress,
    /// The proposal's on-chain execution failed.
    Failed,
}

/// The snapshot of the Merkle tree and the global state at the moment when the proposal was
/// approved.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct SnapshotAndState {
    /// The snapshot of the Merkle tree at the moment when the proposal was approved.
    pub snapshot: MerkleTreeSnapshot,
    /// The timestamp in nanoseconds when the global state was last updated.
    pub timestamp_ns: TimestampNs,
    /// The total amount of veNEAR tokens at the moment when the proposal was approved.
    pub total_venear: NearToken,
    /// The growth configuration of the veNEAR tokens from the global state.
    pub venear_growth_config: VenearGrowthConfig,
}

/// The vote statistics structure that contains the total amount of veNEAR tokens and the total
/// number of votes.
#[derive(Clone, Default)]
#[near(serializers=[borsh, json])]
pub struct VoteStats {
    /// The total venear balance at the updated timestamp.
    pub total_venear: NearToken,

    /// The total number of votes.
    pub total_votes: u32,
}

impl VoteStats {
    pub fn add_vote(&mut self, venear: NearToken) {
        self.total_votes += 1;
        self.total_venear = near_add(self.total_venear, venear);
    }

    pub fn remove_vote(&mut self, venear: NearToken) {
        self.total_votes -= 1;
        self.total_venear = near_sub(self.total_venear, venear);
    }
}

impl Proposal {
    pub fn has_actions(&self) -> bool {
        self.actions.as_ref().is_some_and(|a| !a.is_empty())
    }

    pub fn update(&mut self, timestamp: TimestampNs) {
        match self.status {
            ProposalStatus::Rejected
            | ProposalStatus::Succeeded
            | ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed => {
                return;
            }
            ProposalStatus::Created => {
                if self.expiration_ns.0 > 0 && timestamp.0 >= self.expiration_ns.0 {
                    self.status = ProposalStatus::Expired;
                }
            }
            ProposalStatus::Voting | ProposalStatus::Timelock => {
                let voting_end = self.voting_start_time_ns.unwrap().0 + self.voting_duration_ns.0;
                let timelock_end = voting_end + self.timelock_duration_ns.0;
                if timestamp.0 >= voting_end {
                    let final_status = self.compute_final_status();
                    self.status = if final_status != ProposalStatus::Succeeded {
                        final_status
                    } else if timestamp.0 < timelock_end {
                        ProposalStatus::Timelock
                    } else if self.has_actions() {
                        ProposalStatus::Executable
                    } else {
                        ProposalStatus::Succeeded
                    };
                }
            }
        }
    }

    pub fn compute_final_status(&self) -> ProposalStatus {
        // Quorum check
        let total_supply = self
            .snapshot_and_state
            .as_ref()
            .unwrap()
            .total_venear
            .as_yoctonear();
        let bps_quorum = total_supply * (self.quorum_threshold_bps as u128) / 10_000;
        let quorum_required = std::cmp::max(bps_quorum, self.quorum_floor.as_yoctonear());
        let quorum_met = self.total_votes.total_venear.as_yoctonear() >= quorum_required;

        // Approval threshold check: votes[0] = For, votes[1] = Against
        let for_power = self
            .votes
            .first()
            .map(|v| v.total_venear.as_yoctonear())
            .unwrap_or(0);
        let against_power = self
            .votes
            .get(1)
            .map(|v| v.total_venear.as_yoctonear())
            .unwrap_or(0);
        let denominator = for_power + against_power;
        // Cross-multiply to avoid division: for * 10000 >= threshold * (for + against)
        let approval_met = denominator > 0
            && for_power * 10_000 >= (self.approval_threshold_bps as u128) * denominator;

        if quorum_met && approval_met {
            ProposalStatus::Succeeded
        } else {
            ProposalStatus::Defeated
        }
    }
}

#[near]
impl Contract {
    /// Creates a new proposal with the given metadata.
    /// The proposal is created by the predecessor account and requires a deposit to cover the
    /// storage and the base proposal fee.
    #[payable]
    pub fn create_proposal(
        &mut self,
        metadata: ProposalMetadata,
        actions: Option<Vec<ProposalAction>>,
    ) -> ProposalId {
        self.assert_not_paused();
        let attached_deposit = env::attached_deposit();

        require!(
            !actions.as_ref().is_some_and(|a| a.is_empty()),
            "Actions list cannot be empty"
        );

        let proposer_id = env::predecessor_account_id();
        let proposal_id = self.proposals.len();

        events::emit::create_proposal_action("create_proposal", &proposer_id, proposal_id);

        let creation_time_ns: u64 = env::block_timestamp();
        let expiration_ns = if self.config.proposal_expiration_ns.0 > 0 {
            U64(creation_time_ns + self.config.proposal_expiration_ns.0)
        } else {
            U64(0)
        };
        let proposal = Proposal {
            id: proposal_id,
            creation_time_ns: creation_time_ns.into(),
            proposer_id,
            reviewer_id: None,
            rejecter_id: None,
            voting_start_time_ns: None,
            voting_duration_ns: self.config.voting_duration_ns,
            timelock_duration_ns: self.config.timelock_duration_ns,
            expiration_ns,
            snapshot_and_state: None,
            votes: vec![VoteStats::default(); 3],
            total_votes: VoteStats::default(),
            status: ProposalStatus::Created,
            quorum_threshold_bps: 0,
            quorum_floor: NearToken::from_yoctonear(0),
            approval_threshold_bps: 0,
            actions,
        };
        let storage_usage = env::storage_usage();
        self.proposals.push(proposal.into());
        self.proposals.flush();
        self.proposal_metadata.push(metadata.into());
        self.proposal_metadata.flush();
        let updated_storage_usage = env::storage_usage();
        let storage_added = updated_storage_usage.saturating_sub(storage_usage);
        let storage_added_cost = env::storage_byte_cost()
            .checked_mul(storage_added as _)
            .unwrap();
        let required_deposit = near_add(self.config.base_proposal_fee, storage_added_cost);
        require!(
            attached_deposit >= required_deposit,
            format!(
                "Requires deposit of {}",
                required_deposit.exact_amount_display()
            )
        );
        if attached_deposit > required_deposit {
            let refund = near_sub(attached_deposit, required_deposit);
            Promise::new(env::predecessor_account_id()).transfer(refund);
        }
        proposal_id
    }

    /// Returns the proposal information by the given proposal ID.
    pub fn get_proposal(&self, proposal_id: ProposalId) -> Option<ProposalInfo> {
        self.internal_get_proposal(proposal_id)
            .map(|proposal| ProposalInfo {
                proposal,
                metadata: self
                    .proposal_metadata
                    .get(proposal_id)
                    .unwrap()
                    .clone()
                    .into(),
            })
    }

    /// Returns the number of proposals.
    pub fn get_num_proposals(&self) -> u32 {
        self.proposals.len()
    }

    /// Returns a list of proposals from the given index based on the proposal ID order.
    pub fn get_proposals(&self, from_index: u32, limit: Option<u32>) -> Vec<ProposalInfo> {
        let from_index = from_index;
        let limit = limit.unwrap_or(u32::MAX);
        let to_index = std::cmp::min(from_index.saturating_add(limit), self.get_num_proposals());
        (from_index..to_index)
            .into_iter()
            .filter_map(|i| self.get_proposal(i))
            .collect()
    }

    /// Returns the number of approved proposals.
    pub fn get_num_approved_proposals(&self) -> u32 {
        self.approved_proposals.len()
    }

    /// Returns a list of approved proposals from the given index based on the approved proposals
    /// order.
    pub fn get_approved_proposals(&self, from_index: u32, limit: Option<u32>) -> Vec<ProposalInfo> {
        let from_index = from_index;
        let limit = limit.unwrap_or(u32::MAX);
        let to_index = std::cmp::min(
            from_index.saturating_add(limit),
            self.get_num_approved_proposals(),
        );
        (from_index..to_index)
            .into_iter()
            .filter_map(|i| self.get_proposal(self.approved_proposals[i]))
            .collect()
    }
}

impl Contract {
    pub fn internal_set_proposal(&mut self, proposal: Proposal) {
        let proposal_id = proposal.id;
        self.proposals[proposal_id] = proposal.into();
    }

    pub fn internal_get_proposal(&self, proposal_id: ProposalId) -> Option<Proposal> {
        self.proposals.get(proposal_id).cloned().map(|proposal| {
            let mut proposal: Proposal = proposal.into();
            proposal.update(env::block_timestamp().into());
            proposal
        })
    }

    pub fn internal_expect_proposal_updated(&self, proposal_id: ProposalId) -> Proposal {
        self.internal_get_proposal(proposal_id)
            .expect(format!("Proposal {} is not found", proposal_id).as_str())
    }
}
