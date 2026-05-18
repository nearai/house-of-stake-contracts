use crate::epoch::ext_self_epoch;
use crate::gas::callbacks;
use crate::internal::{effective_stake_for_share_exit, near_from_shares};
use crate::*;
use near_sdk::json_types::U128;
use near_sdk::{NearToken, Promise, PromiseOrValue, env, is_promise_success, near, require};

#[near]
impl Contract {
    /// User-driven unlock: only the lock owner, after `lock.end_ns`. Runs the same per-epoch validator
    /// pipeline as catalog **`lock`** (balance sync / withdraw / settle when due, or fast path when this
    /// pool already settled the current NEAR epoch), then sets the pool row **`Busy`**, queues your unstake,
    /// and continues the withdraw-first / **`unstake`** pipeline.
    #[payable]
    pub fn unlock(&mut self, lock_id: LockId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let lock = self
            .locks
            .get(&lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Lock not found; check the lock id"));
        require!(
            env::predecessor_account_id() == lock.account_id,
            "Only the lock owner can unlock"
        );
        require!(lock.status == LockStatus::Active, "Lock is not active");
        require!(
            env::block_timestamp() >= lock.end_ns.0,
            "Lock period has not ended yet"
        );

        let validator_id = lock.validator_id.clone();
        let validator = self.require_validator(&validator_id);
        self.assert_validator_idle_for_user_action(&validator);

        self.promise_validator_per_epoch_settlement_then(
            validator_id.clone(),
            PerEpochContinue::UnlockQueueUnstake {
                validator_id,
                lock_id,
                account_id: lock.account_id.clone(),
                shares_remove: lock.shares.0,
            },
        )
    }
}

// =============================================================================
// Epoch pipeline: unlock / unstake tail (callbacks from `epoch` settlement dispatch)
// =============================================================================

#[near]
impl Contract {
    #[private]
    /// **[Pipeline 5b]** Share exit, then start **5b′** ([`Contract::promise_post_unlock_unstaked_pipeline`]).
    pub fn on_unlock_tail_after_pre_user_settle(
        &mut self,
        lock_id: LockId,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> Promise {
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy after per-epoch settlement"
        );

        let mut lock = self
            .locks
            .get(&lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Lock not found"));
        require!(
            lock.account_id == account_id,
            "Unlock no longer matches the lock owner; retry"
        );
        require!(
            lock.status == LockStatus::Active,
            "Lock is no longer active; nothing to unlock"
        );
        require!(lock.validator_id == validator_id, "Lock validator mismatch");
        require!(lock.shares.0 == shares_remove, "Lock shares changed; retry");

        let account_log = lock.account_id.clone();
        let validator_log = lock.validator_id.clone();
        self.commit_share_exit(account_id.clone(), validator_id.clone(), shares_remove);
        lock.status = LockStatus::UnlockRequested;
        self.locks.insert(lock_id.clone(), lock);

        crate::events::log_unlock(lock_id.as_str(), &account_log, &validator_log);

        self.promise_post_unlock_unstaked_pipeline(validator_id)
    }

    #[private]
    /// **[Pipeline 5b′]** Unlock tail: `get_account`, optional **2a–2c**, optional **3** (then **6** via **4**).
    pub fn on_unstake_pipeline_pool_account(
        &mut self,
        #[callback] pool_account: PoolAccountView,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool> {
        if !is_promise_success() {
            self.release_validator_pool_pipeline(&validator_id);
            env::panic_str("Could not read pool account from the pool; retry in a few blocks");
        }

        let unstaked = pool_account.unstaked();

        if unstaked.as_yoctonear() > 0 && pool_account.can_withdraw {
            return self
                .try_epoch_withdraw_known_unstaked(
                    validator_id.clone(),
                    unstaked,
                    pool_account.can_withdraw,
                )
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                        .on_after_pool_withdraw_maybe_settle(validator_id, None),
                )
                .into();
        }

        let validator = self.require_validator(&validator_id);
        if validator.last_settlement_epoch < env::epoch_height() {
            return self.try_epoch_stake_or_unstake(validator_id, None).into();
        }
        PromiseOrValue::Value(true)
    }
}

impl Contract {
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

    /// Commits a **share exit** for `account_id` on `validator_id`: burns `shares_remove` pool share units,
    /// prices them into NEAR using the same effective backing as mints, updates validator pending
    /// unstake buckets, and appends a [`PendingUnstakeTranche`] for later [`Contract::withdraw`](crate::Contract::withdraw).
    ///
    /// Same internal path as [`Contract::unlock`] after epoch preliminaries (settlement → claim).
    ///
    /// Pricing uses [`crate::internal::effective_stake_for_share_exit`]: **gross** backing minus the full
    /// unsettled user exit liability [`Validator::pending_user_unstake_total`] (before this commit). That
    /// keeps exits aligned with minting and prevents re-pricing after pool unstake clears
    /// [`Validator::pending_to_unstake`] while claims are still outstanding.
    ///
    /// Returns the NEAR value in **yocto** that was appended as a [`PendingUnstakeTranche`] for `account_id`
    /// on `validator_id` (same units as `near_amt` passed into `NearToken::from_yoctonear` for storage).
    pub(crate) fn commit_share_exit(
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

        // Exit price: same effective backing as mint paths (`pending_user_unstake_total` in the divisor).
        let effective_stake_yocto = effective_stake_for_share_exit(
            validator.total_staked_balance,
            validator.pending_to_stake,
            validator.pending_user_unstake_total,
        );
        require!(
            effective_stake_yocto > 0,
            "Cannot price this exit: no effective stake left for remaining shares; wait for stake or withdraw steps to finish, then retry"
        );

        // NEAR value of this exit when priced; also returned (yocto) for callers that log or chain.
        let near_amt =
            near_from_shares(shares_remove, effective_stake_yocto, validator_total_shares);
        let near_token = NearToken::from_yoctonear(near_amt);

        // Validator: burn pool shares, queue NEAR for `try_epoch_stake_or_unstake` / pool `unstake`, and track
        // gross user-exit liability until claims drain `user_pending_unstake`.
        validator.total_shares = U128(validator_total_shares - shares_remove);
        validator.pending_to_unstake = validator
            .pending_to_unstake
            .checked_add(near_token)
            .expect("pending_to_unstake overflow");
        validator.pending_user_unstake_total = validator
            .pending_user_unstake_total
            .checked_add(near_token)
            .expect("pending_user_unstake_total overflow");

        // User position on this pool: decrement or drop the `(account, validator)` share row.
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
}
