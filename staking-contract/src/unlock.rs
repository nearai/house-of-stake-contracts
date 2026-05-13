use crate::internal::{effective_stake_for_share_exit, near_from_shares};
use crate::*;
use near_sdk::json_types::U128;
use near_sdk::{NearToken, Promise, env, near, require};

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
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );

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

impl Contract {
    /// Release staking shares into the same unstake queue as [`Contract::unlock`] (epoch settlement → claim).
    ///
    /// Pricing uses [`crate::internal::effective_stake_for_share_exit`]: **gross** backing minus the full
    /// unsettled user exit liability [`Validator::pending_user_unstake_total`] (before this enqueue). That
    /// keeps exits aligned with minting and prevents re-pricing after pool unstake clears
    /// [`Validator::pending_to_unstake`] while claims are still outstanding. Returns NEAR yocto moved into
    /// `user_pending_unstake` tranches.
    pub(crate) fn queue_shares_unstake(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> u128 {
        require!(
            shares_remove > 0,
            "Cannot queue unstake: share amount must be greater than zero"
        );
        let mut validator = self.require_validator(&validator_id);

        let validator_total_shares = validator.total_shares.0;
        require!(
            validator_total_shares > 0 && validator_total_shares >= shares_remove,
            "Cannot queue unstake: validator pool has no shares or amount exceeds pool total"
        );

        let effective_stake_yocto = effective_stake_for_share_exit(
            validator.total_staked_balance,
            validator.pending_to_stake,
            validator.pending_user_unstake_total,
        );
        require!(
            effective_stake_yocto > 0,
            "Cannot price this exit: no effective stake left for remaining shares; wait for stake or withdraw steps to finish, then retry"
        );

        let near_amt =
            near_from_shares(shares_remove, effective_stake_yocto, validator_total_shares);
        let near_token = NearToken::from_yoctonear(near_amt);

        validator.total_shares = U128(validator_total_shares - shares_remove);
        validator.pending_to_unstake = validator
            .pending_to_unstake
            .checked_add(near_token)
            .expect("pending_to_unstake overflow");
        validator.pending_user_unstake_total = validator
            .pending_user_unstake_total
            .checked_add(near_token)
            .expect("pending_user_unstake_total overflow");

        let account_validator_shares_key = (account_id.clone(), validator_id.clone());
        let user_shares_on_validator = self
            .user_validator_shares
            .get(&account_validator_shares_key)
            .copied()
            .unwrap_or(0);
        require!(
            user_shares_on_validator >= shares_remove,
            "Cannot queue unstake: account does not hold enough shares on this validator"
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

        let next_withdraw_batch_index = validator.withdraw_batches.len() as u32;
        let mut pending_unstake_tranches = self
            .user_pending_unstake
            .get(&account_validator_shares_key)
            .cloned()
            .unwrap_or_default();
        pending_unstake_tranches.push(PendingUnstakeTranche {
            amount: near_token,
            min_withdraw_batch_index: next_withdraw_batch_index,
        });
        self.user_pending_unstake
            .insert(account_validator_shares_key, pending_unstake_tranches);

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
