#![allow(dead_code)]

use super::VenearTestWorkspace;
use near_sdk::{Gas, NearToken};
use near_workspaces::{Account, AccountId};
use serde_json::json;

pub async fn set_delegations_sorted(
    v: &VenearTestWorkspace,
    delegator: &Account,
    mut entries: Vec<(AccountId, u16)>,
) -> Result<(), Box<dyn std::error::Error>> {
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    let entries_json: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, bps)| json!({ "account_id": id, "bps": bps }))
        .collect();
    delegator
        .call(v.venear.id(), "set_delegations")
        .args_json(json!({ "entries": entries_json }))
        .deposit(NearToken::from_millinear(10))
        .gas(Gas::from_tgas(200))
        .transact()
        .await?
        .into_result()?;
    Ok(())
}
