use crate::*;
use near_sdk::{NearToken, Promise, assert_one_yocto, env, near, require};

#[near]
impl Contract {
    /// NEP-145-style: attach NEAR to register an account for locks and withdrawals.
    /// **Storage:** [`crate::config::Config::min_storage_deposit`] +
    /// [`crate::config::Config::per_lock_storage_stake`] × [`crate::Contract::user_lock_count`] (locks ever
    /// created; not decremented on unlock).
    #[payable]
    pub fn storage_deposit(&mut self) {
        self.assert_not_paused();
        let attached = env::attached_deposit();
        require!(attached.as_yoctonear() > 0, "Attach NEAR for storage");
        let depositor = env::predecessor_account_id();
        let mut account = self.internal_get_account(&depositor).unwrap_or_default();
        account.storage_deposit = account
            .storage_deposit
            .checked_add(attached)
            .expect("Storage deposit overflow; reduce the attached amount");
        self.internal_set_account(depositor, account);
    }

    /// Withdraw prepaid storage above [`crate::config::Config::min_storage_deposit`].
    #[payable]
    pub fn storage_withdraw(&mut self, amount: NearToken) -> Promise {
        assert_one_yocto();
        self.assert_not_paused();
        require!(
            amount.as_yoctonear() > 0,
            "Withdraw amount must be greater than zero"
        );

        let account_id = env::predecessor_account_id();
        let mut account = self
            .internal_get_account(&account_id)
            .expect("Account not registered; call storage_deposit first");

        let storage_yocto = account.storage_deposit.as_yoctonear();
        // Never withdraw more than prepaid: avoids transferring more than recorded storage when
        // `min_storage_deposit` is zero or small (do not rely on saturating math alone).
        require!(
            amount.as_yoctonear() <= storage_yocto,
            "Withdraw exceeds prepaid storage"
        );

        let required_yocto = self.required_storage_deposit_yocto(&account_id, 0);
        let after = storage_yocto
            .checked_sub(amount.as_yoctonear())
            .expect("Internal error: storage withdraw amount was not bounded correctly");
        require!(
            after >= required_yocto,
            "Must retain required storage (min + per-lock stake)"
        );

        account.storage_deposit = NearToken::from_yoctonear(after);
        self.internal_set_account(account_id.clone(), account);

        Promise::new(account_id).transfer(amount)
    }

    pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
        self.internal_get_account(&account_id)
    }
}

impl Contract {
    pub(crate) fn internal_get_account(&self, id: &AccountId) -> Option<Account> {
        self.accounts.get(id).cloned().map(Into::into)
    }

    pub(crate) fn internal_set_account(&mut self, id: AccountId, account: Account) {
        self.accounts.insert(id, account.into());
    }

    fn require_registered_account(&self, account_id: &AccountId) -> Account {
        self.internal_get_account(account_id)
            .expect("Account not registered; call storage_deposit")
    }

    fn assert_storage_deposit_at_least(
        &self,
        account: &Account,
        required_yocto: u128,
        err_msg: &str,
    ) {
        require!(
            account.storage_deposit.as_yoctonear() >= required_yocto,
            err_msg
        );
    }

    /// `extra_locks` = additional locks we are about to add (0 when not creating a lock).
    pub(crate) fn required_storage_deposit_yocto(
        &self,
        account_id: &AccountId,
        extra_locks: u32,
    ) -> u128 {
        let base = self
            .internal_get_config()
            .min_storage_deposit
            .as_yoctonear();
        let per = self
            .internal_get_config()
            .per_lock_storage_stake
            .as_yoctonear();
        let recorded_lock_count =
            self.user_lock_count.get(account_id).copied().unwrap_or(0) as u128;
        let total_locks = recorded_lock_count.saturating_add(u128::from(extra_locks));
        base.saturating_add(per.saturating_mul(total_locks))
    }

    /// Account registered and still meets global [`crate::config::Config::min_storage_deposit`] only (no per-lock surcharge).
    /// Use for claim / withdraw paths so older locks do not force endless storage top-ups.
    pub(crate) fn ensure_min_base_storage(&self, account_id: &AccountId) {
        let account = self.require_registered_account(account_id);
        let min = self
            .internal_get_config()
            .min_storage_deposit
            .as_yoctonear();
        self.assert_storage_deposit_at_least(&account, min, "Top up storage (minimum prepaid)");
    }

    /// Before creating a lock: require prepaid storage for one more lock entry.
    pub(crate) fn ensure_min_storage_for_new_lock(&self, account_id: &AccountId) {
        let account = self.require_registered_account(account_id);
        let need = self.required_storage_deposit_yocto(account_id, 1);
        self.assert_storage_deposit_at_least(&account, need, "Top up storage for another lock");
    }
}
