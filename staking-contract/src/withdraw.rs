use crate::internal::withdraw_batch_credit_yocto;
use crate::*;
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
            "Claim does not match your pending unstake for this batch (contract accounting error)"
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
        require!(
            o_y > 0,
            "You have no unlocked NEAR waiting to claim for this validator"
        );

        let mut v = self.require_validator(&validator_id);
        if o_y > 0 && !v.accounts_with_pending_unstake.contains(&account_id) {
            v.accounts_with_pending_unstake.push(account_id.clone());
        }
        let w_y = v.pending_to_withdraw.as_yoctonear();
        require!(
            w_y > 0,
            "No NEAR is in the withdraw bucket yet; wait until the operator runs epoch_withdraw"
        );
        require!(
            v.pending_user_unstake_total.as_yoctonear() > 0,
            "Nothing to claim: total user liability for this pool is zero. If NEAR is still stuck after everyone has claimed, the owner can call sweep_stranded_withdraw_bucket"
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
            let c_y = withdraw_batch_credit_yocto(b_y, o_elig, l_y);
            if c_y == 0 {
                continue;
            }

            batch.remaining = NearToken::from_yoctonear(b_y - c_y);
            v.pending_to_withdraw = v
                .pending_to_withdraw
                .checked_sub(NearToken::from_yoctonear(c_y))
                .expect(
                    "pending_to_withdraw accounting mismatch; contact the contract maintainers",
                );
            v.pending_user_unstake_total = v
                .pending_user_unstake_total
                .checked_sub(NearToken::from_yoctonear(c_y))
                .expect("pending_user_unstake_total accounting mismatch; contact the contract maintainers");

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

        require!(
            total_credit_yocto > 0,
            "Nothing to claim for this call (rounding produced zero across all withdraw batches)"
        );

        let credit = NearToken::from_yoctonear(total_credit_yocto);

        let mut acc = self.accounts.get(&account_id).cloned().unwrap_or_else(|| {
            env::panic_str("Account not registered; call storage_deposit first")
        });
        acc.withdrawable_balance = acc
            .withdrawable_balance
            .checked_add(credit)
            .expect("withdrawable_balance overflow; amount too large");
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

        let mut v = self.require_validator(&validator_id);
        let w_y = v.pending_to_withdraw.as_yoctonear();
        let t_y = v.pending_user_unstake_total.as_yoctonear();
        require!(
            t_y == 0,
            "Cannot sweep: user liability for this pool must be zero first"
        );
        require!(
            w_y > 0,
            "Cannot sweep: there is no stranded balance in the withdraw bucket"
        );

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
        let mut acc = self.accounts.get(&pred).cloned().unwrap_or_else(|| {
            env::panic_str("Account not registered; call storage_deposit first")
        });
        let bal = acc.withdrawable_balance.as_yoctonear();
        let withdraw_yocto = match amount {
            Some(a) => {
                require!(
                    a.as_yoctonear() <= bal,
                    "Withdraw amount is larger than your withdrawable balance"
                );
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
