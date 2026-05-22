//! Lock paths: pause gate, storage registration, product vs subscription.

mod common;

use common::{
    BUYER, OWNER, acct, ctx, deploy, register_buyer, setup_catalog_near_oneoff,
    setup_catalog_near_subscription, unwrap_sync_lock_id,
};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};
use staking_contract::types::{CatalogStatus, LockStatus, OrderRef};

#[test]
fn lock_for_product_happy_path_records_lock_and_usage() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let lock_id = unwrap_sync_lock_id(c.lock_for_product(Some(price_id.clone()), U64(dur), None));

    let lock = c.get_lock(lock_id.clone()).expect("lock");
    assert_eq!(lock.account_id, acct(BUYER));
    assert_eq!(lock.status, LockStatus::Active);
    match &lock.order {
        OrderRef::ProductPurchase { price_id: p, .. } => assert_eq!(p, &price_id),
        _ => panic!("expected product order"),
    }

    let pr = c.get_price(price_id).expect("price");
    assert_eq!(pr.usage_count, 1);
    assert_eq!(pr.status, CatalogStatus::Active);
}

#[test]
#[should_panic]
fn lock_for_product_fails_without_storage_deposit() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(Some(price_id), U64(dur), None);
}

#[test]
#[should_panic]
fn lock_for_product_fails_when_paused() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    c.pause();

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(Some(price_id), U64(dur), None);
}

#[test]
fn lock_for_subscription_creates_subscription_row() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let lock_id = unwrap_sync_lock_id(c.lock_for_subscription(Some(price_id.clone()), None));

    let sub = c
        .get_subscription_for_price(acct(BUYER), price_id)
        .expect("subscription");
    assert_eq!(sub.last_lock_id, lock_id);
    assert_eq!(
        sub.status,
        staking_contract::types::SubscriptionStatus::Active
    );
}

#[test]
#[should_panic]
fn lock_for_subscription_rejects_one_off_price() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_subscription(Some(price_id), None);
}

#[test]
fn get_config_round_trips() {
    let c = deploy();
    assert!(!c.is_paused());
    assert_eq!(c.get_config().owner_account_id, acct(OWNER));
    assert_eq!(c.get_config().epoch_unstake_settle_epochs, 4);
}

#[test]
fn unpause_allows_lock_again() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    c.pause();
    assert!(c.is_paused());

    testing_env!(ctx(acct(OWNER), NearToken::from_yoctonear(1)));
    c.unpause();
    assert!(!c.is_paused());

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = unwrap_sync_lock_id(c.lock_for_product(Some(price_id), U64(dur), None));
}

#[test]
fn user_lock_count_increments_on_lock() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = unwrap_sync_lock_id(c.lock_for_product(Some(price_id), U64(dur), None));

    let n = c.user_lock_count.get(&acct(BUYER)).copied().expect("count");
    assert_eq!(n, 1);
}
