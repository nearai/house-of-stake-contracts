//! Ownership transfer and owner-only config updates.

mod common;

use common::{
    BUYER, GUARDIAN, NEW_OWNER, OPERATOR, OWNER, POOL, acct, ctx, deploy, deploy_with_config,
    one_yocto,
};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};

#[test]
fn propose_and_accept_ownership_transfers_owner() {
    let mut c = deploy();
    let new_owner = acct(NEW_OWNER);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.propose_new_owner_account_id(Some(new_owner.clone()));

    assert_eq!(
        c.get_config().proposed_new_owner_account_id.as_ref(),
        Some(&new_owner)
    );

    testing_env!(ctx(new_owner.clone(), one_yocto()));
    c.accept_ownership();

    assert_eq!(c.get_config().owner_account_id, new_owner);
    assert!(c.get_config().proposed_new_owner_account_id.is_none());
}

#[test]
#[should_panic(expected = "Only the proposed new owner can call this method")]
fn accept_ownership_rejects_wrong_account() {
    let mut c = deploy();

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.propose_new_owner_account_id(Some(acct(NEW_OWNER)));

    testing_env!(ctx(acct(BUYER), one_yocto()));
    c.accept_ownership();
}

#[test]
#[should_panic(expected = "Only the contract owner can call this method")]
fn non_owner_cannot_set_guardians() {
    let mut c = deploy();

    testing_env!(ctx(acct(BUYER), one_yocto()));
    c.set_guardians(vec![acct(GUARDIAN)]);
}

#[test]
fn owner_sets_epoch_unstake_and_config_round_trips() {
    let mut c = deploy();

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_epoch_unstake_settle_epochs(11);

    assert_eq!(c.get_config().epoch_unstake_settle_epochs, 11);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_per_lock_storage_stake(NearToken::from_millinear(1));

    assert_eq!(
        c.get_config().per_lock_storage_stake,
        NearToken::from_millinear(1)
    );

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_min_storage_deposit(NearToken::from_millinear(200));

    assert_eq!(
        c.get_config().min_storage_deposit,
        NearToken::from_millinear(200)
    );

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_min_lock_amount(NearToken::from_near(2));

    assert_eq!(c.get_config().min_lock_amount, NearToken::from_near(2));

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_lock_bounds(U64(10), U64(1_000_000_000_000_000));

    assert_eq!(c.get_config().min_lock_duration_ns.0, 10);
    assert_eq!(c.get_config().max_lock_duration_ns.0, 1_000_000_000_000_000);
}

/// Guardian (not owner) may pause when listed in `guardians`.
#[test]
fn guardian_can_pause() {
    let mut c = deploy();

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_guardians(vec![acct(GUARDIAN)]);

    testing_env!(ctx(acct(GUARDIAN), one_yocto()));
    c.pause();
    assert!(c.is_paused());
}

#[test]
#[should_panic(expected = "Only a guardian or the contract owner can call this method")]
fn random_account_cannot_pause() {
    let mut c = deploy();

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_guardians(vec![acct(GUARDIAN)]);

    testing_env!(ctx(acct(BUYER), one_yocto()));
    c.pause();
}

#[test]
#[should_panic(expected = "Only an operator can call this method")]
fn non_operator_cannot_epoch_stake_when_operators_configured() {
    let mut cfg = common::base_config();
    cfg.operators = vec![acct(OPERATOR)];
    let mut c = deploy_with_config(cfg);
    common::add_validator_allowlisted(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_yoctonear(1)));
    let _ = c.epoch_stake(acct(POOL));
}

#[test]
fn operator_can_call_epoch_stake_when_configured() {
    let mut cfg = common::base_config();
    cfg.operators = vec![acct(OPERATOR)];
    let mut c = deploy_with_config(cfg);
    let (_pid, price_id) = common::setup_catalog_near_oneoff(&mut c);
    common::register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = c.lock_for_product(Some(price_id), U64(dur), None);

    testing_env!(ctx(acct(OPERATOR), NearToken::from_yoctonear(1)));
    let _ = c.epoch_stake(acct(POOL));
}
