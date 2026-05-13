use crate::gas::callbacks;
use crate::internal::withdraw_batch_credit_yocto;
use crate::*;
use near_sdk::ext_contract;
use near_sdk::{AccountId, NearToken, Promise, PromiseOrValue, env, near, require};

#[ext_contract(ext_self_claim)]
pub trait ExtSelfClaim {
    fn on_claim_after_pool_withdraw_for_user(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    );
}

impl Contract {
    fn user_pending_tranches_total_yocto(
        &self,
        account_validator_key: &(AccountId, ValidatorId),
    ) -> u128 {
        self.user_pending_unstake
            .get(account_validator_key)
            .map(|tranches| {
                tranches
                    .iter()
                    .map(|tranche| tranche.amount.as_yoctonear())
                    .fold(0u128, |sum, yocto| sum.saturating_add(yocto))
            })
            .unwrap_or(0)
    }

    fn sum_user_tranches_eligible_yocto(
        &self,
        account_validator_key: &(AccountId, ValidatorId),
        batch_idx: u32,
    ) -> u128 {
        self.user_pending_unstake
            .get(account_validator_key)
            .map(|tranches| {
                tranches
                    .iter()
                    .filter(|tranche| tranche.min_withdraw_batch_index <= batch_idx)
                    .map(|tranche| tranche.amount.as_yoctonear())
                    .fold(0u128, |sum, yocto| sum.saturating_add(yocto))
            })
            .unwrap_or(0)
    }

    /// Returns true iff the user has no tranches left after the deduction.
    fn reduce_user_tranches_after_batch_claim(
        &mut self,
        account_validator_key: &(AccountId, ValidatorId),
        batch_idx: u32,
        mut deduct_yocto: u128,
    ) -> bool {
        if deduct_yocto == 0 {
            return self
                .user_pending_unstake
                .get(account_validator_key)
                .map_or(true, |t| t.is_empty());
        }
        let mut pending_unstake_tranches = self
            .user_pending_unstake
            .get(account_validator_key)
            .cloned()
            .unwrap_or_default();
        for tranche in pending_unstake_tranches.iter_mut() {
            if tranche.min_withdraw_batch_index > batch_idx {
                continue;
            }
            if deduct_yocto == 0 {
                break;
            }
            let tranche_amount_yocto = tranche.amount.as_yoctonear();
            if tranche_amount_yocto == 0 {
                continue;
            }
            let take_from_tranche_yocto = tranche_amount_yocto.min(deduct_yocto);
            tranche.amount =
                NearToken::from_yoctonear(tranche_amount_yocto - take_from_tranche_yocto);
            deduct_yocto -= take_from_tranche_yocto;
        }
        require!(
            deduct_yocto == 0,
            "Claim does not match your pending unstake for this batch (contract accounting error)"
        );
        pending_unstake_tranches.retain(|tranche| tranche.amount.as_yoctonear() > 0);
        if pending_unstake_tranches.is_empty() {
            self.user_pending_unstake.remove(account_validator_key);
            true
        } else {
            self.user_pending_unstake
                .insert(account_validator_key.clone(), pending_unstake_tranches);
            false
        }
    }

    /// Pro-rata claim from `pending_to_withdraw` / `withdraw_batches` after tranche and bucket preconditions.
    pub(crate) fn claim_unlocked_near_finish(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) {
        let account_validator_key = (account_id.clone(), validator_id.clone());
        let mut validator = self.require_validator(&validator_id);
        let pending_withdraw_bucket_yocto = validator.pending_to_withdraw.as_yoctonear();
        require!(
            pending_withdraw_bucket_yocto > 0,
            "No NEAR is in the withdraw bucket yet; wait until unstaked funds are moved from the pool (e.g. call claim_unlocked_near again after epochs settle), or retry later"
        );
        require!(
            validator.pending_user_unstake_total.as_yoctonear() > 0,
            "Nothing to claim: total user liability for this pool is zero. If NEAR is still stuck after everyone has claimed, the owner can call sweep_stranded_withdraw_bucket"
        );

        let mut total_credit_yocto = 0u128;

        for (batch_idx, batch) in validator.withdraw_batches.iter_mut().enumerate() {
            let batch_remaining_yocto = batch.remaining.as_yoctonear();
            if batch_remaining_yocto == 0 {
                continue;
            }
            let eligible_user_unstake_yocto =
                self.sum_user_tranches_eligible_yocto(&account_validator_key, batch_idx as u32);
            if eligible_user_unstake_yocto == 0 {
                continue;
            }
            let batch_liability_yocto = batch.liability_at_fund.as_yoctonear();
            let claim_credit_yocto = withdraw_batch_credit_yocto(
                batch_remaining_yocto,
                eligible_user_unstake_yocto,
                batch_liability_yocto,
            );
            if claim_credit_yocto == 0 {
                continue;
            }

            batch.remaining = NearToken::from_yoctonear(batch_remaining_yocto - claim_credit_yocto);
            validator.pending_to_withdraw = validator
                .pending_to_withdraw
                .checked_sub(NearToken::from_yoctonear(claim_credit_yocto))
                .expect(
                    "pending_to_withdraw accounting mismatch; contact the contract maintainers",
                );
            validator.pending_user_unstake_total = validator
                .pending_user_unstake_total
                .checked_sub(NearToken::from_yoctonear(claim_credit_yocto))
                .expect("pending_user_unstake_total accounting mismatch; contact the contract maintainers");

            let user_done = self.reduce_user_tranches_after_batch_claim(
                &account_validator_key,
                batch_idx as u32,
                claim_credit_yocto,
            );
            if user_done {
                validator
                    .accounts_with_pending_unstake
                    .retain(|a| *a != account_id);
            }
            total_credit_yocto = total_credit_yocto.saturating_add(claim_credit_yocto);
        }

        if self
            .user_pending_unstake
            .get(&account_validator_key)
            .is_none()
        {
            validator
                .accounts_with_pending_unstake
                .retain(|a| *a != account_id);
        }

        require!(
            total_credit_yocto > 0,
            "Nothing to claim for this call (rounding produced zero across all withdraw batches)"
        );

        let credit = NearToken::from_yoctonear(total_credit_yocto);

        let mut account = self.accounts.get(&account_id).cloned().unwrap_or_else(|| {
            env::panic_str("Account not registered; call storage_deposit first")
        });
        account.withdrawable_balance = account
            .withdrawable_balance
            .checked_add(credit)
            .expect("withdrawable_balance overflow; amount too large");
        self.accounts.insert(account_id.clone(), account);
        self.validators.insert(validator_id.clone(), validator);

        crate::events::log_claim_unlocked(&account_id, &validator_id);
    }
}

