use crate::venear::VenearGrowthConfig;
use crate::*;
use near_sdk::require;

/// The original account details stored in the Merkle Tree.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct AccountV0 {
    /// The account ID of the account. Required for the security of the Merkle Tree proofs.
    pub account_id: AccountId,
    /// The timestamp in nanoseconds when the account was last updated.
    pub update_timestamp: TimestampNs,
    /// The total NEAR balance of the account as reported by the lockup contract and additional
    /// veNEAR accumulated over time.
    pub balance: VenearBalance,
    /// The total amount of NEAR and veNEAR that was delegated to this account.
    pub delegated_balance: PooledVenearBalance,
    /// The delegation details, in case this account has delegated balance to another account.
    pub delegation: Option<AccountDelegation>,
}

/// The runtime account shape and the V1 on-tree format.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct AccountV1 {
    /// The account ID of the account. Required for the security of the Merkle Tree proofs.
    pub account_id: AccountId,
    /// The timestamp in nanoseconds when the account was last updated.
    pub update_timestamp: TimestampNs,
    /// The total NEAR balance of the account as reported by the lockup contract and additional
    /// veNEAR accumulated over time.
    pub balance: VenearBalance,
    /// The total amount of NEAR and veNEAR that was delegated to this account.
    pub delegated_balance: PooledVenearBalance,
    /// The partial delegation entries. The undelegated remainder implicitly stays with self.
    pub delegations: Vec<DelegationEntry>,
}

pub type Account = AccountV1;

/// The details of the delegation of veNEAR from one account to another.
/// In the first version we assume that the whole balance was delegated.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct AccountDelegation {
    /// The account ID of the account that the veNEAR was delegated to.
    pub account_id: AccountId,
}

/// A single partial delegation entry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[near(serializers=[borsh, json])]
pub struct DelegationEntry {
    pub account_id: AccountId,
    pub bps: u16,
}

#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub enum VAccount {
    V0(AccountV0),
    V1(AccountV1),
}

impl From<Account> for VAccount {
    fn from(account: Account) -> Self {
        VAccount::V1(account)
    }
}

impl From<VAccount> for Account {
    fn from(value: VAccount) -> Self {
        match value {
            VAccount::V0(account) => Account {
                account_id: account.account_id,
                update_timestamp: account.update_timestamp,
                balance: account.balance,
                delegated_balance: account.delegated_balance,
                delegations: account
                    .delegation
                    .into_iter()
                    .map(|delegation| DelegationEntry {
                        account_id: delegation.account_id,
                        bps: 10_000,
                    })
                    .collect(),
            },
            VAccount::V1(account) => account,
        }
    }
}

impl Account {
    pub fn delegated_bps(&self) -> u16 {
        self.delegations
            .iter()
            .map(|delegation| u32::from(delegation.bps))
            .sum::<u32>()
            .try_into()
            .expect("delegation bps sum must fit into u16")
    }

    /// Returns veNEAR balance of the account without modifications.
    pub fn total_balance(
        &self,
        current_timestamp: TimestampNs,
        venear_growth_config: &VenearGrowthConfig,
    ) -> NearToken {
        let current_timestamp = truncate_to_seconds(current_timestamp);
        require!(
            current_timestamp >= self.update_timestamp,
            "Timestamp must be increasing"
        );
        let mut delegated_balance = self.delegated_balance;
        delegated_balance.update(
            self.update_timestamp,
            current_timestamp,
            venear_growth_config,
        );
        let total = delegated_balance.total();
        let self_bps = 10_000_u16.saturating_sub(self.delegated_bps());
        if self_bps > 0 {
            let mut balance = self.balance;
            balance.update(
                self.update_timestamp,
                current_timestamp,
                venear_growth_config,
            );
            near_add(total, balance.scale_by_bps(self_bps).total())
        } else {
            total
        }
    }

