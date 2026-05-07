use crate::config::LockupContractConfig;
use crate::*;
use near_sdk::borsh::{self, BorshDeserialize};
use near_sdk::json_types::U64;
#[cfg(target_arch = "wasm32")]
use near_sdk::Gas;

#[cfg(target_arch = "wasm32")]
const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
#[cfg(target_arch = "wasm32")]
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

/// Default applied during migration of contracts deployed before `max_delegations`
/// was made configurable. Matches the previous hardcoded constant.
const DEFAULT_MAX_DELEGATIONS: u32 = 8;

/// Pre-migration `Config` layout (no `max_delegations` field).
#[derive(BorshDeserialize)]
#[borsh(crate = "borsh")]
struct OldConfig {
    lockup_contract_config: Option<LockupContractConfig>,
    unlock_duration_ns: U64,
    staking_pool_whitelist_account_id: AccountId,
    lockup_code_deployers: Vec<AccountId>,
    local_deposit: NearToken,
    min_lockup_deposit: NearToken,
    owner_account_id: AccountId,
    guardians: Vec<AccountId>,
    proposed_new_owner_account_id: Option<AccountId>,
}

/// Pre-migration top-level contract layout. Mirrors `Contract` but with `OldConfig`.
#[derive(BorshDeserialize)]
#[borsh(crate = "borsh")]
struct OldContract {
    tree: MerkleTree<VAccount, VGlobalState>,
    accounts: LookupMap<AccountId, VAccountInternal>,
    config: OldConfig,
    paused: bool,
}

#[near]
impl Contract {
    /// Private method to migrate the contract state during the contract upgrade.
    /// Backfills the new `max_delegations` config field with the legacy default
    /// when the stored state still uses the pre-`max_delegations` layout.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let raw = env::storage_read(b"STATE").expect("Missing contract state");
        if let Ok(current) = Self::try_from_slice(&raw) {
            return current;
        }
        let old = OldContract::try_from_slice(&raw)
            .unwrap_or_else(|_| env::panic_str("Cannot deserialize the contract state."));
        Self {
            tree: old.tree,
            accounts: old.accounts,
            config: Config {
                lockup_contract_config: old.config.lockup_contract_config,
                unlock_duration_ns: old.config.unlock_duration_ns,
                staking_pool_whitelist_account_id: old.config.staking_pool_whitelist_account_id,
                lockup_code_deployers: old.config.lockup_code_deployers,
                local_deposit: old.config.local_deposit,
                min_lockup_deposit: old.config.min_lockup_deposit,
                owner_account_id: old.config.owner_account_id,
                guardians: old.config.guardians,
                proposed_new_owner_account_id: old.config.proposed_new_owner_account_id,
                max_delegations: DEFAULT_MAX_DELEGATIONS,
            },
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
