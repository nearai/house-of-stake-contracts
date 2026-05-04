use crate::*;
use near_sdk::env;

#[cfg(target_arch = "wasm32")]
use near_sdk::{sys, Gas};

#[cfg(target_arch = "wasm32")]
const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
#[cfg(target_arch = "wasm32")]
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

#[near]
impl Contract {
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        env::state_read().unwrap()
    }

    pub fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

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
