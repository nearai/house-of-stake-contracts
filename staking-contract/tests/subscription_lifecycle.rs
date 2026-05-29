//! Cancel-at-period-end, upgrade (immediate), downgrade (next period).

mod common;

use common::{
    BUYER, POOL, VALIDATOR_OWNER_ACCOUNT, acct, add_subscription_price, add_subscription_product,
    ctx, ctx_ts, deploy, register_buyer, setup_catalog_near_subscription,
    testing_env_catalog_callback, unwrap_sync_lock_id,
};
use near_sdk::NearToken;
use near_sdk::PromiseOrValue;
use near_sdk::json_types::U128;
use near_sdk::testing_env;
use staking_contract::types::TransactionStatus;
use staking_contract::types::{LockStatus, SubscriptionStatus};
use staking_contract::utils::AVG_MONTH_NS;

const BASE_TS: u64 = 1_700_000_000_000_000_000;

#[test]
fn cancel_then_renew_after_period_opens_fresh_subscription() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let lock_first = unwrap_sync_lock_id(c.lock(Some(price_id.clone()), None, None));

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id.clone());

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    assert!(sub.cancel_at_period_end);

    let renew_ts = sub.end_ns.0.saturating_add(1);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), renew_ts));
    let lock_second = unwrap_sync_lock_id(c.lock(Some(price_id.clone()), None, None));
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
    let _lock_first = unwrap_sync_lock_id(c.lock(Some(price_id.clone()), None, None));

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
#[should_panic(expected = "Current billing period has ended")]
fn resume_subscription_fails_after_period_end() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_id), None, None));

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id.clone());

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    let after_period_ts = sub.end_ns.0.saturating_add(1);

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_yoctonear(1),
        after_period_ts
    ));
    c.resume_subscription(product_id);
}

#[test]
fn upgrade_subscription_updates_tier_and_lock_amount() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let lock_low = unwrap_sync_lock_id(c.lock(Some(price_low.clone()), None, None));
    let lock_before = c.get_lock(lock_low.clone()).expect("lock");
    let amt_before = lock_before.amount_near.as_yoctonear();

    let sub_before = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    let target_amount = NearToken::from_near(90).as_yoctonear();
    testing_env!(ctx(acct(BUYER), NearToken::from_near(40)));
    let outcome = c.update_subscription(
        sub_before.subscription_id,
        price_high.clone(),
        U128(target_amount),
    );

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub.price_id, price_high);
    let PromiseOrValue::Value(outcome) = outcome else {
        panic!("host tests expect synchronous subscription update")
    };
    assert_eq!(outcome.kind, "changed_immediately");

    let lock_after = c.get_lock(lock_low).expect("lock");
    assert!(lock_after.amount_near.as_yoctonear() > amt_before);
    assert_eq!(lock_after.amount_near.as_yoctonear(), target_amount);
    assert_eq!(lock_after.status, LockStatus::Active);
}

#[test]
fn upgrade_subscription_allows_different_product_on_same_validator() {
    let mut c = deploy();
    let (product_low, price_low) = setup_catalog_near_subscription(&mut c);
    let (product_high, price_high) = add_subscription_product(&mut c, "High product", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low), None, None));
    let sub_before = c
        .get_subscription_for_product(acct(BUYER), product_low.clone())
        .expect("subscription");

    testing_env!(ctx(acct(BUYER), NearToken::from_near(40)));
    let _ = c.update_subscription(
        sub_before.subscription_id.clone(),
        price_high.clone(),
        U128(NearToken::from_near(90).as_yoctonear()),
    );

    assert!(
        c.get_subscription_for_product(acct(BUYER), product_low)
            .is_none(),
        "old product index should be removed after cross-product upgrade"
    );
    let sub_after = c
        .get_subscription_for_product(acct(BUYER), product_high.clone())
        .expect("subscription moved to new product");
    assert_eq!(sub_after.subscription_id, sub_before.subscription_id);
    assert_eq!(sub_after.product_id, product_high);
    assert_eq!(sub_after.price_id, price_high);
}

