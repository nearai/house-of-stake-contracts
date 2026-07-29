//! Validator allowlist: listing and removal when idle.

mod common;

use common::{
    BUYER, CATALOG_MANAGER, GUARDIAN, OWNER, POOL, VALIDATOR_OWNER_ACCOUNT, acct,
    add_validator_allowlisted, ctx, deploy, event_json, one_yocto, register_buyer,
    setup_catalog_near_oneoff, testing_env_catalog_callback,
};
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken, testing_env};
use staking_contract::types::{TransactionStatus, ValidatorStatus};
use staking_contract::validators::{MAX_VALIDATOR_CATALOG_MANAGERS, MAX_VALIDATORS};

fn set_validator_busy(c: &mut staking_contract::Contract) {
    let pool = acct(POOL);
    let mut validator = c.get_validator(pool.clone()).expect("validator").clone();
    validator.tx_status = TransactionStatus::Busy;
    c.validators.insert(pool, validator.into());
}

#[test]
fn get_validators_includes_registered_pool() {
    let mut c = deploy();

    add_validator_allowlisted(&mut c);

    let vs = c.get_validators(0, 10);
    assert_eq!(vs.len(), 1);
    assert_eq!(vs[0].validator_id, acct(POOL));
    assert_eq!(vs[0].status, ValidatorStatus::Active);
    assert!(vs[0].catalog_manager_account_ids.is_empty());
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

    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    let _ = c.lock(Some(price_id), None, Some(U64(dur)));

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.remove_validator(acct(POOL));
}

#[test]
fn owner_can_force_reset_busy_validator_tx_status() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);
    set_validator_busy(&mut c);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.force_reset_validator_busy_status(acct(POOL));

    let validator = c.get_validator(acct(POOL)).expect("validator");
    assert_eq!(validator.tx_status, TransactionStatus::Idle);

    let event = event_json("validator_tx_status_reset");
    assert_eq!(event["data"]["validator_id"], POOL);
    assert_eq!(event["data"]["caller_id"], OWNER);
    assert_eq!(event["data"]["previous_status"], "Busy");
    assert_eq!(event["data"]["new_status"], "Idle");
    assert_eq!(event["data"]["epoch_height"], "100");
    assert_eq!(event["data"]["block_height"], "42");
    assert_eq!(event["data"]["block_timestamp_ns"], "1700000000000000000");
}

#[test]
fn guardian_can_force_reset_busy_validator_tx_status() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);
    set_validator_busy(&mut c);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.set_guardians(vec![acct(GUARDIAN)]);

    testing_env!(ctx(acct(GUARDIAN), one_yocto()));
    c.force_reset_validator_busy_status(acct(POOL));

    let validator = c.get_validator(acct(POOL)).expect("validator");
    assert_eq!(validator.tx_status, TransactionStatus::Idle);
}

#[test]
#[should_panic(expected = "Only a guardian or the contract owner can call this method")]
fn random_account_cannot_force_reset_busy_validator_tx_status() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);
    set_validator_busy(&mut c);

    testing_env!(ctx(acct(BUYER), one_yocto()));
    c.force_reset_validator_busy_status(acct(POOL));
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn force_reset_busy_validator_tx_status_requires_one_yocto() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);
    set_validator_busy(&mut c);

    testing_env!(ctx(acct(OWNER), NearToken::from_near(0)));
    c.force_reset_validator_busy_status(acct(POOL));
}

#[test]
#[should_panic(expected = "Validator not found on the allowlist")]
fn force_reset_busy_validator_tx_status_requires_existing_validator() {
    let mut c = deploy();

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.force_reset_validator_busy_status(acct(POOL));
}

#[test]
#[should_panic(expected = "Validator tx_status is already Idle")]
fn force_reset_busy_validator_tx_status_rejects_idle_validator() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.force_reset_validator_busy_status(acct(POOL));
}

#[test]
#[should_panic(expected = "Validator limit reached")]
fn add_validator_rejects_after_max_validators() {
    let mut c = deploy();

    for index in 0..MAX_VALIDATORS {
        let validator_id: AccountId = format!("pool-{index}.testnet").parse().unwrap();
        testing_env!(ctx(acct(OWNER), one_yocto()));
        c.add_validator(validator_id);
    }

    testing_env!(ctx(acct(OWNER), one_yocto()));
    c.add_validator("pool-over-limit.testnet".parse().unwrap());
}

#[test]
fn validator_owner_can_add_and_remove_multiple_catalog_managers() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct("manager-two.near"),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    let validator = c.get_validator(acct(POOL)).expect("validator");
    assert_eq!(
        validator.catalog_manager_account_ids,
        vec![acct(CATALOG_MANAGER), acct("manager-two.near")]
    );

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.remove_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.remove_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct("manager-missing.near"),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    let validator = c.get_validator(acct(POOL)).expect("validator");
    assert_eq!(
        validator.catalog_manager_account_ids,
        vec![acct("manager-two.near")]
    );
}

#[test]
#[should_panic(expected = "Only the validator owner can call this method")]
fn non_owner_cannot_add_validator_catalog_manager() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(CATALOG_MANAGER),
    );
}

#[test]
#[should_panic(expected = "Only the validator owner can call this method")]
fn catalog_manager_cannot_remove_validator_catalog_manager() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.remove_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct(CATALOG_MANAGER),
        acct(CATALOG_MANAGER),
    );
}

#[test]
#[should_panic(expected = "Validator catalog manager limit reached")]
fn add_validator_catalog_manager_rejects_after_max_managers() {
    let mut c = deploy();
    add_validator_allowlisted(&mut c);

    for index in 0..MAX_VALIDATOR_CATALOG_MANAGERS {
        testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
        c.add_validator_catalog_manager_after_get_owner(
            acct(VALIDATOR_OWNER_ACCOUNT),
            acct(POOL),
            format!("manager-{index}.near").parse().unwrap(),
            acct(VALIDATOR_OWNER_ACCOUNT),
        );
    }

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.add_validator_catalog_manager_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        acct(POOL),
        acct("manager-over-limit.near"),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
}
