use crate::internal::{effective_stake_for_share_exit, near_from_shares};
use crate::*;
use near_sdk::json_types::U128;
use near_sdk::{NearToken, env, near, require};

#[near]
impl Contract {
    /// User-driven unlock: only the lock owner, after `lock.end_ns`; shares convert to NEAR at unlock time.
    #[payable]
    pub fn unlock(&mut self, lock_id: LockId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let mut lock = self
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
        let account_log = lock.account_id.clone();
        let validator_log = lock.validator_id.clone();
        let sh = lock.shares.0;
        self.queue_shares_unstake(lock.account_id.clone(), validator_id, sh);
        lock.status = LockStatus::UnlockRequested;
        self.locks.insert(lock_id.clone(), lock);

        crate::events::log_unlock(lock_id.as_str(), &account_log, &validator_log);
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
        validator_id: AccountId,
        shares_remove: u128,
    ) -> u128 {
        require!(
            shares_remove > 0,
            "Cannot queue unstake: share amount must be greater than zero"
        );
        let mut v = self.require_validator(&validator_id);

        let ts = v.total_shares.0;
        require!(
            ts > 0 && ts >= shares_remove,
            "Cannot queue unstake: validator pool has no shares or amount exceeds pool total"
        );

        let eff = effective_stake_for_share_exit(
            v.total_staked_balance,
            v.pending_to_stake,
            v.pending_user_unstake_total,
        );
        require!(
            eff > 0,
            "Cannot price this exit: no effective stake left for remaining shares. Ask the operator to run refresh_validator_balance, or wait for stake or withdraw steps to finish"
        );

        let near_amt = near_from_shares(shares_remove, eff, ts);
        let near_token = NearToken::from_yoctonear(near_amt);

        v.total_shares = U128(ts - shares_remove);
        v.pending_to_unstake = v
            .pending_to_unstake
            .checked_add(near_token)
            .expect("pending_to_unstake overflow");
        v.pending_user_unstake_total = v
            .pending_user_unstake_total
            .checked_add(near_token)
            .expect("pending_user_unstake_total overflow");

        let ukey = (account_id.clone(), validator_id.clone());
        let us = self.user_validator_shares.get(&ukey).copied().unwrap_or(0);
        require!(
            us >= shares_remove,
            "Cannot queue unstake: account does not hold enough shares on this validator"
        );
        if us == shares_remove {
            self.user_validator_shares.remove(&ukey);
        } else {
            self.user_validator_shares
                .insert(ukey.clone(), us - shares_remove);
        }

        let min_b = v.withdraw_batches.len() as u32;
        let mut tranches = self
            .user_pending_unstake
            .get(&ukey)
            .cloned()
            .unwrap_or_default();
        tranches.push(PendingUnstakeTranche {
            amount: near_token,
            min_withdraw_batch_index: min_b,
        });
        self.user_pending_unstake.insert(ukey, tranches);

        if !v.accounts_with_pending_unstake.contains(&account_id) {
            v.accounts_with_pending_unstake.push(account_id.clone());
        }

        self.validators.insert(validator_id, v);
        near_amt
    }
}
