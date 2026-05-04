//! Catalog and lifecycle enums.

use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, NearToken, near};

/// Stripe-style string IDs (generated in [`crate::ids`]).
pub type ProductId = String;
pub type PriceId = String;
pub type SubscriptionId = String;
pub type LockId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[near(serializers = [borsh, json])]
pub enum Currency {
    Near,
    Usd,
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
    pub validator_id: AccountId,
    pub name: String,
    pub description: String,
    pub status: CatalogStatus,
    pub created_ns: U64,
    pub price_ids: Vec<PriceId>,
    pub usage_count: u64,
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Price {
    pub price_id: PriceId,
    pub product_id: ProductId,
    pub name: String,
    pub description: String,
    pub currency: Currency,
    pub amount: U128,
    pub price_type: PriceType,
    pub billing_period: Option<BillingPeriod>,
    /// Scaling TBD; used with [`crate::internal::required_near_months`].
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
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Lock {
    pub lock_id: LockId,
    pub account_id: AccountId,
    pub validator_id: AccountId,
    pub amount_near: NearToken,
    pub shares: U128,
    pub start_ns: U64,
    pub end_ns: U64,
    pub order: OrderRef,
    pub status: LockStatus,
}
