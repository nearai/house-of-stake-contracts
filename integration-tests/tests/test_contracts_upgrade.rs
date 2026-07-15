mod setup;

use crate::setup::VENEAR_WASM_FILEPATH;
use crate::setup::mainnet::{patch_account, patch_mainnet_contract};
use common::account::Account as VenearAccount;
use common::{Bps, near_add};
use near_sdk::{AccountId, Gas, NearToken};
use near_workspaces::network::Sandbox;
use near_workspaces::{Account, Contract, Worker};
use serde_json::json;
use std::collections::HashMap;

const VENEAR_ID: &str = "venear.dao";

/// Mainnet account with failing set_delegators.
const MISALIGNED_DELEGATOR: &str = "yuensid.near";

/// Growth-rate ceiling used to bound how much veNEAR an account can legitimately accrue between
/// the pre- and post-upgrade voting-power sweeps.
const MAX_ANNUAL_GROWTH_PERCENT: u128 = 100;
const SEC_IN_YEAR: u128 = 365 * 24 * 3_600;
/// Rounding slack per delegation in an account.
const ROUNDING_SLACK: NearToken = NearToken::from_millinear(2);

async fn fetch_accounts(
    venear: &Contract,
) -> Result<Vec<VenearAccount>, Box<dyn std::error::Error>> {
    let num_accounts: u32 = venear.view("get_num_accounts").await?.json()?;
    let mut accounts: Vec<VenearAccount> = vec![];
    while u32::try_from(accounts.len()).unwrap() < num_accounts {
        let batch: Vec<serde_json::Value> = venear
            .view("get_accounts")
            .args_json(json!({ "from_index": accounts.len(), "limit": 500u32 }))
            .await?
            .json()?;
        for info in batch {
            accounts.push(serde_json::from_value(info["account"].clone())?);
        }
    }
    Ok(accounts)
}

/// Voting power under the deployed v1.1.0 formula, which scaled the owner's balance by the
/// aggregate delegated bps instead of subtracting each delegation's exact contribution.
fn legacy_voting_power(account: &VenearAccount) -> NearToken {
    let delegated_bps: u32 = account
        .delegations
        .iter()
        .map(|delegation| u32::from(delegation.bps))
        .sum();
    let delegated_bps = u16::try_from(delegated_bps).expect("delegated bps must fit into u16");
    let self_bps = Bps::new(10_000_u16.saturating_sub(delegated_bps));
    let retained = near_add(
        self_bps * account.balance.near_balance,
        self_bps * account.balance.extra_venear_balance,
    );
    near_add(account.delegated_balance.total(), retained)
}

/// Calls `set_delegations(entries)` as `delegator`. `Err` carries the contract's panic message.
async fn set_delegations(
    delegator: &Account,
    venear: &Contract,
    entries: &serde_json::Value,
) -> Result<Result<(), String>, Box<dyn std::error::Error>> {
    let outcome = delegator
        .call(venear.id(), "set_delegations")
        .args_json(json!({ "entries": entries }))
        .deposit(NearToken::from_millinear(50))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    if outcome.is_success() {
        return Ok(Ok(()));
    }
    Ok(Err(outcome
        .receipt_failures()
        .into_iter()
        // Debug, not Display: only the former carries the contract's panic message.
        .map(|failure| match failure.to_owned().into_result() {
            Err(err) => format!("{err:?}"),
            Ok(value) => format!("unexpected success: {value:?}"),
        })
        .collect::<Vec<_>>()
        .join("; ")))
}

