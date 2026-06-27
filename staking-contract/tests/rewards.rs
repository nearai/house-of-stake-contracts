//! Staking-farm reward accounting tests.

mod common;

use common::{
    BUYER, OWNER, POOL, acct, ctx, ctx_ts, deploy, one_yocto, register_buyer,
    setup_catalog_near_oneoff, unwrap_sync_lock_id,
};
use near_sdk::json_types::{U64, U128};
use near_sdk::{NearToken, testing_env};

const T0: u64 = 1_700_000_000_000_000_000;

fn lock_one_off(c: &mut staking_contract::Contract, price_id: String, amount_near: u128) -> String {
    let dur = c.get_config().min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(amount_near), T0));
    unwrap_sync_lock_id(c.lock(Some(price_id), None, Some(U64(dur))))
}

#[test]
fn rewards_are_disabled_by_default() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(BUYER), one_yocto(), T0 + 1_000));
    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.reward_rate_yocto_per_near_ns, U128(0));
    assert_eq!(reward.unclaimed_rewards, U128(0));
    assert_eq!(reward.claimed_rewards, U128(0));
}

#[test]
fn rewards_accrue_from_stake_amount_elapsed_time_and_rate() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));

    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_yoctonear(0), T0 + 100));
    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.unclaimed_rewards, U128(50_000));
    assert_eq!(reward.claimed_rewards, U128(0));
}

#[test]
fn claim_updates_unclaimed_and_claimed_rewards() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));
    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(BUYER), one_yocto(), T0 + 100));
    let claimed = c.claim_lock_rewards(lock_id.clone());
    assert_eq!(claimed, U128(50_000));

    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.unclaimed_rewards, U128(0));
    assert_eq!(reward.claimed_rewards, U128(50_000));
}

#[test]
fn update_persists_rewards_without_claiming() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));
    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(BUYER), one_yocto(), T0 + 100));
    let state = c.update_lock_rewards(lock_id.clone());
    assert_eq!(state.unclaimed_rewards, U128(50_000));
    assert_eq!(state.claimed_rewards, U128(0));

    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.unclaimed_rewards, U128(50_000));
    assert_eq!(reward.claimed_rewards, U128(0));
}

#[test]
fn rate_changes_apply_prospectively() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));
    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0 + 100));
    c.set_validator_reward_rate(acct(POOL), U128(20));

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_yoctonear(0), T0 + 150));
    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.unclaimed_rewards, U128(100_000));
}

#[test]
fn disabling_rate_stops_future_accrual() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));
    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0 + 100));
    c.set_validator_reward_rate(acct(POOL), U128(0));

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_yoctonear(0),
        T0 + 1_000
    ));
    let reward = c.get_lock_reward(lock_id).expect("reward state");
    assert_eq!(reward.unclaimed_rewards, U128(50_000));
}

#[test]
#[should_panic(expected = "Only the lock owner can claim rewards")]
fn only_lock_owner_can_claim_rewards() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(OWNER), one_yocto(), T0));
    c.set_validator_reward_rate(acct(POOL), U128(10));
    let lock_id = lock_one_off(&mut c, price_id, 50);

    testing_env!(ctx(acct("intruder.near"), one_yocto()));
    c.claim_lock_rewards(lock_id);
}
