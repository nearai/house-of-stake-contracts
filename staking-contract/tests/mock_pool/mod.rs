//! Sandbox helpers: deploy [`mock-staking-pool-contract`] + [`staking-contract`] WASM and drive flows.
//! Build from repo root: `make staking-contract`, `make mock-staking-pool-contract`.

use near_workspaces::Worker;
use near_workspaces::network::Sandbox;
use near_workspaces::operations::Function;
use near_workspaces::types::{Gas as WsGas, NearToken};
use serde_json::json;
use std::path::Path;

/// `10^24` — matches `staking_contract::internal::LOCK_FACTOR_DENOM`.
pub const LOCK_FACTOR_DENOM: &str = "1000000000000000000000000";

/// Prepaid gas for `lock_for_product`, `unlock`, `withdraw`, and `epoch_settle`.
/// NEAR caps prepaid gas at 300 TGas per transaction; long settlement promise chains
/// forward unused gas from this budget.
pub const SETTLEMENT_PIPELINE_GAS_TGAS: u64 = 300;

/// Resolve WASM bytes from typical build outputs (`make …`, `cargo near`, or `cargo build --target wasm32`).
pub fn wasm_from_candidates(rel_paths: &[&str]) -> Result<Vec<u8>, std::io::Error> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for rel in rel_paths {
        let p = root.join(rel);
        if let Ok(bytes) = std::fs::read(&p) {
            return Ok(bytes);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("wasm not found (tried {rel_paths:?})"),
    ))
}

pub fn staking_wasm_bytes() -> Result<Vec<u8>, std::io::Error> {
    wasm_from_candidates(&[
        "res/local/staking_contract.wasm",
        "target/near/staking_contract/staking_contract.wasm",
        "target/wasm32-unknown-unknown/release/staking_contract.wasm",
    ])
}

pub fn mock_pool_wasm_bytes() -> Result<Vec<u8>, std::io::Error> {
    wasm_from_candidates(&[
        "res/local/mock_staking_pool_contract.wasm",
        "target/near/mock_staking_pool_contract/mock_staking_pool_contract.wasm",
        "target/wasm32-unknown-unknown/release/mock_staking_pool_contract.wasm",
    ])
}

/// Args for `staking-contract` `new` in sandbox (no public `epoch_*`; pool work follows user actions).
pub fn staking_new_args_e2e(owner: &near_workspaces::AccountId) -> serde_json::Value {
    json!({
        "config": {
            "owner_account_id": owner,
            "proposed_new_owner_account_id": null,
            "guardians": [],
            "min_lock_duration_ns": "1",
            "max_lock_duration_ns": "10000000000000000000",
            "epoch_unstake_settle_epochs": 1,
            "min_storage_deposit": "10000000000000000000000",
            "per_lock_storage_stake": "0",
            "min_lock_amount": "1000000000000000000000000",
        }
    })
}

/// Advance sandbox blocks until the chain timestamp reaches `target_ns` (used for `unlock` after `lock.end_ns`).
///
/// Uses adaptive bounded steps so very large jumps do not stall on one long `fast_forward` call.
pub async fn fast_forward_until_timestamp(
    worker: &Worker<Sandbox>,
    target_ns: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Conservative defaults for sandbox stability; tune by observed timestamp deltas.
    const DEFAULT_NS_PER_BLOCK: u64 = 1_000_000_000;
    const MIN_STEP_BLOCKS: u64 = 50;
    const MAX_STEP_BLOCKS: u64 = 20_000;
    const MAX_ROUNDS: u32 = 5_000;
    const MAX_STALLED_ROUNDS: u32 = 12;

    let mut ns_per_block = DEFAULT_NS_PER_BLOCK;
    let mut stalled_rounds = 0u32;
    for round in 0..MAX_ROUNDS {
        let ts_before = worker.view_block().await?.timestamp();
        if ts_before >= target_ns {
            return Ok(());
        }
        let gap = target_ns.saturating_sub(ts_before);
        let blocks_guess = (gap / ns_per_block).max(1);
        let blocks = blocks_guess.clamp(MIN_STEP_BLOCKS, MAX_STEP_BLOCKS);
        if round % 10 == 0 {
            eprintln!(
                "[timing][ff-ts] round={} gap_ns={} step_blocks={} est_ns_per_block={}",
                round, gap, blocks, ns_per_block
            );
        }
        worker.fast_forward(blocks).await?;
        let ts_after = worker.view_block().await?.timestamp();
        let advanced_ns = ts_after.saturating_sub(ts_before);

        if advanced_ns == 0 {
            stalled_rounds = stalled_rounds.saturating_add(1);
            if stalled_rounds >= MAX_STALLED_ROUNDS {
                return Err(format!(
                    "timestamp did not advance after {MAX_STALLED_ROUNDS} rounds (target={target_ns}, last={ts_after})"
                )
                .into());
            }
            continue;
        }

        stalled_rounds = 0;
        ns_per_block = (advanced_ns / blocks).max(1);
    }
    let last = worker.view_block().await?.timestamp();
    Err(format!("timestamp {target_ns} not reached after fast_forward (last {last:?})",).into())
}

