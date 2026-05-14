//! `withdraw(validator_id)` edge cases (VM state): pro-rata exit from pool withdraw batches.

mod common;

use common::{BUYER, POOL, acct, ctx, deploy, one_yocto, register_buyer};
use near_sdk::{NearToken, testing_env};
use staking_contract::PendingUnstakeTranche;

#[test]
#[should_panic(expected = "No NEAR is in the withdraw bucket yet")]
fn withdraw_fails_when_pool_withdraw_bucket_empty() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);
    register_buyer(&mut c);

    let ukey = (acct(BUYER), acct(POOL));
    c.user_pending_unstake.insert(
        ukey.clone(),
        vec![PendingUnstakeTranche {
            amount: NearToken::from_near(1),
            min_withdraw_batch_index: 0,
        }],
    );

    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(acct(POOL));
}
