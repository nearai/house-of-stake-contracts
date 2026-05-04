use crate::*;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken, near};

#[derive(Clone, Debug)]
#[near(serializers = [borsh, json])]
pub struct Config {
    pub owner_account_id: AccountId,
    pub proposed_new_owner_account_id: Option<AccountId>,
    pub guardians: Vec<AccountId>,
    pub operators: Vec<AccountId>,
    pub oracle_account_id: AccountId,
    /// Burrow payload: match [`crate::oracle_receiver::OracleAssetOptionalPrice::asset_id`]. Empty = first row that carries a price.
    pub oracle_usd_price_asset_id: String,
    pub oracle_max_age_ns: U64,
    /// If non-zero, [`crate::oracle_receiver::OraclePriceData::recency_duration_sec`] must not exceed this (Burrow-style).
    pub oracle_max_recency_duration_sec: u32,
    pub min_lock_duration_ns: U64,
    pub max_lock_duration_ns: U64,
    pub epoch_unstake_settle_epochs: u64,
    pub min_storage_deposit: NearToken,
    /// Per **lock ever created** (see [`crate::Contract::user_lock_count`]); zero disables the extra
    /// requirement beyond [`Self::min_storage_deposit`].
    pub per_lock_storage_stake: NearToken,
    pub min_lock_amount: NearToken,
}

#[near]
impl Contract {
    pub fn get_config(&self) -> &Config {
        &self.config
    }
}
