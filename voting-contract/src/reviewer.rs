use crate::proposal::{
    MajorityType, ProposalFlow, ProposalInfo, ProposalStatus, SnapshotAndState,
};
use crate::*;
use common::global_state::{GlobalState, VGlobalState};
use common::{TimestampNs, events};
use near_sdk::{Gas, PromiseOrValue, assert_one_yocto};

pub const GAS_FOR_ON_GET_SNAPSHOT: Gas = Gas::from_tgas(30);

#[near]
impl Contract {
    /// Approves a proposal.
    /// Requires 1 yocto attached.
    /// Reviewers only.
    #[payable]
    pub fn approve_proposal(
        &mut self,
        proposal_id: ProposalId,
        majority_type: Option<MajorityType>,
    ) -> PromiseOrValue<Option<ProposalInfo>> {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        self.internal_advance_queue();

        let mut proposal = self.internal_expect_proposal_updated(proposal_id);
        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
        }
        if proposal.flow == ProposalFlow::V2 && majority_type.is_none() {
            env::panic_str("V2 proposals require a majority_type");
        }

        events::emit::approve_proposal_action(&env::predecessor_account_id(), proposal_id);

        proposal.reviewer_id = Some(env::predecessor_account_id());
        proposal.quorum_threshold_bps = self.config.quorum_threshold_bps;
        proposal.quorum_floor = self.config.quorum_floor;
        match proposal.flow {
            ProposalFlow::Classic => {
                proposal.approval_threshold_bps = self.config.approval_threshold_bps;
            }
            ProposalFlow::V2 => {
                proposal.approval_threshold_bps = match majority_type.unwrap() {
                    MajorityType::Simple => self.config.simple_majority_threshold_bps,
                    MajorityType::Strong => self.config.strong_majority_threshold_bps,
                };
                proposal.sandbox_duration_ns = self.config.sandbox_duration_ns;
                proposal.sandbox_threshold_bps = self.config.sandbox_threshold_bps;
            }
        }

        if self.active_proposals.len() < self.config.max_active_proposals {
            proposal.activate(env::block_timestamp().into());
            self.internal_set_proposal(proposal);
            PromiseOrValue::Promise(
                ext_venear::ext(self.config.venear_account_id.clone())
                    .with_unused_gas_weight(1)
                    .get_snapshot()
                    .then(
                        ext_self::ext(env::current_account_id())
                            .with_static_gas(GAS_FOR_ON_GET_SNAPSHOT)
                            .on_get_snapshot(proposal_id),
                    ),
            )
        } else {
            proposal.status = ProposalStatus::Queued;
            self.internal_set_proposal(proposal);
            self.pending_queue.push(proposal_id);
            PromiseOrValue::Value(self.get_proposal(proposal_id))
        }
    }

    /// Rejects (vetoes) a proposal.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the council members.
    #[payable]
    pub fn reject_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_council();
        self.internal_advance_queue();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        match (proposal.flow, proposal.status) {
            (ProposalFlow::Classic, ProposalStatus::Timelock) => {}
            (ProposalFlow::V2, ProposalStatus::Voting | ProposalStatus::Scheduled) => {}
            (_, _) => env::panic_str("Proposal cannot be rejected."),
        }

        proposal.rejecter_id = Some(env::predecessor_account_id());
        proposal.status = ProposalStatus::Rejected;

        events::emit::reject_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
        self.internal_advance_queue();
    }

    /// Slashes a proposal that is still in Created status.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the reviewers.
    #[payable]
    pub fn slash_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.flow == ProposalFlow::V2,
            "Only v2 proposals can be slashed"
        );
        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal can only be slashed while in Created status");
        }

        proposal.status = ProposalStatus::Slashed;

        events::emit::slash_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
    }

    /// Callback that stores the fetched veNEAR snapshot.
    #[private]
    pub fn on_get_snapshot(
        &mut self,
        #[callback] snapshot_and_state: (MerkleTreeSnapshot, VGlobalState),
        proposal_id: ProposalId,
    ) -> ProposalInfo {
        self.assert_not_paused();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Sandbox && proposal.status != ProposalStatus::Voting {
            env::panic_str("Proposal must be in Sandbox or Voting status");
        }
        if proposal.snapshot_and_state.is_some() {
            env::panic_str("Snapshot is already set for this proposal");
        }

        let timestamp: TimestampNs = env::block_timestamp().into();
        let mut global_state: GlobalState = snapshot_and_state.1.into();
        global_state.update(timestamp.into());
        proposal.snapshot_and_state = Some(SnapshotAndState {
            snapshot: snapshot_and_state.0,
            timestamp_ns: timestamp.into(),
            total_venear: global_state.total_venear_balance.total(),
            venear_growth_config: global_state.venear_growth_config,
        });

        self.internal_set_proposal(proposal);
        self.get_proposal(proposal_id).unwrap()
    }
}

impl Contract {
    pub fn assert_called_by_reviewer(&self) {
        require!(
            self.config
                .reviewer_ids
                .contains(&env::predecessor_account_id()),
            "Only the reviewers can call this method"
        );
    }

    pub fn assert_called_by_council(&self) {
        require!(
            self.config
                .council_ids
                .contains(&env::predecessor_account_id()),
            "Only the council can call this method"
        );
    }
}
