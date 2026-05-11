//! Cancel-at-period-end, upgrade (immediate), downgrade (next period).

mod common;

use common::{
    BUYER, POOL, acct, add_subscription_price, ctx, ctx_ts, deploy, register_buyer,
    setup_catalog_near_subscription,
};
use near_sdk::NearToken;
use near_sdk::testing_env;
use staking_contract::types::{LockStatus, SubscriptionStatus};

const BASE_TS: u64 = 1_700_000_000_000_000_000;

#[test]
fn cancel_then_renew_after_period_opens_fresh_subscription() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let lock_first = c.lock_for_subscription(Some(price_id.clone()), None);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id.clone());

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    assert!(sub.cancel_at_period_end);

    let renew_ts = sub.end_ns.0.saturating_add(1);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), renew_ts));
    let lock_second = c.lock_for_subscription(Some(price_id.clone()), None);
    assert_ne!(
        lock_first, lock_second,
        "renewal after cancelled period should mint a new lock"
    );

    let sub_after = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert!(
        !sub_after.cancel_at_period_end,
        "new billing period should clear cancel-at-end"
    );
    assert_eq!(sub_after.status, SubscriptionStatus::Active);
}

#[test]
fn resume_subscription_clears_cancel_before_period_end() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _lock_first = c.lock_for_subscription(Some(price_id.clone()), None);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id.clone());

    let sub_cancelled = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    assert!(sub_cancelled.cancel_at_period_end);
    let period_end_before = sub_cancelled.end_ns.0;

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.resume_subscription(product_id.clone());

    let sub_resumed = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert!(
        !sub_resumed.cancel_at_period_end,
        "resume should clear cancel-at-end within the same period"
    );
    assert_eq!(sub_resumed.end_ns.0, period_end_before);
    assert_eq!(sub_resumed.status, SubscriptionStatus::Active);
}

#[test]
fn upgrade_subscription_updates_tier_and_lock_amount() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let lock_low = c.lock_for_subscription(Some(price_low.clone()), None);
    let lock_before = c.get_lock(lock_low.clone()).expect("lock");
    let amt_before = lock_before.amount_near.as_yoctonear();

    testing_env!(ctx(acct(BUYER), NearToken::from_near(40)));
    let _lock_same = c.upgrade_subscription(price_high.clone());

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub.price_id, price_high);

    let lock_after = c.get_lock(lock_low).expect("lock");
    assert!(lock_after.amount_near.as_yoctonear() > amt_before);
    assert_eq!(lock_after.status, LockStatus::Active);
}

#[test]
fn downgrade_applies_at_next_renewal() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = c.lock_for_subscription(Some(price_high.clone()), None);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.schedule_downgrade_subscription(price_low.clone());

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    assert_eq!(sub.pending_downgrade_price_id.as_ref(), Some(&price_low));

    let renew_ts = sub.end_ns.0.saturating_add(1);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), renew_ts));
    let _ = c.lock_for_subscription(Some(price_low.clone()), None);

    let sub_after = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub_after.price_id, price_low);
    assert!(sub_after.pending_downgrade_price_id.is_none());
    assert_eq!(sub_after.status, SubscriptionStatus::Active);

    let pending = c
        .user_pending_unstake
        .get(&(acct(BUYER), acct(POOL)))
        .expect("prorate should queue surplus on unstake path");
    assert!(
        pending.as_yoctonear() > 0,
        "Phase B tier-gap should queue NEAR to pending unstake"
    );
}