/// Advance by many blocks in smaller chunks to avoid long single fast-forward calls.
pub async fn fast_forward_blocks_chunked(
    worker: &Worker<Sandbox>,
    total_blocks: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    const STEP: u64 = 10_000;
    let mut left = total_blocks;
    while left > 0 {
        let step = left.min(STEP);
        worker.fast_forward(step).await?;
        left = left.saturating_sub(step);
    }
    Ok(())
}

/// Advance until at least `delta_epochs` epoch-id transitions are observed.
pub async fn fast_forward_until_epoch_delta(
    worker: &Worker<Sandbox>,
    delta_epochs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if delta_epochs == 0 {
        return Ok(());
    }
    let mut current_epoch_id = worker.view_block().await?.epoch_id().clone();
    let mut crossed = 0u64;
    const MAX_ROUNDS: u32 = 250;
    for _ in 0..MAX_ROUNDS {
        let now_epoch_id = worker.view_block().await?.epoch_id().clone();
        if now_epoch_id != current_epoch_id {
            crossed = crossed.saturating_add(1);
            current_epoch_id = now_epoch_id;
        }
        if crossed >= delta_epochs {
            return Ok(());
        }
        // Keep each jump moderate to avoid long single fast-forward stalls.
        worker.fast_forward(2_000).await?;
    }
    Err(format!(
        "did not cross {delta_epochs} epoch transitions after fast_forward rounds (crossed {crossed})"
    )
    .into())
}

pub fn near_token_yocto_from_view(
    v: &serde_json::Value,
) -> Result<u128, Box<dyn std::error::Error>> {
    json_near_token_yocto(v).ok_or_else(|| format!("unexpected NearToken JSON: {v}").into())
}

pub fn json_near_token_yocto(v: &serde_json::Value) -> Option<u128> {
    if let Some(s) = v.as_str() {
        return s.parse().ok();
    }
    if let Some(s) = v.get("amount").and_then(|x| x.as_str()) {
        return s.parse().ok();
    }
    None
}

pub fn json_u64_field(v: &serde_json::Value) -> Option<u64> {
    v.as_str()?.parse().ok()
}

pub fn json_u64_field_any(v: &serde_json::Value) -> Option<u64> {
    json_u64_field(v).or_else(|| v.as_u64())
}

pub fn json_tx_status(v: &serde_json::Value) -> Option<&str> {
    v.as_str()
}

/// Validator row from `get_validator`.
pub async fn fetch_validator(
    worker: &Worker<Sandbox>,
    staking_id: &near_workspaces::AccountId,
    pool_id: &near_workspaces::AccountId,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    Ok(worker
        .view(staking_id, "get_validator")
        .args_json(json!({ "validator_id": pool_id }))
        .await?
        .json()?)
}

