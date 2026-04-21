use crate::metadata::ProposalMetadata;
use crate::*;
use common::{TimestampNs, events, near_add, near_sub};
use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::{Gas, Promise};

pub type ProposalId = u32;

const NS_PER_DAY: u64 = 86_400_000_000_000;

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

/// Which proposal flow a proposal follows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum ProposalFlow {
    Classic,
    V2,
}

/// The majority type required for a v2 proposal to pass.
#[derive(Clone, Copy, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum MajorityType {
    /// Simple majority (e.g. >50%).
    Simple,
    /// Strong majority (e.g. >66.67%).
    Strong,
}

#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct Proposal {
    pub id: ProposalId,
    pub creation_time_ns: U64,
    pub proposer_id: AccountId,
    pub reviewer_id: Option<AccountId>,
    pub rejecter_id: Option<AccountId>,
    pub voting_start_time_ns: Option<U64>,
    pub voting_duration_ns: U64,
    pub expiration_ns: U64,
    pub snapshot_and_state: Option<SnapshotAndState>,
    pub votes: Vec<VoteStats>,
    pub total_votes: VoteStats,
    pub status: ProposalStatus,
    pub quorum_threshold_bps: u16,
    pub quorum_floor: NearToken,
    pub approval_threshold_bps: u16,
    pub actions: Option<Vec<ProposalAction>>,
    pub flow: ProposalFlow,
    // Classic only
    pub timelock_duration_ns: U64,
    // V2 only
    pub approval_time_ns: Option<U64>,
    pub bond_amount: NearToken,
    pub sandbox_duration_ns: U64,
    pub sandbox_threshold_bps: u16,
}

/// Borsh-tagged proposal storage envelope. The single `Current` variant wraps `Proposal`; the
/// enum is kept so future schema changes can append new variants without breaking reads.
#[derive(Clone)]
#[near(serializers=[borsh])]
pub enum VProposal {
    Current(Proposal),
}

impl From<Proposal> for VProposal {
    fn from(current: Proposal) -> Self {
        Self::Current(current)
    }
}

impl From<VProposal> for Proposal {
    fn from(v: VProposal) -> Self {
        match v {
            VProposal::Current(p) => p,
        }
    }
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
    /// The proposal was rejected by the council.
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
    /// Tthe proposal was slashed by a reviewer; bond is forfeited.
    Slashed,
    /// Graduates to Scheduled when the sandbox threshold is met.
    Sandbox,
    /// The proposal met the sandbox threshold and is scheduled to start voting.
    Scheduled,
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

/// Returns the next voting start time. In sandbox test mode, starts after 120 seconds.
/// Otherwise, starts on the next Monday 00:00 UTC strictly after `after_ns`.
pub fn next_voting_start_ns(after_ns: u64) -> u64 {
    if cfg!(feature = "sandbox") {
        after_ns + 120 * 1_000_000_000
    } else {
        let days_since_epoch = after_ns / NS_PER_DAY;
        let day_of_week = days_since_epoch % 7;
        let days_until_monday = (10 - day_of_week) % 7 + 1;
        (days_since_epoch + days_until_monday) * NS_PER_DAY
    }
}

impl Proposal {
    pub fn has_actions(&self) -> bool {
        self.actions.as_ref().is_some_and(|a| !a.is_empty())
    }

    pub fn sandbox_threshold_met(&self) -> bool {
        let total_supply = self
            .snapshot_and_state
            .as_ref()
            .unwrap()
            .total_venear
            .as_yoctonear();
        let for_power = self
            .votes
            .first()
            .map(|v| v.total_venear.as_yoctonear())
            .unwrap_or(0);
        let threshold = total_supply * (self.sandbox_threshold_bps as u128) / 10_000;
        for_power >= threshold
    }

    pub fn update(&mut self, timestamp: TimestampNs) {
        match self.flow {
            ProposalFlow::Classic => self.update_classic(timestamp),
            ProposalFlow::V2 => self.update_v2(timestamp),
        }
    }

    fn update_classic(&mut self, timestamp: TimestampNs) {
        match self.status {
            ProposalStatus::Rejected
            | ProposalStatus::Succeeded
            | ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed => {}
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
            ProposalStatus::Slashed | ProposalStatus::Sandbox | ProposalStatus::Scheduled => {
                // Not reachable for classic proposals.
            }
        }
    }

