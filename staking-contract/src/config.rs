use crate::*;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken, near};

/// Minimum attached NEAR for the first lock on a pool with [`crate::validators::Validator::total_shares`]
/// zero (first on-chain delegation). **Hardcoded** (not in [`Config`]); matches a safe pool minimum (1 NEAR).
pub const MIN_FIRST_VALIDATOR_DEPOSIT_NEAR_YOCTO: u128 = 1_000_000_000_000_000_000_000_000;

#[derive(Clone, Debug)]
#[near(serializers = [borsh, json])]
pub struct Config {
    pub owner_account_id: AccountId,
    pub proposed_new_owner_account_id: Option<AccountId>,
    pub guardians: Vec<AccountId>,
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
