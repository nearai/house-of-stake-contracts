use crate::StorageKeys;
use crate::config::Config;
use crate::proposal::{Proposal, ProposalStatus, VProposal};
use crate::*;
use near_sdk::Gas;
use near_sdk::borsh::{self, BorshDeserialize};
use near_sdk::json_types::U64;
use near_sdk::store::{LookupMap, Vector};

const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

const DEFAULT_QUORUM_THRESHOLD_BPS: u16 = 3500;
const DEFAULT_QUORUM_FLOOR_NEAR: u128 = 1000;
const DEFAULT_APPROVAL_THRESHOLD_BPS: u16 = 5000;
const TIMELOCK_DURATION_NS: u64 = 14 * 24 * 60 * 60 * 1_000_000_000; // 14 days
const PROPOSAL_EXPIRATION_NS: u64 = 7 * 24 * 60 * 60 * 1_000_000_000; // 7 days

const COUNCIL_MEMBERS: &[&str] = &[
    "as.near",
    "c65255255d689f74ae46b0a89f04bbaab94d3a51ab9dc4b79b1e9b61e7cf6816",
    "e953bb69d1129e4da87b99739373884a0b57d5e64a65fdc868478f22e6c31eac",
    "fastnear-hos.near",
    "root.near",
];

/// Config from v1.0.2 (without council_ids, timelock, expiration, or quorum).
#[derive(BorshDeserialize)]
#[borsh(crate = "borsh")]
struct OldConfig {
    venear_account_id: AccountId,
    reviewer_ids: Vec<AccountId>,
    owner_account_id: AccountId,
    voting_duration_ns: U64,
    max_number_of_voting_options: u8,
    base_proposal_fee: NearToken,
    vote_storage_fee: NearToken,
    guardians: Vec<AccountId>,
    proposed_new_owner_account_id: Option<AccountId>,
}

/// Contract state from v1.0.2.
/// All proposals are V1(ProposalV1) — the current VProposal enum handles this.
#[derive(BorshDeserialize)]
#[borsh(crate = "borsh")]
struct OldContract {
    config: OldConfig,
    proposals: Vector<VProposal>,
    proposal_metadata: Vector<VProposalMetadata>,
    votes: LookupMap<(AccountId, ProposalId), u8>,
    approved_proposals: Vector<ProposalId>,
    paused: bool,
}

#[near]
impl Contract {
    /// Private method to migrate the contract state from v1.0.2.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let old: OldContract = env::state_read().unwrap();
        let quorum_floor = NearToken::from_near(DEFAULT_QUORUM_FLOOR_NEAR);

        // Migrate proposals
        let mut proposals = Vector::new(StorageKeys::Proposal);
        for old_vp in old.proposals.iter() {
            let mut p: Proposal = old_vp.clone().into();
            p.quorum_threshold_bps = DEFAULT_QUORUM_THRESHOLD_BPS;
            p.quorum_floor = quorum_floor;
            p.approval_threshold_bps = DEFAULT_APPROVAL_THRESHOLD_BPS;
            match p.status {
                ProposalStatus::FinishLegacy => {
                    p.status = p.compute_final_status();
                }
                ProposalStatus::ApprovalLegacy => {
                    p.status = ProposalStatus::Voting;
                }
                _ => {}
            }
            proposals.push(VProposal::Current(p));
        }

        Self {
            config: Config {
                venear_account_id: old.config.venear_account_id,
                reviewer_ids: old.config.reviewer_ids,
                council_ids: COUNCIL_MEMBERS.iter().map(|s| s.parse().unwrap()).collect(),
                owner_account_id: old.config.owner_account_id,
                voting_duration_ns: old.config.voting_duration_ns,
                timelock_duration_ns: U64(TIMELOCK_DURATION_NS),
                base_proposal_fee: old.config.base_proposal_fee,
                vote_storage_fee: old.config.vote_storage_fee,
                guardians: old.config.guardians,
                proposal_expiration_ns: U64(PROPOSAL_EXPIRATION_NS),
                proposed_new_owner_account_id: old.config.proposed_new_owner_account_id,
                quorum_threshold_bps: DEFAULT_QUORUM_THRESHOLD_BPS,
                quorum_floor,
                approval_threshold_bps: DEFAULT_APPROVAL_THRESHOLD_BPS,
            },
            proposals,
            proposal_metadata: old.proposal_metadata,
            votes: old.votes,
            approved_proposals: old.approved_proposals,
            paused: old.paused,
        }
    }

    /// Returns the version of the contract from the Cargo.toml.
    pub fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// Upgrades the contract to the new version.
/// Requires the method to be called by the owner.
/// The input is the new contract code.
/// The contract will call `migrate_state` method on the new contract and then return the config,
/// to verify that the migration was successful.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn upgrade() {
    env::setup_panic_hook();
    let contract: Contract = env::state_read().unwrap();
    contract.assert_owner();
    let current_account_id = env::current_account_id();
    let current_account_id = current_account_id.as_str();
    let migrate_method_name = b"migrate_state".to_vec();
    let get_config_method_name = b"get_config".to_vec();
    let empty_args = b"{}".to_vec();
    unsafe {
        sys::input(0);
        let promise_id = sys::promise_batch_create(
            current_account_id.len() as _,
            current_account_id.as_ptr() as _,
        );
        sys::promise_batch_action_deploy_contract(promise_id, u64::MAX as _, 0);

        // Scheduling state migration.
        sys::promise_batch_action_function_call_weight(
            promise_id,
            migrate_method_name.len() as _,
            migrate_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            MIGRATE_STATE_GAS.as_gas(),
            1,
        );
        // Scheduling to return a config after the migration is completed.
        // It's an extra safety guard for the remote contract upgrades.
        sys::promise_batch_action_function_call(
            promise_id,
            get_config_method_name.len() as _,
            get_config_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            GET_CONFIG_GAS.as_gas(),
        );
        sys::promise_return(promise_id);
    }
}
