//! Catalog and lifecycle enums.

use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, NearToken, near};

/// Stripe-style string IDs (generated in [`crate::ids`]).
pub type ProductId = String;
pub type PriceId = String;
pub type SubscriptionId = String;
pub type LockId = String;

/// Staking pool contract account id (allowlist key, catalog scope, lock pool).
pub type ValidatorId = AccountId;

/// Snapshot from a NEAR staking pool `get_account` view (matches `HumanReadableAccount` fields used here).
#[derive(Clone, Debug, PartialEq, Eq)]
#[near(serializers = [json])]
pub struct PoolAccountView {
    pub unstaked_balance: U128,
    pub staked_balance: U128,
    /// Pool-side unstaked unlock flag (`unstaked_available_epoch_height <= epoch_height`).
    pub can_withdraw: bool,
}

impl PoolAccountView {
    pub fn unstaked(&self) -> NearToken {
        NearToken::from_yoctonear(self.unstaked_balance.0)
    }

    pub fn total_balance(&self) -> NearToken {
        NearToken::from_yoctonear(
            self.unstaked_balance
                .0
                .saturating_add(self.staked_balance.0),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum PriceType {
    OneOff,
    Recurring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum BillingPeriod {
    Monthly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum CatalogStatus {
    Active,
    Archived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum SubscriptionStatus {
    Active,
    Cancelled,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum ValidatorStatus {
    Active,
    Paused,
    Removed,
}

/// Same as lockup-contract: serial async pool operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum TransactionStatus {
    Idle,
    Busy,
}

/// One slice of a user's post-unlock NEAR liability; claims from `pending_to_claim` are allowed
/// when `env::epoch_height() >= available_epoch_height` (see [`crate::Contract::pending_unstake_tranche_available_epoch_height`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub struct PendingUnstakeTranche {
    pub amount: NearToken,
    /// Earliest `epoch_height` (inclusive) for [`crate::Contract::withdraw`]; set at unlock via
    /// [`crate::Contract::pending_unstake_tranche_available_epoch_height`].
    pub available_epoch_height: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum LockStatus {
    Active,
    UnlockRequested,
    Withdrawn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum OrderRef {
    ProductPurchase {
        product_id: ProductId,
        price_id: PriceId,
    },
    Subscription {
        subscription_id: SubscriptionId,
        price_id: PriceId,
        period_start_ns: U64,
        period_end_ns: U64,
    },
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Product {
    pub product_id: ProductId,
    pub validator_id: ValidatorId,
    pub name: String,
    pub description: String,
    pub status: CatalogStatus,
    pub created_ns: U64,
    pub price_ids: Vec<PriceId>,
    /// Default catalog price (`price_*`) for **`lock_for_product`** / **`lock_for_subscription`** when called with **`product_id`** and **`price_id: null`** (see **`set_product_default_price`**).
    /// Only an **active** (unarchived) price id may be stored; archiving the product/price clears this when it matches.
    pub default_price_id: Option<PriceId>,
    pub usage_count: u64,
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Price {
    pub price_id: PriceId,
    pub product_id: ProductId,
    pub name: String,
    pub description: String,
    /// NEAR amount in yoctoNEAR (catalog denomination).
    pub amount: U128,
    pub price_type: PriceType,
    pub billing_period: Option<BillingPeriod>,
    /// Fixed-point lock-weight; see [`crate::internal::LOCK_FACTOR_DENOM`] and [`crate::internal::check_near_price_lock`].
    pub lock_factor_near_months: U128,
    pub status: CatalogStatus,
    pub usage_count: u64,
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Subscription {
    pub subscription_id: SubscriptionId,
    pub account_id: AccountId,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub start_ns: U64,
    pub end_ns: U64,
    pub anchor_day: u8,
    pub last_lock_id: LockId,
    pub status: SubscriptionStatus,
    /// When true, no renewal after the current billing period (`end_ns`); lock still runs until unlock.
    pub cancel_at_period_end: bool,
    /// Lower tier to apply at the start of the **next** billing period (Phase A: no mid-cycle refund).
    pub pending_downgrade_price_id: Option<PriceId>,
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Lock {
    pub lock_id: LockId,
    pub account_id: AccountId,
    pub validator_id: ValidatorId,
    pub amount_near: NearToken,
    pub shares: U128,
    pub start_ns: U64,
    pub end_ns: U64,
    pub order: OrderRef,
    pub status: LockStatus,
}

/// Payload chained after [`Contract::promise_validator_per_epoch_settlement_then`] for catalog lock,
/// unlock, or **user withdraw**: either the full pre-user pipeline ran (balance sync → withdraw-if-ready →
/// [`crate::epoch::Contract::try_epoch_stake_or_unstake`]), or the pool had **already** settled this NEAR epoch and
/// the contract skipped that pipeline and jumped straight here (cached **`total_staked_balance`**).
#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub enum PerEpochContinue {
    CatalogLockMint {
        validator_id: ValidatorId,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    },
    UnlockQueueUnstake {
        validator_id: ValidatorId,
        lock_id: LockId,
        account_id: AccountId,
        shares_remove: u128,
    },
    /// After shared per-epoch settlement: batch claim + NEAR transfer for [`crate::Contract::withdraw`].
    WithdrawUserTransfer {
        validator_id: ValidatorId,
        account_id: AccountId,
    },
    /// Public [`crate::epoch::Contract::epoch_settle`]: no user tail after the shared pipeline.
    SettleOnly { validator_id: ValidatorId },
    /// Mid-period subscription tier upgrade after pre-user settlement (`subscriptions.rs`).
    SubscriptionUpgrade {
        validator_id: ValidatorId,
        buyer: AccountId,
        deposit: NearToken,
        new_price_id: PriceId,
        subscription_id: SubscriptionId,
    },
}

impl PerEpochContinue {
    pub fn validator_id(&self) -> &ValidatorId {
        match self {
            Self::CatalogLockMint { validator_id, .. }
            | Self::UnlockQueueUnstake { validator_id, .. }
            | Self::WithdrawUserTransfer { validator_id, .. }
            | Self::SettleOnly { validator_id }
            | Self::SubscriptionUpgrade { validator_id, .. } => validator_id,
        }
    }
}
