use crate::config::Config;
use crate::proposal::{Proposal, VProposal, is_active_status};
use crate::*;
use common::Bps;
use near_sdk::borsh::{self, BorshDeserialize};
use near_sdk::json_types::U64;
use near_sdk::store::{IterableSet, LookupMap, Vector};
#[cfg(target_arch = "wasm32")]
use near_sdk::{Gas, sys};

#[cfg(target_arch = "wasm32")]
const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
#[cfg(target_arch = "wasm32")]
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

// Defaults applied when migrating a contract that predates the merged flow.
const DEFAULT_BOND_AMOUNT_NEAR: u128 = 100;
// TODO: change the migration default treasury once the real treasury account is set up.
const DEFAULT_TREASURY_ACCOUNT_ID: &str = "hos-deposits.sputnik-dao.near";
const DEFAULT_SIMPLE_MAJORITY_BPS: Bps = Bps::new(5_000);
const DEFAULT_STRONG_MAJORITY_BPS: Bps = Bps::new(6_667);
const DEFAULT_SANDBOX_DURATION_NS: u64 = 7 * 24 * 60 * 60 * 1_000_000_000; // 7 days
const DEFAULT_SANDBOX_THRESHOLD_BPS: Bps = Bps::new(3_000);
const DEFAULT_MAX_ACTIVE_PROPOSALS: u32 = 3;
const DEFAULT_FAST_TRACK_PROPOSAL_EXPIRATION_NS: u64 = 2 * 24 * 60 * 60 * 1_000_000_000; // 2 days
const DEFAULT_FAST_TRACK_VOTING_DURATION_NS: u64 = 5 * 24 * 60 * 60 * 1_000_000_000; // 5 days

/// Config from v1.0.3 (pre-merge). No FastTrack fields.
#[derive(Clone, BorshDeserialize, near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "borsh")]
struct OldConfig {
    venear_account_id: AccountId,
    reviewer_ids: Vec<AccountId>,
    council_ids: Vec<AccountId>,
    owner_account_id: AccountId,
    voting_duration_ns: U64,
    timelock_duration_ns: U64,
    base_proposal_fee: NearToken,
    vote_storage_fee: NearToken,
    guardians: Vec<AccountId>,
    proposal_expiration_ns: U64,
    proposed_new_owner_account_id: Option<AccountId>,
    quorum_threshold_bps: u16,
    quorum_floor: NearToken,
    approval_threshold_bps: u16,
}

/// Contract state from v1.0.3.
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
    /// Private method to migrate the contract state from v1.0.3 (pre-merge) to the merged flow.
    /// Rewrites every legacy proposal into the unified `Proposal` shape with `flow = Classic`.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let mut old: OldContract = env::state_read().unwrap();

        let mut proposals = Vector::new(StorageKeys::Proposal);
        let mut active_proposals = IterableSet::new(StorageKeys::ActiveProposals);
        for (idx, legacy) in old.proposals.iter().enumerate() {
            let proposal: Proposal = legacy.clone().into();
            if is_active_status(proposal.status) {
                active_proposals.insert(ProposalId::try_from(idx).unwrap());
            }
            proposals.push(VProposal::Current(proposal));
        }

        // The merged flow no longer tracks `approved_proposals` separately; release its storage.
        old.approved_proposals.clear();

        Self {
            config: Config {
                venear_account_id: old.config.venear_account_id,
                reviewer_ids: old.config.reviewer_ids,
                council_ids: old.config.council_ids,
                owner_account_id: old.config.owner_account_id,
                classic_voting_duration_ns: old.config.voting_duration_ns,
                fast_track_voting_duration_ns: U64(DEFAULT_FAST_TRACK_VOTING_DURATION_NS),
                timelock_duration_ns: old.config.timelock_duration_ns,
                base_proposal_fee: old.config.base_proposal_fee,
                bond_amount: NearToken::from_near(DEFAULT_BOND_AMOUNT_NEAR),
                treasury_account_id: DEFAULT_TREASURY_ACCOUNT_ID.parse().unwrap(),
                vote_storage_fee: old.config.vote_storage_fee,
                guardians: old.config.guardians,
                classic_proposal_expiration_ns: old.config.proposal_expiration_ns,
                fast_track_proposal_expiration_ns: U64(DEFAULT_FAST_TRACK_PROPOSAL_EXPIRATION_NS),
                proposed_new_owner_account_id: old.config.proposed_new_owner_account_id,
                quorum_threshold_bps: Bps::new(old.config.quorum_threshold_bps),
                quorum_floor: old.config.quorum_floor,
                approval_threshold_bps: Bps::new(old.config.approval_threshold_bps),
                simple_majority_threshold_bps: DEFAULT_SIMPLE_MAJORITY_BPS,
                strong_majority_threshold_bps: DEFAULT_STRONG_MAJORITY_BPS,
                sandbox_duration_ns: U64(DEFAULT_SANDBOX_DURATION_NS),
                sandbox_threshold_bps: DEFAULT_SANDBOX_THRESHOLD_BPS,
                max_active_proposals: DEFAULT_MAX_ACTIVE_PROPOSALS,
            },
            proposals,
            proposal_metadata: old.proposal_metadata,
            votes: old.votes,
            paused: old.paused,
            pending_queue: Vector::new(StorageKeys::PendingQueue),
            active_proposals,
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
