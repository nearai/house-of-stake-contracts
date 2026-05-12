use crate::*;
use common::U256;
use near_sdk::{AccountId, NearToken, Promise, env, near, require};

impl Contract {
    fn user_pending_tranches_total_yocto(&self, ukey: &(AccountId, AccountId)) -> u128 {
        self.user_pending_unstake
            .get(ukey)
            .map(|trs| {
                trs.iter()
                    .map(|t| t.amount.as_yoctonear())
                    .fold(0u128, |a, b| a.saturating_add(b))
            })
            .unwrap_or(0)
    }

    fn sum_user_tranches_eligible_yocto(
        &self,
        ukey: &(AccountId, AccountId),
        batch_idx: u32,
    ) -> u128 {
        self.user_pending_unstake
            .get(ukey)
            .map(|trs| {
                trs.iter()
                    .filter(|t| t.min_withdraw_batch_index <= batch_idx)
                    .map(|t| t.amount.as_yoctonear())
                    .fold(0u128, |a, b| a.saturating_add(b))
            })
            .unwrap_or(0)
    }

    /// Returns true iff the user has no tranches left after the deduction.
    fn reduce_user_tranches_after_batch_claim(
        &mut self,
        ukey: &(AccountId, AccountId),
        batch_idx: u32,
        mut deduct_yocto: u128,
    ) -> bool {
        if deduct_yocto == 0 {
            return self
                .user_pending_unstake
                .get(ukey)
                .map_or(true, |t| t.is_empty());
        }
        let mut trs = self
            .user_pending_unstake
            .get(ukey)
            .cloned()
            .unwrap_or_default();
        for t in trs.iter_mut() {
            if t.min_withdraw_batch_index > batch_idx {
                continue;
            }
            if deduct_yocto == 0 {
                break;
            }
            let a = t.amount.as_yoctonear();
            if a == 0 {
                continue;
            }
            let sub = a.min(deduct_yocto);
            t.amount = NearToken::from_yoctonear(a - sub);
            deduct_yocto -= sub;
        }
        require!(
            deduct_yocto == 0,
            "Internal: claim exceeds eligible tranches for this batch"
        );
        trs.retain(|t| t.amount.as_yoctonear() > 0);
        if trs.is_empty() {
            self.user_pending_unstake.remove(ukey);
            true
        } else {
            self.user_pending_unstake.insert(ukey.clone(), trs);
            false
        }
    }
}

#[near]
impl Contract {
    /// Move a pro-rata share of each withdraw batch into this account's `withdrawable_balance`.
    /// Run after [`crate::epoch::Contract::epoch_withdraw`] has pulled NEAR from the pool into the contract bucket.
    #[payable]
    pub fn claim_unlocked_near(&mut self, validator_id: AccountId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let account_id = env::predecessor_account_id();
        self.ensure_min_base_storage(&account_id);

        let ukey = (account_id.clone(), validator_id.clone());
        let o_y = self.user_pending_tranches_total_yocto(&ukey);
        require!(o_y > 0, "No pending unlocked NEAR for this validator");

        let mut v = self
            .validators
            .get(&validator_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown validator"));
        if o_y > 0 && !v.accounts_with_pending_unstake.contains(&account_id) {
            v.accounts_with_pending_unstake.push(account_id.clone());
        }
        let w_y = v.pending_to_withdraw.as_yoctonear();
        require!(
            w_y > 0,
            "Nothing in withdraw bucket yet; wait for epoch_withdraw"
        );
        require!(
            v.pending_user_unstake_total.as_yoctonear() > 0,
            "Nothing to claim (liability total is zero). If the pool bucket still holds NEAR after all users have claimed, call sweep_stranded_withdraw_bucket"
        );

        let mut total_credit_yocto = 0u128;

        for (batch_idx, batch) in v.withdraw_batches.iter_mut().enumerate() {
            let b_y = batch.remaining.as_yoctonear();
            if b_y == 0 {
                continue;
            }
            let o_elig = self.sum_user_tranches_eligible_yocto(&ukey, batch_idx as u32);
            if o_elig == 0 {
                continue;
            }
            let l_y = batch.liability_at_fund.as_yoctonear();
            if l_y == 0 {
                continue;
            }
            // Frozen denominator per batch so later unlocks cannot dilute an older bucket; use U256
            // for (b * o) / L so yocto-scale products do not saturate u128.
            let credit_raw = (U256::from(b_y) * U256::from(o_elig)) / U256::from(l_y);
            let mut c_y = credit_raw.as_u128().min(o_elig).min(b_y);
            if c_y == 0 && b_y > 0 && o_elig > 0 {
                c_y = 1.min(o_elig).min(b_y);
            }
            if c_y == 0 {
                continue;
            }

            batch.remaining = NearToken::from_yoctonear(b_y - c_y);
            v.pending_to_withdraw = v
                .pending_to_withdraw
                .checked_sub(NearToken::from_yoctonear(c_y))
                .expect("pending_to_withdraw underflow");
            v.pending_user_unstake_total = v
                .pending_user_unstake_total
                .checked_sub(NearToken::from_yoctonear(c_y))
                .expect("total underflow");

            let user_done =
                self.reduce_user_tranches_after_batch_claim(&ukey, batch_idx as u32, c_y);
            if user_done {
                v.accounts_with_pending_unstake.retain(|a| *a != account_id);
            }
            total_credit_yocto = total_credit_yocto.saturating_add(c_y);
        }

        if self.user_pending_unstake.get(&ukey).is_none() {
            v.accounts_with_pending_unstake.retain(|a| *a != account_id);
        }

        require!(total_credit_yocto > 0, "Nothing to claim (rounding)");

        let credit = NearToken::from_yoctonear(total_credit_yocto);

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
        self.validators.insert(validator_id.clone(), v);

        crate::events::log_claim_unlocked(&account_id, &validator_id);
    }

    /// When [`Validator::pending_user_unstake_total`] is zero but [`Validator::pending_to_withdraw`] is still
    /// positive (e.g. pool rounding so `w > t` after the last user claims), transfer that remainder to the
    /// contract owner.
    #[payable]
    pub fn sweep_stranded_withdraw_bucket(&mut self, validator_id: AccountId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        self.assert_owner();

        let mut v = self
            .validators
            .get(&validator_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown validator"));
        let w_y = v.pending_to_withdraw.as_yoctonear();
        let t_y = v.pending_user_unstake_total.as_yoctonear();
        require!(t_y == 0, "User liability must be zero before sweeping");
        require!(w_y > 0, "No stranded balance in withdraw bucket");

        v.pending_to_withdraw = NearToken::from_near(0);
        v.withdraw_batches.clear();
        self.validators.insert(validator_id, v);

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

    /// `floor(w*o/t)` is zero but bucket and liability are positive — claim uses 1 yocto dust rule.
    #[test]
    fn pro_rata_tiny_bucket_dust_minimum() {
        let w_y: u128 = 1;
        let o_y: u128 = 1;
        let t_y: u128 = 2;
        let floor = ((U256::from(w_y) * U256::from(o_y)) / U256::from(t_y)).as_u128();
        assert_eq!(floor, 0);
        let mut credit = floor.min(o_y).min(w_y);
        if credit == 0 && w_y > 0 && o_y > 0 {
            credit = 1.min(o_y).min(w_y);
        }
        assert_eq!(credit, 1);
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
