//! Sandbox tests for **`staking-contract`** using [`mock-staking-pool-contract`] as the pool implementation.
//! Flows follow the **lazy epoch pipeline** (`lock` / `unlock` / `withdraw(validator_id)` / `epoch_settle`); there are no public `epoch_stake` / `epoch_unstake` / `epoch_withdraw` RPCs.
//! Requires built WASMs (repo root): `make staking-contract`, `make mock-staking-pool-contract`.

mod mock_pool;

use mock_pool::{
    SETTLEMENT_PIPELINE_GAS_TGAS, add_validator_pair, call_epoch_settle,
    create_one_off_product_and_price, deploy_staking_and_mock_pool, fast_forward_until_timestamp,
    json_near_token_yocto, json_u64_field, mock_pool_wasm_bytes, near_token_yocto_from_view,
    staking_wasm_bytes,
};
use near_workspaces::types::{Gas as WsGas, NearToken};
use serde_json::json;

#[tokio::test]
async fn staking_get_validators_includes_allowlisted_pool() -> Result<(), Box<dyn std::error::Error>>
{
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let validators: Vec<serde_json::Value> = worker
        .view(staking.id(), "get_validators")
        .args_json(json!({ "from_index": 0_u64, "limit": 10_u64 }))
        .await?
        .json()?;
    let found = validators
        .iter()
        .any(|v| v["validator_id"].as_str() == Some(pool.id().as_str()));
    assert!(
        found,
        "get_validators should return the validator row for the pool"
    );

    Ok(())
}

#[tokio::test]
async fn staking_epoch_settle_fast_path_succeeds_after_lock_consumed_slot()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": "1000000000000000",
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    // `lock_for_product` already ran the per-epoch pipeline; a second `epoch_settle` is a fast-path no-op.
    call_epoch_settle(&buyer, &staking, pool.id())
        .await?
        .into_result()?;

    Ok(())
}

#[tokio::test]
async fn staking_two_locks_aggregate_then_epoch_settle_next_epoch_clears_pending()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer_a = worker.dev_create_account().await?;
    let buyer_b = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    let lock_dur = "1000000000000000";
    for buyer in [&buyer_a, &buyer_b] {
        buyer
            .call(staking.id(), "storage_deposit")
            .deposit(NearToken::from_millinear(500))
            .gas(WsGas::from_tgas(50))
            .transact()
            .await?
            .into_result()?;

        buyer
            .call(staking.id(), "lock_for_product")
            .args_json(json!({
                "price_id": price_id,
                "lock_duration_ns": lock_dur,
                "product_id": null,
            }))
            .deposit(NearToken::from_near(50))
            .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
            .transact()
            .await?
            .into_result()?;
    }

    let v_mid: serde_json::Value = worker
        .view(staking.id(), "get_validator")
        .args_json(json!({ "validator_id": pool.id() }))
        .await?
        .json()?;
    let pending_mid = json_near_token_yocto(&v_mid["pending_to_stake"]).unwrap_or(0);
    assert!(
        pending_mid > 0,
        "second lock in the same NEAR epoch should leave stake pending until a later epoch can settle"
    );

    worker.fast_forward(100_000).await?;

    let settle = call_epoch_settle(&buyer_a, &staking, pool.id()).await?;
    settle.into_result()?;

    let v_after: serde_json::Value = worker
        .view(staking.id(), "get_validator")
        .args_json(json!({ "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(
        json_near_token_yocto(&v_after["pending_to_stake"]).unwrap_or(0),
        0,
        "epoch_settle in a fresh NEAR epoch should clear accumulated pending_to_stake"
    );

    let bal_json: serde_json::Value = worker
        .view(pool.id(), "get_account_total_balance")
        .args_json(json!({ "account_id": staking.id() }))
        .await?
        .json()?;
    assert!(
        near_token_yocto_from_view(&bal_json)? > 0,
        "pool should hold the contract’s combined stake"
    );

    Ok(())
}

#[tokio::test]
async fn staking_pause_validator_blocks_new_lock_for_product()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    staking
        .call(staking.id(), "pause_validator")
        .args_json(json!({ "validator_id": pool.id() }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    let outcome = buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": "1000000000000000",
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?;

    assert!(
        outcome.is_failure(),
        "lock_for_product should fail while validator is paused"
    );

    Ok(())
}

#[tokio::test]
async fn staking_contract_pause_blocks_epoch_settle() -> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": "1000000000000000",
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    staking
        .call(staking.id(), "pause")
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(20))
        .transact()
        .await?
        .into_result()?;

    let outcome = call_epoch_settle(&buyer, &staking, pool.id()).await?;

    assert!(
        outcome.is_failure(),
        "epoch_settle should fail when staking-contract is globally paused"
    );

    Ok(())
}