pub async fn buyer_storage_deposit(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
) -> Result<(), Box<dyn std::error::Error>> {
    buyer
        .call(staking_id, "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

pub async fn buyer_lock_for_product(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    price_id: &str,
    lock_duration_ns: &str,
    deposit_near: u128,
) -> Result<String, Box<dyn std::error::Error>> {
    let lock_id: String = buyer
        .call(staking_id, "lock_for_product")
        .args_json(json!({
            "price_id": price_id,
            "lock_duration_ns": lock_duration_ns,
            "product_id": null,
        }))
        .deposit(NearToken::from_near(deposit_near))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?
        .json()?;
    Ok(lock_id)
}

pub async fn buyer_lock_for_subscription(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    price_id: &str,
    deposit_near: u128,
) -> Result<String, Box<dyn std::error::Error>> {
    let lock_id: String = buyer
        .call(staking_id, "lock_for_subscription")
        .args_json(json!({
            "price_id": price_id,
            "product_id": null,
        }))
        .deposit(NearToken::from_near(deposit_near))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?
        .json()?;
    Ok(lock_id)
}

pub async fn buyer_cancel_subscription(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    product_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    buyer
        .call(staking_id, "cancel_subscription")
        .args_json(json!({ "product_id": product_id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

pub async fn buyer_unlock(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    lock_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    buyer
        .call(staking_id, "unlock")
        .args_json(json!({ "lock_id": lock_id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

pub async fn buyer_withdraw(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    pool_id: &near_workspaces::AccountId,
) -> Result<(), Box<dyn std::error::Error>> {
    buyer_withdraw_result(buyer, staking_id, pool_id)
        .await?
        .into_result()?;
    Ok(())
}

pub async fn buyer_withdraw_result(
    buyer: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    pool_id: &near_workspaces::AccountId,
) -> Result<near_workspaces::result::ExecutionFinalResult, near_workspaces::error::Error> {
    buyer
        .call(staking_id, "withdraw")
        .args_json(json!({ "validator_id": pool_id }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await
}

pub async fn call_epoch_settle(
    caller: &near_workspaces::Account,
    staking_id: &near_workspaces::AccountId,
    pool_id: &near_workspaces::AccountId,
) -> Result<near_workspaces::result::ExecutionFinalResult, near_workspaces::error::Error> {
    caller
        .call(staking_id, "epoch_settle")
        .args_json(json!({ "validator_id": pool_id }))
        .gas(WsGas::from_tgas(SETTLEMENT_PIPELINE_GAS_TGAS))
        .transact()
        .await
}

pub async fn pool_set_fail_get_account(
    validator_owner: &near_workspaces::Account,
    pool_id: &near_workspaces::AccountId,
    fail: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    validator_owner
        .call(pool_id, "set_fail_get_account")
        .args_json(json!({ "fail": fail }))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

pub async fn pool_set_fail_next_withdraw(
    validator_owner: &near_workspaces::Account,
    pool_id: &near_workspaces::AccountId,
    fail: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    validator_owner
        .call(pool_id, "set_fail_next_withdraw")
        .args_json(json!({ "fail": fail }))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

pub async fn pool_total_balance_yocto(
    worker: &Worker<Sandbox>,
    pool_id: &near_workspaces::AccountId,
    staking_id: &near_workspaces::AccountId,
) -> Result<u128, Box<dyn std::error::Error>> {
    let bal_json: serde_json::Value = worker
        .view(pool_id, "get_account_total_balance")
        .args_json(json!({ "account_id": staking_id }))
        .await?
        .json()?;
    near_token_yocto_from_view(&bal_json)
}

/// Deploy staking + mock pool, allowlist pool, and create a one-off catalog price.
pub async fn setup_staking_fixture(
    worker: &Worker<Sandbox>,
) -> Result<
    (
        near_workspaces::Account,
        near_workspaces::Account,
        near_workspaces::Account,
        String,
        String,
    ),
    Box<dyn std::error::Error>,
> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

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
    let (product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    Ok((staking, pool, validator_owner, product_id, price_id))
}

/// Same as [`setup_staking_fixture`] but allows overriding `epoch_unstake_settle_epochs`.
pub async fn setup_staking_fixture_with_unstake_settle_epochs(
    worker: &Worker<Sandbox>,
    epoch_unstake_settle_epochs: u64,
) -> Result<
    (
        near_workspaces::Account,
        near_workspaces::Account,
        near_workspaces::Account,
        String,
        String,
    ),
    Box<dyn std::error::Error>,
> {
    let staking_wasm = staking_wasm_bytes().map_err(|e| format!("staking wasm: {e}"))?;
    let pool_wasm = mock_pool_wasm_bytes().map_err(|e| format!("mock pool wasm: {e}"))?;

    let staking = worker.dev_create_account().await?;
    let pool = worker.dev_create_account().await?;
    let validator_owner = worker.dev_create_account().await?;

    staking
        .batch(staking.id())
        .deploy(&staking_wasm)
        .call(
            Function::new("new")
                .args_json(json!({
                    "config": {
                        "owner_account_id": staking.id(),
                        "proposed_new_owner_account_id": null,
                        "guardians": [],
                        "min_lock_duration_ns": "1",
                        "max_lock_duration_ns": "10000000000000000000",
                        "epoch_unstake_settle_epochs": epoch_unstake_settle_epochs,
                        "min_storage_deposit": "10000000000000000000000",
                        "per_lock_storage_stake": "0",
                        "min_lock_amount": "1000000000000000000000000",
                    }
                }))
                .gas(WsGas::from_tgas(50)),
        )
        .transact()
        .await?
        .into_result()?;

    pool.batch(pool.id())
        .deploy(&pool_wasm)
        .call(
            Function::new("new")
                .args_json(json!({ "owner_id": validator_owner.id() }))
                .gas(WsGas::from_tgas(30)),
        )
        .transact()
        .await?
        .into_result()?;

    add_validator_pair(&staking, &pool).await?;
    let (product_id, price_id) =
        create_one_off_product_and_price(&staking, &pool, &validator_owner).await?;

    Ok((staking, pool, validator_owner, product_id, price_id))
}

/// Deploy staking contract on `staking`, mock pool on `pool`; pool owner = `validator_owner`.
pub async fn deploy_staking_and_mock_pool(
    staking: &near_workspaces::Account,
    pool: &near_workspaces::Account,
    validator_owner: &near_workspaces::AccountId,
    staking_wasm: &[u8],
    pool_wasm: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    staking
        .batch(staking.id())
        .deploy(staking_wasm)
        .call(
            Function::new("new")
                .args_json(staking_new_args_e2e(staking.id()))
                .gas(WsGas::from_tgas(50)),
        )
        .transact()
        .await?
        .into_result()?;

    pool.batch(pool.id())
        .deploy(pool_wasm)
        .call(
            Function::new("new")
                .args_json(json!({ "owner_id": validator_owner }))
                .gas(WsGas::from_tgas(30)),
        )
        .transact()
        .await?
        .into_result()?;

    Ok(())
}

pub async fn add_validator_pair(
    staking: &near_workspaces::Account,
    pool: &near_workspaces::Account,
) -> Result<(), Box<dyn std::error::Error>> {
    staking
        .call(staking.id(), "add_validator")
        .args_json(json!({
            "validator_id": pool.id(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}

/// Validator owner: create an active one-off product + price on [`staking-contract`] (pool must be allowlisted).
pub async fn create_one_off_product_and_price(
    staking: &near_workspaces::Account,
    pool: &near_workspaces::Account,
    validator_owner: &near_workspaces::Account,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let cp = validator_owner
        .call(staking.id(), "create_product")
        .args_json(json!({
            "validator_id": pool.id(),
            "name": "Fixture Product",
            "description": "sandbox",
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(200))
        .transact()
        .await?;
    assert!(cp.is_success(), "create_product: {:#?}", cp.outcomes());
    let product_id: String = cp.into_result()?.json()?;

    let cpr = validator_owner
        .call(staking.id(), "create_price")
        .args_json(json!({
            "product_id": product_id,
            "name": "One-off",
            "description": "",
            "amount": "1",
            "price_type": "OneOff",
            "billing_period": null,
            "lock_factor_near_months": LOCK_FACTOR_DENOM,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(200))
        .transact()
        .await?;
    assert!(cpr.is_success(), "create_price: {:#?}", cpr.outcomes());
    let price_id: String = cpr.into_result()?.json()?;
    Ok((product_id, price_id))
}

/// Validator owner: create an active recurring-monthly product + price for subscription flows.
pub async fn create_subscription_product_and_price(
    staking: &near_workspaces::Account,
    pool: &near_workspaces::Account,
    validator_owner: &near_workspaces::Account,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let cp = validator_owner
        .call(staking.id(), "create_product")
        .args_json(json!({
            "validator_id": pool.id(),
            "name": "Fixture Sub Product",
            "description": "sandbox-subscription",
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(200))
        .transact()
        .await?;
    assert!(cp.is_success(), "create_product: {:#?}", cp.outcomes());
    let product_id: String = cp.into_result()?.json()?;

    let cpr = validator_owner
        .call(staking.id(), "create_price")
        .args_json(json!({
            "product_id": product_id,
            "name": "Monthly",
            "description": "",
            "amount": "1",
            "price_type": "Recurring",
            "billing_period": "Monthly",
            "lock_factor_near_months": LOCK_FACTOR_DENOM,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(WsGas::from_tgas(200))
        .transact()
        .await?;
    assert!(cpr.is_success(), "create_price: {:#?}", cpr.outcomes());
    let price_id: String = cpr.into_result()?.json()?;
    Ok((product_id, price_id))
}
