//! `storage_withdraw` retention and bounds.

mod common;

use common::{BUYER, acct, base_config, ctx, deploy_with_config, register_buyer};
use near_sdk::{NearToken, testing_env};

#[test]
#[should_panic(expected = "Withdraw exceeds prepaid storage")]
fn storage_withdraw_rejects_amount_above_prepaid() {
    let mut c = deploy_with_config(base_config());
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.storage_withdraw(NearToken::from_near(1_000));
}

#[test]
#[should_panic(expected = "Must retain required storage (min + per-record stake)")]
fn storage_withdraw_rejects_dropping_below_required_retention() {
    let mut cfg = base_config();
    cfg.per_lock_storage_stake = NearToken::from_millinear(50);
    cfg.min_storage_deposit = NearToken::from_millinear(100);

    let mut c = deploy_with_config(cfg);
    let (_pid, price_id) = common::setup_catalog_near_oneoff(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_millinear(200)));
    c.storage_deposit();

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = c.lock(Some(price_id), None, Some(near_sdk::json_types::U64(dur)));

    // After one lock, required prepaid is min 100m + 50m × 1 lock = 150m; withdrawing 60m would drop below.
    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    c.storage_withdraw(NearToken::from_millinear(60));
}
