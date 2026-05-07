use crate::*;
use common::events;
use near_sdk::Promise;

/// Validates a slice of DelegationEntry for use in set_delegations..
fn validate_delegations(
    entries: &[DelegationEntry],
    owner: &AccountId,
    max_delegations: u32,
) -> Result<u16, &'static str> {
    if entries.len() as u64 > max_delegations as u64 {
        return Err("Too many delegations");
    }
    let mut sum_bps: u32 = 0;
    for (i, entry) in entries.iter().enumerate() {
        if entry.bps == 0 || entry.bps > 10_000 {
            return Err("Invalid bps (must be 1..=10000)");
        }
        if &entry.account_id == owner {
            return Err("Cannot delegate to self");
        }
        if i > 0 && entry.account_id <= entries[i - 1].account_id {
            return Err("Entries must be sorted ascending by account_id with no duplicates");
        }
        sum_bps += entry.bps as u32;
    }
    if sum_bps > 10_000 {
        return Err("Total bps exceeds 10000");
    }
    Ok(sum_bps as u16)
}

#[near]
impl Contract {
    /// Set partial delegations for the caller's account.
    /// Atomically replaces the caller's entire delegation set.
    /// The `entries` Vec must be:
    ///   - Sorted ascending by account_id (no duplicates)
    ///   - Each entry bps in [1, 10_000]
    ///   - Sum of all bps ≤ 10,000
    ///   - No self-delegation
    ///   - At most `config.max_delegations` entries
    ///   - All delegate account_ids must be registered in veNEAR
    /// Requires attached deposit ≥ storage growth cost. Refunds overpay.
    #[payable]
    pub fn set_delegations(&mut self, entries: Vec<DelegationEntry>) {
        self.assert_not_paused();
        let attached = env::attached_deposit();
        require!(attached.as_yoctonear() > 0, "Requires attached deposit");
        let predecessor_id = env::predecessor_account_id();

        validate_delegations(&entries, &predecessor_id, self.config.max_delegations)
            .unwrap_or_else(|err| env::panic_str(err));

        let storage_before = env::storage_usage();

        let mut owner = self.internal_expect_account_updated(&predecessor_id);

        let old_delegations = std::mem::take(&mut owner.delegations);
        for delegation in &old_delegations {
            let mut delegate = self.internal_expect_account_updated(&delegation.account_id);
            delegate.delegated_balance = delegate
                .delegated_balance
                .pooled_sub_scaled(&owner.balance, delegation.bps);
            self.internal_set_account(delegation.account_id.clone(), delegate);
        }

        for entry in &entries {
            let mut delegate = self.internal_expect_account_updated(&entry.account_id);
            delegate.delegated_balance = delegate
                .delegated_balance
                .pooled_add_scaled(&owner.balance, entry.bps);
            self.internal_set_account(entry.account_id.clone(), delegate);
        }

        events::emit::delegation_change(&predecessor_id, &old_delegations, &entries);
        owner.delegations = entries;
        self.internal_set_account(predecessor_id.clone(), owner);

        let storage_after = env::storage_usage();
        if storage_after > storage_before {
            let storage_cost = env::storage_byte_cost()
                .checked_mul((storage_after - storage_before).into())
                .expect("Storage cost overflow");
            require!(
                attached >= storage_cost,
                format!(
                    "Insufficient deposit for storage. Required: {}",
                    storage_cost
                )
            );
            let refund = attached.checked_sub(storage_cost).unwrap();
            Promise::new(predecessor_id).transfer(refund);
        }
    }
}
