use crate::metadata::ProposalMetadata;
use crate::*;
use common::{Bps, TimestampNs, events, near_add, near_sub};
use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::{Gas, Promise};

pub use common::voting::{MajorityType, ProposalStatus, VoteOption};

pub type ProposalId = u32;

const NS_PER_DAY: u64 = 86_400_000_000_000;

/// Bytes reserved at proposal-creation time to cover the future `SnapshotAndState` write.
const SNAPSHOT_STORAGE_RESERVE_BYTES: u64 = 101;

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

/// Which proposal flow a proposal follows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum ProposalFlow {
    Classic,
    FastTrack,
}

#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct Proposal {
    pub id: ProposalId,
    pub creation_time_ns: U64,
    pub proposer_id: AccountId,
    pub reviewer_id: Option<AccountId>,
    pub rejecter_id: Option<AccountId>,
    pub approval_time_ns: Option<U64>,
    pub voting_start_time_ns: Option<U64>,
    pub voting_duration_ns: U64,
    pub expiration_ns: U64,
    pub snapshot_and_state: Option<SnapshotAndState>,
    pub votes: Vec<VoteStats>,
    pub total_votes: VoteStats,
    pub status: ProposalStatus,
    pub quorum_threshold_bps: Bps,
    pub quorum_floor: NearToken,
    pub approval_threshold_bps: Bps,
    pub actions: Option<Vec<ProposalAction>>,
    pub flow: ProposalFlow,
    // Classic only
    pub timelock_duration_ns: U64,
    // FastTrack only
    pub sandbox_start_time_ns: Option<U64>,
    pub bond_amount: NearToken,
    pub sandbox_duration_ns: U64,
    pub sandbox_threshold_bps: Bps,
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
/// Otherwise, starts on the next Monday 00:00 CET (fixed UTC+1) strictly after `after_ns`.
pub fn next_voting_start_ns(after_ns: u64) -> u64 {
    if cfg!(feature = "sandbox") {
        after_ns + 120 * 1_000_000_000
    } else {
        // CET = UTC+1. Compute the boundary in CET-shifted coords, then shift back.
        const CET_OFFSET_NS: u64 = 3600 * 1_000_000_000;
        let shifted = after_ns + CET_OFFSET_NS;
        let days_since_epoch = shifted / NS_PER_DAY;
        let day_of_week = days_since_epoch % 7;
        let days_until_monday = (10 - day_of_week) % 7 + 1;
        (days_since_epoch + days_until_monday) * NS_PER_DAY - CET_OFFSET_NS
    }
}

impl Proposal {
    pub fn has_actions(&self) -> bool {
        self.actions.as_ref().is_some_and(|a| !a.is_empty())
    }

    /// Transitions a proposal into its first active status.
    pub fn activate(&mut self, start_time: U64) {
        match self.flow {
            ProposalFlow::Classic => {
                self.voting_start_time_ns = Some(start_time);
                self.status = ProposalStatus::Voting;
            }
            ProposalFlow::FastTrack => {
                self.sandbox_start_time_ns = Some(start_time);
                self.status = ProposalStatus::Sandbox;
            }
        }
    }

    /// Latest timestamp at which an active proposal still occupies its slot. Used by the
    /// scheduler to backdate the start of a queued proposal that fills a freed slot.
    pub fn active_end_time_ns(&self) -> u64 {
        match self.flow {
            ProposalFlow::Classic => {
                let voting_end = self.voting_start_time_ns.unwrap().0 + self.voting_duration_ns.0;
                if self.compute_final_status() == ProposalStatus::Succeeded {
                    voting_end + self.timelock_duration_ns.0
                } else {
                    voting_end
                }
            }
            ProposalFlow::FastTrack => {
                if let Some(voting_start) = self.voting_start_time_ns {
                    voting_start.0 + self.voting_duration_ns.0
                } else {
                    self.sandbox_start_time_ns.unwrap().0 + self.sandbox_duration_ns.0
                }
            }
        }
    }

    pub fn sandbox_threshold_met(&self) -> bool {
        let Some(snapshot) = self.snapshot_and_state.as_ref() else {
            return false;
        };
        let for_power = self
            .votes
            .first()
            .map(|v| v.total_venear)
            .unwrap_or(NearToken::from_yoctonear(0));
        let threshold = self.sandbox_threshold_bps * snapshot.total_venear;
        for_power >= threshold
    }

