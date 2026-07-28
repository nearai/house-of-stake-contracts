//! `withdraw(validator_id)` edge cases (VM state): claim bucket and epoch-gated tranches.

mod common;

use common::{
    BUYER, POOL, acct, ctx, deploy, one_yocto, register_buyer,
    testing_env_transfer_callback_failure,
};
use near_sdk::{NearToken, testing_env};
use staking_contract::{PendingUnstakeTranche, TransactionStatus};

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
fn pending_unstake_view_zeroes_missing_account_tranches() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let view = c.get_account_pending_unstake(acct(BUYER), acct(POOL));

    assert_eq!(view.account_id, acct(BUYER));
    assert_eq!(view.validator_id, acct(POOL));
    assert_eq!(view.current_epoch_height, 100);
    assert_eq!(view.epoch_unstake_settle_epochs, 4);
    assert_eq!(view.total_pending_yocto.0, 0);
    assert_eq!(view.epoch_eligible_yocto.0, 0);
    assert_eq!(view.withdrawable_yocto.0, 0);
    assert_eq!(view.next_available_epoch_height, None);
    assert_eq!(view.wait_epochs, None);
    assert_eq!(view.pending_to_claim_yocto.0, 0);
    assert!(!view.can_withdraw_now);
    assert!(view.tranches.is_empty());
}

#[test]
fn pending_unstake_view_reports_waiting_epoch_tranches() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let pool = acct(POOL);
    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(2),
                available_epoch_height: 103,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(1),
                available_epoch_height: 101,
            },
        ],
    );

    let view = c.get_account_pending_unstake(acct(BUYER), pool);

    assert_eq!(
        view.total_pending_yocto.0,
        NearToken::from_near(3).as_yoctonear()
    );
    assert_eq!(view.epoch_eligible_yocto.0, 0);
    assert_eq!(view.withdrawable_yocto.0, 0);
    assert_eq!(view.next_available_epoch_height, Some(101));
    assert_eq!(view.wait_epochs, Some(1));
    assert!(!view.can_withdraw_now);
    assert_eq!(view.tranches.len(), 2);
    assert!(
        view.tranches
            .iter()
            .all(|tranche| !tranche.is_epoch_eligible)
    );
}

#[test]
fn pending_unstake_view_requires_claim_bucket_to_cover_all_epoch_eligible_tranches() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let pool = acct(POOL);
    let mut validator = c.get_validator(pool.clone()).expect("validator row");
    validator.pending_to_claim = NearToken::from_near(4);
    c.validators.insert(pool.clone(), validator.into());

    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(3),
                available_epoch_height: 100,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(2),
                available_epoch_height: 0,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(7),
                available_epoch_height: 105,
            },
        ],
    );

    let view = c.get_account_pending_unstake(acct(BUYER), pool);

    assert_eq!(
        view.total_pending_yocto.0,
        NearToken::from_near(12).as_yoctonear()
    );
    assert_eq!(
        view.epoch_eligible_yocto.0,
        NearToken::from_near(5).as_yoctonear()
    );
    assert_eq!(view.withdrawable_yocto.0, 0);
    assert_eq!(
        view.pending_to_claim_yocto.0,
        NearToken::from_near(4).as_yoctonear()
    );
    assert_eq!(view.next_available_epoch_height, Some(105));
    assert_eq!(view.wait_epochs, Some(5));
    assert!(!view.can_withdraw_now);
    assert_eq!(
        view.tranches
            .iter()
            .filter(|tranche| tranche.is_epoch_eligible)
            .count(),
        2
    );
}

#[test]
fn pending_unstake_view_reports_withdrawable_when_claim_bucket_covers_eligible_tranches() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let pool = acct(POOL);
    let mut validator = c.get_validator(pool.clone()).expect("validator row");
    validator.pending_to_claim = NearToken::from_near(10);
    c.validators.insert(pool.clone(), validator.into());

    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(3),
                available_epoch_height: 99,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(2),
                available_epoch_height: 100,
            },
        ],
    );

    let view = c.get_account_pending_unstake(acct(BUYER), pool);

    assert_eq!(
        view.epoch_eligible_yocto.0,
        NearToken::from_near(5).as_yoctonear()
    );
    assert_eq!(
        view.withdrawable_yocto.0,
        NearToken::from_near(5).as_yoctonear()
    );
    assert_eq!(view.next_available_epoch_height, None);
    assert_eq!(view.wait_epochs, None);
    assert!(view.can_withdraw_now);
    assert!(
        view.tranches
            .iter()
            .all(|tranche| tranche.is_epoch_eligible)
    );
}

