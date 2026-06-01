#![allow(clippy::too_many_arguments)]

pub mod accounts;
pub mod config;
pub mod epoch;
pub mod events;
pub mod gas;
pub mod governance;
pub mod ids;
pub mod lock;
pub mod pause;
pub mod payments;
pub mod prices;
pub mod products;
pub mod subscriptions;
pub mod types;
pub mod unlock;
pub mod upgrade;
pub mod utils;
pub mod validators;
pub mod withdraw;

pub use config::Config;
pub use types::*;

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
    SubscriptionsByAccount,
<<<<<<< HEAD
    Purchases,
    PurchaseIds,
    PurchasesByAccount,
    PurchasesByProduct,
    UserPurchaseCount,
    RevenueByValidator,
    RevenueByProduct,
    SubscriptionIds,
=======
    SubscriptionIds,
    PendingUpdateTargetPriceCounts,
    PendingUpdateTargetProductCounts,
>>>>>>> origin/feat/stake-dao
}

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct Contract {
    /// Protocol configuration: owner, guardians, pause-independent bounds (`min_lock_amount`,
    /// lock duration range), epoch settle epochs, storage minimums, and per-lock storage stake.
    pub config: VConfig,
    /// When `true`, user-facing mutating methods reject until [`crate::pause::Contract::unpause`].
    pub paused: bool,
    /// Allowlisted staking pools (`validator_id` = pool account). Holds share-pool and epoch pipeline state
    /// per [`Validator`].
    pub validators: LookupMap<ValidatorId, VValidator>,
    /// Creation order of allowlisted pools; drives paginated [`crate::validators::Contract::get_validators`].
    pub validator_ids: Vector<ValidatorId>,
    /// Creation order of catalog products; stable ordering for [`crate::products::Contract::get_products`].
    pub product_ids: Vector<ProductId>,
    /// Products keyed by id (`prod_*`); validator-scoped via [`Product::validator_id`](crate::types::Product::validator_id).
    pub products: LookupMap<ProductId, VProduct>,
    /// Price lines (`price_*` ids); [`Price::product_id`](crate::types::Price::product_id) links to a product.
    pub prices: LookupMap<PriceId, VPrice>,
    /// Per-user accounting: NEP-145-style registered storage (`storage_deposit`).
    pub accounts: LookupMap<AccountId, VAccount>,
    /// Subscription records keyed by [`Subscription::subscription_id`] (`sub_*`).
    pub subscriptions: LookupMap<SubscriptionId, VSubscription>,
    /// Active and historical locks keyed by [`Lock::lock_id`] (`lock_*`).
    pub locks: LookupMap<LockId, VLock>,
    /// User stake position on a pool: `(AccountId, ValidatorId)` → outstanding share units (integer, same scale as [`Validator::total_shares`]). [`ValidatorId`](crate::types::ValidatorId) is the pool contract account.
    pub user_validator_shares: LookupMap<(AccountId, ValidatorId), u128>,
    /// After unlock, NEAR liability slices for this user on this pool until [`crate::Contract::withdraw`]
    /// (epoch-gated; paid from `pending_to_claim` once pool funds are in the contract claim bucket).
    pub user_pending_unstake: LookupMap<(AccountId, ValidatorId), Vec<PendingUnstakeTranche>>,
    /// Monotonic count of locks created per account; multiplied by [`Config::per_lock_storage_stake`] for prepaid lock storage.
    pub user_lock_count: LookupMap<AccountId, u32>,
    /// Direct one-off payment records keyed by [`Purchase::purchase_id`] (`pay_*`).
    pub purchases: LookupMap<PurchaseId, VPurchase>,
    /// Creation order of direct payment records; drives paginated purchase views.
    pub purchase_ids: Vector<PurchaseId>,
    /// Secondary index: purchaser account → purchase ids.
    pub purchases_by_account: LookupMap<AccountId, Vec<PurchaseId>>,
    /// Secondary index: product id → purchase ids.
    pub purchases_by_product: LookupMap<ProductId, Vec<PurchaseId>>,
    /// Monotonic count of direct purchases created per account; multiplied by [`Config::per_purchase_storage_stake`].
    pub user_purchase_count: LookupMap<AccountId, u32>,
    /// Withdrawable direct-payment revenue aggregated by validator pool account.
    pub revenue_by_validator: LookupMap<ValidatorId, NearToken>,
    /// Direct-payment revenue aggregated by product for accounting views.
    pub revenue_by_product: LookupMap<ProductId, NearToken>,
    /// Secondary index: `(subscriber, product_id)` → `subscription_id` for at-most-one subscription per product per account.
    pub subscription_by_account_product: LookupMap<(AccountId, ProductId), SubscriptionId>,
    /// Secondary index: `subscriber` → owned subscription ids. Used for account-level listing and
    /// subscription-specific plan changes without scanning the full catalog.
    pub subscriptions_by_account: LookupMap<AccountId, Vec<SubscriptionId>>,
    /// Creation order of subscription ids. Used by catalog admin guards to detect pending references.
    pub subscription_ids: Vector<SubscriptionId>,
<<<<<<< HEAD
=======
    /// Pending subscription-update target price reference counts, used by bounded catalog guards.
    pub pending_update_target_price_counts: LookupMap<PriceId, u32>,
    /// Pending subscription-update target product reference counts, used by bounded catalog guards.
    pub pending_update_target_product_counts: LookupMap<ProductId, u32>,
>>>>>>> origin/feat/stake-dao
    /// Counter mixed into deterministic ids ([`crate::ids`]) for products, prices, subscriptions, locks.
    pub id_nonce: u64,
}

#[near]
impl Contract {
    #[init]
    pub fn new(config: Config) -> Self {
        crate::config::require_min_lock_amount_at_protocol_floor(&config.min_lock_amount);
        Self {
            config: config.into(),
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
            purchases: LookupMap::new(StorageKeys::Purchases),
            purchase_ids: Vector::new(StorageKeys::PurchaseIds),
            purchases_by_account: LookupMap::new(StorageKeys::PurchasesByAccount),
            purchases_by_product: LookupMap::new(StorageKeys::PurchasesByProduct),
            user_purchase_count: LookupMap::new(StorageKeys::UserPurchaseCount),
            revenue_by_validator: LookupMap::new(StorageKeys::RevenueByValidator),
            revenue_by_product: LookupMap::new(StorageKeys::RevenueByProduct),
            subscription_by_account_product: LookupMap::new(
                StorageKeys::SubscriptionByAccountProduct,
            ),
            subscriptions_by_account: LookupMap::new(StorageKeys::SubscriptionsByAccount),
            subscription_ids: Vector::new(StorageKeys::SubscriptionIds),
<<<<<<< HEAD
=======
            pending_update_target_price_counts: LookupMap::new(
                StorageKeys::PendingUpdateTargetPriceCounts,
            ),
            pending_update_target_product_counts: LookupMap::new(
                StorageKeys::PendingUpdateTargetProductCounts,
            ),
>>>>>>> origin/feat/stake-dao
            id_nonce: 0,
        }
    }
}
