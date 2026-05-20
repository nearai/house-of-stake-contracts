//! Sandbox E2E tests for the **epoch settlement pipeline** ([`epoch.rs`](../src/epoch.rs)).
//!
//! Exercises real cross-contract promises against [`mock-staking-pool-contract`]: `get_account`,
//! `deposit_and_stake`, `unstake`, withdraw-from-pool, and public `epoch_settle` retries.
//!
//! Build WASMs from repo root: `make staking-contract`, `make mock-staking-pool-contract`.
//! Run: `cargo test -p staking-contract --test sandbox_epoch_settlement`

mod mock_pool;

use mock_pool::{
    buyer_cancel_subscription, buyer_lock_for_product, buyer_lock_for_subscription,
    buyer_storage_deposit, buyer_unlock, buyer_withdraw, call_epoch_settle,
    create_subscription_product_and_price, fast_forward_until_timestamp, fetch_validator,
    json_near_token_yocto, json_tx_status, json_u64_field, pool_total_balance_yocto,
    setup_staking_fixture,
};
use serde_json::json;

const SHORT_LOCK_NS: &str = "1000000000";

#[tokio::test]
async fn lock_runs_settlement_pipeline_and_clears_tx_status()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    buyer_lock_for_product(&buyer, staking.id(), &price_id, "1000000000000000", 50).await?;

    let v = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_tx_status(&v["tx_status"]),
        Some("Idle"),
        "lock promise chain must release validator pipeline mutex"
    );
    assert!(
        json_near_token_yocto(&v["pending_to_stake"]).unwrap_or(0) > 0,
        "lock mints shares and queues pending_to_stake; pool stake follows a later settle"
    );
    let pool_total = pool_total_balance_yocto(&worker, pool.id(), staking.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v["total_staked_balance"]).unwrap_or(0),
        pool_total,
        "pre-user refresh should keep cached total_staked_balance aligned with the pool view"
    );

    Ok(())
}

#[tokio::test]
async fn epoch_settle_fast_path_succeeds_when_slot_already_consumed()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    buyer_lock_for_product(&buyer, staking.id(), &price_id, "1000000000000000", 50).await?;

    let again = call_epoch_settle(&buyer, staking.id(), pool.id()).await?;
    again.into_result()?;

    let v = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(json_tx_status(&v["tx_status"]), Some("Idle"));

    Ok(())
}

#[tokio::test]
async fn epoch_settle_same_epoch_fast_path_leaves_pending_until_next_epoch()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer_a = worker.dev_create_account().await?;
    let buyer_b = worker.dev_create_account().await?;

    for buyer in [&buyer_a, &buyer_b] {
        buyer_storage_deposit(buyer, staking.id()).await?;
        buyer_lock_for_product(buyer, staking.id(), &price_id, "1000000000000000", 50).await?;
    }

    let v_mid = fetch_validator(&worker, staking.id(), pool.id()).await?;
    let pending_mid = json_near_token_yocto(&v_mid["pending_to_stake"]).unwrap_or(0);
    assert!(
        pending_mid > 0,
        "second lock in the same NEAR epoch should leave stake pending until a later epoch"
    );

    // Fast-path `epoch_settle` (slot already used this NEAR epoch) is a no-op for pending queues.
    call_epoch_settle(&buyer_a, staking.id(), pool.id())
        .await?
        .into_result()?;

    let v_after = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v_after["pending_to_stake"]).unwrap_or(0),
        pending_mid,
        "same-epoch epoch_settle must not clear pending_to_stake; advance epoch and settle again"
    );

    Ok(())
}

#[tokio::test]
async fn two_locks_then_epoch_settle_next_epoch_stakes_on_pool()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer_a = worker.dev_create_account().await?;
    let buyer_b = worker.dev_create_account().await?;

    for buyer in [&buyer_a, &buyer_b] {
        buyer_storage_deposit(buyer, staking.id()).await?;
        buyer_lock_for_product(buyer, staking.id(), &price_id, "1000000000000000", 50).await?;
    }

    let pool_before = pool_total_balance_yocto(&worker, pool.id(), staking.id()).await?;

    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer_a, staking.id(), pool.id())
        .await?
        .into_result()?;

    let v = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v["pending_to_stake"]).unwrap_or(0),
        0,
        "epoch_settle in a fresh NEAR epoch should clear accumulated pending_to_stake"
    );

    let pool_after = pool_total_balance_yocto(&worker, pool.id(), staking.id()).await?;
    assert!(
        pool_after > pool_before,
        "pool should receive combined stake from epoch_settle"
    );

    Ok(())
}

