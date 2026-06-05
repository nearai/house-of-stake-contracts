mod setup;

use crate::setup::{VENEAR_WASM_FILEPATH, VOTING_WASM_FILEPATH};
use base64::Engine;
use near_sdk::{Gas, NearToken};
use near_workspaces::network::Sandbox;
use near_workspaces::types::{KeyType, SecretKey};
use near_workspaces::{AccessKey, Account, AccountDetailsPatch, AccountId, Contract, Worker};
use serde_json::json;

const MAINNET_RPC: &str = "https://rpc.intea.rs";
const VOTING_ID: &str = "vote.dao";
const VENEAR_ID: &str = "venear.dao";
/// Both production contracts are owned by the same Sputnik DAO root account, so a single
/// patched sandbox account stands in for the owner of both during this end-to-end test.
const SHARED_OWNER_ID: &str = "hos-root.sputnik-dao.near";

const DAY_NS: u64 = 24 * 60 * 60 * 1_000_000_000;

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

/// Fetch all contract state from mainnet using INTEAR's paginated `view_state` extension.
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

    println!(
        "Fetched {} state entries from {account_id}",
        all_state.len()
    );
    Ok(all_state)
}

/// Pull a mainnet contract (code + every state entry) into the sandbox and attach a fresh
/// full-access key so the test can sign transactions for it.
async fn patch_mainnet_contract(
    sandbox: &Worker<Sandbox>,
    client: &reqwest::Client,
    account_id_str: &str,
) -> Result<Contract, Box<dyn std::error::Error>> {
    let account_id: AccountId = account_id_str.parse()?;
    let code = fetch_mainnet_code(client, account_id_str).await?;
    let state = fetch_mainnet_state(client, account_id_str).await?;
    let sk = SecretKey::from_seed(KeyType::ED25519, account_id_str);

    sandbox
        .patch(&account_id)
        .account(AccountDetailsPatch::default().balance(NearToken::from_near(100)))
        .access_key(sk.public_key(), AccessKey::full_access())
        .code(&code)
        .states(state.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
        .transact()
        .await?;

    Ok(Contract::from_secret_key(account_id, sk, sandbox))
}

/// Patch an empty NEAR account (no contract) into existence so the test can sign as it.
/// Used for the production owner / a sandbox reviewer / a sandbox proposer.
async fn patch_account(
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

async fn create_classic_proposal(
    voting: &Contract,
    proposer: &Account,
) -> Result<u32, Box<dyn std::error::Error>> {
    let outcome = proposer
        .call(voting.id(), "create_proposal")
        .args_json(json!({
            "metadata": {
                "title": "Upgrade smoke test",
                "description": "Created post-upgrade to verify voting <-> venear cross-contract calls.",
            },
            "actions": serde_json::Value::Null,
            "flow": "Classic",
        }))
        .deposit(NearToken::from_near(2))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    if !outcome.is_success() {
        return Err(format!(
            "create_proposal (Classic) failed: {:#?}",
            outcome.outcomes()
        )
        .into());
    }
    Ok(outcome.json()?)
}

async fn create_fst_proposal(
    voting: &Contract,
    proposer: &Account,
) -> Result<u32, Box<dyn std::error::Error>> {
    // FastTrack requires `bond_amount` (100 NEAR after migration) + `base_proposal_fee` + storage.
    let outcome = proposer
        .call(voting.id(), "create_proposal")
        .args_json(json!({
            "metadata": {
                "title": "Upgrade smoke test (FastTrack)",
                "description": "Created post-upgrade to verify the FastTrack flow against new venear.",
            },
            "actions": serde_json::Value::Null,
            "flow": "FastTrack",
        }))
        .deposit(NearToken::from_near(101))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    if !outcome.is_success() {
        return Err(format!(
            "create_proposal (FastTrack) failed: {:#?}",
            outcome.outcomes()
        )
        .into());
    }
    Ok(outcome.json()?)
}

async fn approve_classic(
    voting: &Contract,
    reviewer: &Account,
    proposal_id: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = reviewer
        .call(voting.id(), "approve_proposal")
        .args_json(json!({
            "proposal_id": proposal_id,
            "majority_type": serde_json::Value::Null,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(200))
        .transact()
        .await?;
    if !outcome.is_success() {
        return Err(format!(
            "approve_proposal (Classic) failed: {:#?}",
            outcome.outcomes()
        )
        .into());
    }
    Ok(())
}

async fn approve_fst(
    voting: &Contract,
    reviewer: &Account,
    proposal_id: u32,
    majority: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = reviewer
        .call(voting.id(), "approve_proposal")
        .args_json(json!({
            "proposal_id": proposal_id,
            "majority_type": majority,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(200))
        .transact()
        .await?;
    if !outcome.is_success() {
        return Err(format!(
            "approve_proposal (FastTrack) failed: {:#?}",
            outcome.outcomes()
        )
        .into());
    }
    Ok(())
}

/// Pick the highest-balance mainnet veNEAR holders directly from the (patched-in) venear
/// merkle tree, patch them into the sandbox with our own signing keys, and return their
/// account handles. Doing it dynamically keeps the test from depending on a specific
/// mainnet account ID that may churn over time.
async fn patch_mainnet_voters(
    sandbox: &Worker<Sandbox>,
    venear: &Contract,
    count: usize,
) -> Result<Vec<Account>, Box<dyn std::error::Error>> {
    let accounts: Vec<serde_json::Value> = venear
        .view("get_accounts")
        .args_json(json!({ "from_index": 0u32, "limit": 50u32 }))
        .await?
        .json()?;
    // An account's *votable* balance is `delegated_balance + own_balance * self_bps`. Anyone who
    // has delegated 100% of their voting power out (v1.0.1 stores this as a non-null `delegation`
    // entry; v1 partials show up as `delegations: [...]`) contributes 0 from their own balance.
    // Filter those out so we never try to vote from an account whose effective balance is zero.
    let mut ranked: Vec<(u128, String)> = accounts
        .iter()
        .filter_map(|info| {
            let account = &info["account"];
            let id = account["account_id"].as_str()?.to_string();
            let delegated_out = !account["delegation"].is_null()
                || account["delegations"]
                    .as_array()
                    .map(|d| !d.is_empty())
                    .unwrap_or(false);
            if delegated_out {
                return None;
            }
            let own_near: u128 = account["balance"]["near_balance"].as_str()?.parse().ok()?;
            let incoming_near: u128 = account["delegated_balance"]["near_balance"]
                .as_str()?
                .parse()
                .ok()?;
            let total = own_near.checked_add(incoming_near)?;
            if total == 0 {
                return None;
            }
            Some((total, id))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));

    let mut voters = Vec::new();
    for (_, id) in ranked.into_iter().take(count) {
        voters.push(patch_account(sandbox, &id, NearToken::from_near(5)).await?);
    }
    if voters.len() < count {
        return Err(format!(
            "Expected {count} mainnet voters but only resolved {}",
            voters.len()
        )
        .into());
    }
    Ok(voters)
}

async fn cast_vote(
    venear: &Contract,
    voting: &Contract,
    voter: &Account,
    proposal_id: u32,
    option: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (merkle_proof, v_account): (serde_json::Value, serde_json::Value) = venear
        .view("get_proof")
        .args_json(json!({ "account_id": voter.id() }))
        .await?
        .json()?;
    let outcome = voter
        .call(voting.id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": option,
            "merkle_proof": merkle_proof,
            "v_account": v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    if !outcome.is_success() {
        return Err(format!(
            "vote({option}) by {} on #{proposal_id} failed: {:#?}",
            voter.id(),
            outcome.outcomes()
        )
        .into());
    }
    Ok(())
}

#[tokio::test]
#[ignore]
/// To run this test: `cargo test test_contracts_upgrade -- --ignored --nocapture`
///
/// End-to-end migration safety check against live mainnet state for both governance contracts:
///   1. Pull `vote.dao` and `venear.dao` code+state from mainnet into the sandbox.
///   2. Upgrade voting first; confirm `migrate_state()` preserved its config and proposals.
///   3. Drive a fresh Classic proposal through `create_proposal` + `approve_proposal` so the
///      upgraded voting must call `get_snapshot()` on the still-legacy venear — proves the
///      partial-upgrade window (voting only) keeps working.
///   4. Upgrade venear; confirm `migrate_state()` preserved its config and backfilled
///      `max_delegations` to the legacy default.
///   5. Drive a second proposal end-to-end so the upgraded voting talks to the upgraded venear.
async fn test_contracts_upgrade() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = near_workspaces::sandbox().await?;
    let client = reqwest::Client::new();

    // Mainnet snapshot in sandbox.
    let voting = patch_mainnet_contract(&sandbox, &client, VOTING_ID).await?;
    let venear = patch_mainnet_contract(&sandbox, &client, VENEAR_ID).await?;
    let owner = patch_account(&sandbox, SHARED_OWNER_ID, NearToken::from_near(1_000)).await?;

    // Baseline: confirm we're testing against the expected pre-upgrade versions.
    let old_voting_version: String = voting.view("get_version").await?.json()?;
    let old_venear_version: String = venear.view("get_version").await?.json()?;
    assert_eq!(
        old_voting_version, "1.0.3",
        "Mainnet voting should be on v1.0.3 — the baseline migrate_state() upgrades from",
    );
    assert_eq!(
        old_venear_version, "1.0.1",
        "Mainnet venear should be on v1.0.1 — the baseline migrate_state() upgrades from",
    );

    let old_voting_config: serde_json::Value = voting.view("get_config").await?.json()?;
    let old_venear_config: serde_json::Value = venear.view("get_config").await?.json()?;
    let num_proposals_before: u32 = voting.view("get_num_proposals").await?.json()?;

    // Sanity-check that the live v1.0.3 voting config still carries every field
    // `migrate_state()` reads off of `OldConfig` — if mainnet ever rotates onto a
    // different layout, this test must fail loudly rather than silently skip work.
    for field in [
        "venear_account_id",
        "reviewer_ids",
        "council_ids",
        "owner_account_id",
        "voting_duration_ns",
        "timelock_duration_ns",
        "base_proposal_fee",
        "vote_storage_fee",
        "guardians",
        "proposal_expiration_ns",
        "proposed_new_owner_account_id",
        "quorum_threshold_bps",
        "quorum_floor",
        "approval_threshold_bps",
    ] {
        assert!(
            old_voting_config.get(field).is_some(),
            "v1.0.3 voting config missing expected field `{field}`: {old_voting_config:#}",
        );
    }
    assert_eq!(
        old_voting_config["owner_account_id"].as_str().unwrap(),
        SHARED_OWNER_ID,
        "Test assumes vote.dao is owned by the patched sandbox owner",
    );
    assert_eq!(
        old_venear_config["owner_account_id"].as_str().unwrap(),
        SHARED_OWNER_ID,
        "Test assumes venear.dao is owned by the patched sandbox owner",
    );

    // Capture the legacy proposals' JSON straight off the live v1.0.3 contract so that, after the
    // upgrade, we can diff each migrated proposal field-by-field against its pre-migration value.
    let old_proposals: Vec<serde_json::Value> = voting
        .view("get_proposals")
        .args_json(json!({ "from_index": 0u32, "limit": num_proposals_before }))
        .await?
        .json()?;

    // ============================================================
    // Phase 1: upgrade voting only.
    // ============================================================
    let voting_wasm = std::fs::read(VOTING_WASM_FILEPATH)?;
    let outcome = owner
        .call(voting.id(), "upgrade")
        .args(voting_wasm)
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Failed to upgrade voting contract: {:#?}",
        outcome.outcomes()
    );

    let new_voting_version: String = voting.view("get_version").await?.json()?;
    assert_eq!(
        new_voting_version,
        env!("CARGO_PKG_VERSION"),
        "Post-upgrade voting version should match the local workspace version",
    );

    let new_voting_config: serde_json::Value = voting.view("get_config").await?.json()?;

    // Fields carried verbatim from v1.0.3's `OldConfig` by `migrate_state()`.
    for field in [
        "venear_account_id",
        "reviewer_ids",
        "council_ids",
        "owner_account_id",
        "base_proposal_fee",
        "vote_storage_fee",
        "guardians",
        "proposed_new_owner_account_id",
        "quorum_threshold_bps",
        "quorum_floor",
        "approval_threshold_bps",
    ] {
        assert_eq!(
            new_voting_config[field], old_voting_config[field],
            "Voting field `{field}` was not preserved by migrate_state()",
        );
    }

    // Durations + merged-flow defaults seeded by migrate_state().
    assert_eq!(
        new_voting_config["timelock_duration_ns"],
        (14 * DAY_NS).to_string()
    );
    assert_eq!(
        new_voting_config["classic_proposal_expiration_ns"],
        (7 * DAY_NS).to_string()
    );
    assert_eq!(
        new_voting_config["fast_track_proposal_expiration_ns"],
        (2 * DAY_NS).to_string()
    );
    assert_eq!(
        new_voting_config["classic_voting_duration_ns"],
        (14 * DAY_NS).to_string()
    );
    assert_eq!(
        new_voting_config["fast_track_voting_duration_ns"],
        (5 * DAY_NS).to_string()
    );
    assert_eq!(new_voting_config["simple_majority_threshold_bps"], 5000);
    assert_eq!(new_voting_config["strong_majority_threshold_bps"], 6667);
    assert_eq!(
        new_voting_config["sandbox_duration_ns"],
        (7 * DAY_NS).to_string()
    );
    assert_eq!(new_voting_config["sandbox_threshold_bps"], 3000);
    assert_eq!(
        new_voting_config["bond_amount"],
        json!(NearToken::from_near(100))
    );
    assert_eq!(new_voting_config["treasury_account_id"], "norfolks.near");
    assert_eq!(new_voting_config["max_active_proposals"], 3);

    let num_proposals_after: u32 = voting.view("get_num_proposals").await?.json()?;
    assert_eq!(
        num_proposals_after, num_proposals_before,
        "Proposal count must not change across the voting upgrade",
    );

    // Verify migration is correct
    let migrated_proposals: Vec<serde_json::Value> = voting
        .view("get_proposals")
        .args_json(json!({ "from_index": 0u32, "limit": num_proposals_before }))
        .await?
        .json()?;
    assert_eq!(
        u32::try_from(migrated_proposals.len()).unwrap(),
        num_proposals_before,
        "Every legacy proposal must remain fetchable after migrate_state()",
    );
    for (idx, (old, new)) in old_proposals
        .iter()
        .zip(migrated_proposals.iter())
        .enumerate()
    {
        // Fields the migration must carry over verbatim from the v1.0.3 proposal (+ its metadata).
        // `status` is deliberately excluded: both contracts re-`update()` on read against the
        // current block time, so it can legitimately differ between the two reads.
        for field in [
            "id",
            "creation_time_ns",
            "proposer_id",
            "reviewer_id",
            "rejecter_id",
            "voting_start_time_ns",
            "voting_duration_ns",
            "timelock_duration_ns",
            "expiration_ns",
            "snapshot_and_state",
            "votes",
            "total_votes",
            "quorum_threshold_bps",
            "quorum_floor",
            "approval_threshold_bps",
            "actions",
            "title",
            "description",
        ] {
            assert_eq!(
                new[field], old[field],
                "Migrated proposal {idx} field `{field}` was not preserved by migrate_state()\n\
                 old: {old:#}\nnew: {new:#}",
            );
        }
        assert_eq!(
            new["id"].as_u64().unwrap(),
            u64::try_from(idx).unwrap(),
            "Migrated proposal {idx} should keep its index as `id`: {new:#}",
        );
        // New merged-flow fields the migration seeds onto every legacy (Classic) proposal.
        assert_eq!(
            new["flow"], "Classic",
            "migrate_state() rewrites every legacy proposal with flow=Classic: {new:#}",
        );
        assert_eq!(
            new["approval_time_ns"], old["voting_start_time_ns"],
            "Legacy proposal {idx} should backfill approval_time_ns from voting_start_time_ns: {new:#}",
        );
        assert!(
            new["sandbox_start_time_ns"].is_null(),
            "Legacy proposal {idx} should have no sandbox_start_time_ns: {new:#}",
        );
        assert_eq!(
            new["bond_amount"], "0",
            "Legacy proposal {idx} should carry a zero bond: {new:#}",
        );
        assert_eq!(
            new["sandbox_duration_ns"], "0",
            "Legacy proposal {idx} should carry a zero sandbox duration: {new:#}",
        );
        assert_eq!(
            new["sandbox_threshold_bps"], 0,
            "Legacy proposal {idx} should carry a zero sandbox threshold: {new:#}",
        );
    }

    // ============================================================
    // Phase 2: new voting + OLD venear.
    //
    // Approving a proposal triggers against the legacy v1.0.1 venear.
    // ============================================================

    // Add a sandbox reviewer through governance so we can drive `approve_proposal` ourselves.
    let reviewer = patch_account(&sandbox, "reviewer.test", NearToken::from_near(10)).await?;
    let mut reviewers: Vec<AccountId> =
        serde_json::from_value(new_voting_config["reviewer_ids"].clone())?;
    reviewers.push(reviewer.id().clone());
    owner
        .call(voting.id(), "set_reviewer_ids")
        .args_json(json!({ "reviewer_ids": reviewers }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    // Big balance: covers a FastTrack 100 NEAR bond + a few Classic fees with room to spare.
    let proposer = patch_account(&sandbox, "proposer.test", NearToken::from_near(250)).await?;
    // Pull a few real mainnet veNEAR holders into the sandbox so we can sign vote() calls as them.
    let voters = patch_mainnet_voters(&sandbox, &venear, 3).await?;

    let pid_old_classic = create_classic_proposal(&voting, &proposer).await?;
    approve_classic(&voting, &reviewer, pid_old_classic).await?;

    let proposal: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_old_classic }))
        .await?
        .json()?;
    assert!(
        proposal["snapshot_and_state"].is_object(),
        "snapshot_and_state must be populated by the cross-contract call into the OLD venear: \
         {proposal:#}",
    );
    assert_eq!(
        proposal["status"], "Voting",
        "Approved proposal should land in Voting status once the snapshot promise resolves",
    );

    // Cast a vote from each patched voter so we exercise vote() against the old venear's
    // merkle root (proof is verified inside the voting contract using the snapshot we just stored).
    cast_vote(&venear, &voting, &voters[0], pid_old_classic, "For").await?;
    cast_vote(&venear, &voting, &voters[1], pid_old_classic, "Against").await?;
    cast_vote(&venear, &voting, &voters[2], pid_old_classic, "Abstain").await?;

    let proposal: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_old_classic }))
        .await?
        .json()?;
    assert_eq!(
        proposal["votes"][0]["total_votes"], 1,
        "one For vote landed"
    );
    assert_eq!(
        proposal["votes"][1]["total_votes"], 1,
        "one Against vote landed"
    );
    assert_eq!(
        proposal["votes"][2]["total_votes"], 1,
        "one Abstain vote landed"
    );
    assert_eq!(
        proposal["total_votes"]["total_votes"], 3,
        "total_votes should be the sum across all three options",
    );

    // ============================================================
    // Phase 3: upgrade venear.
    // ============================================================
    let venear_wasm = std::fs::read(VENEAR_WASM_FILEPATH)?;
    let outcome = owner
        .call(venear.id(), "upgrade")
        .args(venear_wasm)
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Failed to upgrade venear contract: {:#?}",
        outcome.outcomes()
    );

    let new_venear_version: String = venear.view("get_version").await?.json()?;
    assert_eq!(
        new_venear_version,
        env!("CARGO_PKG_VERSION"),
        "Post-upgrade venear version should match the local workspace version",
    );

    let new_venear_config: serde_json::Value = venear.view("get_config").await?.json()?;
    for field in [
        "lockup_contract_config",
        "unlock_duration_ns",
        "staking_pool_whitelist_account_id",
        "lockup_code_deployers",
        "local_deposit",
        "min_lockup_deposit",
        "owner_account_id",
        "guardians",
        "proposed_new_owner_account_id",
    ] {
        assert_eq!(
            new_venear_config[field], old_venear_config[field],
            "Venear field `{field}` was not preserved by migrate_state()",
        );
    }
    // `max_delegations` is the only field migrate_state() adds — it backfills the legacy default.
    assert_eq!(new_venear_config["max_delegations"], 8);

    // ============================================================
    // Phase 4: new voting + updated venear.
    // ============================================================

    let pid_new_classic = create_classic_proposal(&voting, &proposer).await?;
    approve_classic(&voting, &reviewer, pid_new_classic).await?;

    let proposal: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_new_classic }))
        .await?
        .json()?;
    assert!(
        proposal["snapshot_and_state"].is_object(),
        "snapshot_and_state must be populated by the cross-contract call into the NEW venear: \
         {proposal:#}",
    );
    assert_eq!(
        proposal["status"], "Voting",
        "Approved proposal should land in Voting status once the snapshot promise resolves",
    );

    cast_vote(&venear, &voting, &voters[0], pid_new_classic, "For").await?;
    cast_vote(&venear, &voting, &voters[1], pid_new_classic, "Against").await?;
    let proposal: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_new_classic }))
        .await?
        .json()?;
    assert_eq!(
        proposal["votes"][0]["total_votes"], 1,
        "Classic For vote landed"
    );
    assert_eq!(
        proposal["votes"][1]["total_votes"], 1,
        "Classic Against vote landed"
    );

    // FastTrack flow: approve with a Simple majority and verify the proposal lands in Sandbox.
    //
    // Drop the sandbox graduation threshold to 1 bps so our handful of test voters can actually
    // push the proposal past it — the production default of 30% would require us to control the
    // majority of mainnet veNEAR, which isn't realistic in a sandbox. The threshold is captured
    // onto the proposal at approval time, so this must happen before `approve_fst`.
    owner
        .call(voting.id(), "set_sandbox_threshold_bps")
        .args_json(json!({ "sandbox_threshold_bps": 1u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let pid_new_fst = create_fst_proposal(&voting, &proposer).await?;
    let fst_created: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_new_fst }))
        .await?
        .json()?;
    assert_eq!(
        fst_created["bond_amount"],
        json!(NearToken::from_near(100)),
        "FastTrack proposal must hold the 100 NEAR bond until approval",
    );

    approve_fst(&voting, &reviewer, pid_new_fst, "Simple").await?;

    let fst_approved: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_new_fst }))
        .await?
        .json()?;
    assert_eq!(
        fst_approved["status"], "Sandbox",
        "FastTrack proposal should land in Sandbox after approval",
    );
    assert!(
        fst_approved["snapshot_and_state"].is_object(),
        "FastTrack approval must also snapshot from the upgraded venear: {fst_approved:#}",
    );
    assert_eq!(
        fst_approved["bond_amount"],
        json!(NearToken::from_yoctonear(0)),
        "Bond must be forwarded to the treasury once approved",
    );

    // Only "For" votes are allowed during the Sandbox window — exercise that path. With the
    // sandbox threshold dropped to 1 bps, this single `For` vote also crosses the graduation
    // threshold, so the proposal must flip Sandbox → Scheduled inline (the contract re-checks the
    // threshold on every vote). A follow-up vote in Scheduled status would fail (only Sandbox /
    // Voting accept votes), so we stop after one and assert the transition.
    cast_vote(&venear, &voting, &voters[0], pid_new_fst, "For").await?;
    let fst_after_vote: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_new_fst }))
        .await?
        .json()?;
    assert_eq!(
        fst_after_vote["votes"][0]["total_votes"], 1,
        "FastTrack Sandbox For vote should land",
    );
    assert_eq!(
        fst_after_vote["status"], "Scheduled",
        "FastTrack proposal should graduate Sandbox → Scheduled once the 1 bps threshold is met",
    );
    assert!(
        !fst_after_vote["voting_start_time_ns"].is_null(),
        "Graduating to Scheduled must seed voting_start_time_ns for the upcoming Voting window",
    );

    // Queue exercise: three active slots are now occupied (old classic + new classic +
    // new FastTrack Sandbox), so a fourth approval must land in Queued under the default cap.
    let queue_state: serde_json::Value = voting.view("get_queue_state").await?.json()?;
    assert_eq!(
        queue_state["active_proposals"].as_array().unwrap().len(),
        3,
        "Active slots should be full going into the queue test",
    );
    assert_eq!(
        queue_state["pending_queue"].as_array().unwrap().len(),
        0,
        "Pending queue should be empty before we push a fourth proposal in",
    );

    let pid_queued = create_classic_proposal(&voting, &proposer).await?;
    approve_classic(&voting, &reviewer, pid_queued).await?;
    let queued: serde_json::Value = voting
        .view("get_proposal")
        .args_json(json!({ "proposal_id": pid_queued }))
        .await?
        .json()?;
    assert_eq!(
        queued["status"], "Queued",
        "Fourth approved proposal must land Queued because the default max_active_proposals=3 is full",
    );
    assert!(
        queued["snapshot_and_state"].is_null(),
        "Queued proposals do not take a snapshot until they are promoted",
    );

    let queue_state: serde_json::Value = voting.view("get_queue_state").await?.json()?;
    assert_eq!(
        queue_state["pending_queue"],
        json!([pid_queued]),
        "pending_queue should now contain exactly the queued proposal",
    );
    assert_eq!(
        queue_state["active_proposals"].as_array().unwrap().len(),
        3,
        "Active count should still be at the cap",
    );

    Ok(())
}
