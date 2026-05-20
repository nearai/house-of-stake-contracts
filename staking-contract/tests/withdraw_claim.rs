//! `withdraw(validator_id)` edge cases (VM state): claim bucket and epoch-gated tranches.

mod common;

use common::{BUYER, POOL, acct, ctx, deploy, one_yocto, register_buyer};
use near_sdk::{NearToken, testing_env};
use staking_contract::PendingUnstakeTranche;

#[test]
#[should_panic(expected = "No NEAR is claimable yet")]
fn withdraw_fails_when_pool_withdraw_bucket_empty() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);
    register_buyer(&mut c);

    let ukey = (acct(BUYER), acct(POOL));
    c.user_pending_unstake.insert(
        ukey.clone(),
        vec![PendingUnstakeTranche {
            amount: NearToken::from_near(1),
            available_epoch_height: 0,
        }],
    );

    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(acct(POOL));
}

#[test]
#[should_panic(expected = "Claim bucket cannot cover all claimable tranches yet")]
fn withdraw_fails_when_bucket_smaller_than_claimable_sum() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);
    register_buyer(&mut c);

    let pool = acct(POOL);
    let mut validator = c
        .get_validator(pool.clone())
        .expect("validator row")
        .clone();
    validator.pending_to_claim = NearToken::from_near(12);
    validator.pending_to_unstake = NearToken::from_near(3);
    c.validators.insert(pool.clone(), validator);

    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(10),
                available_epoch_height: 0,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(5),
                available_epoch_height: 0,
            },
        ],
    );

    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(pool);
}

#[test]
fn withdraw_removes_all_claimable_tranches_and_pays_sum() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);
    register_buyer(&mut c);

    let pool = acct(POOL);
    let mut validator = c
        .get_validator(pool.clone())
        .expect("validator row")
        .clone();
    validator.pending_to_claim = NearToken::from_near(20);
    validator.pending_to_unstake = NearToken::from_near(0);
    c.validators.insert(pool.clone(), validator);

    let ukey = (acct(BUYER), pool.clone());
    c.user_pending_unstake.insert(
        ukey.clone(),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(10),
                available_epoch_height: 0,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(5),
                available_epoch_height: 0,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(3),
                available_epoch_height: 999,
            },
        ],
    );

    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(pool.clone());

    let validator = c.get_validator(pool).expect("validator row");
    assert_eq!(validator.pending_to_claim, NearToken::from_near(5));
    assert_eq!(validator.pending_to_unstake, NearToken::from_near(0));

    let remaining = c
        .user_pending_unstake
        .get(&ukey)
        .expect("future tranche remains");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].amount, NearToken::from_near(3));
    assert_eq!(remaining[0].available_epoch_height, 999);
}
