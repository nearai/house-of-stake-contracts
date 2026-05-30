//! Catalog, lifecycle, and versioned on-chain storage types.

use crate::config::Config;
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

/// NEP-145-style prepaid storage for a registered user.
#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Account {
    pub storage_deposit: NearToken,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            storage_deposit: NearToken::from_near(0),
        }
    }
}

/// Allowlisted staking pool row (pool contract account id + share-pool accounting).
#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Validator {
    /// Staking pool contract account (= catalog `validator_id` / lock `validator_id`).
    pub validator_id: ValidatorId,
    /// Whether new locks are allowed (**`Active`**) or blocked (**`Paused`**), or this pool is **`Removed`**.
    pub status: ValidatorStatus,

    /// Total issued stake.dao **share units** for this pool (integer; same scale as per-user shares).
    pub total_shares: U128,
    /// Cached **staked NEAR** for this contract’s account on the pool, from the last successful
    /// pool `get_account` total balance (plus bookkeeping updates on stake/unstake/withdraw paths). Used with
    /// `pending_*` for share mint/burn pricing—not updated until the next pool read or accounting step.
    pub total_staked_balance: NearToken,
    /// `block_timestamp` (nanoseconds) when `total_staked_balance` was last synced from the pool (or validator was added).
    pub last_balance_refresh_ns: U64,

    /// NEAR waiting to be sent to the pool via **`deposit_and_stake`** (aggregated locks; net-settled vs
    /// `pending_to_unstake` in `Contract::try_epoch_stake_or_unstake` in `epoch.rs`).
    pub pending_to_stake: NearToken,
    /// NEAR queued to leave the pool via **`unstake`** (user unlocks etc.; net-settled vs `pending_to_stake`).
    pub pending_to_unstake: NearToken,
    /// Epoch height recorded after the last successful pool `unstake` callback; gates further unstakes
    /// (with [`Config::epoch_unstake_settle_epochs`]).
    pub last_unstake_epoch: u64,
    /// Last NEAR `epoch_height` for which this validator completed the **pre–user-action** pipeline for a request:
    /// **sync** `total_staked_balance` from the pool (at most once per epoch for catalog flows), **withdraw**
    /// from the pool when eligible, then at most one **net** pool `deposit_and_stake` / `unstake` / net-zero
    /// clearance for that epoch (same mutex the staking pool enforces per account). Successful callbacks
    /// on stake, unstake, and net-zero settlement set this to `env::epoch_height()`. When this equals the
    /// current epoch, user flows **skip** another pool `get_account` refresh for this validator until the
    /// next NEAR epoch.
    pub last_settlement_epoch: u64,
    /// NEAR that has been unstaked on the pool side and is expected to be moved by pool `withdraw`
    /// into this contract.
    pub pending_to_withdraw: NearToken,
    /// NEAR already moved from pool into this contract and now claimable by users.
    pub pending_to_claim: NearToken,
    /// Accounts that currently have at least one non-empty tranche in **`user_pending_unstake`** for this pool.
    pub accounts_with_pending_unstake: Vec<AccountId>,

    /// At most one in-flight cross-contract **mutating** pool pipeline for this validator (`Idle` vs `Busy`).
    pub tx_status: TransactionStatus,
}

impl Validator {
    /// Total user-exit liability across all buckets.
    pub fn pending_user_liability_yocto(&self) -> u128 {
        self.pending_to_unstake
            .as_yoctonear()
            .saturating_add(self.pending_to_withdraw.as_yoctonear())
            .saturating_add(self.pending_to_claim.as_yoctonear())
    }

    /// Liability still outside this contract (pool stake + pool unstaked).
    pub fn pending_not_in_contract_yocto(&self) -> u128 {
        self.pending_to_unstake
            .as_yoctonear()
            .saturating_add(self.pending_to_withdraw.as_yoctonear())
    }

    pub fn gross_stake_yocto(&self) -> u128 {
        self.total_staked_balance
            .as_yoctonear()
            .saturating_add(self.pending_to_stake.as_yoctonear())
    }

