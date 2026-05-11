//! NEP-145 style bounds with `per_lock_storage_stake`.

mod common;

use common::{BUYER, acct, base_config, ctx, deploy_with_config, setup_catalog_near_oneoff};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};

#[test]
#[should_panic]
fn first_lock_fails_if_storage_below_min_plus_per_lock() {
    let mut cfg = base_config();
    cfg.per_lock_storage_stake = NearToken::from_millinear(50);
    // min 100m + 1 lock × 50m = 150m; register only 120m
    cfg.min_storage_deposit = NearToken::from_millinear(100);

    let mut c = deploy_with_config(cfg);
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_millinear(120)));
    c.storage_deposit();

    let dur = c.config.min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(Some(price_id), U64(dur), None);
}

#[test]
fn first_lock_succeeds_with_sufficient_combined_storage() {
    let mut cfg = base_config();
    cfg.per_lock_storage_stake = NearToken::from_millinear(50);
    cfg.min_storage_deposit = NearToken::from_millinear(100);

    let mut c = deploy_with_config(cfg);
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_millinear(200)));
    c.storage_deposit();

    let dur = c.config.min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = c.lock_for_product(Some(price_id), U64(dur), None);
}
