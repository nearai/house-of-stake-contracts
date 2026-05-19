#![allow(clippy::too_many_arguments)]

pub mod accounts;
pub mod config;
pub mod epoch;
pub mod events;
pub mod gas;
pub mod governance;
pub mod ids;
pub mod internal;
pub mod lock;
pub mod pause;
pub mod prices;
pub mod products;
pub mod subscriptions;
pub mod types;
pub mod unlock;
pub mod upgrade;
pub mod validators;
pub mod withdraw;

pub use accounts::Account;
pub use config::Config;
pub use types::*;
pub use validators::Validator;

use near_sdk::store::{LookupMap, Vector};
use near_sdk::{AccountId, BorshStorageKey, PanicOnDefault, near};

#[derive(BorshStorageKey)]
#[near]
enum StorageKeys {
    Validators,
    ValidatorIds,
    ProductIds,
    Products,
    Prices,
    Accounts,
    Subscriptions,
    Locks,
    UserValidatorShares,
    UserPendingUnstake,
    UserLockCount,
    SubscriptionByAccountProduct,
}

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct Contract {
    /// Protocol configuration: owner, guardians, pause-independent bounds (`min_lock_amount`,
    /// lock duration range), epoch settle epochs, storage minimums, and per-lock storage stake.
    pub config: Config,
    /// When `true`, user-facing mutating methods reject until [`crate::pause::Contract::unpause`].
    pub paused: bool,
    /// Allowlisted staking pools (`validator_id` = pool account). Holds share-pool and epoch pipeline state
    /// per [`Validator`].
    pub validators: LookupMap<ValidatorId, Validator>,
    /// Creation order of allowlisted pools; drives paginated [`crate::validators::Contract::get_validators`].
    pub validator_ids: Vector<ValidatorId>,
    /// Creation order of catalog products; stable ordering for [`crate::products::Contract::get_products`].
    pub product_ids: Vector<ProductId>,
    /// Products keyed by id (`prod_*`); validator-scoped via [`Product::validator_id`](crate::types::Product::validator_id).
    pub products: LookupMap<ProductId, Product>,
    /// Price lines (`price_*` ids); [`Price::product_id`](crate::types::Price::product_id) links to a product.
    pub prices: LookupMap<PriceId, Price>,
    /// Per-user accounting: NEP-145-style registered storage (`storage_deposit`).
    pub accounts: LookupMap<AccountId, Account>,
    /// Subscription records keyed by [`Subscription::subscription_id`] (`sub_*`).
    pub subscriptions: LookupMap<SubscriptionId, Subscription>,
    /// Active and historical locks keyed by [`Lock::lock_id`] (`lock_*`).
    pub locks: LookupMap<LockId, Lock>,
    /// User stake position on a pool: `(AccountId, ValidatorId)` → outstanding share units (integer, same scale as [`Validator::total_shares`]). [`ValidatorId`](crate::types::ValidatorId) is the pool contract account.
    pub user_validator_shares: LookupMap<(AccountId, ValidatorId), u128>,
    /// After unlock, NEAR liability slices for this user on this pool until [`crate::Contract::withdraw`]
    /// (epoch-gated; paid from `pending_to_withdraw` once pool funds are in the bucket).
    pub user_pending_unstake: LookupMap<(AccountId, ValidatorId), Vec<PendingUnstakeTranche>>,
    /// Monotonic count of locks created per account; multiplied by [`Config::per_lock_storage_stake`] for prepaid lock storage.
    pub user_lock_count: LookupMap<AccountId, u32>,
    /// Secondary index: `(subscriber, product_id)` → `subscription_id` for at-most-one subscription per product per account.
    pub subscription_by_account_product: LookupMap<(AccountId, ProductId), SubscriptionId>,
    /// Counter mixed into deterministic ids ([`crate::ids`]) for products, prices, subscriptions, locks.
    pub id_nonce: u64,
}

#[near]
impl Contract {
    #[init]
    pub fn new(config: Config) -> Self {
        crate::config::require_min_lock_amount_at_protocol_floor(&config.min_lock_amount);
        Self {
            config,
            paused: false,
            validators: LookupMap::new(StorageKeys::Validators),
            validator_ids: Vector::new(StorageKeys::ValidatorIds),
            product_ids: Vector::new(StorageKeys::ProductIds),
            products: LookupMap::new(StorageKeys::Products),
            prices: LookupMap::new(StorageKeys::Prices),
            accounts: LookupMap::new(StorageKeys::Accounts),
            subscriptions: LookupMap::new(StorageKeys::Subscriptions),
            locks: LookupMap::new(StorageKeys::Locks),
            user_validator_shares: LookupMap::new(StorageKeys::UserValidatorShares),
            user_pending_unstake: LookupMap::new(StorageKeys::UserPendingUnstake),
            user_lock_count: LookupMap::new(StorageKeys::UserLockCount),
            subscription_by_account_product: LookupMap::new(
                StorageKeys::SubscriptionByAccountProduct,
            ),
            id_nonce: 0,
        }
    }
}

impl Contract {
    pub(crate) fn collect_paginated<T, F>(
        &self,
        from_index: u64,
        limit: u64,
        total_len: u64,
        mut fetch: F,
    ) -> Vec<T>
    where
        F: FnMut(u32) -> Option<T>,
    {
        let mut out = Vec::new();
        let mut i = from_index;
        while i < total_len && (out.len() as u64) < limit {
            if let Some(item) = fetch(i as u32) {
                out.push(item);
            }
            i += 1;
        }
        out
    }
}