#[test]
fn get_subscriptions_for_account_lists_all_owned_subscriptions() {
    let mut c = deploy();
    let (product_one, price_one) = setup_catalog_near_subscription(&mut c);
    let (product_two, price_two) = add_subscription_product(&mut c, "Second product", 2);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_one.clone()), None, None));

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_near(50),
        BASE_TS.saturating_add(1)
    ));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_two.clone()), None, None));

    let subs = c.get_subscriptions_for_account(acct(BUYER), 0, 10);
    assert_eq!(subs.len(), 2);
    assert!(
        subs.iter()
            .any(|sub| { sub.product_id == product_one && sub.price_id == price_one })
    );
    assert!(
        subs.iter()
            .any(|sub| { sub.product_id == product_two && sub.price_id == price_two })
    );
}

#[test]
fn upgrade_subscription_uses_projected_billing_window_after_stale_end_ns() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low), None, None));

    let stored_end = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription")
        .end_ns
        .0;
    let past_period_ts = stored_end.saturating_add(1);
    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_near(40),
        past_period_ts
    ));

    let projected_end = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription")
        .end_ns
        .0;
    assert!(projected_end > past_period_ts);

    let sub_before = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    let _ = c.update_subscription(
        sub_before.subscription_id,
        price_high.clone(),
        U128(NearToken::from_near(90).as_yoctonear()),
    );

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub.price_id, price_high);
    assert_eq!(sub.end_ns.0, projected_end);
}

#[test]
#[should_panic(expected = "Cannot delete this price while it is in use")]
fn upgraded_subscription_price_is_marked_in_use() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low), None, None));

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    testing_env!(ctx(acct(BUYER), NearToken::from_near(40)));
    let _ = c.update_subscription(
        sub.subscription_id,
        price_high.clone(),
        U128(NearToken::from_near(90).as_yoctonear()),
    );

    let high = c.get_price(price_high.clone()).expect("high price");
    assert_eq!(high.usage_count, 1);

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.delete_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        price_high,
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
}

#[test]
#[should_panic(expected = "This price is not active")]
fn upgrade_callback_rejects_price_archived_after_entry_checks() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low), None, None));
    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.archive_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        price_high.clone(),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    let mut validator = c.get_validator(acct(POOL)).expect("validator");
    validator.tx_status = TransactionStatus::Busy;
    c.validators.insert(acct(POOL), validator.into());

    testing_env!(ctx(acct(common::STAKING), NearToken::from_near(0)));
    let _ = c.on_subscription_update_after_settle(
        acct(BUYER),
        NearToken::from_near(40),
        price_high,
        U128(NearToken::from_near(90).as_yoctonear()),
        sub.subscription_id,
        acct(POOL),
    );
}

#[test]
fn downgrade_applies_at_next_renewal() {
    let mut c = deploy();
    let (product_id, price_low) = setup_catalog_near_subscription(&mut c);
    let price_high = add_subscription_price(&mut c, product_id.clone(), "High", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_high.clone()), None, None));

    let sub_high = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.update_subscription(
        sub_high.subscription_id,
        price_low.clone(),
        U128(NearToken::from_near(25).as_yoctonear()),
    );

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    assert_eq!(sub.pending_downgrade_price_id.as_ref(), Some(&price_low));

    let renew_ts = sub.end_ns.0.saturating_add(1);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(25), renew_ts));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low.clone()), None, None));

    let sub_after = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert_eq!(sub_after.price_id, price_low);
    assert!(sub_after.pending_downgrade_price_id.is_none());
    assert_eq!(sub_after.status, SubscriptionStatus::Active);

    let pending_tranches = c
        .user_pending_unstake
        .get(&(acct(BUYER), acct(POOL)))
        .expect("prorate should queue surplus on unstake path");
    let pending_yocto: u128 = pending_tranches
        .iter()
        .map(|t| t.amount.as_yoctonear())
        .sum();
    assert!(
        pending_yocto > 0,
        "Phase B tier-gap should queue NEAR to pending unstake"
    );
}

