use crate::internal::{mint_shares, near_from_shares};
use crate::*;
use near_sdk::json_types::{U64, U128};
use near_sdk::{
    AccountId, NearToken, Promise, assert_one_yocto, env, is_promise_success, near, require,
};

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Validator {
    /// Staking pool contract account (= catalog `validator_id` / lock `validator_id`).
    pub validator_id: ValidatorId,
    /// Whether new locks are allowed (**`Active`**) or blocked (**`Paused`**), or this pool is **`Removed`**.
    pub status: ValidatorStatus,

    /// Total issued stake.dao **share units** for this pool (integer; same scale as per-user shares).
    pub total_shares: U128,
    /// Cached **staked NEAR** for this contract’s account on the pool, from the last successful
    /// pool `get_account` total balance (plus bookkeeping updates on stake/unstake/withdraw paths). Used with
    /// `pending_*` for share mint/burn pricing—not updated until the next pool read or accounting step.
    pub total_staked_balance: NearToken,
    /// `block_timestamp` (nanoseconds) when `total_staked_balance` was last synced from the pool (or validator was added).
    pub last_balance_refresh_ns: U64,

    /// NEAR waiting to be sent to the pool via **`deposit_and_stake`** (aggregated locks; net-settled vs
    /// `pending_to_unstake` in `Contract::try_epoch_stake_or_unstake` in `epoch.rs`).
    pub pending_to_stake: NearToken,
    /// NEAR queued to leave the pool via **`unstake`** (user unlocks etc.; net-settled vs `pending_to_stake`).
    pub pending_to_unstake: NearToken,
    /// Epoch height recorded after the last successful pool `unstake` callback; gates further unstakes
    /// (with [`crate::config::Config::epoch_unstake_settle_epochs`]).
    pub last_unstake_epoch: u64,
    /// Last NEAR `epoch_height` for which this validator completed the **pre–user-action** pipeline for a request:
    /// **sync** `total_staked_balance` from the pool (at most once per epoch for catalog flows), **withdraw**
    /// from the pool when eligible, then at most one **net** pool `deposit_and_stake` / `unstake` / net-zero
    /// clearance for that epoch (same mutex the staking pool enforces per account). Successful callbacks
    /// on stake, unstake, and net-zero settlement set this to `env::epoch_height()`. When this equals the
    /// current epoch, user flows **skip** another pool `get_account` refresh for this validator until the
    /// next NEAR epoch.
    pub last_settlement_epoch: u64,
    /// NEAR that has been unstaked on the pool side and is expected to be moved by pool `withdraw`
    /// into this contract.
    pub pending_to_withdraw: NearToken,
    /// NEAR already moved from pool into this contract and now claimable by users.
    pub pending_to_claim: NearToken,
    /// Accounts that currently have at least one non-empty tranche in **`user_pending_unstake`** for this pool.
    pub accounts_with_pending_unstake: Vec<AccountId>,

    /// At most one in-flight cross-contract **mutating** pool pipeline for this validator (`Idle` vs `Busy`).
    pub tx_status: TransactionStatus,
}

impl Validator {
    /// Total user-exit liability across all buckets.
    pub fn pending_user_liability_yocto(&self) -> u128 {
        self.pending_to_unstake
            .as_yoctonear()
            .saturating_add(self.pending_to_withdraw.as_yoctonear())
            .saturating_add(self.pending_to_claim.as_yoctonear())
    }

    /// Liability still outside this contract (pool stake + pool unstaked).
    pub fn pending_not_in_contract_yocto(&self) -> u128 {
        self.pending_to_unstake
            .as_yoctonear()
            .saturating_add(self.pending_to_withdraw.as_yoctonear())
    }

    pub fn gross_stake_yocto(&self) -> u128 {
        self.total_staked_balance
            .as_yoctonear()
            .saturating_add(self.pending_to_stake.as_yoctonear())
    }

    /// NEAR backing **remaining** circulating shares: gross effective stake minus user liability that
    /// is still outside this contract (`pending_to_unstake + pending_to_withdraw`).
    ///
    /// **Solvency:** share pricing must not use gross backing alone after shares burn down.
    /// `pending_to_claim` is already in-contract cash and should not reduce pool-side backing again.
    pub fn net_stake_yocto(&self) -> u128 {
        self.gross_stake_yocto()
            .saturating_sub(self.pending_not_in_contract_yocto())
    }
}

