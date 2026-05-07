//! Sandbox tests for `staking-contract` WASM (Linux CI / Apple Silicon).
//! Build WASM: `make staking-contract` from `house-of-stake-contracts/`.

use near_workspaces::operations::Function;
use near_workspaces::types::{Gas, NearToken};
use serde_json::json;

fn staking_wasm_bytes() -> Result<Vec<u8>, std::io::Error> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for rel in [
        "res/local/staking_contract.wasm",
        "target/near/staking_contract/staking_contract.wasm",
    ] {
        let p = root.join(rel);
        if let Ok(bytes) = std::fs::read(&p) {
            return Ok(bytes);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "staking_contract.wasm missing (run from repo: make staking-contract)",
    ))
}

fn staking_new_args(owner: &near_workspaces::AccountId) -> serde_json::Value {
    json!({
        "config": {
            "owner_account_id": owner,
            "proposed_new_owner_account_id": null,
            "guardians": [],
            "operators": [],
            "min_lock_duration_ns": "1",
            "max_lock_duration_ns": "10000000000000000000",
            "epoch_unstake_settle_epochs": 4,
            "min_storage_deposit": "10000000000000000000000",
            "per_lock_storage_stake": "0",
            "min_lock_amount": "1000000000000000000000",
        }
    })
}

#[tokio::test]
async fn staking_contract_deploy_and_get_config() -> Result<(), Box<dyn std::error::Error>> {
    let wasm = staking_wasm_bytes().map_err(|e| format!("{e}"))?;
    let worker = near_workspaces::sandbox().await?;
    let contract_account = worker.dev_create_account().await?;
    let owner = contract_account.id().clone();

    let outcome = contract_account
        .batch(contract_account.id())
        .deploy(&wasm)
        .call(
            Function::new("new")
                .args_json(staking_new_args(&owner))
                .gas(Gas::from_tgas(50)),
        )
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "deploy+init failed: {:#?}",
        outcome.outcomes()
    );

    let config: serde_json::Value = worker
        .view(contract_account.id(), "get_config")
        .await?
        .json()?;
    let oid = config["owner_account_id"].as_str().expect("owner");
    assert_eq!(oid, owner.as_str());

    Ok(())
}

#[tokio::test]
async fn staking_contract_storage_deposit_get_account() -> Result<(), Box<dyn std::error::Error>> {
    let wasm = staking_wasm_bytes().map_err(|e| format!("{e}"))?;
    let worker = near_workspaces::sandbox().await?;
    let contract_account = worker.dev_create_account().await?;
    let owner = contract_account.id().clone();

    let outcome = contract_account
        .batch(contract_account.id())
        .deploy(&wasm)
        .call(
            Function::new("new")
                .args_json(staking_new_args(&owner))
                .gas(Gas::from_tgas(50)),
        )
        .transact()
        .await?;
    assert!(outcome.is_success(), "deploy+init failed");

    let user = worker.dev_create_account().await?;
    user.call(contract_account.id(), "storage_deposit")
        .deposit(NearToken::from_millinear(500))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?
        .into_result()?;

    let acc: serde_json::Value = worker
        .view(contract_account.id(), "get_account")
        .args_json(json!({ "account_id": user.id() }))
        .await?
        .json()?;
    let storage = acc["storage_deposit"]
        .as_str()
        .expect("storage_deposit string");
    assert_ne!(storage, "0");

    Ok(())
}
