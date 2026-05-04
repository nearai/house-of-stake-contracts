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
pub mod oracle;
pub mod oracle_receiver;
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
use near_sdk::{near, AccountId, BorshStorageKey, NearToken, PanicOnDefault};

#[derive(BorshStorageKey)]
#[near]
enum StorageKeys {
    Validators,
    ValidatorIds,
    Products,
    Prices,
    Accounts,
    Subscriptions,
    Locks,
    UserValidatorShares,
    UserPendingUnstake,
}

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct Contract {
    pub config: Config,
    pub paused: bool,
    pub validators: LookupMap<AccountId, Validator>,
    pub validator_ids: Vector<AccountId>,
    pub products: LookupMap<ProductId, Product>,
    pub prices: LookupMap<PriceId, Price>,
    pub accounts: LookupMap<AccountId, Account>,
    pub subscriptions: LookupMap<SubscriptionId, Subscription>,
    pub locks: LookupMap<LockId, Lock>,
    /// (user, validator_pool) -> share units (yocto-scale integer).
    pub user_validator_shares: LookupMap<(AccountId, AccountId), u128>,
    /// NEAR queued from unlock, waiting for epoch distribution after withdraw from pool.
    pub user_pending_unstake: LookupMap<(AccountId, AccountId), NearToken>,
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
            products: LookupMap::new(StorageKeys::Products),
            prices: LookupMap::new(StorageKeys::Prices),
            accounts: LookupMap::new(StorageKeys::Accounts),
            subscriptions: LookupMap::new(StorageKeys::Subscriptions),
            locks: LookupMap::new(StorageKeys::Locks),
            user_validator_shares: LookupMap::new(StorageKeys::UserValidatorShares),
            user_pending_unstake: LookupMap::new(StorageKeys::UserPendingUnstake),
            id_nonce: 0,
        }
    }
}
