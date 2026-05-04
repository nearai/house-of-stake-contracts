use crate::*;
use near_sdk::{NearToken, Promise, env, near, require};

#[near]
impl Contract {
    /// Move a pro-rata share of [`Validator::pending_to_withdraw`] into this account's `withdrawable_balance`.
    /// Run after [`crate::epoch::Contract::epoch_withdraw`] has pulled NEAR from the pool into the contract bucket.
    #[payable]
    pub fn claim_unlocked_near(&mut self, validator_pool: AccountId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let account_id = env::predecessor_account_id();
        self.ensure_min_base_storage(&account_id);

        let ukey = (account_id.clone(), validator_pool.clone());
        let o = self
            .user_pending_unstake
            .get(&ukey)
            .cloned()
            .unwrap_or(NearToken::from_near(0));
        require!(
            o.as_yoctonear() > 0,
            "No pending unlocked NEAR for this validator"
        );

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown validator"));
        let w = v.pending_to_withdraw;
        let t = v.pending_user_unstake_total;
        require!(
            w.as_yoctonear() > 0 && t.as_yoctonear() > 0,
            "Nothing in withdraw bucket yet; wait for epoch_withdraw"
        );

        let w_y = w.as_yoctonear();
        let o_y = o.as_yoctonear();
        let t_y = t.as_yoctonear();

        let mut credit_yocto = w_y.saturating_mul(o_y).checked_div(t_y).unwrap_or(0);
        credit_yocto = credit_yocto.min(o_y).min(w_y);
        require!(credit_yocto > 0, "Nothing to claim (rounding)");

        let credit = NearToken::from_yoctonear(credit_yocto);

        v.pending_to_withdraw = w
            .checked_sub(credit)
            .expect("pending_to_withdraw underflow");
        v.pending_user_unstake_total = t.checked_sub(credit).expect("total underflow");

        let new_o = o.checked_sub(credit).expect("user pending underflow");
        if new_o.as_yoctonear() == 0 {
            self.user_pending_unstake.remove(&ukey);
        } else {
            self.user_pending_unstake.insert(ukey, new_o);
        }

        let mut acc = self
            .accounts
            .get(&account_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("No account; call storage_deposit"));
        acc.withdrawable_balance = acc
            .withdrawable_balance
            .checked_add(credit)
            .expect("withdrawable overflow");
        self.accounts.insert(account_id.clone(), acc);
        self.validators.insert(validator_pool.clone(), v);

        crate::events::log_claim_unlocked(&account_id, &validator_pool);
    }

    /// Withdraw NEAR that has been credited to `withdrawable_balance` after epoch withdraw completes.
    #[payable]
    pub fn withdraw(&mut self, amount: Option<NearToken>) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let pred = env::predecessor_account_id();
        let mut acc = self
            .accounts
            .get(&pred)
            .cloned()
            .unwrap_or_else(|| env::panic_str("No account; call storage_deposit"));
        let bal = acc.withdrawable_balance.as_yoctonear();
        let withdraw_yocto = match amount {
            Some(a) => {
                require!(a.as_yoctonear() <= bal, "Too much");
                a.as_yoctonear()
            }
            None => bal,
        };
        require!(withdraw_yocto > 0, "Nothing to withdraw");

        acc.withdrawable_balance = NearToken::from_yoctonear(bal - withdraw_yocto);
        self.accounts.insert(pred.clone(), acc);

        crate::events::log_withdraw(&pred, withdraw_yocto);

        Promise::new(pred).transfer(NearToken::from_yoctonear(withdraw_yocto))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pro_rata_claim_rounding() {
        let w: u128 = 100;
        let o: u128 = 30;
        let t: u128 = 100;
        let c = w.saturating_mul(o) / t;
        assert_eq!(c, 30);
    }
}
