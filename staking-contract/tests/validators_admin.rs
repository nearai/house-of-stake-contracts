//! Validator allowlist: listing and removal when idle.

mod common;

use common::{OWNER, POOL, acct, add_validator_allowlisted, ctx, deploy, one_yocto};
use near_sdk::testing_env;
use staking_contract::types::ValidatorStatus;

#[test]
fn list_validator_ids_includes_registered_pool() {
    let mut c = deploy();

    add_validator_allowlisted(&mut c);

    let ids = c.list_validator_ids(0, 10);
    assert_eq!(ids, vec![acct(POOL)]);

    let vs = c.get_validators(0, 10);
    assert_eq!(vs.len(), 1);
    assert_eq!(vs[0].pool_account_id, acct(POOL));
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
