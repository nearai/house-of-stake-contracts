use crate::*;
use near_sdk::{env, near, require, NearToken};

#[near(serializers = [borsh, json])]
pub struct Account {
    pub storage_deposit: NearToken,
    pub withdrawable_balance: NearToken,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            storage_deposit: NearToken::from_near(0),
            withdrawable_balance: NearToken::from_near(0),
        }
    }
}

#[near]
impl Contract {
    /// NEP-145-style: attach NEAR to register an account for locks and withdrawals.
    #[payable]
    pub fn storage_deposit(&mut self) {
        self.assert_not_paused();
        let dep = env::attached_deposit();
        require!(dep.as_yoctonear() > 0, "Attach NEAR for storage");
        let pred = env::predecessor_account_id();
        let mut acc = self.accounts.get(&pred).unwrap_or_default();
        acc.storage_deposit = acc
            .storage_deposit
            .checked_add(dep)
            .expect("storage_deposit overflow");
        self.accounts.insert(pred, acc);
    }
}

impl Contract {
    pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
        self.accounts.get(&account_id)
    }

    pub fn ensure_min_storage(&self, account_id: &AccountId) {
        let a = self.accounts.get(account_id).expect("Account not registered; call storage_deposit");
        require!(
            a.storage_deposit >= self.config.min_storage_deposit,
            "Top up storage"
        );
    }
}
