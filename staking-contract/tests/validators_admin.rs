//! Validator allowlist: listing and removal when idle.

mod common;

use common::{
    BUYER, OWNER, POOL, acct, add_validator_allowlisted, ctx, deploy, one_yocto, register_buyer,
    setup_catalog_near_oneoff,
};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};
use staking_contract::types::ValidatorStatus;

#[test]
fn get_validators_includes_registered_pool() {
    let mut c = deploy();

    add_validator_allowlisted(&mut c);

    let vs = c.get_validators(0, 10);
    assert_eq!(vs.len(), 1);
    assert_eq!(vs[0].validator_id, acct(POOL));
    assert_eq!(vs[0].status, ValidatorStatus::Active);
}

#[test]
fn remove_validator_on_idle_pool_marks_removed() {
    let mut c = deploy();

    add_validator_allowlisted(&mut c);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.remove_validator(acct(POOL));

    let v = c.get_validator(acct(POOL)).expect("validator row retained");
    assert_eq!(v.status, ValidatorStatus::Removed);
}

#[test]
#[should_panic(expected = "Cannot remove this validator")]
fn remove_validator_fails_while_pending_stake_exists() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = c.lock_for_product(Some(price_id), U64(dur), None);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.remove_validator(acct(POOL));
}
