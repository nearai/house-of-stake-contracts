//! User exit payouts: pull NEAR from the contract per-pool **withdraw bucket** (`pending_to_withdraw`)
//! against this account `user_pending_unstake` tranches (epoch-gated), then transfer to the user.
//! **WASM:** [`crate::epoch::Contract::promise_validator_per_epoch_settlement_then`] runs first (same as
//! `lock` / `unlock`). **Non-WASM:** the payout tail runs directly for `testing_env!` tests.
//! Private callback **`on_withdraw_user_after_epoch_settlement`** (epoch dispatch) lives in this module.

use crate::gas::callbacks;
use crate::*;
use near_sdk::ext_contract;
use near_sdk::{AccountId, NearToken, Promise, env, near, require};

#[ext_contract(ext_self_withdraw)]
/// Self-call after an optional pool `withdraw` prefetched funds into `pending_to_withdraw`.
pub trait ExtSelfWithdraw {
    fn on_withdraw_after_pool_withdraw_for_user(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise;
}

#[near]
impl Contract {
    /// Claim NEAR from `pending_to_withdraw` for this account on `validator_id` (epoch-eligible tranches)
    /// and **transfer it immediately** to the caller.
    ///
    /// **WASM:** Uses [`crate::epoch::Contract::promise_validator_per_epoch_settlement_then`] so balance
    /// refresh, withdraw-if-ready, and net settle run before the payout (same ordering as **`lock`** /
    /// **`unlock`**).
    ///
    /// **Non-WASM:** Runs the payout tail only (host `testing_env!` does not execute pool promise chains;
    /// see `lock.rs`).
    ///
    /// The payout tail may prefetch a pool `withdraw` when the in-contract bucket is empty (LAZY_EPOCH_PIPELINE §2b).
    #[payable]
    pub fn withdraw(&mut self, validator_id: ValidatorId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let account_id = env::predecessor_account_id();
        self.ensure_min_base_storage(&account_id);

        let account_validator_key = (account_id.clone(), validator_id.clone());
        // Must have tranches from a prior `commit_share_exit` / unlock on this pool.
        let user_pending_tranches_yocto =
            self.user_pending_tranches_total_yocto(&account_validator_key);
        require!(
            user_pending_tranches_yocto > 0,
            "You have no unlocked NEAR waiting to claim for this validator"
        );

        let mut validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        // Keep this account on the validator’s worklist for settle/withdraw scheduling.
        if user_pending_tranches_yocto > 0
            && !validator
                .accounts_with_pending_unstake
                .contains(&account_id)
        {
            validator
                .accounts_with_pending_unstake
                .push(account_id.clone());
        }
        self.validators.insert(validator_id.clone(), validator);

        #[cfg(not(target_arch = "wasm32"))]
        {
            return self.withdraw_user_transfer_tail(account_id, validator_id);
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.promise_validator_per_epoch_settlement_then(
                validator_id.clone(),
                PerEpochContinue::WithdrawUserTransfer {
                    validator_id,
                    account_id,
                },
            )
        }
    }

    #[private]
    /// Continuation after `try_epoch_withdraw`: bucket should now hold NEAR for claims + transfer.
    pub fn on_withdraw_after_pool_withdraw_for_user(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        self.withdraw_user_transfer_tail(account_id, validator_id)
    }

    #[private]
    /// After shared per-epoch settlement (`epoch.rs`): user withdraw (claim + transfer; may prefetch pool withdraw).
    pub fn on_withdraw_user_after_epoch_settlement(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        self.withdraw_user_transfer_tail(account_id, validator_id)
    }
}

impl Contract {
    /// Sum of all `PendingUnstakeTranche.amount` for this `(account, validator)`.
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

    /// Sum of tranche NEAR with `epoch_height >= tranche.available_epoch_height`.
    fn sum_user_tranches_eligible_by_epoch_yocto(
        &self,
        account_validator_key: &(AccountId, ValidatorId),
        epoch_height: u64,
    ) -> u128 {
        self.user_pending_unstake
            .get(account_validator_key)
            .map(|tranches| {
                tranches
                    .iter()
                    .filter(|tranche| epoch_height >= tranche.available_epoch_height)
                    .map(|tranche| tranche.amount.as_yoctonear())
                    .fold(0u128, |sum, yocto| sum.saturating_add(yocto))
            })
            .unwrap_or(0)
    }