    fn update_v2(&mut self, timestamp: TimestampNs) {
        match self.status {
            ProposalStatus::Rejected
            | ProposalStatus::Succeeded
            | ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed
            | ProposalStatus::Slashed => {}
            ProposalStatus::Created => {
                if self.expiration_ns.0 > 0 && timestamp.0 >= self.expiration_ns.0 {
                    self.status = ProposalStatus::Expired;
                }
            }
            ProposalStatus::Sandbox => {
                let sandbox_end = self.approval_time_ns.unwrap().0 + self.sandbox_duration_ns.0;
                if timestamp.0 >= sandbox_end {
                    self.status = ProposalStatus::Defeated;
                }
            }
            ProposalStatus::Scheduled | ProposalStatus::Voting => {
                let voting_start = self.voting_start_time_ns.unwrap().0;
                let voting_end = voting_start + self.voting_duration_ns.0;
                if timestamp.0 >= voting_end {
                    let final_status = self.compute_final_status();
                    self.status = if final_status != ProposalStatus::Succeeded {
                        final_status
                    } else if self.has_actions() {
                        ProposalStatus::Executable
                    } else {
                        ProposalStatus::Succeeded
                    };
                } else if timestamp.0 >= voting_start {
                    self.status = ProposalStatus::Voting;
                }
            }
            ProposalStatus::Timelock => {
                // Not reachable for v2 proposals.
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
    /// Creates a new proposal of the given flow.
    ///
    /// * Classic: requires a deposit covering the storage and `base_proposal_fee`.
    /// * V2: requires a deposit covering the storage and `bond_amount`. The bond stays locked on
    ///   the proposal and is recoverable via `claim_bond` once the proposal reaches a terminal
    ///   non-slashed state.
    #[payable]
    pub fn create_proposal(
        &mut self,
        metadata: ProposalMetadata,
        actions: Option<Vec<ProposalAction>>,
        flow: ProposalFlow,
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
        let (timelock_duration_ns, bond_amount, flow_fee) = match flow {
            ProposalFlow::Classic => (
                self.config.timelock_duration_ns,
                NearToken::from_yoctonear(0),
                self.config.base_proposal_fee,
            ),
            ProposalFlow::V2 => (U64(0), self.config.bond_amount, self.config.bond_amount),
        };
        let proposal = Proposal {
            id: proposal_id,
            creation_time_ns: creation_time_ns.into(),
            proposer_id,
            reviewer_id: None,
            rejecter_id: None,
            voting_start_time_ns: None,
            voting_duration_ns: self.config.voting_duration_ns,
            expiration_ns,
            snapshot_and_state: None,
            votes: vec![VoteStats::default(); 3],
            total_votes: VoteStats::default(),
            status: ProposalStatus::Created,
            quorum_threshold_bps: 0,
            quorum_floor: NearToken::from_yoctonear(0),
            approval_threshold_bps: 0,
            actions,
            flow,
            timelock_duration_ns,
            approval_time_ns: None,
            bond_amount,
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: 0,
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
        let required_deposit = near_add(flow_fee, storage_added_cost);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date_ns(year: i32, month: u32, day: u32) -> u64 {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_nanos_opt()
            .unwrap() as u64
    }

    #[test]
    fn test_next_monday_from_each_weekday() {
        // 2026-04-06 is Monday, next Monday is 2026-04-13
        let expected = date_ns(2026, 4, 13);

        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 6)), expected); // Monday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 7)), expected); // Tuesday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 8)), expected); // Wednesday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 9)), expected); // Thursday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 10)), expected); // Friday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 11)), expected); // Saturday
        assert_eq!(next_voting_start_ns(date_ns(2026, 4, 12)), expected); // Sunday
    }

    #[test]
    fn test_next_monday_from_time_within_day() {
        let expected = date_ns(2026, 4, 13);
        assert_eq!(
            next_voting_start_ns(date_ns(2026, 4, 6) + 12 * 3600 * 1_000_000_000),
            expected
        );
        assert_eq!(
            next_voting_start_ns(date_ns(2026, 4, 7) - 1_000_000_000),
            expected
        );
        assert_eq!(
            next_voting_start_ns(date_ns(2026, 4, 12) + 12 * 3600 * 1_000_000_000),
            expected
        );
        assert_eq!(
            next_voting_start_ns(date_ns(2026, 4, 13) - 1_000_000_000),
            expected
        );
    }
}
