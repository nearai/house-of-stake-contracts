use crate::proposal::{
    Proposal, ProposalAction, ProposalFlow, ProposalStatus, SnapshotAndState, VoteStats,
};
use crate::*;
use common::Bps;
use near_sdk::json_types::U64;

/// Oldest pre-classic proposal shape (deployed `V1`, borsh tag 0).
#[derive(Clone)]
#[near(serializers=[borsh])]
pub struct ProposalV1 {
    id: ProposalId,
    creation_time_ns: U64,
    proposer_id: AccountId,
    reviewer_id: Option<AccountId>,
    voting_start_time_ns: Option<U64>,
    voting_duration_ns: U64,
    rejected: bool,
    snapshot_and_state: Option<SnapshotAndState>,
    votes: Vec<VoteStats>,
    total_votes: VoteStats,
    status: ProposalStatus,
}

impl From<ProposalV1> for Proposal {
    fn from(v1: ProposalV1) -> Self {
        Self {
            id: v1.id,
            creation_time_ns: v1.creation_time_ns,
            proposer_id: v1.proposer_id,
            reviewer_id: v1.reviewer_id,
            rejecter_id: None,
            approval_time_ns: v1.voting_start_time_ns,
            voting_start_time_ns: v1.voting_start_time_ns,
            voting_duration_ns: v1.voting_duration_ns,
            timelock_duration_ns: U64(0),
            expiration_ns: U64(0),
            snapshot_and_state: v1.snapshot_and_state,
            votes: v1.votes,
            total_votes: v1.total_votes,
            status: v1.status,
            quorum_threshold_bps: Bps::ZERO,
            quorum_floor: NearToken::from_yoctonear(0),
            approval_threshold_bps: Bps::ZERO,
            actions: None,
            sandbox_start_time_ns: None,
            bond_amount: NearToken::from_yoctonear(0),
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: Bps::ZERO,
            flow: ProposalFlow::Classic,
        }
    }
}

/// Legacy classic-flow proposal shape (deployed `Current`, borsh tag 1).
#[derive(Clone)]
#[near(serializers=[borsh])]
pub struct ProposalV2 {
    id: ProposalId,
    creation_time_ns: U64,
    proposer_id: AccountId,
    reviewer_id: Option<AccountId>,
    rejecter_id: Option<AccountId>,
    voting_start_time_ns: Option<U64>,
    voting_duration_ns: U64,
    timelock_duration_ns: U64,
    expiration_ns: U64,
    snapshot_and_state: Option<SnapshotAndState>,
    votes: Vec<VoteStats>,
    total_votes: VoteStats,
    status: ProposalStatus,
    quorum_threshold_bps: u16,
    quorum_floor: NearToken,
    approval_threshold_bps: u16,
    actions: Option<Vec<ProposalAction>>,
}

impl From<ProposalV2> for Proposal {
    fn from(c: ProposalV2) -> Self {
        Self {
            id: c.id,
            creation_time_ns: c.creation_time_ns,
            proposer_id: c.proposer_id,
            reviewer_id: c.reviewer_id,
            rejecter_id: c.rejecter_id,
            // Pre-queue Classic proposals were activated at approval time, so
            // voting_start_time_ns is the best available approximation of approval_time_ns.
            approval_time_ns: c.voting_start_time_ns,
            voting_start_time_ns: c.voting_start_time_ns,
            voting_duration_ns: c.voting_duration_ns,
            timelock_duration_ns: c.timelock_duration_ns,
            expiration_ns: c.expiration_ns,
            snapshot_and_state: c.snapshot_and_state,
            votes: c.votes,
            total_votes: c.total_votes,
            status: c.status,
            quorum_threshold_bps: Bps::new(c.quorum_threshold_bps),
            quorum_floor: c.quorum_floor,
            approval_threshold_bps: Bps::new(c.approval_threshold_bps),
            actions: c.actions,
            sandbox_start_time_ns: None,
            bond_amount: NearToken::from_yoctonear(0),
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: Bps::ZERO,
            flow: ProposalFlow::Classic,
        }
    }
}
