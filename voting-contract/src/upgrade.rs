use crate::config::Config;
use crate::*;
use near_sdk::borsh::{self, BorshDeserialize};
use near_sdk::json_types::U64;
use near_sdk::store::{LookupMap, Vector};
use near_sdk::Gas;

const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

/// Config from v1.0.2 and earlier (without council_ids and timelock_duration_ns).
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

/// Contract state from v1.0.2 and earlier.
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
    /// Private method to migrate the contract state during the contract upgrade.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let old: OldContract = env::state_read().unwrap();
        Self {
            config: Config {
                venear_account_id: old.config.venear_account_id,
                reviewer_ids: old.config.reviewer_ids,
                owner_account_id: old.config.owner_account_id,
                voting_duration_ns: old.config.voting_duration_ns,
                max_number_of_voting_options: old.config.max_number_of_voting_options,
                base_proposal_fee: old.config.base_proposal_fee,
                vote_storage_fee: old.config.vote_storage_fee,
                guardians: old.config.guardians,
                proposed_new_owner_account_id: old.config.proposed_new_owner_account_id,
                council_ids: vec![],
                timelock_duration_ns: U64(0),
                proposal_expiration_ns: U64(0),
            },
            proposals: old.proposals,
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
#[no_mangle]
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
