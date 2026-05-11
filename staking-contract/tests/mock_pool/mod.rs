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

/// Empty `operators` so any account may call `epoch_*`; `epoch_unstake_settle_epochs: 1` shortens waits.
pub fn staking_new_args_e2e(owner: &near_workspaces::AccountId) -> serde_json::Value {
    json!({
        "config": {
            "owner_account_id": owner,
            "proposed_new_owner_account_id": null,
            "guardians": [],
            "operators": [],
            "min_lock_duration_ns": "1",
            "max_lock_duration_ns": "10000000000000000000",
            "epoch_unstake_settle_epochs": 1,
            "min_storage_deposit": "10000000000000000000000",
            "per_lock_storage_stake": "0",
            "min_lock_amount": "1000000000000000000000",
        }
    })
}

/// Advance sandbox blocks until the chain timestamp reaches `target_ns` (used for `unlock` after `lock.end_ns`).
///
/// Uses larger `fast_forward` steps when far behind so multi-day lock windows remain reachable in CI.
pub async fn fast_forward_until_timestamp(
    worker: &Worker<Sandbox>,
    target_ns: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_ROUNDS: u32 = 250;
    for _ in 0..MAX_ROUNDS {
        let ts = worker.view_block().await?.timestamp();
        if ts >= target_ns {
            return Ok(());
        }
        let gap = target_ns.saturating_sub(ts);
        let blocks = if gap > 500_000_000_000_000 {
            80_000u64
        } else if gap > 50_000_000_000_000 {
            25_000
        } else if gap > 5_000_000_000_000 {
            5_000
        } else if gap > 500_000_000_000 {
            500
        } else if gap > 50_000_000_000 {
            100
        } else {
            25
        };
        worker.fast_forward(blocks).await?;
    }
    let last = worker.view_block().await?.timestamp();
    Err(format!("timestamp {target_ns} not reached after fast_forward (last {last:?})",).into())
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
            "pool_account_id": pool.id(),
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
