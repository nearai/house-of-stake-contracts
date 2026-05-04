use crate::internal::{effective_stake_yocto, near_from_shares};
use crate::*;
use near_sdk::json_types::U128;
use near_sdk::{env, near, require, NearToken};

#[near]
impl Contract {
    /// User-driven unlock after `lock.end_ns` (Solution 1).
    #[payable]
    pub fn unlock(&mut self, lock_id: LockId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let mut lock = self.locks.get(&lock_id).cloned().expect("Unknown lock");
        require!(
            env::predecessor_account_id() == lock.account_id,
            "Only lock owner"
        );
        require!(lock.status == LockStatus::Active, "Lock not active");
        require!(
            env::block_timestamp() >= lock.end_ns.0,
            "Lock still active"
        );

        let validator_id = lock.validator_id.clone();
        let mut v = self
            .validators
            .get(&validator_id)
            .cloned()
            .expect("validator");

        let eff = effective_stake_yocto(v.total_staked_balance, v.pending_to_stake);
        let ts = v.total_shares.0;
        let sh = lock.shares.0;
        let near_amt = near_from_shares(sh, eff, ts);
        require!(ts >= sh && ts > 0, "share underflow");

        v.total_shares = U128(ts - sh);
        let near_token = NearToken::from_yoctonear(near_amt);
        v.pending_to_unstake = v
            .pending_to_unstake
            .checked_add(near_token)
            .expect("pending unstake overflow");
        v.pending_user_unstake_total = v
            .pending_user_unstake_total
            .checked_add(near_token)
            .expect("pending user total overflow");

        let ukey = (lock.account_id.clone(), validator_id.clone());
        let us = self
            .user_validator_shares
            .get(&ukey)
            .copied()
            .unwrap_or(0);
        require!(us >= sh, "user shares");
        if us == sh {
            self.user_validator_shares.remove(&ukey);
        } else {
            self.user_validator_shares.insert(ukey.clone(), us - sh);
        }

        let pending = self
            .user_pending_unstake
            .get(&ukey)
            .cloned()
            .unwrap_or(NearToken::from_near(0));
        self.user_pending_unstake.insert(
            ukey,
            pending.checked_add(near_token).expect("pending"),
        );

        lock.status = LockStatus::UnlockRequested;
        self.locks.insert(lock_id, lock);
        self.validators.insert(validator_id, v);
    }
}
