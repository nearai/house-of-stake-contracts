use crate::proposal::{MajorityType, ProposalInfo, ProposalStatus, SnapshotAndState};
use crate::*;
use common::global_state::{GlobalState, VGlobalState};
use common::{events, TimestampNs};
use near_sdk::{assert_one_yocto, ext_contract, Gas, Promise};

pub const GAS_FOR_ON_GET_SNAPSHOT: Gas = Gas::from_tgas(30);

#[near]
impl Contract {
    /// Approves the proposal to start the voting process.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the reviewers.
    #[payable]
    pub fn approve_proposal(
        &mut self,
        proposal_id: ProposalId,
        majority_type: MajorityType,
    ) -> Promise {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
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

    /// Rejects (vetoes) the proposal during the voting or scheduled period.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the council members.
    #[payable]
    pub fn reject_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_council();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        match proposal.status {
            ProposalStatus::Voting | ProposalStatus::Scheduled => {}
            _ => env::panic_str(
                "Proposal can only be rejected during the voting or scheduled period",
            ),
        }

        proposal.rejecter_id = Some(env::predecessor_account_id());
        proposal.status = ProposalStatus::Rejected;

        events::emit::reject_proposal_action(&env::predecessor_account_id(), proposal_id);

        self.internal_set_proposal(proposal);
    }

    /// Slashes the proposal, forfeiting the bond.
    /// Requires 1 yocto attached to the call.
    /// Can only be called by the reviewers.
    #[payable]
    pub fn slash_proposal(&mut self, proposal_id: ProposalId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_called_by_reviewer();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

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
        majority_type: MajorityType,
    ) -> ProposalInfo {
        self.assert_not_paused();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        if proposal.status != ProposalStatus::Created {
            env::panic_str("Proposal is not in the Created status");
        }

        let timestamp: TimestampNs = env::block_timestamp().into();

        proposal.reviewer_id = Some(reviewer_id);
        proposal.approval_time_ns = Some(timestamp);

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
        proposal.approval_threshold_bps = match majority_type {
            MajorityType::Simple => self.config.simple_majority_threshold_bps,
            MajorityType::Strong => self.config.strong_majority_threshold_bps,
        };
        proposal.sandbox_duration_ns = self.config.sandbox_duration_ns;
        proposal.sandbox_threshold_bps = self.config.sandbox_threshold_bps;
        proposal.status = ProposalStatus::Sandbox;

        let bond = proposal.bond_amount;
        let proposer_id = proposal.proposer_id.clone();
        proposal.bond_amount = NearToken::from_yoctonear(0);

        self.internal_set_proposal(proposal.clone());
        self.approved_proposals.push(proposal_id);

        if bond.as_yoctonear() > 0 {
            Promise::new(proposer_id).transfer(bond);
        }

        events::emit::proposal_sandbox_action(proposal_id);

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
        majority_type: MajorityType,
    );
}
