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
pub mod pool_callbacks;
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
use near_sdk::{AccountId, BorshStorageKey, NearToken, PanicOnDefault, near};

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
    pub config: Config,
    pub paused: bool,
    pub validators: LookupMap<AccountId, Validator>,
    pub validator_ids: Vector<AccountId>,
    /// Stable ordering for [`crate::products::Contract::list_product_ids`].
    pub product_ids: Vector<ProductId>,
    pub products: LookupMap<ProductId, Product>,
    pub prices: LookupMap<PriceId, Price>,
    pub accounts: LookupMap<AccountId, Account>,
    pub subscriptions: LookupMap<SubscriptionId, Subscription>,
    pub locks: LookupMap<LockId, Lock>,
    /// (user, validator_pool) -> share units (yocto-scale integer).
    pub user_validator_shares: LookupMap<(AccountId, AccountId), u128>,
    /// NEAR queued from unlock, waiting for epoch distribution after withdraw from pool.
    pub user_pending_unstake: LookupMap<(AccountId, AccountId), NearToken>,
    /// Locks ever created per account (increments on each new lock; used for per-lock storage prepaid).
    pub user_lock_count: LookupMap<AccountId, u32>,
    /// One subscription row per `(user, product_id)`; tier is [`Subscription::price_id`] (upgrade/downgrade).
    pub subscription_by_account_product: LookupMap<(AccountId, ProductId), SubscriptionId>,
    pub id_nonce: u64,
}

#[near]
impl Contract {
    #[init]
    pub fn new(config: Config) -> Self {
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