    pub fn update(&mut self, timestamp: TimestampNs) {
        match self.flow {
            ProposalFlow::Classic => self.update_classic(timestamp),
            ProposalFlow::FastTrack => self.update_fast_track(timestamp),
        }
    }

    fn update_classic(&mut self, timestamp: TimestampNs) {
        match self.status {
            ProposalStatus::Rejected
            | ProposalStatus::Vetoed
            | ProposalStatus::Succeeded
            | ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed
            | ProposalStatus::ApprovalLegacy
            | ProposalStatus::FinishLegacy => {}
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
            ProposalStatus::Queued => {
                // Queued waits for explicit promotion by the scheduler.
            }
        }
    }

    fn update_fast_track(&mut self, timestamp: TimestampNs) {
        match self.status {
            ProposalStatus::Rejected
            | ProposalStatus::Succeeded
            | ProposalStatus::Expired
            | ProposalStatus::Defeated
            | ProposalStatus::Executable
            | ProposalStatus::InProgress
            | ProposalStatus::Failed
            | ProposalStatus::Slashed
            | ProposalStatus::Vetoed
            | ProposalStatus::ApprovalLegacy
            | ProposalStatus::FinishLegacy => {}
            ProposalStatus::Created => {
                if self.expiration_ns.0 > 0 && timestamp.0 >= self.expiration_ns.0 {
                    self.status = ProposalStatus::Expired;
                }
            }
            ProposalStatus::Sandbox => {
                let sandbox_end =
                    self.sandbox_start_time_ns.unwrap().0 + self.sandbox_duration_ns.0;
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
                // Not reachable for FastTrack proposals.
            }
            ProposalStatus::Queued => {
                // Queued waits for explicit promotion by the scheduler.
            }
        }
    }

    pub fn compute_final_status(&self) -> ProposalStatus {
        let Some(snapshot) = self.snapshot_and_state.as_ref() else {
            return ProposalStatus::Defeated;
        };
        // Quorum check
        let bps_quorum = self.quorum_threshold_bps * snapshot.total_venear;
        let quorum_required = std::cmp::max(bps_quorum, self.quorum_floor);
        let quorum_met = self.total_votes.total_venear >= quorum_required;

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
            && for_power * 10_000 >= u128::from(self.approval_threshold_bps) * denominator;

        if quorum_met && approval_met {
            ProposalStatus::Succeeded
        } else {
            ProposalStatus::Defeated
        }
    }
}

#[near]
impl Contract {
    /// Creates a new proposal. Deposit covers storage, `base_proposal_fee`, and (FastTrack only)
    /// `bond_amount`.
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
        let (flow_expiration_ns, voting_duration_ns, timelock_duration_ns, bond_amount) = match flow
        {
            ProposalFlow::Classic => (
                self.config.proposal_expiration_ns,
                self.config.classic_voting_duration_ns,
                self.config.timelock_duration_ns,
                NearToken::from_yoctonear(0),
            ),
            ProposalFlow::FastTrack => (
                self.config.fast_track_proposal_expiration_ns,
                self.config.fast_track_voting_duration_ns,
                U64(0),
                self.config.bond_amount,
            ),
        };
        let expiration_ns = if flow_expiration_ns.0 > 0 {
            U64(creation_time_ns + flow_expiration_ns.0)
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
            voting_duration_ns,
            expiration_ns,
            snapshot_and_state: None,
            votes: vec![VoteStats::default(); 3],
            total_votes: VoteStats::default(),
            status: ProposalStatus::Created,
            quorum_threshold_bps: Bps::ZERO,
            quorum_floor: NearToken::from_yoctonear(0),
            approval_threshold_bps: Bps::ZERO,
            actions,
            flow,
            approval_time_ns: None,
            timelock_duration_ns,
            sandbox_start_time_ns: None,
            bond_amount,
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: Bps::ZERO,
        };
        let storage_usage = env::storage_usage();
        self.proposals.push(proposal.into());
        self.proposals.flush();
        self.proposal_metadata.push(metadata.into());
        self.proposal_metadata.flush();
        let updated_storage_usage = env::storage_usage() + SNAPSHOT_STORAGE_RESERVE_BYTES;
        let storage_added = updated_storage_usage.saturating_sub(storage_usage);
        let storage_added_cost = env::storage_byte_cost()
            .checked_mul(u128::from(storage_added))
            .unwrap();
        let required_deposit = near_add(
            near_add(bond_amount, self.config.base_proposal_fee),
            storage_added_cost,
        );
        require!(
            attached_deposit >= required_deposit,
            format!(
                "Requires deposit of {}",
                required_deposit.exact_amount_display()
            )
        );
        if attached_deposit > required_deposit {
            let refund = near_sub(attached_deposit, required_deposit);
            Promise::new(env::predecessor_account_id())
                .transfer(refund)
                .detach();
        }
        proposal_id
    }

    pub fn get_proposal(&self, proposal_id: ProposalId) -> Option<ProposalInfo> {
        let proposal = self
            .get_proposals_virtual_updates()
            .remove(&proposal_id)
            .or_else(|| self.internal_get_proposal(proposal_id).map(|(p, _)| p))?;
        Some(ProposalInfo {
            proposal,
            metadata: self.proposal_metadata.get(proposal_id)?.clone().into(),
        })
    }

    /// Returns the number of proposals.
    pub fn get_num_proposals(&self) -> u32 {
        self.proposals.len()
    }

    pub fn get_proposals(&self, from_index: u32, limit: Option<u32>) -> Vec<ProposalInfo> {
        let mut overrides = self.get_proposals_virtual_updates();
        let limit = limit.unwrap_or(u32::MAX);
        let to_index = std::cmp::min(from_index.saturating_add(limit), self.get_num_proposals());
        (from_index..to_index)
            .map(|i| {
                let proposal = overrides
                    .remove(&i)
                    .unwrap_or_else(|| self.internal_get_proposal(i).unwrap().0);
                ProposalInfo {
                    proposal,
                    metadata: self.proposal_metadata.get(i).unwrap().clone().into(),
                }
            })
            .collect()
    }
}