#[near]
impl Contract {
    /// Contract owner: add a validator pool to the allowlist. Pool ownership for catalog operations is
    /// always verified via `get_owner_id()` on the pool ([`crate::products`]).
    #[payable]
    pub fn add_validator(&mut self, validator_id: ValidatorId) {
        assert_one_yocto();
        self.assert_owner();
        require!(
            self.validators.get(&validator_id).is_none(),
            "Validator already exists"
        );

        let new_validator = Validator {
            validator_id: validator_id.clone(),
            status: ValidatorStatus::Active,
            total_shares: U128(0),
            total_staked_balance: NearToken::from_near(0),
            last_balance_refresh_ns: U64(env::block_timestamp()),
            pending_to_stake: NearToken::from_near(0),
            pending_to_unstake: NearToken::from_near(0),
            last_unstake_epoch: 0,
            last_settlement_epoch: 0,
            pending_to_withdraw: NearToken::from_near(0),
            pending_to_claim: NearToken::from_near(0),
            accounts_with_pending_unstake: Vec::new(),
            tx_status: TransactionStatus::Idle,
        };
        self.validators.insert(validator_id.clone(), new_validator);
        self.validator_ids.push(validator_id.clone());
        crate::events::log_validator_added(&validator_id);
    }

    #[payable]
    pub fn pause_validator(&mut self, validator_id: ValidatorId) {
        assert_one_yocto();
        self.assert_owner();
        let mut validator = self.require_validator(&validator_id);
        validator.status = ValidatorStatus::Paused;
        self.validators.insert(validator_id, validator);
    }

    #[payable]
    pub fn remove_validator(&mut self, validator_id: ValidatorId) {
        assert_one_yocto();
        self.assert_owner();
        let mut validator = self.require_validator(&validator_id);
        require!(
            validator.total_shares.0 == 0
                && validator.pending_to_stake.as_yoctonear() == 0
                && validator.pending_to_unstake.as_yoctonear() == 0
                && validator.pending_to_withdraw.as_yoctonear() == 0
                && validator.pending_to_claim.as_yoctonear() == 0,
            "Cannot remove this validator: all stake, pending stake/unstake, and withdraw bucket must be cleared first"
        );
        validator.status = ValidatorStatus::Removed;
        self.validators.insert(validator_id, validator);
    }

    // -------------------------------------------------------------------------
    // Public validator view functions
    // -------------------------------------------------------------------------

    pub fn get_validator(&self, validator_id: ValidatorId) -> Option<Validator> {
        self.validators.get(&validator_id).cloned()
    }

    /// Paginated validator records (stable allowlist order in [`Contract::validator_ids`]).
    pub fn get_validators(&self, from_index: u64, limit: u64) -> Vec<Validator> {
        let total_len = self.validator_ids.len() as u64;
        self.collect_paginated(from_index, limit, total_len, |index| {
            self.validator_ids
                .get(index)
                .and_then(|id| self.validators.get(id).cloned())
        })
    }
}

impl Contract {
    /// Preamble for pool-owner catalog RPCs: 1 yocto, not paused, validator allowlisted.
    pub(crate) fn catalog_admin_entry_for_pool(
        &self,
        validator_id: &ValidatorId,
    ) -> (ValidatorId, AccountId) {
        assert_one_yocto();
        self.assert_not_paused();
        self.assert_validator_allowlisted(validator_id);
        (validator_id.clone(), env::predecessor_account_id())
    }

