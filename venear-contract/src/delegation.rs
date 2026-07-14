use crate::*;
use common::events;
use near_sdk::Promise;

/// Validates a slice of DelegationEntry for use in set_delegations.
/// The `Bps` newtype already guarantees each entry's `bps <= 10_000` at deserialization, so this
/// function only enforces the rules that go beyond a single entry's range.
fn validate_delegations(
    entries: &[DelegationEntry],
    owner: &AccountId,
    max_delegations: u32,
) -> Result<u16, &'static str> {
    if u64::try_from(entries.len()).unwrap() > u64::from(max_delegations) {
        return Err("Too many delegations");
    }
    let mut sum_bps: u32 = 0;
    for (i, entry) in entries.iter().enumerate() {
        if entry.bps.is_zero() {
            return Err("Invalid bps (cannot be 0)");
        }
        if &entry.account_id == owner {
            return Err("Cannot delegate to self");
        }
        if i > 0 && entry.account_id <= entries[i - 1].account_id {
            return Err("Entries must be sorted ascending by account_id with no duplicates");
        }
        sum_bps += u32::from(entry.bps);
    }
    if sum_bps > 10_000 {
        return Err("Total bps exceeds 10000");
    }
    Ok(u16::try_from(sum_bps).unwrap())
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
        require!(attached > NearToken::ZERO, "Requires attached deposit");
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

        self.tree.flush();
        let storage_after = env::storage_usage();
        let refund = if storage_after > storage_before {
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
            attached.checked_sub(storage_cost).unwrap()
        } else {
            attached
        };
        if refund > NearToken::ZERO {
            Promise::new(predecessor_id).transfer(refund).detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{entry, fresh_contract, set_ctx};

    /// Register `caller` with 1 NEAR and each delegate with zero, so set_delegations can scale onto them.
    fn registered_contract(caller: &AccountId, delegates: &[&str]) -> Contract {
        let mut contract = fresh_contract(caller.clone());
        contract.internal_register_account(caller, NearToken::from_near(1));
        for delegate in delegates {
            let id: AccountId = delegate.parse().unwrap();
            contract.internal_register_account(&id, NearToken::from_yoctonear(0));
        }
        contract
    }

    fn delegated_total(contract: &Contract, account_id: &str) -> NearToken {
        contract
            .get_account_info(account_id.parse().unwrap())
            .unwrap()
            .account
            .delegated_balance
            .total()
    }

    #[test]
    fn accepts_partial_sum_under_full() {
        let caller: AccountId = "caller.near".parse().unwrap();
        let mut contract = registered_contract(&caller, &["a.near", "b.near"]);
        let entries = vec![entry("a.near", 2_500), entry("b.near", 1_500)];
        set_ctx(caller.clone(), NearToken::from_near(1).as_yoctonear(), 0);
        contract.set_delegations(entries.clone());

        assert_eq!(
            contract
                .get_account_info(caller)
                .unwrap()
                .account
                .delegations,
            entries
        );
        assert_eq!(
            delegated_total(&contract, "a.near"),
            NearToken::from_millinear(250)
        );
        assert_eq!(
            delegated_total(&contract, "b.near"),
            NearToken::from_millinear(150)
        );
    }

    #[test]
    fn accepts_sum_exactly_full() {
        let caller: AccountId = "caller.near".parse().unwrap();
        let mut contract = registered_contract(&caller, &["a.near", "b.near"]);
        let entries = vec![entry("a.near", 4_000), entry("b.near", 6_000)];
        set_ctx(caller.clone(), NearToken::from_near(1).as_yoctonear(), 0);
        contract.set_delegations(entries.clone());

        assert_eq!(
            contract
                .get_account_info(caller)
                .unwrap()
                .account
                .delegations,
            entries
        );
        assert_eq!(
            delegated_total(&contract, "a.near"),
            NearToken::from_millinear(400)
        );
        assert_eq!(
            delegated_total(&contract, "b.near"),
            NearToken::from_millinear(600)
        );
    }

    #[test]
    fn accepts_single_full_entry() {
        let caller: AccountId = "caller.near".parse().unwrap();
        let mut contract = registered_contract(&caller, &["a.near"]);
        let entries = vec![entry("a.near", 10_000)];
        set_ctx(caller.clone(), NearToken::from_near(1).as_yoctonear(), 0);
        contract.set_delegations(entries.clone());

        assert_eq!(
            contract
                .get_account_info(caller)
                .unwrap()
                .account
                .delegations,
            entries
        );
        assert_eq!(
            delegated_total(&contract, "a.near"),
            NearToken::from_near(1)
        );
    }

    #[test]
    #[should_panic(expected = "Invalid bps (cannot be 0)")]
    fn set_delegations_rejects_zero_bps() {
        let caller: AccountId = "caller.near".parse().unwrap();
        let mut contract = fresh_contract(caller.clone());
        set_ctx(caller, 1, 0);
        contract.set_delegations(vec![entry("a.near", 2_500), entry("b.near", 0)]);
    }

    #[test]
    #[should_panic(expected = "Total bps exceeds 10000")]
    fn set_delegations_rejects_sum_over_full() {
        let caller: AccountId = "caller.near".parse().unwrap();
        let mut contract = fresh_contract(caller.clone());
        set_ctx(caller, 1, 0);
        contract.set_delegations(vec![entry("a.near", 6_000), entry("b.near", 4_001)]);
    }
}
