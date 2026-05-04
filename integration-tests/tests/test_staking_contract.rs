//! Smoke test: deploy `staking-contract`, call `new`, read `get_config`.
//! Requires WASM at `res/local/staking_contract.wasm` or `target/near/...` — run `make staking-contract`
//! from `house-of-stake-contracts/`.

use near_sdk::Gas;
use near_workspaces::operations::Function;
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

#[tokio::test]
async fn staking_contract_deploy_and_get_config() -> Result<(), Box<dyn std::error::Error>> {
    let wasm = staking_wasm_bytes().map_err(|e| format!("{e}"))?;
    let worker = near_workspaces::sandbox().await?;
    let contract_account = worker.dev_create_account().await?;
    let owner = contract_account.id().clone();

    let args = json!({
        "config": {
            "owner_account_id": owner,
            "proposed_new_owner_account_id": null,
            "guardians": [],
            "operators": [],
            "oracle_account_id": owner,
            "oracle_max_age_ns": "1000000000000000000",
            "oracle_max_recency_duration_sec": 0,
            "min_lock_duration_ns": "1",
            "max_lock_duration_ns": "10000000000000000000",
            "epoch_unstake_settle_epochs": 4,
            "min_storage_deposit": "10000000000000000000000",
            "per_lock_storage_stake": "0",
            "min_lock_amount": "1000000000000000000000",
        }
    });

    let outcome = contract_account
        .batch(contract_account.id())
        .deploy(&wasm)
        .call(
            Function::new("new")
                .args_json(args)
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
