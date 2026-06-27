mod common;

use common::{
    BUYER, POOL, STAKING, VALIDATOR_OWNER_ACCOUNT, acct, ctx, ctx_ts, deploy, one_yocto,
    register_buyer, setup_catalog_farm, testing_env_catalog_callback,
    testing_env_catalog_callback_at,
};
use near_sdk::json_types::U128;
use near_sdk::{NearToken, PromiseOrValue, testing_env};
use staking_contract::farm::{FARM_REWARD_RATE_DENOM, NS_PER_SECOND, YOCTO_PER_NEAR};
use staking_contract::types::{FarmPosition, FarmStatus, PriceMetadata};

const BASE_TS: u64 = 1_700_000_000_000_000_000;
const SIX_DAYS_NS: u64 = 6 * 86_400 * NS_PER_SECOND as u64;
const SPEC_REWARD_RATE: u128 = 3_858_024_691;

fn unwrap_sync_position(result: PromiseOrValue<FarmPosition>) -> FarmPosition {
    match result {
        PromiseOrValue::Value(position) => position,
        PromiseOrValue::Promise(_) => panic!("unit tests expect synchronous farm stake"),
    }
}

#[test]
fn farm_stake_accrues_rewards_in_account_view() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(100), BASE_TS));
    let position = unwrap_sync_position(c.stake(product_id.clone(), None));
    assert_eq!(position.status, FarmStatus::Active);

    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_yoctonear(0),
        BASE_TS + SIX_DAYS_NS
    ));
    let account = c.get_farm_account(acct(BUYER));
    assert_eq!(account.unclaimed_reward_units.0, 199_900);
    assert_eq!(account.total_earned_reward_units.0, 199_900);
}

#[test]
fn farm_stake_twice_aggregates_one_position() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(5), BASE_TS));
    let first = unwrap_sync_position(c.stake(product_id.clone(), None));
    testing_env!(ctx_ts(
        acct(BUYER),
        NearToken::from_near(7),
        BASE_TS + NS_PER_SECOND as u64
    ));
    let second = unwrap_sync_position(c.stake(product_id.clone(), None));

    assert!(second.shares.0 > first.shares.0);
    let positions = c.get_farm_positions_for_account(acct(BUYER), 0, 10);
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].shares.0, second.shares.0);
}

#[test]
fn full_unstake_closes_position_and_rolls_rewards_into_account() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(100), BASE_TS));
    let position = unwrap_sync_position(c.stake(product_id.clone(), None));

    testing_env!(ctx_ts(acct(BUYER), one_yocto(), BASE_TS + SIX_DAYS_NS));
    let _ = c.unstake(product_id.clone(), None);
    testing_env!(ctx_ts(
        acct(STAKING),
        NearToken::from_yoctonear(0),
        BASE_TS + SIX_DAYS_NS
    ));
    c.resolve_farm_unstake(
        acct(BUYER),
        product_id.clone(),
        acct(POOL),
        position.shares.0,
    );

    let closed = c
        .get_farm_position(acct(BUYER), product_id)
        .expect("position");
    assert_eq!(closed.status, FarmStatus::Closed);
    assert_eq!(closed.shares.0, 0);
    let account = c.get_farm_account(acct(BUYER));
    assert_eq!(account.accumulated_reward_units.0, 199_900);
    assert_eq!(account.unclaimed_reward_units.0, 0);
}

#[test]
fn update_farm_reward_rate_settles_old_rate_first() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx_ts(acct(BUYER), NearToken::from_near(100), BASE_TS));
    let _ = unwrap_sync_position(c.stake(product_id, None));

    testing_env_catalog_callback_at(acct(VALIDATOR_OWNER_ACCOUNT), BASE_TS + SIX_DAYS_NS);
    c.update_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        price_id.clone(),
        None,
        None,
        Some(PriceMetadata {
            max_amount: None,
            farm_reward_rate: Some(U128(SPEC_REWARD_RATE * 2)),
        }),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    let pool = c.get_farm_pool(price_id).expect("farm pool");
    assert!(pool.acc_reward_per_share.0 > 0);
    assert_eq!(pool.reward_rate.0, SPEC_REWARD_RATE * 2);
}

#[test]
fn storage_unregister_fails_with_active_farm_position() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(1)));
    let _ = unwrap_sync_position(c.stake(product_id, None));
    testing_env!(ctx(acct(BUYER), one_yocto()));
    assert!(!c.storage_unregister(None));
}

#[test]
#[should_panic(expected = "Cannot archive a farm price while it has active stake")]
fn archive_farm_price_fails_with_active_stake() {
    let mut c = deploy();
    let (product_id, price_id) = setup_catalog_farm(
        &mut c,
        SPEC_REWARD_RATE,
        NearToken::from_near(1).as_yoctonear(),
        None,
    );
    register_buyer(&mut c);

    testing_env!(ctx(acct(BUYER), NearToken::from_near(1)));
    let _ = unwrap_sync_position(c.stake(product_id, None));

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.archive_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        price_id,
        acct(VALIDATOR_OWNER_ACCOUNT),
    );
}

#[test]
fn farm_reward_rate_denominator_is_stable() {
    assert_eq!(FARM_REWARD_RATE_DENOM, 1_000_000_000_000);
    assert_eq!(YOCTO_PER_NEAR, NearToken::from_near(1).as_yoctonear());
}