#[tokio::test]
async fn staking_withdraw_succeeds_after_unlock_and_epoch_gates()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    // Short lock so `fast_forward_until_timestamp` reaches `end_ns` quickly (config allows `min_lock_duration_ns` = 1).
    let lock_duration_ns = "1000000000";
    let lock_id: String = buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": lock_duration_ns,
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?
        .json()?;

    let lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let end_ns = json_u64_field(&lock["end_ns"]).expect("lock.end_ns");
    fast_forward_until_timestamp(&worker, end_ns.saturating_add(1)).await?;

    buyer
        .call(staking.id(), "unlock")
        .args_json(json!({ "lock_id": lock_id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    worker.fast_forward(8000).await?;

    buyer
        .call(staking.id(), "withdraw")
        .args_json(json!({ "validator_id": pool.id() }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    Ok(())
}

#[tokio::test]
async fn staking_withdraw_fails_when_pool_withdraw_bucket_not_ready()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    let lock_duration_ns = "1000000000";
    let lock_id: String = buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": lock_duration_ns,
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?
        .json()?;

    let lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let end_ns = json_u64_field(&lock["end_ns"]).expect("lock.end_ns");
    fast_forward_until_timestamp(&worker, end_ns.saturating_add(1)).await?;

    buyer
        .call(staking.id(), "unlock")
        .args_json(json!({ "lock_id": lock_id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    // Not enough NEAR epochs for `epoch_unstake_settle_epochs` / pool withdraw-from-pool prefetch gates.
    worker.fast_forward(50).await?;

    let early_claim = buyer
        .call(staking.id(), "withdraw")
        .args_json(json!({ "validator_id": pool.id() }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?;

    assert!(
        early_claim.is_failure(),
        "withdraw should fail while the in-contract withdraw bucket is still empty under settle gates"
    );

    Ok(())
}

#[tokio::test]
async fn staking_create_product_fails_if_signer_is_not_pool_owner()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let stranger = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let outcome = stranger
        .call(staking.id(), "create_product")
        .args_json(json!({
            "validator_id": pool.id(),
            "name": "X",
            "description": "Y",
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?;

    assert!(
        outcome.is_failure(),
        "create_product must fail when signer is not the pool owner from get_owner_id"
    );

    Ok(())
}

#[tokio::test]
async fn staking_validator_total_staked_balance_matches_pool_after_lock()
-> Result<(), Box<dyn std::error::Error>> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let worker = near_workspaces::sandbox().await?;
    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;
    let buyer = worker.dev_create_account().await?;

    deploy_staking_and_mock_pool(
        &staking,
        &pool,
        validator_owner.id(),
        &staking_wasm,
        &pool_wasm,
    )
    .await?;
    add_validator_pair(&staking, &pool).await?;

    let (_product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    buyer
        .call(staking.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    buyer
        .call(staking.id(), "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": "1000000000000000",
            "product_id": null,
        }))
        .deposit(NearToken::from_near(50))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;

    let pool_total_json: serde_json::Value = worker
        .view(pool.id(), "get_account_total_balance")
        .args_json(json!({ "account_id": staking.id() }))
        .await?
        .json()?;
    let pool_total = near_token_yocto_from_view(&pool_total_json)?;

    let v: serde_json::Value = worker
        .view(staking.id(), "get_validator")
        .args_json(json!({ "validator_id": pool.id() }))
        .await?
        .json()?;
    let recorded = json_near_token_yocto(&v["total_staked_balance"]).unwrap_or(0);

    assert_eq!(
        recorded, pool_total,
        "after `lock_for_product`, pre-user settlement should align Validator.total_staked_balance with the pool view"
    );

    Ok(())
}