/// A proposal occupies an active slot while it is in Sandbox, Timelock, Scheduled, or Voting status.
pub fn is_active_status(status: ProposalStatus) -> bool {
    matches!(
        status,
        ProposalStatus::Sandbox
            | ProposalStatus::Scheduled
            | ProposalStatus::Voting
            | ProposalStatus::Timelock
    )
}

impl Contract {
    pub fn internal_set_proposal(&mut self, proposal: Proposal) {
        let proposal_id = proposal.id;
        let new_active = is_active_status(proposal.status);
        let was_active = self.active_proposals.contains(&proposal_id);
        if !was_active && new_active {
            self.active_proposals.insert(proposal_id);
        } else if was_active && !new_active {
            self.active_proposals.remove(&proposal_id);
        }

        self.proposals[proposal_id] = proposal.into();
    }

    pub fn internal_get_proposal(&self, proposal_id: ProposalId) -> Option<(Proposal, bool)> {
        self.proposals.get(proposal_id).cloned().map(|proposal| {
            let mut proposal: Proposal = proposal.into();
            let old_status = proposal.status;
            proposal.update(env::block_timestamp().into());
            let changed = proposal.status != old_status;
            (proposal, changed)
        })
    }

    pub fn internal_expect_proposal_updated(&self, proposal_id: ProposalId) -> Proposal {
        self.internal_get_proposal(proposal_id)
            .expect(format!("Proposal {} is not found", proposal_id).as_str())
            .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::date_ns;

    #[test]
    fn test_next_monday_from_each_weekday() {
        // 2026-04-06 is Monday CET, next Monday CET is 2026-04-13.
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
        // All inputs lie within Tuesday 2026-04-07 CET; result is next Monday CET.
        let expected = date_ns(2026, 4, 13);
        let day = date_ns(2026, 4, 7);
        assert_eq!(next_voting_start_ns(day), expected);
        assert_eq!(next_voting_start_ns(day + 1), expected);
        assert_eq!(
            next_voting_start_ns(day + 12 * 3600 * 1_000_000_000),
            expected
        );
        assert_eq!(
            next_voting_start_ns(day + 24 * 3600 * 1_000_000_000 - 1),
            expected
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    //! Contract-level boundary tests for the proposal lifecycle.
    //!
    //! Every test here drives state through the public `Contract` API
    //! (`create_proposal` / `approve_proposal` / `vote` / `get_proposal`),
    //! then observes the resulting status via `contract.get_proposal()`.
    //! Reading through `get_proposal` is what triggers the implicit
    //! `update()` call, so the equivalence classes for time-driven status
    //! transitions are exercised through the same code path users see.
    //!
    //! Boundary values are documented inline next to each test.

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn approve_classic_proposal_moves_to_voting_status() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(100)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Voting
        );
    }

    #[test]
    fn approve_fasttrack_proposal_moves_to_sandbox_status() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(100)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Sandbox
        );
    }

    #[test]
    fn classic_voting_before_end_is_still_voting() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end - 1),
            ProposalStatus::Voting
        );
    }

    #[test]
    fn classic_voting_at_end_with_actions_enters_timelock() {
        // Pre-set a non-empty action list at creation time so the post-voting
        // branch hits the Timelock arm.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(
            proposer(),
            NearToken::from_near(200).as_yoctonear(),
            TEST_NOW_NS,
        );
        let id = contract.create_proposal(
            ProposalMetadata {
                title: Some("t".to_string()),
                description: None,
                link: None,
            },
            Some(vec![ProposalAction::Transfer {
                receiver_id: acc("dest.test.near"),
                amount: NearToken::from_near(1),
            }]),
            ProposalFlow::Classic,
        );
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Timelock
        );
    }

    #[test]
    fn classic_voting_with_zero_timelock_skips_to_succeeded() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_timelock_duration(0);
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn classic_voting_after_end_failing_quorum_is_defeated() {
        // No votes cast -> total_votes = 0 -> below the 35% quorum.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn classic_timelock_then_succeeded_signaling_only() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        let timelock_end = voting_end + default_config().timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, timelock_end - 1),
            ProposalStatus::Timelock
        );
        assert_eq!(
            status_at(&contract, id, timelock_end),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn created_proposal_expires_exactly_at_deadline() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_proposal_expiration(3600);
        let id = create_proposal(&mut contract, ProposalFlow::Classic);

        let expiration_ns = TEST_NOW_NS + contract.get_config().proposal_expiration_ns.0;
        assert_eq!(
            status_at(&contract, id, expiration_ns - 1),
            ProposalStatus::Created
        );
        assert_eq!(
            status_at(&contract, id, expiration_ns),
            ProposalStatus::Expired
        );
    }

    #[test]
    fn created_proposal_with_disabled_expiration_stays_created() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_proposal_expiration(0);
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        let very_far = TEST_NOW_NS + 10 * 365 * 24 * 3600 * 1_000_000_000;
        assert_eq!(status_at(&contract, id, very_far), ProposalStatus::Created);
    }

    #[test]
    fn sandbox_at_duration_end_becomes_defeated() {
        // Reviewer approves a fast-track proposal but no voter ever clears
        // the sandbox threshold; once the sandbox window elapses, the
        // proposal is Defeated regardless of any partial For votes.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        // 50 < 100 (10% of 1000) so the threshold isn't met.
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let sandbox_end = TEST_NOW_NS + default_config().sandbox_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, sandbox_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn sandbox_threshold_met_promotes_to_scheduled() {
        // For = 300 NEAR exactly hits the 30% sandbox bps threshold of the
        // 1 000 NEAR supply.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(300)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Scheduled
        );
    }

    #[test]
    fn sandbox_threshold_one_yocto_short_stays_in_sandbox() {
        let threshold = NearToken::from_near(300);
        let just_below = NearToken::from_yoctonear(threshold.as_yoctonear() - 1);
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), just_below),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Sandbox
        );
    }

    #[test]
    fn voting_outcome_quorum_exact_and_approval_exact_succeeds() {
        // total = 1 000 NEAR, quorum 35% -> 350 NEAR.
        // for-voter = 175, against-voter = 175 -> total = 350 (quorum exact),
        // approval = 50% (exact, succeeds at >= 5000 bps).
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(175)),
                VoterSpec::new(against_voter(), NearToken::from_near(175)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        cast_vote(
            &mut contract,
            &fixture,
            against_voter(),
            id,
            VoteOption::Against,
        );

        // After timelock, the signaling-only proposal lands as Succeeded.
        let cfg = default_config();
        let after_timelock =
            TEST_NOW_NS + cfg.classic_voting_duration_ns.0 + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, after_timelock),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn voting_outcome_approval_one_yocto_below_is_defeated() {
        // For = 200 NEAR - 1 yocto, Against = 200 NEAR. Quorum (35%) trivially met,
        // but approval is just below 50% so the proposal must lose.
        let for_v = NearToken::from_yoctonear(NearToken::from_near(200).as_yoctonear() - 1);
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), for_v),
                VoterSpec::new(against_voter(), NearToken::from_near(200)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        cast_vote(
            &mut contract,
            &fixture,
            against_voter(),
            id,
            VoteOption::Against,
        );

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn voting_outcome_pure_abstain_is_defeated_even_with_quorum() {
        // 500 NEAR Abstain meets the 35% quorum (350 NEAR), but For+Against
        // is empty so the approval denominator is zero — Defeated.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(0)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(500)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(
            &mut contract,
            &fixture,
            abstain_voter(),
            id,
            VoteOption::Abstain,
        );

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn voting_outcome_quorum_floor_dominates_bps_when_higher() {
        // bps quorum 35% of 1 000 = 350 NEAR. Floor = 400 NEAR overrides.
        // For + Against = 350 NEAR -> below floor -> Defeated.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(175)),
                VoterSpec::new(against_voter(), NearToken::from_near(175)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_quorum_floor(NearToken::from_near(400));
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        cast_vote(
            &mut contract,
            &fixture,
            against_voter(),
            id,
            VoteOption::Against,
        );

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn voting_outcome_for_only_succeeds() {
        // For = 400, Against = 0. Quorum met (400 >= 350), denominator = 400,
        // approval = 100% > 50%. Pure-For winning path.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let cfg = default_config();
        let after_timelock =
            TEST_NOW_NS + cfg.classic_voting_duration_ns.0 + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, after_timelock),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn voting_outcome_against_only_is_defeated() {
        // For = 0, Against = 400. Quorum met (400 >= 350) but denominator = 400,
        // approval = 0% — distinct from abstain-only (where denominator = 0).
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(against_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(
            &mut contract,
            &fixture,
            against_voter(),
            id,
            VoteOption::Against,
        );

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn voting_outcome_quorum_one_yocto_below_is_defeated() {
        // Quorum = 35% of 1000 = 350 NEAR. Cast 350 NEAR - 1 yocto total. Below.
        let for_amount = NearToken::from_yoctonear(
            NearToken::from_near(350).as_yoctonear() - 1,
        );
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), for_amount)],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_end = TEST_NOW_NS + default_config().classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn fasttrack_voting_after_end_failing_quorum_is_defeated() {
        // For = 300 (30% sandbox threshold met -> Scheduled). Quorum is 35% =
        // 350, so 300 < quorum -> Defeated at FastTrack voting_end.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(300))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Scheduled
        );

        let voting_start = next_voting_start_ns(TEST_NOW_NS);
        let voting_end = voting_start + default_config().fast_track_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Defeated
        );
    }

    #[test]
    fn fasttrack_voting_with_actions_at_end_is_executable() {
        // FastTrack with an action: For=400 passes both sandbox threshold and
        // simple-majority quorum/approval, lands Executable at voting_end.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(
            proposer(),
            NearToken::from_near(200).as_yoctonear(),
            TEST_NOW_NS,
        );
        let id = contract.create_proposal(
            ProposalMetadata {
                title: Some("ft-actions".to_string()),
                description: None,
                link: None,
            },
            Some(vec![ProposalAction::Transfer {
                receiver_id: acc("dest.test.near"),
                amount: NearToken::from_near(1),
            }]),
            ProposalFlow::FastTrack,
        );
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, Some(MajorityType::Simple));
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let voting_start = next_voting_start_ns(TEST_NOW_NS);
        let voting_end = voting_start + default_config().fast_track_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Executable
        );
    }

    #[test]
    fn fasttrack_created_proposal_expires_exactly_at_deadline() {
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_fast_track_proposal_expiration(3600);
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);

        let expiration_ns = TEST_NOW_NS + contract.get_config().fast_track_proposal_expiration_ns.0;
        assert_eq!(
            status_at(&contract, id, expiration_ns - 1),
            ProposalStatus::Created
        );
        assert_eq!(
            status_at(&contract, id, expiration_ns),
            ProposalStatus::Expired
        );
    }

    #[test]
    fn voting_outcome_quorum_floor_satisfied_at_boundary_succeeds() {
        // Same floor as above (400 NEAR). For+Against = 400 NEAR exactly.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(200)),
                VoterSpec::new(against_voter(), NearToken::from_near(200)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        set_ctx(owner(), 1, TEST_NOW_NS);
        contract.set_quorum_floor(NearToken::from_near(400));
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        cast_vote(
            &mut contract,
            &fixture,
            against_voter(),
            id,
            VoteOption::Against,
        );

        let cfg = default_config();
        let after_timelock =
            TEST_NOW_NS + cfg.classic_voting_duration_ns.0 + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, after_timelock),
            ProposalStatus::Succeeded
        );
    }

    // ---------------------------------------------------------------
    // active_end_time_ns — direct unit tests on the four flow/status
    // branches that drive queue backdating.
    // ---------------------------------------------------------------

    fn read_raw(contract: &Contract, id: ProposalId) -> Proposal {
        contract.proposals.get(id).cloned().unwrap().into()
    }

    #[test]
    fn active_end_time_classic_succeeded_includes_timelock() {
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let cfg = default_config();
        let raw = read_raw(&contract, id);
        assert_eq!(
            raw.active_end_time_ns(),
            TEST_NOW_NS + cfg.classic_voting_duration_ns.0 + cfg.timelock_duration_ns.0
        );
    }

    #[test]
    fn active_end_time_classic_defeated_is_voting_end() {
        // No votes cast -> compute_final_status returns Defeated -> active_end
        // is just voting_end, not voting_end + timelock.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, id, Some(&fixture));

        let raw = read_raw(&contract, id);
        assert_eq!(
            raw.active_end_time_ns(),
            TEST_NOW_NS + default_config().classic_voting_duration_ns.0
        );
    }

    #[test]
    fn active_end_time_fasttrack_with_voting_start_is_voting_end() {
        // For = 400 flips sandbox -> Scheduled, sets voting_start_time_ns.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(400))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let raw = read_raw(&contract, id);
        let voting_start = raw.voting_start_time_ns.unwrap().0;
        assert_eq!(
            raw.active_end_time_ns(),
            voting_start + default_config().fast_track_voting_duration_ns.0
        );
    }

    #[test]
    fn active_end_time_fasttrack_sandbox_only_is_sandbox_end() {
        // No vote cast -> still in Sandbox -> voting_start_time_ns is None,
        // active_end falls back to sandbox_start + sandbox_duration.
        let fixture = snapshot_with_voters(
            &[VoterSpec::new(for_voter(), NearToken::from_near(50))],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        approve_proposal(&mut contract, id, Some(&fixture));

        let raw = read_raw(&contract, id);
        assert!(raw.voting_start_time_ns.is_none());
        assert_eq!(
            raw.active_end_time_ns(),
            TEST_NOW_NS + default_config().sandbox_duration_ns.0
        );
    }

    #[test]
    fn sandbox_threshold_met_returns_false_without_snapshot() {
        // A FastTrack proposal sits in Created with no snapshot until approval.
        // sandbox_threshold_met must short-circuit on the None branch instead
        // of indexing into `votes`.
        let mut contract = fresh_contract();
        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        let raw = read_raw(&contract, id);
        assert!(raw.snapshot_and_state.is_none());
        assert_eq!(raw.sandbox_threshold_met(), false);
    }
}