    /// NEAR backing **remaining** circulating shares: gross effective stake minus user liability that
    /// is still outside this contract (`pending_to_unstake + pending_to_withdraw`).
    ///
    /// **Solvency:** share pricing must not use gross backing alone after shares burn down.
    /// `pending_to_claim` is already in-contract cash and should not reduce pool-side backing again.
    pub fn net_stake_yocto(&self) -> u128 {
        self.gross_stake_yocto()
            .saturating_sub(self.pending_not_in_contract_yocto())
    }
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
    /// Default catalog price (`price_*`) for **`lock`** when called with **`product_id`** and **`price_id: null`** (see **`set_product_default_price`**).
    /// Only an **active** (unarchived) price id may be stored; archiving the product/price clears this when it matches.
    pub default_price_id: Option<PriceId>,
    pub usage_count: u64,
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct PriceMetadata {
    /// Optional inclusive upper bound for variable subscription stake amounts.
    pub max_amount: Option<U128>,
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
    /// Fixed-point lock-weight; see [`crate::utils::LOCK_FACTOR_DENOM`] and [`crate::utils::check_near_price_lock`].
    pub lock_factor_near_months: U128,
    /// Optional metadata for price-specific constraints. For recurring subscription prices,
    /// `max_amount` is the inclusive upper bound for the selected stake amount.
    pub metadata: Option<PriceMetadata>,
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
    /// Target stake amount to apply with `pending_downgrade_price_id` at the next renewal.
    pub pending_downgrade_target_amount: Option<NearToken>,
    /// Timestamp when the pending downgrade becomes effective.
    pub pending_downgrade_apply_ns: Option<U64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[near(serializers = [json])]
pub struct SubscriptionPlanChangeOutcome {
    pub kind: String,
    pub subscription_id: SubscriptionId,
    pub target_price_id: PriceId,
    pub target_amount: U128,
    pub lock_id: Option<LockId>,
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

/// User-facing tail chained after [`Contract::promise_validator_per_epoch_settlement_then`]:
/// either the full pre-user pipeline ran (balance sync → withdraw-if-ready →
/// [`crate::epoch::Contract::try_epoch_stake_or_unstake`]), or the pool had **already** settled this NEAR epoch and
/// the contract skipped that pipeline and jumped straight here (cached **`total_staked_balance`**).
#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub enum UserAction {
    /// Mint a catalog lock after settlement (`lock`).
    CommitLock {
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
    /// Mid-period subscription update after pre-user settlement (`subscriptions.rs`).
    SubscriptionUpdate {
        validator_id: ValidatorId,
        buyer: AccountId,
        deposit: NearToken,
        target_price_id: PriceId,
        target_amount: U128,
        subscription_id: SubscriptionId,
    },
}

impl UserAction {
    pub fn validator_id(&self) -> &ValidatorId {
        match self {
            Self::CommitLock { validator_id, .. }
            | Self::UnlockQueueUnstake { validator_id, .. }
            | Self::WithdrawUserTransfer { validator_id, .. }
            | Self::SettleOnly { validator_id }
            | Self::SubscriptionUpdate { validator_id, .. } => validator_id,
        }
    }

    /// NEAR attached on the entry receipt for payable flows (`lock_*`, `update_subscription`).
    /// Used to refund when the async pre-user pipeline aborts before mint / upgrade commits.
    pub fn payable_refund(&self) -> Option<(AccountId, NearToken)> {
        match self {
            Self::CommitLock { buyer, locked, .. } => Some((buyer.clone(), *locked)),
            Self::SubscriptionUpdate { buyer, deposit, .. } => Some((buyer.clone(), *deposit)),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// Versioned borsh wrappers (`LookupMap` values and `Contract.config`)
//
// Append new variants at the end when layouts change (do not reorder). See [`upgrade.rs`](upgrade.rs)
// for migration patterns (compare [`voting-contract`](../voting-contract/src/proposal.rs) `VProposal`).
//
// [`VConfig`]: [`AsRef`] / [`AsMut`] for in-place `Contract.config` access. Map values: [`From`] / `into()` only.
// Do not match `V*::V0` outside this module.
// -----------------------------------------------------------------------------

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VConfig {
    V0(Config),
}

impl From<Config> for VConfig {
    fn from(value: Config) -> Self {
        Self::V0(value)
    }
}

impl From<VConfig> for Config {
    fn from(value: VConfig) -> Self {
        match value {
            VConfig::V0(inner) => inner,
        }
    }
}

impl AsRef<Config> for VConfig {
    fn as_ref(&self) -> &Config {
        match self {
            VConfig::V0(c) => c,
        }
    }
}

impl AsMut<Config> for VConfig {
    fn as_mut(&mut self) -> &mut Config {
        match self {
            VConfig::V0(c) => c,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VValidator {
    V0(Validator),
}

impl From<Validator> for VValidator {
    fn from(value: Validator) -> Self {
        Self::V0(value)
    }
}

impl From<VValidator> for Validator {
    fn from(value: VValidator) -> Self {
        match value {
            VValidator::V0(inner) => inner,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VProduct {
    V0(Product),
}

impl From<Product> for VProduct {
    fn from(value: Product) -> Self {
        Self::V0(value)
    }
}

impl From<VProduct> for Product {
    fn from(value: VProduct) -> Self {
        match value {
            VProduct::V0(inner) => inner,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VPrice {
    V0(Price),
}

impl From<Price> for VPrice {
    fn from(value: Price) -> Self {
        Self::V0(value)
    }
}

impl From<VPrice> for Price {
    fn from(value: VPrice) -> Self {
        match value {
            VPrice::V0(inner) => inner,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VAccount {
    V0(Account),
}

impl From<Account> for VAccount {
    fn from(value: Account) -> Self {
        Self::V0(value)
    }
}

impl From<VAccount> for Account {
    fn from(value: VAccount) -> Self {
        match value {
            VAccount::V0(inner) => inner,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VSubscription {
    V0(Subscription),
}

impl From<Subscription> for VSubscription {
    fn from(value: Subscription) -> Self {
        Self::V0(value)
    }
}

impl From<VSubscription> for Subscription {
    fn from(value: VSubscription) -> Self {
        match value {
            VSubscription::V0(inner) => inner,
        }
    }
}

#[derive(Clone)]
#[near(serializers = [borsh])]
pub enum VLock {
    V0(Lock),
}

impl From<Lock> for VLock {
    fn from(value: Lock) -> Self {
        Self::V0(value)
    }
}

impl From<VLock> for Lock {
    fn from(value: VLock) -> Self {
        match value {
            VLock::V0(inner) => inner,
        }
    }
}

#[cfg(test)]
mod versioned_tests {
    use super::*;
    use near_sdk::json_types::U64;

    #[test]
    fn vconfig_v0_roundtrip() {
        let owner: AccountId = "owner.near".parse().unwrap();
        let cfg = Config {
            owner_account_id: owner.clone(),
            proposed_new_owner_account_id: None,
            guardians: vec![],
            min_lock_duration_ns: U64(1),
            max_lock_duration_ns: U64(2),
            epoch_unstake_settle_epochs: 4,
            min_storage_deposit: NearToken::from_near(1),
            per_lock_storage_stake: NearToken::from_near(0),
            min_lock_amount: NearToken::from_near(1),
        };
        let v: VConfig = cfg.clone().into();
        let back: Config = v.into();
        assert_eq!(back.owner_account_id, owner);
    }
}