    /// Pool `get_owner_id` promise chained to a catalog owner-check callback.
    pub(crate) fn promise_pool_get_owner_id_then(
        validator_id: ValidatorId,
        tail: Promise,
    ) -> Promise {
        crate::epoch::ext_staking_pool::ext(validator_id)
            .with_static_gas(crate::gas::staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(tail)
    }

    /// Pool must be on the allowlist. Catalog methods confirm the caller against the pool's
    /// `get_owner_id()` via a cross-contract call (see `products.rs` and `prices.rs`).
    pub(crate) fn assert_validator_allowlisted(&self, validator_id: &ValidatorId) {
        require!(
            self.validators.get(validator_id).is_some(),
            "Validator not found on the allowlist"
        );
    }

    pub(crate) fn assert_validator_active_for_lock(&self, validator_id: &ValidatorId) {
        let validator = self.require_validator(validator_id);
        require!(
            validator.status == ValidatorStatus::Active,
            "This validator is paused or removed; new locks are not allowed on it"
        );
    }

    pub(crate) fn require_validator(&self, validator_id: &ValidatorId) -> Validator {
        self.validators
            .get(validator_id)
            .cloned()
            .expect("Validator not found on the allowlist")
    }

    /// User entry (`lock` / `unlock` / `withdraw` / pipeline **0**) must not start while a pool pipeline is in flight.
    pub(crate) fn assert_validator_idle_for_user_action(&self, validator: &Validator) {
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
    }

    pub(crate) fn require_validator_idle(&self, validator_id: &ValidatorId) -> Validator {
        let validator = self.require_validator(validator_id);
        self.assert_validator_idle_for_user_action(&validator);
        validator
    }

    pub(crate) fn assert_validator_busy(&self, validator: &Validator, err_msg: &str) {
        require!(validator.tx_status == TransactionStatus::Busy, err_msg);
    }

    pub(crate) fn require_validator_busy(
        &self,
        validator_id: &ValidatorId,
        err_msg: &str,
    ) -> Validator {
        let validator = self.require_validator(validator_id);
        self.assert_validator_busy(&validator, err_msg);
        validator
    }

    /// True once a pool `unstake` is on record and [`crate::config::Config::epoch_unstake_settle_epochs`]
    /// have passed since [`Validator::last_unstake_epoch`] (gates withdraw-from-pool and further unstakes).
    pub(crate) fn validator_unstake_waiting_finished(&self, validator: &Validator) -> bool {
        validator.last_unstake_epoch > 0
            && env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs)
    }

    /// NEAR `epoch_height` from which a new [`PendingUnstakeTranche`] may participate in
    /// [`crate::Contract::withdraw`] (when `env::epoch_height() >=` this value).
    ///
    /// 1. `unstake_start_epoch = max(current_epoch_height, last_unstake_epoch + epoch_unstake_settle_epochs)`
    /// 2. `available_epoch_height = unstake_start_epoch + epoch_unstake_settle_epochs`
    ///
    /// Uses [`crate::config::Config::epoch_unstake_settle_epochs`].
    pub(crate) fn pending_unstake_tranche_available_epoch_height(
        &self,
        validator: &Validator,
    ) -> u64 {
        let current_epoch_height = env::epoch_height();
        let settle = self.config.epoch_unstake_settle_epochs;
        let unstake_start_epoch =
            current_epoch_height.max(validator.last_unstake_epoch.saturating_add(settle));
        unstake_start_epoch.saturating_add(settle)
    }

    /// Mints pool share units for `deposit`, bumps [`Validator::pending_to_stake`], and credits the buyer's
    /// `(account, validator)` share balance. Used by catalog lock mint and subscription upgrade.
    pub(crate) fn internal_stake(
        &mut self,
        buyer: &AccountId,
        validator_id: &ValidatorId,
        deposit: NearToken,
    ) -> u128 {
        let mut validator = self.require_validator(validator_id);
        let net_stake = validator.net_stake_yocto();
        let validator_total_shares = validator.total_shares.0;
        if validator_total_shares > 0 {
            require!(
                net_stake > 0,
                "No effective stake for share minting; wait for balance refresh or settlement"
            );
        }
        let new_shares = mint_shares(validator_total_shares, net_stake, deposit.as_yoctonear());
        validator.total_shares = U128(validator_total_shares.saturating_add(new_shares));
        validator.pending_to_stake = validator
            .pending_to_stake
            .checked_add(deposit)
            .expect("pending_to_stake overflow when recording this lock");
        let user_validator_shares_key = (buyer.clone(), validator_id.clone());
        let user_shares_before = self
            .user_validator_shares
            .get(&user_validator_shares_key)
            .copied()
            .unwrap_or(0);
        self.user_validator_shares.insert(
            user_validator_shares_key,
            user_shares_before.saturating_add(new_shares),
        );
        self.validators.insert(validator_id.clone(), validator);
        new_shares
    }

