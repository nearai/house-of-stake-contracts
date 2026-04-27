use crate::proposal::{
    MajorityType, ProposalFlow, ProposalInfo, ProposalStatus, SnapshotAndState,
};
use crate::queue::activate_proposal;
use crate::*;
use common::global_state::{GlobalState, VGlobalState};
use common::{TimestampNs, events};
use near_sdk::json_types::U64;
use near_sdk::{Gas, Promise, assert_one_yocto, ext_contract};

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
    ) -> Promise {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
        }
        if proposal.flow == ProposalFlow::V2 && !majority_type.is_some() {
            env::panic_str("V2 proposals require a majority_type");
        }

        events::emit::approve_proposal_action(&env::predecessor_account_id(), proposal_id);

        ext_venear::ext(self.config.venear_account_id.clone())
            .with_unused_gas_weight(1)
            .get_snapshot()
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_ON_GET_SNAPSHOT)
                    .on_get_snapshot(env::predecessor_account_id(), proposal_id, majority_type),
            )
    }

    /// Rejects (vetoes) a proposal.
    /// Classic: allowed during the timelock period.
    /// V2: allowed during the Voting or Scheduled period.
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
        // The rejection just freed an active slot (V2 Voting/Scheduled → Rejected). Run the
        // scheduler so the next queued proposal can fill it without waiting on another call.
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

    /// A callback after the snapshot is received for approving the proposal.
    #[private]
    pub fn on_get_snapshot(
        &mut self,
        #[callback] snapshot_and_state: (MerkleTreeSnapshot, VGlobalState),
        reviewer_id: AccountId,
        proposal_id: ProposalId,
        majority_type: Option<MajorityType>,
    ) -> ProposalInfo {
        self.assert_not_paused();
        // Free any stale slots before deciding whether this approval can activate immediately.
        self.internal_advance_queue();

        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
        }

        let timestamp: TimestampNs = env::block_timestamp().into();

        proposal.reviewer_id = Some(reviewer_id);

        let mut global_state: GlobalState = snapshot_and_state.1.into();
        global_state.update(timestamp.into());
        proposal.snapshot_and_state = Some(SnapshotAndState {
            snapshot: snapshot_and_state.0,
            timestamp_ns: timestamp.into(),
            total_venear: global_state.total_venear_balance.total(),
            venear_growth_config: global_state.venear_growth_config,
        });
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

        self.approved_proposals.push(proposal_id);

        if (self.active_proposals.len() as u64) < self.config.max_active_proposals as u64 {
            let now: U64 = env::block_timestamp().into();
            activate_proposal(&mut proposal, now);
            self.internal_set_proposal(proposal);
        } else {
            proposal.status = ProposalStatus::Queued;
            self.internal_set_proposal(proposal);
            self.pending_queue.push(proposal_id);
        }

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

#[allow(dead_code)]
#[ext_contract(ext_venear)]
trait ExtVenear {
    fn get_snapshot(&self);
}

#[allow(dead_code)]
#[ext_contract(ext_self)]
trait ExtSelf {
    fn on_get_snapshot(
        &mut self,
        reviewer_id: AccountId,
        proposal_id: ProposalId,
        majority_type: Option<MajorityType>,
    );
}
