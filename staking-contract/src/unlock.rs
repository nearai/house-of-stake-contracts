use crate::*;
use near_sdk::{Promise, env, near, require};

#[near]
impl Contract {
    /// User-driven unlock: only the lock owner, after `lock.end_ns`. Runs the same per-epoch validator
    /// pipeline as catalog **`lock`** (balance sync / withdraw / settle when due, or fast path when this
    /// pool already settled the current NEAR epoch), then **`internal_unstake`** (queues `pending_to_unstake`;
    /// pool `unstake` follows on the next settlement-driven flow or **`epoch_settle`**).
    #[payable]
    pub fn unlock(&mut self, lock_id: LockId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let buyer = env::predecessor_account_id();
        let lock = self.require_lock_owned_by(
            &lock_id,
            &buyer,
            "Lock not found; check the lock id",
            "Only the lock owner can unlock",
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
    /// **[Pipeline 5b]** Share exit after pre-user settlement (**0–3**); pool `unstake` for this exit is deferred.
    pub fn resolve_unlock(
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

        let mut lock = self.require_lock(&lock_id, "Lock not found");
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
        self.internal_unstake(account_id.clone(), validator_id.clone(), shares_remove);
        lock.status = LockStatus::UnlockRequested;
        self.locks.insert(lock_id.clone(), lock);

        crate::events::log_unlock(lock_id.as_str(), &account_log, &validator_log);

        Promise::new(env::current_account_id())
    }
}

impl Contract {
    pub(crate) fn require_lock(&self, lock_id: &LockId, not_found_msg: &str) -> Lock {
        self.locks
            .get(lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str(not_found_msg))
    }

    pub(crate) fn require_lock_owned_by(
        &self,
        lock_id: &LockId,
        owner: &AccountId,
        not_found_msg: &str,
        owner_mismatch_msg: &str,
    ) -> Lock {
        let lock = self.require_lock(lock_id, not_found_msg);
        require!(lock.account_id == *owner, owner_mismatch_msg);
        lock
    }

    pub(crate) fn require_subscription_lock_owned_by(
        &self,
        sub: &Subscription,
        buyer: &AccountId,
    ) -> Lock {
        let lock = self
            .locks
            .get(&sub.last_lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("No lock is linked to this subscription"));
        require!(
            lock.account_id == *buyer,
            "Only the lock owner can change this subscription lock"
        );
        require!(lock.status == LockStatus::Active, "Lock is not active");
        lock
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
}
