//! Documented golden-path sandbox E2E for one-off catalog locks:
//!
//! `lock_for_product` → `epoch_settle` (stake on pool) → wait for `lock.end_ns` → `unlock` →
//! two `epoch_settle` passes (pool unstake + withdraw into contract) → `withdraw(validator_id)` → buyer receives NEAR.
//!
//! Build: `make staking-contract-test mock-staking-pool-contract` (from repo root).
//! Run: `cargo test -p staking-contract --test sandbox_golden_path`

mod mock_pool;

use mock_pool::{
    buyer_lock_for_product, buyer_storage_deposit, buyer_withdraw,
    drive_validator_settlement_epochs, fetch_validator, json_near_token_yocto, json_tx_status,
    setup_staking_fixture, unlock_lock_after_expiry,
};
use serde_json::json;

const SHORT_LOCK_NS: &str = "1000000000";

/// End-to-end exit path for a single one-off lock (see module docs).
#[tokio::test]
async fn golden_path_lock_settle_unlock_withdraw_pays_buyer()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, _owner, _product_id, price_id) = setup_staking_fixture(&worker).await?;
    let buyer = worker.dev_create_account().await?;
    let operator = worker.dev_create_account().await?;

    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id =
        buyer_lock_for_product(&buyer, staking.id(), &price_id, SHORT_LOCK_NS, 50).await?;

    // Step 1 — stake queued NEAR from the lock onto the mock pool.
    drive_validator_settlement_epochs(&worker, &operator, staking.id(), pool.id(), 1).await?;
    let v_staked = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(
        json_near_token_yocto(&v_staked["pending_to_stake"]).unwrap_or(0),
        0,
        "epoch_settle should move lock stake onto the pool"
    );
    assert_eq!(json_tx_status(&v_staked["tx_status"]), Some("Idle"));

    // Step 2 — unlock after lock expiry (queues pool unstake).
    unlock_lock_after_expiry(&worker, &buyer, staking.id(), &lock_id).await?;
    let lock_unlocked: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    assert_eq!(lock_unlocked["status"], json!("UnlockRequested"));
    let v_unlock = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert!(
        json_near_token_yocto(&v_unlock["pending_to_unstake"]).unwrap_or(0) > 0,
        "unlock should queue validator pending_to_unstake"
    );

    // Step 3 — two fresh epochs: pool unstake, then pool withdraw into pending_to_claim.
    drive_validator_settlement_epochs(&worker, &operator, staking.id(), pool.id(), 2).await?;

    // Step 4 — user withdraw receives NEAR.
    let balance_before = buyer.view_account().await?.balance;
    buyer_withdraw(&buyer, staking.id(), pool.id()).await?;
    let balance_after = buyer.view_account().await?.balance;
    assert!(
        balance_after > balance_before,
        "withdraw should transfer claimable NEAR to the buyer"
    );

    let v_done = fetch_validator(&worker, staking.id(), pool.id()).await?;
    assert_eq!(json_tx_status(&v_done["tx_status"]), Some("Idle"));

    Ok(())
}