#[tokio::test]
async fn unlock_queues_unstake_then_epoch_settle_next_epoch_clears_pending()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id =
        buyer_lock_for_product(&buyer, staking.id(), &price_id, SHORT_LOCK_NS, 50).await?;

    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer, staking.id(), pool.id())
        .await?
        .into_result()?;

    let lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let end_ns = json_u64_field(&lock["end_ns"]).expect("lock.end_ns");
    fast_forward_until_timestamp(&worker, end_ns.saturating_add(1)).await?;

    buyer_unlock(&buyer, staking.id(), &lock_id).await?;

    let v_after_unlock = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert!(
        json_near_token_yocto(&v_after_unlock["pending_to_unstake"]).unwrap_or(0) > 0,
        "unlock should queue NEAR for the next pool unstake"
    );
    assert_eq!(
        json_tx_status(&v_after_unlock["tx_status"]),
        Some("Idle"),
        "unlock pipeline must release Busy after share exit"
    );

    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer, staking.id(), pool.id())
        .await?
        .into_result()?;

    let v_settled = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v_settled["pending_to_unstake"]).unwrap_or(0),
        0,
        "epoch_settle should run pool unstake and clear pending_to_unstake"
    );

    Ok(())
}

#[tokio::test]
async fn epoch_settle_net_zero_when_stake_and_unstake_pending_match()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id =
        buyer_lock_for_product(&buyer, staking.id(), &price_id, SHORT_LOCK_NS, 50).await?;

    // Stake the first lock's pending queue in a fresh epoch.
    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer, staking.id(), pool.id())
        .await?
        .into_result()?;

    let lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let end_ns = json_u64_field(&lock["end_ns"]).expect("lock.end_ns");
    fast_forward_until_timestamp(&worker, end_ns.saturating_add(1)).await?;

    // Same NEAR epoch: unlock queues unstake; a second lock queues matching stake (no pool op yet).
    buyer_unlock(&buyer, staking.id(), &lock_id).await?;
    buyer_lock_for_product(&buyer, staking.id(), &price_id, SHORT_LOCK_NS, 50).await?;

    let v_before = fetch_validator(&worker, staking.id(), pool.id()).await?;
    let stake_p = json_near_token_yocto(&v_before["pending_to_stake"]).unwrap_or(0);
    let unstake_p = json_near_token_yocto(&v_before["pending_to_unstake"]).unwrap_or(0);
    assert!(
        stake_p > 0 && unstake_p > 0,
        "unlock + lock in the same epoch should leave both pending queues non-zero"
    );
    assert_eq!(
        stake_p, unstake_p,
        "matching 50 NEAR lock/unlock should produce equal pending stake and unstake"
    );

    let pool_before = pool_total_balance_yocto(&worker, pool.id(), staking.id()).await?;

    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer, staking.id(), pool.id())
        .await?
        .into_result()?;

    let v_after = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v_after["pending_to_stake"]).unwrap_or(0),
        0
    );
    assert_eq!(
        json_near_token_yocto(&v_after["pending_to_unstake"]).unwrap_or(0),
        0
    );
    let pool_after = pool_total_balance_yocto(&worker, pool.id(), staking.id()).await?;
    assert_eq!(
        pool_after, pool_before,
        "net-zero settle should clear pending without a pool deposit_and_stake or unstake"
    );

    Ok(())
}

#[tokio::test]
async fn withdraw_runs_settlement_prefetch_before_payout() -> Result<(), Box<dyn std::error::Error>>
{
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id =
        buyer_lock_for_product(&buyer, staking.id(), &price_id, SHORT_LOCK_NS, 50).await?;

    let lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let end_ns = json_u64_field(&lock["end_ns"]).expect("lock.end_ns");
    fast_forward_until_timestamp(&worker, end_ns.saturating_add(1)).await?;

    buyer_unlock(&buyer, staking.id(), &lock_id).await?;

    worker.fast_forward(8_000).await?;

    let balance_before = buyer.view_account().await?.balance;
    buyer_withdraw(&buyer, staking.id(), pool.id()).await?;

    let balance_after = buyer.view_account().await?.balance;
    assert!(
        balance_after > balance_before,
        "withdraw should transfer NEAR after settlement prefetch and tranche payout"
    );

    Ok(())
}

