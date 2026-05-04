//! Shared NEAR `testing_env` helpers for host-side contract tests (`tests/*.rs`).

#![allow(dead_code)]

use near_sdk::json_types::{U64, U128};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{AccountId, NearToken, VMContext, testing_env};
use staking_contract::internal::LOCK_FACTOR_DENOM;
use staking_contract::types::{BillingPeriod, Currency, PriceType};
use staking_contract::{Config, Contract};
use std::str::FromStr;

pub fn acct(s: &str) -> AccountId {
    AccountId::from_str(s).expect("valid account id")
}

pub const STAKING: &str = "staking.test.near";
pub const OWNER: &str = "owner.near";
pub const POOL: &str = "pool.near";
pub const VOWNER: &str = "vowner.near";
pub const BUYER: &str = "buyer.near";
pub const ORACLE: &str = "oracle.near";

/// Baseline config; override fields in tests as needed.
pub fn base_config() -> Config {
    Config {
        owner_account_id: acct(OWNER),
        proposed_new_owner_account_id: None,
        guardians: vec![],
        operators: vec![],
        oracle_account_id: acct(ORACLE),
        oracle_usd_price_asset_id: String::new(),
        oracle_max_age_ns: U64(86_400_000_000_000_000),
        oracle_max_recency_duration_sec: 0,
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

pub fn deploy_with_config(config: Config) -> Contract {
    Contract::new(config)
}

pub fn deploy() -> Contract {
    deploy_with_config(base_config())
}

/// Owner registers pool; validator owner creates active catalog entries for NEAR one-off purchase.
pub fn setup_catalog_near_oneoff(contract: &mut Contract) -> (String, String) {
    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    contract.add_validator(acct(POOL), acct(VOWNER));

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    let product_id = contract.create_product(acct(POOL), "Plan".into(), "Desc".into());

    let price_id = contract.create_price(
        product_id.clone(),
        "Price".into(),
        "".into(),
        Currency::Near,
        U128(1),
        PriceType::OneOff,
        None,
        U128(LOCK_FACTOR_DENOM),
    );
    (product_id, price_id)
}

/// Recurring monthly NEAR price for subscription tests.
pub fn setup_catalog_near_subscription(contract: &mut Contract) -> (String, String) {
    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    contract.add_validator(acct(POOL), acct(VOWNER));

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    let product_id = contract.create_product(acct(POOL), "Sub product".into(), "Desc".into());

    let price_id = contract.create_price(
        product_id.clone(),
        "Monthly".into(),
        "".into(),
        Currency::Near,
        U128(1),
        PriceType::Recurring,
        Some(BillingPeriod::Monthly),
        U128(LOCK_FACTOR_DENOM),
    );
    (product_id, price_id)
}

pub fn register_buyer(contract: &mut Contract) {
    testing_env!(ctx(acct(BUYER), NearToken::from_millinear(500)));
    contract.storage_deposit();
}
