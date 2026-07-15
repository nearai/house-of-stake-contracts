use crate::*;
use common::PooledVenearBalance;
#[cfg(target_arch = "wasm32")]
use near_sdk::Gas;
use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
#[cfg(target_arch = "wasm32")]
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

#[near]
impl Contract {
    /// Private method to migrate the contract state during the contract upgrade.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let mut contract: Contract = env::state_read().unwrap();
        let timestamp = env::block_timestamp().into();
        // Delegates with a non-zero stored balance start at zero so orphaned balances get cleared.
        let mut recomputed: BTreeMap<AccountId, PooledVenearBalance> = BTreeMap::new();
        for index in 0..contract.tree.len() {
            let mut account: Account = contract.tree.get_by_index(index).unwrap().clone().into();
            if account.delegated_balance != PooledVenearBalance::default() {
                recomputed.entry(account.account_id.clone()).or_default();
            }
            if account.delegations.is_empty() {
                continue;
            }
            account.update(timestamp, contract.internal_get_venear_growth_config());
            for delegation in &account.delegations {
                let delegated_balance = recomputed
                    .get(&delegation.account_id)
                    .copied()
                    .unwrap_or_default()
                    .pooled_add_delegation(&account.balance, delegation.bps);
                recomputed.insert(delegation.account_id.clone(), delegated_balance);
            }
        }
        for (account_id, delegated_balance) in recomputed {
            let mut delegate = contract.internal_expect_account_updated(&account_id);
            if delegate.delegated_balance == delegated_balance {
                continue;
            }
            delegate.delegated_balance = delegated_balance;
            contract.internal_set_account(account_id, delegate);
        }
        contract
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
