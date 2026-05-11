//! Shared NEAR `testing_env` helpers for host-side contract tests (`tests/*.rs`).

#![allow(dead_code)]

use near_sdk::json_types::{U64, U128};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{
    AccountId, NearToken, PromiseResult, RuntimeFeesConfig, VMContext, serde_json, test_vm_config,
    testing_env,
};
use staking_contract::internal::LOCK_FACTOR_DENOM;
use staking_contract::types::{BillingPeriod, PriceType};
use staking_contract::{Config, Contract};
use std::collections::HashMap;
use std::str::FromStr;

pub fn acct(s: &str) -> AccountId {
    AccountId::from_str(s).expect("valid account id")
}

pub const STAKING: &str = "staking.test.near";
pub const OWNER: &str = "owner.near";
pub const POOL: &str = "pool.near";
/// Account that owns the staking pool (`get_owner_id`), used when simulating catalog callbacks.
pub const VALIDATOR_OWNER_ACCOUNT: &str = "vowner.near";
pub const BUYER: &str = "buyer.near";
pub const NEW_OWNER: &str = "newowner.near";
pub const GUARDIAN: &str = "guardian.near";

#[inline]
pub fn one_yocto() -> NearToken {
    NearToken::from_yoctonear(1)
}

/// Baseline config; override fields in tests as needed.
pub fn base_config() -> Config {
    Config {
        owner_account_id: acct(OWNER),
        proposed_new_owner_account_id: None,
        guardians: vec![],
        operators: vec![],
        min_lock_duration_ns: U64(1),
        max_lock_duration_ns: U64(u64::MAX / 8),
        epoch_unstake_settle_epochs: 4,
        min_storage_deposit: NearToken::from_millinear(100),
        per_lock_storage_stake: NearToken::from_near(0),
        min_lock_amount: NearToken::from_near(1),
    }
}

pub fn ctx(pred: AccountId, attached: NearToken) -> VMContext {
    VMContextBuilder::new()
        .current_account_id(acct(STAKING))
        .predecessor_account_id(pred.clone())
        .signer_account_id(pred)
        .attached_deposit(attached)
        .account_balance(NearToken::from_near(500))
        .block_height(42)
        .block_timestamp(1_700_000_000_000_000_000)
        .build()
}

pub fn ctx_ts(pred: AccountId, attached: NearToken, block_timestamp_ns: u64) -> VMContext {
    VMContextBuilder::new()
        .current_account_id(acct(STAKING))
        .predecessor_account_id(pred.clone())
        .signer_account_id(pred)
        .attached_deposit(attached)
        .account_balance(NearToken::from_near(500))
        .block_height(42)
        .block_timestamp(block_timestamp_ns)
        .build()
}

/// Context for `#[private]` catalog callbacks (`*_after_get_owner`): contract calls itself.
fn ctx_catalog_callback() -> VMContext {
    let id = acct(STAKING);
    VMContextBuilder::new()
        .current_account_id(id.clone())
        .predecessor_account_id(id.clone())
        .signer_account_id(id)
        .attached_deposit(NearToken::from_near(0))
        .account_balance(NearToken::from_near(500))
        .block_height(42)
        .block_timestamp(1_700_000_000_000_000_000)
        .build()
}

/// Simulates `get_owner_id` resolving to `pool_owner` so `is_promise_success()` passes in callbacks.
pub fn testing_env_catalog_callback(pool_owner: AccountId) {
    let payload = serde_json::to_vec(&pool_owner).expect("serialize pool owner AccountId");
    testing_env!(
        ctx_catalog_callback(),
        test_vm_config(),
        RuntimeFeesConfig::test(),
        HashMap::default(),
        vec![PromiseResult::Successful(payload)],
    );
}

pub fn deploy_with_config(config: Config) -> Contract {
    Contract::new(config)
}

pub fn deploy() -> Contract {
    deploy_with_config(base_config())
}

/// Owner registers pool; validator owner creates active catalog entries for NEAR one-off purchase.
pub fn setup_catalog_near_oneoff(contract: &mut Contract) -> (String, String) {
    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    contract.add_validator(acct(POOL), acct(VALIDATOR_OWNER_ACCOUNT));

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    let product_id = contract.create_product_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        "Plan".into(),
        "Desc".into(),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    let price_id = contract.create_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        product_id.clone(),
        "Price".into(),
        "".into(),
        U128(1),
        PriceType::OneOff,
        None,
        U128(LOCK_FACTOR_DENOM),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
    (product_id, price_id)
}

/// Recurring monthly NEAR price for subscription tests.
/// Extra recurring monthly price on an existing product (for tier upgrade/downgrade tests).
pub fn add_subscription_price(
    contract: &mut Contract,
    product_id: String,
    name: &str,
    amount_yocto: u128,
) -> String {
    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    contract.create_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        product_id,
        name.into(),
        "".into(),
        U128(amount_yocto),
        PriceType::Recurring,
        Some(BillingPeriod::Monthly),
        U128(LOCK_FACTOR_DENOM),
        acct(VALIDATOR_OWNER_ACCOUNT),
    )
}

pub fn setup_catalog_near_subscription(contract: &mut Contract) -> (String, String) {
    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    contract.add_validator(acct(POOL), acct(VALIDATOR_OWNER_ACCOUNT));

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    let product_id = contract.create_product_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        "Sub product".into(),
        "Desc".into(),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    let price_id = contract.create_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        product_id.clone(),
        "Monthly".into(),
        "".into(),
        U128(1),
        PriceType::Recurring,
        Some(BillingPeriod::Monthly),
        U128(LOCK_FACTOR_DENOM),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
    (product_id, price_id)
}

pub fn register_buyer(contract: &mut Contract) {
    testing_env!(ctx(acct(BUYER), NearToken::from_millinear(500)));
    contract.storage_deposit();
}
