use crate::*;
use near_sdk::{NearToken, Promise, env, near, require};

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
    /// **Storage:** [`crate::config::Config::min_storage_deposit`] +
    /// [`crate::config::Config::per_lock_storage_stake`] × [`crate::Contract::user_lock_count`] (locks ever
    /// created; not decremented on unlock).
    #[payable]
    pub fn storage_deposit(&mut self) {
        self.assert_not_paused();
        let dep = env::attached_deposit();
        require!(dep.as_yoctonear() > 0, "Attach NEAR for storage");
        let pred = env::predecessor_account_id();
        let mut acc = self.accounts.get(&pred).cloned().unwrap_or_default();
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

        let storage_yocto = acc.storage_deposit.as_yoctonear();
        require!(
            amount.as_yoctonear() <= storage_yocto,
            "Withdraw exceeds prepaid storage"
        );

        let required = self.required_storage_deposit_yocto(&pred, 0);
        let after = storage_yocto
            .checked_sub(amount.as_yoctonear())
            .expect("amount bound above");
        require!(
            after >= required,
            "Must retain required storage (min + per-lock stake)"
        );

        acc.storage_deposit = NearToken::from_yoctonear(after);
        self.accounts.insert(pred.clone(), acc);

        Promise::new(pred).transfer(amount)
    }

    pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
        self.accounts.get(&account_id).cloned()
    }
}

impl Contract {
    /// `extra_locks` = additional locks we are about to add (0 when not creating a lock).
    pub(crate) fn required_storage_deposit_yocto(
        &self,
        account_id: &AccountId,
        extra_locks: u32,
    ) -> u128 {
        let base = self.config.min_storage_deposit.as_yoctonear();
        let per = self.config.per_lock_storage_stake.as_yoctonear();
        let cnt = self.user_lock_count.get(account_id).copied().unwrap_or(0) as u128;
        let total_locks = cnt.saturating_add(u128::from(extra_locks));
        base.saturating_add(per.saturating_mul(total_locks))
    }

    /// Account registered and still meets global [`crate::config::Config::min_storage_deposit`] only (no per-lock surcharge).
    /// Use for claim / withdraw paths so older locks do not force endless storage top-ups.
    pub(crate) fn ensure_min_base_storage(&self, account_id: &AccountId) {
        let a = self
            .accounts
            .get(account_id)
            .expect("Account not registered; call storage_deposit");
        let min = self.config.min_storage_deposit.as_yoctonear();
        require!(
            a.storage_deposit.as_yoctonear() >= min,
            "Top up storage (minimum prepaid)"
        );
    }

    /// Full prepaid requirement including `per_lock_storage_stake` × locks recorded in [`crate::Contract::user_lock_count`].
    pub fn ensure_min_storage(&self, account_id: &AccountId) {
        let a = self
            .accounts
            .get(account_id)
            .expect("Account not registered; call storage_deposit");
        let need = self.required_storage_deposit_yocto(account_id, 0);
        require!(a.storage_deposit.as_yoctonear() >= need, "Top up storage");
    }

    /// Before creating a lock: require prepaid storage for one more lock entry.
    pub(crate) fn ensure_min_storage_for_new_lock(&self, account_id: &AccountId) {
        let a = self
            .accounts
            .get(account_id)
            .expect("Account not registered; call storage_deposit");
        let need = self.required_storage_deposit_yocto(account_id, 1);
        require!(
            a.storage_deposit.as_yoctonear() >= need,
            "Top up storage for another lock"
        );
    }
}