#[test]
fn downgrade_applies_to_different_product_at_next_renewal() {
    let mut c = deploy();
    let (product_low, price_low) = setup_catalog_near_subscription(&mut c);
    let (product_high, price_high) = add_subscription_product(&mut c, "High product", 10);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_high.clone()), None, None));
    let sub_before = c
        .get_subscription_for_product(acct(BUYER), product_high.clone())
        .expect("subscription");
    let subscription_id = sub_before.subscription_id.clone();

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.update_subscription(
        subscription_id.clone(),
        price_low.clone(),
        U128(NearToken::from_near(25).as_yoctonear()),
    );

    let sub = c
        .get_subscription_for_product(acct(BUYER), product_high.clone())
        .expect("subscription");
    assert_eq!(sub.pending_downgrade_price_id.as_ref(), Some(&price_low));
    assert!(
        c.get_subscription_for_product(acct(BUYER), product_low.clone())
            .is_none(),
        "target product index should not move before renewal"
    );

    let renew_ts = sub.end_ns.0.saturating_add(1);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(25), renew_ts));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_low.clone()), None, None));

    assert!(
        c.get_subscription_for_product(acct(BUYER), product_high)
            .is_none(),
        "old product index should be removed after cross-product downgrade renewal"
    );
    let sub_after = c
        .get_subscription_for_product(acct(BUYER), product_low.clone())
        .expect("subscription moved to target product");
    assert_eq!(sub_after.subscription_id, subscription_id);
    assert_eq!(sub_after.product_id, product_low);
    assert_eq!(sub_after.price_id, price_low);
    assert!(sub_after.pending_downgrade_price_id.is_none());
    assert_eq!(sub_after.status, SubscriptionStatus::Active);
}

#[test]
#[should_panic(expected = "No subscription for this product; subscribe first")]
fn cancel_subscription_fails_without_subscription() {
    let mut c = deploy();
    let (product_id, _) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id);
}

#[test]
fn active_subscription_view_projects_next_cycle_after_period_end() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_id.clone()), None, None));

    let stored = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    let stored_end = stored.end_ns.0;

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_yoctonear(0),
        stored_end.saturating_add(1),
    ));
    let projected_product = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    let projected_price = c
        .get_subscription_for_price(acct(BUYER), price_id)
        .expect("subscription");
    let projected_by_id = c
        .get_subscription(projected_product.subscription_id.clone())
        .expect("subscription");

    assert_eq!(projected_product.start_ns.0, stored_end);
    assert!(projected_product.end_ns.0 > stored_end);
    assert_eq!(projected_product.end_ns.0, projected_price.end_ns.0);
    assert_eq!(projected_product.end_ns.0, projected_by_id.end_ns.0);
}

#[test]
fn cancelled_subscription_view_does_not_project_after_period_end() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_id.clone()), None, None));

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.cancel_subscription(product_id.clone());

    let cancelled = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");
    let cancelled_end = cancelled.end_ns.0;

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_yoctonear(0),
        cancelled_end.saturating_add(1),
    ));
    let after = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    let after_by_price = c
        .get_subscription_for_price(acct(BUYER), price_id)
        .expect("subscription");

    assert!(after.cancel_at_period_end);
    assert_eq!(after.end_ns.0, cancelled_end);
    assert_eq!(after_by_price.end_ns.0, cancelled_end);
}

#[test]
fn cancel_subscription_normalizes_stale_window_before_marking_cancel_at_end() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_near_subscription(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(50), BASE_TS));
    let _ = unwrap_sync_lock_id(c.lock(Some(price_id), None, None));
    let initial = c
        .get_subscription_for_product(acct(BUYER), product_id.clone())
        .expect("subscription");

    let late_ts = initial
        .end_ns
        .0
        .saturating_add((AVG_MONTH_NS.saturating_mul(2)) as u64);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_yoctonear(1), late_ts));
    c.cancel_subscription(product_id.clone());

    let after = c
        .get_subscription_for_product(acct(BUYER), product_id)
        .expect("subscription");
    assert!(
        after.end_ns.0 > late_ts,
        "cancel-at-period-end should use current virtual cycle, not stale stored end_ns"
    );
    assert!(after.cancel_at_period_end);
}
