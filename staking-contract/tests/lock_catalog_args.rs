//! `price_id` / `product_id` resolution for catalog locks (`resolve_price_id_for_lock`).

mod common;

use common::{
    BUYER, POOL, acct, ctx, deploy, register_buyer, set_default_price_for_product,
    setup_catalog_near_oneoff, setup_catalog_near_subscription, unwrap_sync_lock_id,
};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};
use staking_contract::types::OrderRef;

#[test]
#[should_panic(expected = "Provide only one of price_id or product_id")]
fn lock_for_product_rejects_both_price_id_and_product_id() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_oneoff(&mut c);
    set_default_price_for_product(&mut c, product_id.clone(), price_id.clone());
    register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(Some(price_id), U64(dur), Some(product_id));
}

#[test]
#[should_panic(expected = "Provide price_id or product_id")]
fn lock_for_product_rejects_missing_price_and_product() {
    let mut c = deploy();
    let (_product_id, _price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(None, U64(dur), None);
}

#[test]
fn lock_for_product_resolves_product_id_via_default_price() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_oneoff(&mut c);
    set_default_price_for_product(&mut c, product_id.clone(), price_id.clone());
    register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let lock_id = unwrap_sync_lock_id(c.lock_for_product(None, U64(dur), Some(product_id.clone())));

    let lock = c.get_lock(lock_id).expect("lock");
    assert_eq!(lock.validator_id, acct(POOL));
    match &lock.order {
        OrderRef::ProductPurchase { price_id: p, .. } => assert_eq!(p, &price_id),
        _ => panic!("expected product purchase"),
    }
}

#[test]
#[should_panic(expected = "Provide only one of price_id or product_id")]
fn lock_for_subscription_rejects_both_price_id_and_product_id() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    set_default_price_for_product(&mut c, product_id.clone(), price_id.clone());
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_subscription(Some(price_id), Some(product_id));
}

#[test]
#[should_panic(expected = "Provide price_id or product_id")]
fn lock_for_subscription_rejects_missing_price_and_product() {
    let mut c = deploy();
    let (_product_id, _price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_subscription(None, None);
}

#[test]
fn lock_for_subscription_resolves_product_id_via_default_price() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    set_default_price_for_product(&mut c, product_id.clone(), price_id.clone());
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let lock_id = unwrap_sync_lock_id(c.lock_for_subscription(None, Some(product_id.clone())));

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub.last_lock_id, lock_id);
}
