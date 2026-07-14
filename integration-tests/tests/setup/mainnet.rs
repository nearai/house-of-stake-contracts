//! Helpers for pulling live mainnet contracts (code + state) into a sandbox.
#![allow(dead_code)]

use base64::Engine;
use near_sdk::NearToken;
use near_workspaces::network::Sandbox;
use near_workspaces::types::{KeyType, SecretKey};
use near_workspaces::{AccessKey, Account, AccountDetailsPatch, AccountId, Contract, Worker};
use serde_json::json;

pub const MAINNET_RPC: &str = "https://rpc.intea.rs";

/// Fetch the account's on-chain `storage_usage`. Patched accounts must carry the real value:
/// the sandbox patch defaults it to 0, and the first state write that replaces an existing
/// value then underflows the account's storage accounting and crashes the sandbox node.
pub async fn fetch_mainnet_storage_usage(
    client: &reqwest::Client,
    account_id: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let response: serde_json::Value = client
        .post(MAINNET_RPC)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "query",
            "params": {
                "request_type": "view_account",
                "finality": "final",
                "account_id": account_id
            }
        }))
        .send()
        .await?
        .json()
        .await?;
    response["result"]["storage_usage"]
        .as_u64()
        .ok_or_else(|| "Missing storage_usage in view_account response".into())
}

pub async fn fetch_mainnet_code(
    client: &reqwest::Client,
    account_id: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let response: serde_json::Value = client
        .post(MAINNET_RPC)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "query",
            "params": {
                "request_type": "view_code",
                "finality": "final",
                "account_id": account_id
            }
        }))
        .send()
        .await?
        .json()
        .await?;
    let code_base64 = response["result"]["code_base64"]
        .as_str()
        .ok_or("Missing code_base64 in response")?;
    Ok(base64::engine::general_purpose::STANDARD.decode(code_base64)?)
}

/// Fetch all contract state from mainnet using INTEAR's paginated `view_state` extension.
pub async fn fetch_mainnet_state(
    client: &reqwest::Client,
    account_id: &str,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Box<dyn std::error::Error>> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut all_state: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let params = if let Some(ref c) = cursor {
            json!({
                "request_type": "INTEAR_paginated_view_state",
                "finality": "final",
                "account_id": account_id,
                "prefix_base64": "",
                "cursor_base64": c
            })
        } else {
            json!({
                "request_type": "INTEAR_paginated_view_state",
                "finality": "final",
                "account_id": account_id,
                "prefix_base64": ""
            })
        };

        let response: serde_json::Value = client
            .post(MAINNET_RPC)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": "1",
                "method": "query",
                "params": params
            }))
            .send()
            .await?
            .json()
            .await?;

        let result = &response["result"];
        let values = result["values"]
            .as_array()
            .ok_or("Missing values in paginated state response")?;

        for item in values {
            let key = b64.decode(item["key"].as_str().unwrap())?;
            let value = b64.decode(item["value"].as_str().unwrap())?;
            all_state.push((key, value));
        }

        match result.get("next_cursor") {
            Some(serde_json::Value::String(next)) => cursor = Some(next.clone()),
            _ => break,
        }
    }

    println!(
        "Fetched {} state entries from {account_id}",
        all_state.len()
    );
    Ok(all_state)
}

/// Pull a mainnet contract (code + every state entry) into the sandbox and attach a fresh
/// full-access key so the test can sign transactions for it.
pub async fn patch_mainnet_contract(
    sandbox: &Worker<Sandbox>,
    client: &reqwest::Client,
    account_id_str: &str,
) -> Result<Contract, Box<dyn std::error::Error>> {
    let account_id: AccountId = account_id_str.parse()?;
    let code = fetch_mainnet_code(client, account_id_str).await?;
    let state = fetch_mainnet_state(client, account_id_str).await?;
    let storage_usage = fetch_mainnet_storage_usage(client, account_id_str).await?;
    let sk = SecretKey::from_seed(KeyType::ED25519, account_id_str);

    sandbox
        .patch(&account_id)
        .account(
            AccountDetailsPatch::default()
                .balance(NearToken::from_near(100))
                .storage_usage(storage_usage),
        )
        .access_key(sk.public_key(), AccessKey::full_access())
        .code(&code)
        .states(state.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
        .transact()
        .await?;

    Ok(Contract::from_secret_key(account_id, sk, sandbox))
}

/// Patch a plain NEAR account (no contract) into existence so the test can sign as it.
pub async fn patch_account(
    sandbox: &Worker<Sandbox>,
    account_id_str: &str,
    balance: NearToken,
) -> Result<Account, Box<dyn std::error::Error>> {
    let account_id: AccountId = account_id_str.parse()?;
    let sk = SecretKey::from_seed(KeyType::ED25519, account_id_str);
    sandbox
        .patch(&account_id)
        .account(AccountDetailsPatch::default().balance(balance))
        .access_key(sk.public_key(), AccessKey::full_access())
        .transact()
        .await?;
    Ok(Account::from_secret_key(account_id, sk, sandbox))
}