#[tokio::test]
async fn cancel_subscription_after_long_idle_normalizes_end_and_renews_from_fresh_subscription()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _product_id_oneoff, _price_id_oneoff) =
        setup_staking_fixture(&worker).await?;
    let (sub_product_id, sub_price_id) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let _first_lock = buyer_lock_for_subscription(&buyer, staking.id(), &sub_price_id, 50).await?;

    let sub_initial: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": sub_product_id,
        }))
        .await?
        .json()?;
    let sid_initial = sub_initial["subscription_id"]
        .as_str()
        .expect("subscription_id")
        .to_string();
    let start_ns = json_u64_field(&sub_initial["start_ns"]).expect("start_ns");
    let end_ns = json_u64_field(&sub_initial["end_ns"]).expect("end_ns");
    let period_ns = end_ns.saturating_sub(start_ns);
    let late_ts = end_ns
        .saturating_add(period_ns.saturating_mul(2))
        .saturating_add(1);
    fast_forward_until_timestamp(&worker, late_ts).await?;

    buyer_cancel_subscription(&buyer, staking.id(), &sub_product_id).await?;

    let sub_after_cancel: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": sub_product_id,
        }))
        .await?
        .json()?;
    let cancel_end_ns = json_u64_field(&sub_after_cancel["end_ns"]).expect("end_ns");
    assert!(
        cancel_end_ns > late_ts,
        "cancel-at-period-end should normalize stale subscription window to current virtual cycle"
    );
    assert_eq!(sub_after_cancel["cancel_at_period_end"], json!(true));

    fast_forward_until_timestamp(&worker, cancel_end_ns.saturating_add(1)).await?;
    let _second_lock = buyer_lock_for_subscription(&buyer, staking.id(), &sub_price_id, 50).await?;

    let sub_after_renew: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": sub_product_id,
        }))
        .await?
        .json()?;
    let sid_after = sub_after_renew["subscription_id"]
        .as_str()
        .expect("subscription_id");
    assert_ne!(
        sid_after, sid_initial,
        "after cancel-at-period-end boundary, renewal should create a fresh subscription row"
    );
    assert_eq!(sub_after_renew["cancel_at_period_end"], json!(false));

    Ok(())
}

#[tokio::test]
async fn cancel_subscription_after_long_idle_then_unlock_requests_unstake()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _product_id_oneoff, _price_id_oneoff) =
        setup_staking_fixture(&worker).await?;
    let (sub_product_id, sub_price_id) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let buyer = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id = buyer_lock_for_subscription(&buyer, staking.id(), &sub_price_id, 50).await?;

    // Ensure initial pending stake is settled before we run unlock later.
    worker.fast_forward(100_000).await?;
    call_epoch_settle(&buyer, staking.id(), pool.id())
        .await?
        .into_result()?;

    let sub_initial: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": sub_product_id,
        }))
        .await?
        .json()?;
    let start_ns = json_u64_field(&sub_initial["start_ns"]).expect("start_ns");
    let end_ns = json_u64_field(&sub_initial["end_ns"]).expect("end_ns");
    let period_ns = end_ns.saturating_sub(start_ns);
    let late_ts = end_ns
        .saturating_add(period_ns.saturating_mul(2))
        .saturating_add(1);
    fast_forward_until_timestamp(&worker, late_ts).await?;

    buyer_cancel_subscription(&buyer, staking.id(), &sub_product_id).await?;
    let sub_after_cancel: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": sub_product_id,
        }))
        .await?
        .json()?;
    let cancel_end_ns = json_u64_field(&sub_after_cancel["end_ns"]).expect("end_ns");
    fast_forward_until_timestamp(&worker, cancel_end_ns.saturating_add(1)).await?;

    buyer_unlock(&buyer, staking.id(), &lock_id).await?;

    let lock_after: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    assert_eq!(lock_after["status"], json!("UnlockRequested"));

    let v_after_unlock = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert!(
        json_near_token_yocto(&v_after_unlock["pending_to_unstake"]).unwrap_or(0) > 0,
        "unlock after cancel-at-end should queue pending_to_unstake for later settle"
    );
    assert_eq!(
        json_tx_status(&v_after_unlock["tx_status"]),
        Some("Idle"),
        "unlock flow should release validator Busy status"
    );

    Ok(())
}
