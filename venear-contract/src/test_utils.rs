#![cfg(test)]
//! Fixtures for fast unit tests against `Contract` without a sandbox.

use crate::Contract;
use crate::config::Config;
use common::Bps;
use common::account::DelegationEntry;
use common::lockup_update::LockupUpdateV1;
use common::test_utils::{acc, fixed_rate_growth_config};
pub use common::test_utils::{owner, set_ctx};
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken};

pub fn entry(account_id: &str, bps: u16) -> DelegationEntry {
    DelegationEntry {
        account_id: acc(account_id),
        bps: Bps::new(bps),
    }
}

/// Applies a lockup update for `caller` at `timestamp_ns` reporting `locked` NEAR.
pub fn apply_lockup_update(
    contract: &mut Contract,
    caller: &AccountId,
    locked: NearToken,
    timestamp_ns: u64,
    nonce: u64,
) {
    set_ctx(caller.clone(), 0, timestamp_ns);
    let account_internal = contract.internal_get_account_internal(caller).unwrap();
    contract.internal_lockup_update(
        caller.clone(),
        account_internal,
        LockupUpdateV1 {
            locked_near_balance: locked,
            timestamp: timestamp_ns.into(),
            lockup_update_nonce: nonce.into(),
        },
    );
}

pub fn fresh_contract(predecessor: AccountId) -> Contract {
    set_ctx(predecessor, 0, 0);
    let config = Config {
        lockup_contract_config: None,
        unlock_duration_ns: U64(0),
        staking_pool_whitelist_account_id: acc("whitelist.near"),
        lockup_code_deployers: vec![],
        local_deposit: NearToken::from_yoctonear(0),
        min_lockup_deposit: NearToken::from_yoctonear(0),
        owner_account_id: owner(),
        guardians: vec![],
        proposed_new_owner_account_id: None,
        max_delegations: 10,
    };
    Contract::new(config, fixed_rate_growth_config())
}