#[near]
impl Contract {
    /// Move a pro-rata share of each withdraw batch into this account's `withdrawable_balance`.
    /// Run after unstaked NEAR has been moved from the staking pool into the contract bucket.
    ///
    /// When the bucket is empty but the chain is far enough past the last unstake to withdraw from the
    /// pool, this method pulls from the pool first (same internal path as unlock settlement), then
    /// credits your claim in one transaction.
    #[payable]
    pub fn claim_unlocked_near(&mut self, validator_id: ValidatorId) -> PromiseOrValue<()> {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let account_id = env::predecessor_account_id();
        self.ensure_min_base_storage(&account_id);

        let account_validator_key = (account_id.clone(), validator_id.clone());
        let user_pending_tranches_yocto =
            self.user_pending_tranches_total_yocto(&account_validator_key);
        require!(
            user_pending_tranches_yocto > 0,
            "You have no unlocked NEAR waiting to claim for this validator"
        );

        let mut validator = self.require_validator(&validator_id);
        if user_pending_tranches_yocto > 0
            && !validator
                .accounts_with_pending_unstake
                .contains(&account_id)
        {
            validator
                .accounts_with_pending_unstake
                .push(account_id.clone());
        }
        let pending_withdraw_bucket_yocto = validator.pending_to_withdraw.as_yoctonear();

        let may_prefetch_pool_withdraw = pending_withdraw_bucket_yocto == 0
            && validator.last_unstake_epoch > 0
            && env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs)
            && validator.tx_status == TransactionStatus::Idle;

        if may_prefetch_pool_withdraw {
            self.validators.insert(validator_id.clone(), validator);
            return self
                .try_epoch_withdraw(validator_id.clone(), false)
                .then(
                    ext_self_claim::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_CLAIM_AFTER_POOL_WITHDRAW)
                        .on_claim_after_pool_withdraw_for_user(account_id, validator_id),
                )
                .into();
        }

        self.validators.insert(validator_id.clone(), validator);
        self.claim_unlocked_near_finish(account_id, validator_id);
        PromiseOrValue::Value(())
    }

    #[private]
    pub fn on_claim_after_pool_withdraw_for_user(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) {
        self.claim_unlocked_near_finish(account_id, validator_id);
    }

    /// When [`Validator::pending_user_unstake_total`] is zero but [`Validator::pending_to_withdraw`] is still
    /// positive (e.g. pool rounding so `w > t` after the last user claims), transfer that remainder to the
    /// contract owner.
    #[payable]
    pub fn sweep_stranded_withdraw_bucket(&mut self, validator_id: ValidatorId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        self.assert_owner();

        let mut validator = self.require_validator(&validator_id);
        let pending_withdraw_bucket_yocto = validator.pending_to_withdraw.as_yoctonear();
        let pending_user_unstake_liability_yocto =
            validator.pending_user_unstake_total.as_yoctonear();
        require!(
            pending_user_unstake_liability_yocto == 0,
            "Cannot sweep: user liability for this pool must be zero first"
        );
        require!(
            pending_withdraw_bucket_yocto > 0,
            "Cannot sweep: there is no stranded balance in the withdraw bucket"
        );

        validator.pending_to_withdraw = NearToken::from_near(0);
        validator.withdraw_batches.clear();
        self.validators.insert(validator_id, validator);

        Promise::new(self.config.owner_account_id.clone())
            .transfer(NearToken::from_yoctonear(pending_withdraw_bucket_yocto))
    }

    /// Withdraw NEAR that has been credited to `withdrawable_balance` after epoch withdraw completes.
    #[payable]
    pub fn withdraw(&mut self, amount: Option<NearToken>) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let withdrawer = env::predecessor_account_id();
        let mut account = self.accounts.get(&withdrawer).cloned().unwrap_or_else(|| {
            env::panic_str("Account not registered; call storage_deposit first")
        });
        let withdrawable_yocto = account.withdrawable_balance.as_yoctonear();
        let withdraw_yocto = match amount {
            Some(requested) => {
                require!(
                    requested.as_yoctonear() <= withdrawable_yocto,
                    "Withdraw amount is larger than your withdrawable balance"
                );
                requested.as_yoctonear()
            }
            None => withdrawable_yocto,
        };
        require!(withdraw_yocto > 0, "Nothing to withdraw");

        account.withdrawable_balance =
            NearToken::from_yoctonear(withdrawable_yocto - withdraw_yocto);
        self.accounts.insert(withdrawer.clone(), account);

        crate::events::log_withdraw(&withdrawer, withdraw_yocto);

        Promise::new(withdrawer).transfer(NearToken::from_yoctonear(withdraw_yocto))
    }
}