#[cfg(test)]
mod create_proposal_tests {
    //! Boundary and validation tests for `Contract::create_proposal` reached
    //! via the real public API. These exercise the deposit-required arithmetic
    //! plus action-list validation.

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn create_proposal_classic_assigns_sequential_ids() {
        let mut contract = fresh_contract();

        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        let id_a = contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        let id_b = contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        let id_c = contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);

        assert_eq!(id_a, 0);
        assert_eq!(id_b, 1);
        assert_eq!(id_c, 2);
        assert_eq!(contract.get_num_proposals(), 3);
    }

    #[test]
    #[should_panic(expected = "Actions list cannot be empty")]
    fn create_proposal_rejects_empty_actions_list() {
        let mut contract = fresh_contract();
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        contract.create_proposal(proposal_metadata("t"), Some(vec![]), ProposalFlow::Classic);
    }

    #[test]
    #[should_panic(expected = "Requires deposit of")]
    fn create_proposal_classic_rejects_insufficient_deposit() {
        let mut contract = fresh_contract();
        // Far below the storage fee + base proposal fee.
        set_ctx(proposer(), 1, TEST_NOW_NS);
        contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);
    }

    #[test]
    #[should_panic(expected = "Requires deposit of")]
    fn create_proposal_fasttrack_rejects_deposit_missing_bond() {
        let mut contract = fresh_contract();
        let cfg = default_config();
        // Cover base proposal fee but NOT the bond — falls just short.
        let deposit = cfg.base_proposal_fee.as_yoctonear() + NearToken::from_near(1).as_yoctonear();
        set_ctx(proposer(), deposit, TEST_NOW_NS);
        contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::FastTrack);
    }

    #[test]
    fn create_proposal_classic_with_expiration_records_absolute_deadline() {
        // default_config already enables a non-zero classic expiration; no setter call needed.
        let mut contract = fresh_contract();
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        let id = contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);
        let p: Proposal = contract.proposals.get(id).cloned().unwrap().into();
        assert_eq!(
            p.expiration_ns,
            U64(TEST_NOW_NS + contract.get_config().proposal_expiration_ns.0)
        );
    }

    #[test]
    fn create_proposal_fasttrack_with_expiration_records_absolute_deadline() {
        // default_config already enables a non-zero FastTrack expiration; no setter call needed.
        let mut contract = fresh_contract();
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        let id = contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::FastTrack);
        let p: Proposal = contract.proposals.get(id).cloned().unwrap().into();
        assert_eq!(
            p.expiration_ns,
            U64(TEST_NOW_NS + contract.get_config().fast_track_proposal_expiration_ns.0)
        );
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn create_proposal_when_paused_panics() {
        let mut contract = fresh_contract();
        contract.paused = true;
        set_ctx(proposer(), over_deposit_yocto(), TEST_NOW_NS);
        contract.create_proposal(proposal_metadata("t"), None, ProposalFlow::Classic);
    }

    #[test]
    fn get_proposal_returns_none_for_invalid_id() {
        let contract = fresh_contract();
        assert!(contract.get_proposal(0).is_none());
        assert!(contract.get_proposal(999).is_none());
    }

    #[test]
    fn get_proposals_with_limit_none_returns_all() {
        let mut contract = fresh_contract();
        let _ = create_proposal(&mut contract, ProposalFlow::Classic);
        let _ = create_proposal(&mut contract, ProposalFlow::Classic);
        let _ = create_proposal(&mut contract, ProposalFlow::FastTrack);

        let all = contract.get_proposals(0, None);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].proposal.id, 0);
        assert_eq!(all[1].proposal.id, 1);
        assert_eq!(all[2].proposal.id, 2);
    }

    #[test]
    fn get_proposals_pagination_respects_limit_and_offset() {
        let mut contract = fresh_contract();
        for _ in 0..5 {
            let _ = create_proposal(&mut contract, ProposalFlow::Classic);
        }

        let page = contract.get_proposals(1, Some(2));
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].proposal.id, 1);
        assert_eq!(page[1].proposal.id, 2);
    }

    #[test]
    fn get_proposals_handles_out_of_bounds_from() {
        let mut contract = fresh_contract();
        let _ = create_proposal(&mut contract, ProposalFlow::Classic);
        assert!(contract.get_proposals(10, None).is_empty());
        assert!(contract.get_proposals(10, Some(5)).is_empty());
    }

    #[test]
    fn get_proposals_limit_zero_returns_empty() {
        let mut contract = fresh_contract();
        let _ = create_proposal(&mut contract, ProposalFlow::Classic);
        assert!(contract.get_proposals(0, Some(0)).is_empty());
    }
}