    /// Deducts `deduct_yocto` from epoch-eligible tranches in FIFO order. Returns true when
    /// `user_pending_unstake` for this key is now empty.
    fn reduce_user_tranches_after_claim(
        &mut self,
        account_validator_key: &(AccountId, ValidatorId),
        epoch_height: u64,
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
            if epoch_height < tranche.available_epoch_height {
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
            "Claim does not match your pending unstake for this withdraw (contract accounting error)"
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

    /// Claim from `pending_to_withdraw` after tranche epoch and bucket preconditions.
    /// Credits `min(eligible_tranche_yocto, pending_to_withdraw)` for this user, then reduces tranches
    /// and validator totals. Returns NEAR for the caller to transfer (this fn does not send tokens).
    pub(crate) fn withdraw_unlocked_near_finish(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> NearToken {
        let account_validator_key = (account_id.clone(), validator_id.clone());
        let mut validator = self.require_validator(&validator_id);
        let pending_withdraw_bucket_yocto = validator.pending_to_withdraw.as_yoctonear();
        require!(
            pending_withdraw_bucket_yocto > 0,
            "No NEAR is in the withdraw bucket yet; wait until unstaked funds are moved from the pool (e.g. call withdraw again after epochs settle), or retry later"
        );
        require!(
            validator.pending_user_unstake_total.as_yoctonear() > 0,
            "Nothing to claim: total user liability for this pool is zero (no tranches left to match the withdraw bucket)"
        );

        let eh = env::epoch_height();
        let eligible_yocto =
            self.sum_user_tranches_eligible_by_epoch_yocto(&account_validator_key, eh);
        require!(
            eligible_yocto > 0,
            "Nothing to claim yet: wait until `epoch_height >=` your tranche's available epoch height"
        );

        let credit_yocto = eligible_yocto.min(pending_withdraw_bucket_yocto);
        require!(
            credit_yocto > 0,
            "Nothing to claim for this call (zero credit after bucket cap)"
        );

        validator.pending_to_withdraw = validator
            .pending_to_withdraw
            .checked_sub(NearToken::from_yoctonear(credit_yocto))
            .expect(
                "pending_to_withdraw accounting mismatch; contact the contract maintainers",
            );
        validator.pending_user_unstake_total = validator
            .pending_user_unstake_total
            .checked_sub(NearToken::from_yoctonear(credit_yocto))
            .expect("pending_user_unstake_total accounting mismatch; contact the contract maintainers");

        let user_done = self.reduce_user_tranches_after_claim(&account_validator_key, eh, credit_yocto);
        if user_done {
            validator
                .accounts_with_pending_unstake
                .retain(|a| *a != account_id);
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

        let credit = NearToken::from_yoctonear(credit_yocto);

        self.validators.insert(validator_id.clone(), validator);

        crate::events::log_withdraw(&account_id, &validator_id, credit.as_yoctonear());
        credit
    }

    /// After optional shared settlement (WASM): claim from withdraw bucket and transfer, or prefetch pool
    /// withdraw when the bucket is still empty (§2b). Used by `withdraw` and `on_withdraw_after_pool_withdraw_for_user`.
    pub(crate) fn withdraw_user_transfer_tail(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        let validator = self.require_validator(&validator_id);
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
            return self.try_epoch_withdraw(validator_id.clone(), false).then(
                ext_self_withdraw::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_CLAIM_AFTER_POOL_WITHDRAW)
                    .on_withdraw_after_pool_withdraw_for_user(account_id, validator_id),
            );
        }

        self.validators.insert(validator_id.clone(), validator);
        let credit = self.withdraw_unlocked_near_finish(account_id.clone(), validator_id);
        Promise::new(account_id).transfer(credit)
    }
}
