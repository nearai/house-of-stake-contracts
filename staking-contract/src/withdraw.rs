use crate::*;
use common::U256;
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
            w.as_yoctonear() > 0,
            "Nothing in withdraw bucket yet; wait for epoch_withdraw"
        );
        require!(
            t.as_yoctonear() > 0,
            "Nothing to claim (liability total is zero). If the pool bucket still holds NEAR after all users have claimed, call sweep_stranded_withdraw_bucket"
        );

        let w_y = w.as_yoctonear();
        let o_y = o.as_yoctonear();
        let t_y = t.as_yoctonear();

        // Pro-rata: credit <= o and sum of credits across users equals min(w, t) when w <= t; when w > t,
        // the last claims leave stranded NEAR in `pending_to_withdraw` for `sweep_stranded_withdraw_bucket`.
        // Use U256 for (w * o) / t so yocto-scale products do not saturate u128.
        let credit_raw = (U256::from(w_y) * U256::from(o_y)) / U256::from(t_y);
        let mut credit_yocto = credit_raw.as_u128();
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

    /// When [`Validator::pending_user_unstake_total`] is zero but [`Validator::pending_to_withdraw`] is still
    /// positive (e.g. pool rounding so `w > t` after the last user claims), transfer that remainder to the
    /// contract owner.
    #[payable]
    pub fn sweep_stranded_withdraw_bucket(&mut self, validator_pool: AccountId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        self.assert_owner();

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown validator"));
        let w_y = v.pending_to_withdraw.as_yoctonear();
        let t_y = v.pending_user_unstake_total.as_yoctonear();
        require!(t_y == 0, "User liability must be zero before sweeping");
        require!(w_y > 0, "No stranded balance in withdraw bucket");

        v.pending_to_withdraw = NearToken::from_near(0);
        self.validators.insert(validator_pool, v);

        Promise::new(self.config.owner_account_id.clone()).transfer(NearToken::from_yoctonear(w_y))
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
    use common::U256;

    #[test]
    fn pro_rata_claim_rounding() {
        let w: u128 = 100;
        let o: u128 = 30;
        let t: u128 = 100;
        let c = ((U256::from(w) * U256::from(o)) / U256::from(t)).as_u128();
        assert_eq!(c, 30);
    }

    /// `w * o` can exceed `u128::MAX` at yocto scale; pro-rata must use `U256` for the product.
    #[test]
    fn pro_rata_large_product_needs_u256() {
        let w_y: u128 = 1u128 << 64;
        let o_y: u128 = 1u128 << 64;
        let t_y: u128 = 1u128 << 64;
        assert!(
            w_y.checked_mul(o_y).is_none(),
            "sanity: product overflows u128; math must use U256"
        );
        let credit_raw = (U256::from(w_y) * U256::from(o_y)) / U256::from(t_y);
        assert_eq!(credit_raw.as_u128(), 1u128 << 64);
    }
}