    /// Commits an **internal unstake** for `account_id` on `validator_id`: burns `shares_remove` pool share
    /// units, prices them into NEAR using the same effective backing as mints, updates validator pending
    /// unstake buckets, and appends a [`PendingUnstakeTranche`] for later
    /// [`Contract::withdraw`](crate::Contract::withdraw).
    ///
    /// Same internal path as [`Contract::unlock`] after epoch preliminaries (settlement -> claim).
    ///
    /// Pricing uses [`Validator::net_stake_yocto`]: **gross** backing minus unsettled user exit liability
    /// outside this contract (`pending_to_unstake + pending_to_withdraw`) before this commit. That
    /// keeps exits aligned with minting and prevents re-pricing after pool unstake clears
    /// [`Validator::pending_to_unstake`] while claims are still outstanding.
    ///
    /// Returns the NEAR value in **yocto** that was appended as a [`PendingUnstakeTranche`] for `account_id`
    /// on `validator_id` (same units as `near_amt` passed into `NearToken::from_yoctonear` for storage).
    pub(crate) fn internal_unstake(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> u128 {
        require!(
            shares_remove > 0,
            "Cannot exit shares: amount must be greater than zero"
        );
        let mut validator = self.require_validator(&validator_id);

        // Pool must have enough outstanding share units to burn.
        let validator_total_shares = validator.total_shares.0;
        require!(
            validator_total_shares > 0 && validator_total_shares >= shares_remove,
            "Cannot exit shares: validator pool has no shares or amount exceeds pool total"
        );

        // Exit price: same effective backing as mint paths (unsettled user liability in the divisor).
        let net_stake = validator.net_stake_yocto();
        require!(
            net_stake > 0,
            "Cannot price this exit: no effective stake left for remaining shares; wait for stake or withdraw steps to finish, then retry"
        );

        // NEAR value of this exit when priced; also returned (yocto) for callers that log or chain.
        let near_amt = near_from_shares(shares_remove, net_stake, validator_total_shares);
        let near_token = NearToken::from_yoctonear(near_amt);

        // Validator: burn pool shares, queue NEAR for `try_epoch_stake_or_unstake` / pool `unstake`, and track
        // gross user-exit liability until claims drain `user_pending_unstake`.
        validator.total_shares = U128(validator_total_shares - shares_remove);
        validator.pending_to_unstake = validator
            .pending_to_unstake
            .checked_add(near_token)
            .expect("pending_to_unstake overflow");

        // User position on this pool: decrement or drop the `(account, validator)` share balance.
        let account_validator_shares_key = (account_id.clone(), validator_id.clone());
        let user_shares_on_validator = self
            .user_validator_shares
            .get(&account_validator_shares_key)
            .copied()
            .unwrap_or(0);
        require!(
            user_shares_on_validator >= shares_remove,
            "Cannot exit shares: account does not hold enough shares on this validator"
        );
        if user_shares_on_validator == shares_remove {
            self.user_validator_shares
                .remove(&account_validator_shares_key);
        } else {
            self.user_validator_shares.insert(
                account_validator_shares_key.clone(),
                user_shares_on_validator - shares_remove,
            );
        }

        // Epoch gate for `withdraw`: see [`Contract::pending_unstake_tranche_available_epoch_height`].
        let available_epoch_height =
            self.pending_unstake_tranche_available_epoch_height(&validator);
        let mut pending_unstake_tranches = self
            .user_pending_unstake
            .get(&account_validator_shares_key)
            .cloned()
            .unwrap_or_default();
        pending_unstake_tranches.push(PendingUnstakeTranche {
            amount: near_token,
            available_epoch_height,
        });
        self.user_pending_unstake
            .insert(account_validator_shares_key, pending_unstake_tranches);

        // Validator-level index of accounts that still have queued or claimable exit NEAR.
        if !validator
            .accounts_with_pending_unstake
            .contains(&account_id)
        {
            validator
                .accounts_with_pending_unstake
                .push(account_id.clone());
        }

        self.validators.insert(validator_id, validator);
        near_amt
    }

    /// After pool `get_owner_id`: promise ok, not paused, caller is pool owner.
    pub(crate) fn assert_validator_owner(&self, pool_owner: AccountId, caller: &AccountId) {
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == *caller,
            "Only the validator owner can call this method"
        );
    }
}