    pub fn update(
        &mut self,
        current_timestamp: TimestampNs,
        venear_growth_config: &VenearGrowthConfig,
    ) {
        let current_timestamp = truncate_to_seconds(current_timestamp);
        require!(
            current_timestamp >= self.update_timestamp,
            "Timestamp must be increasing"
        );
        self.balance.update(
            self.update_timestamp,
            current_timestamp,
            venear_growth_config,
        );
        self.delegated_balance.update(
            self.update_timestamp,
            current_timestamp,
            venear_growth_config,
        );
        self.update_timestamp = current_timestamp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_id(value: &str) -> AccountId {
        value.parse().unwrap()
    }

    fn sample_balance(near: u128, extra: u128) -> VenearBalance {
        VenearBalance {
            near_balance: NearToken::from_yoctonear(near),
            extra_venear_balance: NearToken::from_yoctonear(extra),
        }
    }

    fn sample_account(delegations: Vec<DelegationEntry>) -> Account {
        Account {
            account_id: account_id("owner.near"),
            update_timestamp: 123.into(),
            balance: sample_balance(5, 7),
            delegated_balance: Default::default(),
            delegations,
        }
    }

    #[test]
    fn from_vaccount_v0_without_delegation_creates_empty_delegations() {
        let account: Account = VAccount::V0(AccountV0 {
            account_id: account_id("owner.near"),
            update_timestamp: 1.into(),
            balance: sample_balance(10, 20),
            delegated_balance: Default::default(),
            delegation: None,
        })
        .into();

        assert!(account.delegations.is_empty());
    }

    #[test]
    fn from_vaccount_v0_with_delegation_creates_full_entry() {
        let account: Account = VAccount::V0(AccountV0 {
            account_id: account_id("owner.near"),
            update_timestamp: 1.into(),
            balance: sample_balance(10, 20),
            delegated_balance: Default::default(),
            delegation: Some(AccountDelegation {
                account_id: account_id("delegate.near"),
            }),
        })
        .into();

        assert_eq!(account.delegations.len(), 1);
        assert_eq!(
            account.delegations[0].account_id,
            account_id("delegate.near")
        );
        assert_eq!(account.delegations[0].bps, 10_000);
    }

    #[test]
    fn from_vaccount_v1_preserves_delegations() {
        let original = sample_account(vec![
            DelegationEntry {
                account_id: account_id("a.near"),
                bps: 2_500,
            },
            DelegationEntry {
                account_id: account_id("b.near"),
                bps: 7_500,
            },
        ]);

        let account: Account = VAccount::V1(original.clone()).into();

        assert_eq!(account.delegations.len(), 2);
        assert_eq!(account.delegations[0].bps, 2_500);
        assert_eq!(account.delegations[1].bps, 7_500);
    }

    #[test]
    fn delegated_bps_empty_is_zero() {
        assert_eq!(sample_account(vec![]).delegated_bps(), 0);
    }

    #[test]
    fn delegated_bps_sums_partial_entries() {
        let account = sample_account(vec![
            DelegationEntry {
                account_id: account_id("a.near"),
                bps: 1_234,
            },
            DelegationEntry {
                account_id: account_id("b.near"),
                bps: 2_000,
            },
            DelegationEntry {
                account_id: account_id("c.near"),
                bps: 766,
            },
        ]);
        assert_eq!(account.delegated_bps(), 4_000);
    }

    #[test]
    fn delegated_bps_full_allocation() {
        let account = sample_account(vec![
            DelegationEntry {
                account_id: account_id("a.near"),
                bps: 6_000,
            },
            DelegationEntry {
                account_id: account_id("b.near"),
                bps: 4_000,
            },
        ]);
        assert_eq!(account.delegated_bps(), 10_000);
    }

    #[test]
    #[should_panic(expected = "delegation bps sum must fit into u16")]
    fn delegated_bps_overflow_panics() {
        let account = sample_account(vec![
            DelegationEntry {
                account_id: account_id("a.near"),
                bps: u16::MAX,
            },
            DelegationEntry {
                account_id: account_id("b.near"),
                bps: 1,
            },
        ]);
        let _ = account.delegated_bps();
    }
}