#[test]
fn pending_unstake_view_disables_withdraw_when_contract_paused() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let pool = acct(POOL);
    let mut validator = c.get_validator(pool.clone()).expect("validator row");
    validator.pending_to_claim = NearToken::from_near(10);
    c.validators.insert(pool.clone(), validator.into());
    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![PendingUnstakeTranche {
            amount: NearToken::from_near(3),
            available_epoch_height: 100,
        }],
    );
    c.paused = true;

    let view = c.get_account_pending_unstake(acct(BUYER), pool);

    assert_eq!(
        view.epoch_eligible_yocto.0,
        NearToken::from_near(3).as_yoctonear()
    );
    assert_eq!(
        view.pending_to_claim_yocto.0,
        NearToken::from_near(10).as_yoctonear()
    );
    assert_eq!(view.withdrawable_yocto.0, 0);
    assert!(!view.can_withdraw_now);
    assert!(
        view.tranches
            .iter()
            .all(|tranche| tranche.is_epoch_eligible)
    );
}

#[test]
fn pending_unstake_view_disables_withdraw_when_validator_busy() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);

    let pool = acct(POOL);
    let mut validator = c.get_validator(pool.clone()).expect("validator row");
    validator.pending_to_claim = NearToken::from_near(10);
    validator.tx_status = TransactionStatus::Busy;
    c.validators.insert(pool.clone(), validator.into());
    c.user_pending_unstake.insert(
        (acct(BUYER), pool.clone()),
        vec![PendingUnstakeTranche {
            amount: NearToken::from_near(3),
            available_epoch_height: 100,
        }],
    );

    let view = c.get_account_pending_unstake(acct(BUYER), pool);

    assert_eq!(
        view.epoch_eligible_yocto.0,
        NearToken::from_near(3).as_yoctonear()
    );
    assert_eq!(
        view.pending_to_claim_yocto.0,
        NearToken::from_near(10).as_yoctonear()
    );
    assert_eq!(view.withdrawable_yocto.0, 0);
    assert!(!view.can_withdraw_now);
    assert!(
        view.tranches
            .iter()
            .all(|tranche| tranche.is_epoch_eligible)
    );
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
    c.validators.insert(pool.clone(), validator.into());

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
    c.validators.insert(pool.clone(), validator.into());

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

#[test]
fn failed_withdraw_transfer_restores_claim_bucket_and_tranches() {
    let mut c = deploy();
    common::add_validator_allowlisted(&mut c);
    register_buyer(&mut c);

    let pool = acct(POOL);
    let mut validator = c
        .get_validator(pool.clone())
        .expect("validator row")
        .clone();
    validator.pending_to_claim = NearToken::from_near(20);
    c.validators.insert(pool.clone(), validator.into());

    let claimed_tranches = vec![
        PendingUnstakeTranche {
            amount: NearToken::from_near(10),
            available_epoch_height: 0,
        },
        PendingUnstakeTranche {
            amount: NearToken::from_near(5),
            available_epoch_height: 100,
        },
    ];
    let future_tranche = PendingUnstakeTranche {
        amount: NearToken::from_near(3),
        available_epoch_height: 999,
    };
    let ukey = (acct(BUYER), pool.clone());
    let mut all_tranches = claimed_tranches.clone();
    all_tranches.push(future_tranche.clone());
    c.user_pending_unstake.insert(ukey.clone(), all_tranches);

    testing_env!(ctx(acct(BUYER), one_yocto()));
    let _ = c.withdraw(pool.clone());

    assert_eq!(
        c.get_validator(pool.clone())
            .expect("validator row")
            .pending_to_claim,
        NearToken::from_near(5)
    );

    testing_env_transfer_callback_failure();
    assert!(!c.on_user_withdraw_transfer_done(
        acct(BUYER),
        pool.clone(),
        claimed_tranches,
        NearToken::from_near(15),
    ));

    assert_eq!(
        c.get_validator(pool)
            .expect("validator row")
            .pending_to_claim,
        NearToken::from_near(20)
    );
    assert_eq!(
        c.user_pending_unstake
            .get(&ukey)
            .expect("tranches restored")
            .clone(),
        vec![
            PendingUnstakeTranche {
                amount: NearToken::from_near(10),
                available_epoch_height: 0,
            },
            PendingUnstakeTranche {
                amount: NearToken::from_near(5),
                available_epoch_height: 100,
            },
            future_tranche,
        ]
    );
}