#[tokio::test]
#[ignore]
/// To run this test: `cargo test test_venear_upgrade -- --ignored --nocapture`
///
/// Upgrade safety check for the venear delegation-pool migration against live mainnet state.
///   1. Pull `venear.dao` code + state from mainnet into the sandbox, record every account's
///      voting power, and confirm yuensid.near cannot clear its delegation on the deployed code.
///   2. Upgrade venear. `migrate_state()` recomputes every delegation pool from the exact
///      `delegation_contribution` formula; v1.1.0 credited pools with a bps-scaled amount whose
///      rounding drifts as veNEAR grows, so subtracting a delegation could underflow the pool.
///   3. Every account's voting power must survive the migration unchanged, up to veNEAR growth
///      between the two sweeps plus a milliNEAR or so of rounding per delegation.
///   4. yuensid.near, stuck before the upgrade, can now clear its delegation set and restore it.
async fn test_venear_upgrade() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox: Worker<Sandbox> = near_workspaces::sandbox().await?;
    let client = reqwest::Client::new();

    // Mainnet snapshot in sandbox.
    let venear = patch_mainnet_contract(&sandbox, &client, VENEAR_ID).await?;
    let old_config: serde_json::Value = venear.view("get_config").await?.json()?;
    let owner = patch_account(
        &sandbox,
        old_config["owner_account_id"].as_str().unwrap(),
        NearToken::from_near(1_000),
    )
    .await?;

    // Baseline: confirm we're testing against the expected pre-upgrade version.
    let old_version: String = venear.view("get_version").await?.json()?;
    assert_eq!(
        old_version, "1.1.0",
        "Mainnet venear should be on v1.1.0 — the legacy-delegation-formula baseline",
    );

    // ============================================================
    // Phase 1: pre-upgrade baseline — the whole merkle tree, and every account's voting power
    // under the formula the deployed v1.1.0 uses.
    // ============================================================
    let old_accounts = fetch_accounts(&venear).await?;
    let old_powers: Vec<NearToken> = old_accounts.iter().map(legacy_voting_power).collect();

    let num_delegators = old_accounts
        .iter()
        .filter(|account| !account.delegations.is_empty())
        .count();
    let stuck = old_accounts
        .iter()
        .find(|account| account.account_id.as_str() == MISALIGNED_DELEGATOR)
        .cloned()
        .unwrap_or_else(|| {
            panic!("{MISALIGNED_DELEGATOR} should be registered in the imported mainnet state")
        });
    println!(
        "Mainnet venear state: {} accounts, {num_delegators} of them delegating; the pinned \
         regression case {MISALIGNED_DELEGATOR} delegates {:?}",
        old_accounts.len(),
        stuck.delegations,
    );

    // Verify yuensid.near can't change delegators.
    let stuck_account =
        patch_account(&sandbox, MISALIGNED_DELEGATOR, NearToken::from_near(2)).await?;
    let cleared = json!([]);
    let failure = match set_delegations(&stuck_account, &venear, &cleared).await? {
        Err(failure) => failure,
        Ok(()) => {
            panic!("set_delegations([]) by {MISALIGNED_DELEGATOR} succeeded before the upgrade")
        }
    };
    println!("Pre-upgrade: {MISALIGNED_DELEGATOR} cannot clear its delegations: {failure}");

    // ============================================================
    // Phase 2: upgrade venear.
    // ============================================================
    let venear_wasm = std::fs::read(VENEAR_WASM_FILEPATH)?;
    let outcome = owner
        .call(venear.id(), "upgrade")
        .args(venear_wasm)
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Failed to upgrade venear contract: {:#?}",
        outcome.outcomes()
    );
    println!(
        "venear upgrade burnt {} TGas total, heaviest receipt burnt {} TGas",
        outcome.total_gas_burnt.as_tgas(),
        outcome
            .outcomes()
            .iter()
            .map(|o| o.gas_burnt.as_tgas())
            .max()
            .unwrap()
    );

    let new_version: String = venear.view("get_version").await?.json()?;
    assert_eq!(
        new_version,
        env!("CARGO_PKG_VERSION"),
        "Post-upgrade venear version should match the local workspace version",
    );
    // v1.1.0 already has the current config layout, so migrate_state() is an identity migration.
    let new_config: serde_json::Value = venear.view("get_config").await?.json()?;
    assert_eq!(
        new_config, old_config,
        "Venear config must be preserved verbatim by migrate_state()",
    );

    // ============================================================
    // Phase 3: voting power must survive the migration. The pools were rewritten and the formula
    // that reads them replaced, so every account is re-measured with the post-upgrade
    // `owned_total()` and compared against its v1.1.0 baseline.
    // ============================================================
    let new_accounts = fetch_accounts(&venear).await?;
    assert_eq!(
        new_accounts.len(),
        old_accounts.len(),
        "migrate_state() must not add or drop accounts",
    );

    // How many delegations feed each account's pool. The new formula truncates every incoming
    // delegation's near share to whole milliNEAR and leaves the remainder with its owner, so a
    // pool's rounding error grows with the number of delegators behind it.
    let mut incoming: HashMap<&AccountId, u128> = HashMap::new();
    for account in &old_accounts {
        for delegation in &account.delegations {
            *incoming.entry(&delegation.account_id).or_default() += 1;
        }
    }

    let mut max_drift = (0u128, String::new());
    for ((old_account, old), new_account) in old_accounts.iter().zip(&old_powers).zip(&new_accounts)
    {
        let account_id = &old_account.account_id;
        assert_eq!(
            account_id, &new_account.account_id,
            "migrate_state() must not reorder the merkle tree",
        );
        assert_eq!(
            old_account.delegations, new_account.delegations,
            "migrate_state() must not touch the delegations of {account_id}",
        );

        let old = old.as_yoctonear();
        let new = new_account.owned_total().as_yoctonear();
        // Both sweeps are served by the contract, which grows each account to the timestamp of
        // the block that served it — so the accounts themselves carry the exact window over which
        // veNEAR legitimately accrued between the two readings.
        let elapsed_sec =
            u128::from(new_account.update_timestamp.0 - old_account.update_timestamp.0)
                / 1_000_000_000;
        let growth_bound = old * MAX_ANNUAL_GROWTH_PERCENT * elapsed_sec / (100 * SEC_IN_YEAR);
        let delegations = incoming.get(account_id).copied().unwrap_or_default()
            + u128::try_from(old_account.delegations.len()).unwrap();
        let tolerance = growth_bound + (delegations + 1) * ROUNDING_SLACK.as_yoctonear();
        let drift = old.abs_diff(new);
        assert!(
            drift <= tolerance,
            "Voting power of {account_id} changed by {drift} yoctoNEAR across the migration \
             ({old} -> {new}), more than the {tolerance} yoctoNEAR allowed for {elapsed_sec} s of \
             growth plus rounding over its {delegations} delegations",
        );
        if drift > max_drift.0 {
            max_drift = (drift, account_id.to_string());
        }
    }
    println!(
        "Voting power preserved for all {} accounts; largest drift {} yoctoNEAR, on {}",
        old_accounts.len(),
        max_drift.0,
        max_drift.1,
    );

    // ============================================================
    // Phase 4: the stuck delegator can change its delegations again.
    // ============================================================

    // The very call that underflowed under the legacy pool credits now goes through.
    set_delegations(&stuck_account, &venear, &cleared)
        .await?
        .map_err(|failure| {
            format!(
                "set_delegations([]) by the previously-stuck {MISALIGNED_DELEGATOR} still fails \
                 after the upgrade: {failure}"
            )
        })?;
    let info: serde_json::Value = venear
        .view("get_account_info")
        .args_json(json!({ "account_id": MISALIGNED_DELEGATOR }))
        .await?
        .json()?;
    assert_eq!(
        info["account"]["delegations"], cleared,
        "{MISALIGNED_DELEGATOR} should have no delegations left after clearing them",
    );

    // And it can delegate again: restore the original set.
    let entries = serde_json::to_value(&stuck.delegations)?;
    set_delegations(&stuck_account, &venear, &entries)
        .await?
        .map_err(|failure| {
            format!(
                "restoring the delegations {:?} of the previously-stuck {MISALIGNED_DELEGATOR} \
                 failed: {failure}",
                stuck.delegations
            )
        })?;
    let info: serde_json::Value = venear
        .view("get_account_info")
        .args_json(json!({ "account_id": MISALIGNED_DELEGATOR }))
        .await?
        .json()?;
    assert_eq!(
        info["account"]["delegations"], entries,
        "{MISALIGNED_DELEGATOR} should hold its original delegation set again",
    );
    println!(
        "Post-upgrade: {MISALIGNED_DELEGATOR} cleared and restored its delegations {:?}",
        stuck.delegations
    );

    Ok(())
}
