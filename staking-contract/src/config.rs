use crate::*;
use near_sdk::json_types::U64;
use near_sdk::{near, AccountId, NearToken};

#[derive(Clone, Debug)]
#[near(serializers = [borsh, json])]
pub struct Config {
    pub owner_account_id: AccountId,
    pub proposed_new_owner_account_id: Option<AccountId>,
    pub guardians: Vec<AccountId>,
    pub operators: Vec<AccountId>,
    pub oracle_account_id: AccountId,
    pub oracle_max_age_ns: U64,
    pub min_lock_duration_ns: U64,
    pub max_lock_duration_ns: U64,
    pub epoch_unstake_settle_epochs: u64,
    pub min_storage_deposit: NearToken,
    pub min_lock_amount: NearToken,
}

#[near]
impl Contract {
    pub fn get_config(&self) -> &Config {
        &self.config
    }
}
