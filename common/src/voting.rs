use crate::*;

/// The fixed voting options for proposals.
#[derive(Clone, Copy, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum VoteOption {
    For,
    Against,
    Abstain,
}

/// The status of the proposal
#[derive(Clone, Copy, Debug, PartialEq)]
#[near(serializers=[borsh, json])]
pub enum ProposalStatus {
    /// The proposal was created and is waiting for the approver to approve it.
    Created,
    /// The proposal was rejected by a reviewer before approval.
    Rejected,
    /// Legacy: the proposal was approved by the approver and is waiting for the voting to start.
    ApprovalLegacy,
    /// The proposal is in the voting phase.
    Voting,
    /// Legacy: the proposal voting is finished and the results are available.
    FinishLegacy,
    /// The proposal was vetoed by a council member during the timelock period.
    Vetoed,
    /// The voting has ended and the proposal is in the timelock period awaiting potential council veto.
    Timelock,
    /// The proposal expired before being approved by a reviewer.
    Expired,
    /// The proposal voting has finished, quorum was met and approval threshold was met.
    Succeeded,
    /// The proposal voting has finished, but quorum was not met or approval threshold was not met.
    Defeated,
    /// The proposal passed and has actions ready for on-chain execution.
    Executable,
    /// The proposal actions are being executed (dispatched, awaiting callback).
    InProgress,
    /// The proposal's on-chain execution failed.
    Failed,
}
