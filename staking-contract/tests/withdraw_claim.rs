//! `withdraw`, `claim_unlocked_near` edge cases (VM state).

mod common;

use common::{BUYER, POOL, acct, ctx, deploy, one_yocto, register_buyer};
use near_sdk::{NearToken, testing_env};
use staking_contract::PendingUnstakeTranche;

#[test]
#[should_panic(expected = "No NEAR is in the withdraw bucket yet")]
fn claim_unlocked_near_fails_when_pool_withdraw_bucket_empty() {
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
    c.claim_unlocked_near(acct(POOL));
}

#[test]
fn withdraw_partial_leaves_remainder() {
    let mut c = deploy();
    register_buyer(&mut c);

    let mut acc = c.accounts.get(&acct(BUYER)).expect("account").clone();
    acc.withdrawable_balance = NearToken::from_near(10);
    c.accounts.insert(acct(BUYER), acc);

    let take = NearToken::from_near(3);
    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(Some(take));

    let after = c.get_account(acct(BUYER)).expect("account");
    assert_eq!(
        after.withdrawable_balance.as_yoctonear(),
        NearToken::from_near(7).as_yoctonear()
    );
}

#[test]
#[should_panic(expected = "Withdraw amount is larger than your withdrawable balance")]
fn withdraw_rejects_partial_above_withdrawable() {
    let mut c = deploy();
    register_buyer(&mut c);

    let mut acc = c.accounts.get(&acct(BUYER)).expect("account").clone();
    acc.withdrawable_balance = NearToken::from_near(5);
    c.accounts.insert(acct(BUYER), acc);

    testing_env!(ctx(acct(BUYER), one_yocto()));
    c.withdraw(Some(NearToken::from_near(10)));
}
