#![cfg(test)]
//! Fixtures for fast unit tests against `Contract` without a sandbox.

use crate::Contract;
use crate::config::Config;
use common::account::DelegationEntry;
use common::test_utils::{acc, fixed_rate_growth_config};
pub use common::test_utils::{owner, set_ctx};
use common::Bps;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken};

pub fn entry(account_id: &str, bps: u16) -> DelegationEntry {
    DelegationEntry {
        account_id: acc(account_id),
        bps: Bps::new(bps),
    }
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
