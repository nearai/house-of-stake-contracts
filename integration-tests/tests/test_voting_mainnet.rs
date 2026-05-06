mod setup;

use crate::setup::VOTING_WASM_FILEPATH;
use base64::Engine;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::{Gas, NearToken};
use near_workspaces::types::{KeyType, SecretKey};
use near_workspaces::{AccountDetailsPatch, AccountId, Contract};
use serde_json::json;
use std::fmt::Write as _;

#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
struct TestOldConfig {
    venear_account_id: String,
    reviewer_ids: Vec<String>,
    owner_account_id: String,
    voting_duration_ns: u64,
    max_number_of_voting_options: u8,
    base_proposal_fee: u128,
    vote_storage_fee: u128,
    guardians: Vec<String>,
    proposed_new_owner_account_id: Option<String>,
}

const MAINNET_RPC: &str = "https://rpc.intea.rs";

async fn fetch_mainnet_code(
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

/// Fetch all contract state from mainnet using paginated INTEAR RPC.
async fn fetch_mainnet_state(
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

    println!("Fetched {} state entries from mainnet", all_state.len());
    Ok(all_state)
}

#[tokio::test]
#[ignore]
/// To run this test: `cargo test test_voting_upgrade_from_mainnet -- --ignored --nocapture`
async fn test_voting_upgrade_from_mainnet() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = near_workspaces::sandbox().await?;
    let client = reqwest::Client::new();
    let voting_account_id: AccountId = "vote.dao".parse()?;

    // Fetch contract code and state from mainnet
    let code = fetch_mainnet_code(&client, "vote.dao").await?;
    let state_entries = fetch_mainnet_state(&client, "vote.dao").await?;

    // Set up the contract in sandbox with mainnet code + state
    let sk = SecretKey::from_seed(KeyType::ED25519, "vote.dao.test");
    sandbox
        .patch(&voting_account_id)
        .account(AccountDetailsPatch::default().balance(NearToken::from_near(100)))
        .access_key(sk.public_key(), near_workspaces::AccessKey::full_access())
        .code(&code)
        .states(
            state_entries
                .iter()
                .map(|(k, v)| (k.as_slice(), v.as_slice())),
        )
        .transact()
        .await?;

    let voting_contract = Contract::from_secret_key(voting_account_id, sk, &sandbox);

    // Read pre-migration state
    let old_config: serde_json::Value = voting_contract.view("get_config").await?.json()?;
    assert!(
        old_config.get("council_ids").is_none(),
        "Old contract should not have council_ids"
    );

    let num_proposals: u32 = voting_contract.view("get_num_proposals").await?.json()?;
    println!("Number of proposals on mainnet: {num_proposals}");

    let old_proposals: Vec<serde_json::Value> = voting_contract
        .view("get_proposals")
        .args_json(json!({"from_index": 0u32, "limit": num_proposals}))
        .await?
        .json()?;

    // Patch STATE to change owner to a sandbox
    let owner = sandbox.dev_create_account().await?;

    let state_bytes = state_entries
        .iter()
        .find(|(k, _)| k == b"STATE")
        .map(|(_, v)| v.clone())
        .expect("STATE key not found in contract state");

    let mut cursor = std::io::Cursor::new(&state_bytes);
    let mut old_cfg = TestOldConfig::deserialize_reader(&mut cursor)?;
    let rest_offset = cursor.position() as usize;
    let rest = &state_bytes[rest_offset..];

    old_cfg.owner_account_id = owner.id().to_string();

    let mut new_state = borsh::to_vec(&old_cfg)?;
    new_state.extend_from_slice(rest);

    sandbox
        .patch_state(voting_contract.id(), b"STATE", &new_state)
        .await?;

    // Perform upgrade using the real upgrade() flow
    let voting_wasm = std::fs::read(VOTING_WASM_FILEPATH)?;
    let outcome = owner
        .call(voting_contract.id(), "upgrade")
        .args(voting_wasm)
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Failed to upgrade voting contract: {:#?}",
        outcome.outcomes()
    );

    // Verify config migration
    let config: serde_json::Value = voting_contract.view("get_config").await?.json()?;

    assert_eq!(config["quorum_threshold_bps"], 3500);
    assert_eq!(config["quorum_floor"], json!(NearToken::from_near(1000)));
    assert_eq!(config["approval_threshold_bps"], 5000);
    assert_eq!(
        config["council_ids"],
        json!([
            "as.near",
            "c65255255d689f74ae46b0a89f04bbaab94d3a51ab9dc4b79b1e9b61e7cf6816",
            "e953bb69d1129e4da87b99739373884a0b57d5e64a65fdc868478f22e6c31eac",
            "fastnear-hos.near",
            "root.near",
            "norfolks.near",
        ])
    );
    // 14 days and 7 days respectively, seeded by migrate_state().
    assert_eq!(
        config["timelock_duration_ns"],
        (14u64 * 24 * 60 * 60 * 1_000_000_000).to_string()
    );
    assert_eq!(
        config["proposal_expiration_ns"],
        (7u64 * 24 * 60 * 60 * 1_000_000_000).to_string()
    );
    // Per-flow voting durations are seeded by migrate_state(): 14 days classic, 5 days FastTrack.
    assert_eq!(
        config["classic_voting_duration_ns"],
        (14u64 * 24 * 60 * 60 * 1_000_000_000).to_string()
    );
    assert_eq!(
        config["fast_track_voting_duration_ns"],
        (5u64 * 24 * 60 * 60 * 1_000_000_000).to_string()
    );

    assert_eq!(config["venear_account_id"], old_config["venear_account_id"]);
    assert_eq!(config["reviewer_ids"], old_config["reviewer_ids"]);
    assert_eq!(config["base_proposal_fee"], old_config["base_proposal_fee"]);
    assert_eq!(config["vote_storage_fee"], old_config["vote_storage_fee"]);
    assert_eq!(config["guardians"], old_config["guardians"]);
    assert_eq!(
        config["owner_account_id"].as_str().unwrap(),
        owner.id().as_str()
    );

    // Verify proposals after migration
    let post_proposals: Vec<serde_json::Value> = voting_contract
        .view("get_proposals")
        .args_json(json!({"from_index": 0u32, "limit": num_proposals}))
        .await?
        .json()?;
    assert_eq!(
        post_proposals.len(),
        old_proposals.len(),
        "Proposal count changed"
    );

    // Build table to compare old and new proposals
    let mut table = String::new();
    writeln!(table, "{:=<80}", "")?;
    writeln!(
        table,
        "{:<4} {:<30} {:>12} {:>12} {:>10}",
        "ID", "Title", "Old Status", "New Status", "Approved"
    )?;
    writeln!(table, "{:-<80}", "")?;

    for (i, new) in post_proposals.iter().enumerate() {
        let old = &old_proposals[i];

        let old_status = old["status"].as_str().unwrap();
        let new_status = new["status"].as_str().unwrap();
        let title = old["title"].as_str().unwrap();
        let title_short: String = title.chars().take(28).collect();
        let approved = old["reviewer_id"].is_string();

        writeln!(
            table,
            "{:<4} {:<30} {:>12} {:>12} {:>10}",
            i,
            title_short,
            old_status,
            new_status,
            if approved { "yes" } else { "no" },
        )?;

        // Assertions
        assert_eq!(new["quorum_threshold_bps"], 3500, "Proposal {i}");
        assert_eq!(
            new["quorum_floor"],
            json!(NearToken::from_near(1000)),
            "Proposal {i}"
        );
        assert_eq!(new["approval_threshold_bps"], 5000, "Proposal {i}");

        if old_status == "Finished" {
            assert!(
                new_status == "Succeeded" || new_status == "Defeated",
                "Proposal {i} was Finished, got {new_status}"
            );
        }

        assert_eq!(new["votes"], old["votes"], "Proposal {i} votes changed");
    }

    writeln!(table, "{:=<80}", "")?;
    println!("{table}");

    Ok(())
}