#[cfg(test)]
mod long_flow_tests {
    //! End-to-end-shaped tests that walk a proposal through every
    //! lifecycle stage via the public `Contract` API.
    //!
    //! These complement the surgical boundary tests in `lifecycle_tests`
    //! by checking that intermediate stages compose correctly when a
    //! proposal traverses all of them in sequence.
    use super::*;
    use crate::test_utils::*;

    #[test]
    fn classic_signaling_full_flow_created_to_succeeded() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        // 1. Create proposal -> Created
        let id = create_proposal(&mut contract, ProposalFlow::Classic);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Created
        );

        // 2. Reviewer approves -> Voting (with snapshot wired by helper)
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Voting
        );

        // 3. Cast a passing For vote.
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        // 4. Past voting_end but before timelock_end -> Timelock.
        let cfg = default_config();
        let voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Timelock
        );

        // 5. Past timelock_end with no actions -> Succeeded.
        let timelock_end = voting_end + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, timelock_end),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn classic_full_flow_with_actions_ends_executable() {
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        set_ctx(
            proposer(),
            NearToken::from_near(200).as_yoctonear(),
            TEST_NOW_NS,
        );
        let id = contract.create_proposal(
            ProposalMetadata {
                title: Some("with-actions".to_string()),
                description: None,
                link: None,
            },
            Some(vec![ProposalAction::Transfer {
                receiver_id: acc("dest.test.near"),
                amount: NearToken::from_near(1),
            }]),
            ProposalFlow::Classic,
        );
        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, None);
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);

        let cfg = default_config();
        let voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        let timelock_end = voting_end + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, timelock_end),
            ProposalStatus::Executable
        );
    }

    #[test]
    fn fasttrack_signaling_full_flow_created_to_succeeded() {
        // Voter holds 400 NEAR (40%) — clears both the 10% sandbox threshold
        // and the 50% approval threshold on its own.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        let id = create_proposal(&mut contract, ProposalFlow::FastTrack);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Created
        );

        set_ctx(reviewer(), 1, TEST_NOW_NS);
        let _ = contract.approve_proposal(id, Some(MajorityType::Simple));
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Sandbox
        );

        // Cast the For vote that meets the sandbox threshold -> Scheduled.
        cast_vote(&mut contract, &fixture, for_voter(), id, VoteOption::For);
        assert_eq!(
            status_at(&contract, id, TEST_NOW_NS),
            ProposalStatus::Scheduled
        );

        // The scheduled start is on the next Monday CET. Read at exactly that
        // boundary -> Voting.
        let voting_start = next_voting_start_ns(TEST_NOW_NS);
        assert_eq!(
            status_at(&contract, id, voting_start),
            ProposalStatus::Voting
        );

        // Past voting_end without further votes against -> Succeeded
        // (the for vote is preserved through the Sandbox -> Voting transition).
        let voting_end = voting_start + default_config().fast_track_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, id, voting_end),
            ProposalStatus::Succeeded
        );
    }

    #[test]
    fn concurrent_three_classic_proposals_at_default_cap_lifecycle() {
        // Default cap = 3. Approve three independent classic proposals,
        // give each a different vote outcome, advance through voting +
        // timelock, and verify each lands in its expected terminal status
        // without interference.
        let fixture = snapshot_with_voters(
            &[
                VoterSpec::new(for_voter(), NearToken::from_near(400)),
                VoterSpec::new(against_voter(), NearToken::from_near(100)),
                VoterSpec::new(abstain_voter(), NearToken::from_near(50)),
            ],
            NearToken::from_near(1_000),
        );
        let mut contract = fresh_contract();

        let a = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, a, Some(&fixture));
        let b = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, b, Some(&fixture));
        let c = create_proposal(&mut contract, ProposalFlow::Classic);
        approve_proposal(&mut contract, c, Some(&fixture));

        assert_active_with_status(&contract, a, ProposalStatus::Voting);
        assert_active_with_status(&contract, b, ProposalStatus::Voting);
        assert_active_with_status(&contract, c, ProposalStatus::Voting);

        // A: 400 For + 100 Against -> approval 80% (>= 50% quorum 35% trivially met) -> Succeeded.
        let (proof, v_account) = fixture.proof_for(&for_voter());
        set_ctx(
            for_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(a, VoteOption::For, proof, v_account);
        let (proof, v_account) = fixture.proof_for(&against_voter());
        set_ctx(
            against_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(a, VoteOption::Against, proof, v_account);

        // B: only Abstain. Meets quorum but denominator = 0 -> Defeated.
        let (proof, v_account) = fixture.proof_for(&for_voter());
        set_ctx(
            for_voter(),
            NearToken::from_millinear(10).as_yoctonear(),
            TEST_NOW_NS,
        );
        contract.vote(b, VoteOption::Abstain, proof, v_account);

        // C: no votes -> below quorum -> Defeated.

        // Advance to voting_end: A enters Timelock (signaling-only stays
        // there for the timelock window), B and C immediately Defeated.
        let cfg = default_config();
        let voting_end = TEST_NOW_NS + cfg.classic_voting_duration_ns.0;
        assert_eq!(
            status_at(&contract, a, voting_end),
            ProposalStatus::Timelock
        );
        assert_eq!(
            status_at(&contract, b, voting_end),
            ProposalStatus::Defeated
        );
        assert_eq!(
            status_at(&contract, c, voting_end),
            ProposalStatus::Defeated
        );

        // After timelock_end, A drops to Succeeded; B and C unchanged.
        let timelock_end = voting_end + cfg.timelock_duration_ns.0;
        assert_eq!(
            status_at(&contract, a, timelock_end),
            ProposalStatus::Succeeded
        );
        assert_eq!(
            status_at(&contract, b, timelock_end),
            ProposalStatus::Defeated
        );
        assert_eq!(
            status_at(&contract, c, timelock_end),
            ProposalStatus::Defeated
        );

        // active_proposals must be empty now.
        assert!(contract.get_queue_state().active_proposals.is_empty());
    }

}
