use crate::*;
use near_sdk::{env, near, require, NearToken, Promise};

#[derive(Clone)]
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
        let mut acc = self
            .accounts
            .get(&pred)
            .cloned()
            .unwrap_or_default();
        acc.storage_deposit = acc
            .storage_deposit
            .checked_add(dep)
            .expect("storage_deposit overflow");
        self.accounts.insert(pred, acc);
    }

    /// Withdraw prepaid storage above [`crate::config::Config::min_storage_deposit`].
    #[payable]
    pub fn storage_withdraw(&mut self, amount: NearToken) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        require!(amount.as_yoctonear() > 0, "amount");

        let pred = env::predecessor_account_id();
        let mut acc = self
            .accounts
            .get(&pred)
            .cloned()
            .expect("No account; call storage_deposit");

        let min = self.config.min_storage_deposit.as_yoctonear();
        let after = acc
            .storage_deposit
            .as_yoctonear()
            .saturating_sub(amount.as_yoctonear());
        require!(after >= min, "Must retain min_storage_deposit");

        acc.storage_deposit = NearToken::from_yoctonear(after);
        self.accounts.insert(pred.clone(), acc);

        Promise::new(pred).transfer(amount)
    }
}

impl Contract {
    pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
        self.accounts.get(&account_id).cloned()
    }

    pub fn ensure_min_storage(&self, account_id: &AccountId) {
        let a = self.accounts.get(account_id).expect("Account not registered; call storage_deposit");
        require!(
            a.storage_deposit >= self.config.min_storage_deposit,
            "Top up storage"
        );
    }
}
