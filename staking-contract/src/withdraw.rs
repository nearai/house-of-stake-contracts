//! User exit payouts: NEAR sitting in the pool-scoped **withdraw bucket** ([`Validator::pending_to_withdraw`])
//! is matched against this account’s `user_pending_unstake` tranches
//! (epoch-gated), then sent to the user.
//!
//! **Flow (WASM):** `withdraw` → shared per-epoch settlement → [`Contract::payout_user_withdraw`] (claim bucket + transfer).
//!
//! **Non-WASM:** `testing_env!` skips promise chains; [`Contract::payout_user_withdraw`] runs directly from [`Contract::withdraw`].

use crate::*;
use near_sdk::{AccountId, NearToken, Promise, env, near, require};

// ---------------------------------------------------------------------------
// Public `withdraw` + private promise callback (WASM continuation)
// ---------------------------------------------------------------------------

#[near]
impl Contract {
    /// Claim NEAR from `pending_to_withdraw` for this account on `validator_id` (epoch-eligible tranches)
    /// and **transfer it immediately** to the caller.
    ///
    /// **WASM:** Uses [`crate::epoch::Contract::promise_validator_per_epoch_settlement_then`] so balance
    /// refresh, withdraw-if-ready, and net settle run before the payout (same ordering as **`lock`** /
    /// **`unlock`**).
    ///
    /// **Non-WASM:** Runs [`Contract::payout_user_withdraw`] only (host `testing_env!` does not execute pool promise chains;
    /// see `lock.rs`).
    #[payable]
    pub fn withdraw(&mut self, validator_id: ValidatorId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let account_id = env::predecessor_account_id();
        self.ensure_min_base_storage(&account_id);

        let account_validator_key = (account_id.clone(), validator_id.clone());
        // Must have tranches from a prior `commit_share_exit` / unlock on this pool.
        let user_pending_tranches_yocto =
            self.sum_user_unstake_tranches_yocto(&account_validator_key);
        require!(
            user_pending_tranches_yocto > 0,
            "You have no unlocked NEAR waiting to claim for this validator"
        );

        let mut validator = self.require_validator(&validator_id);
        self.assert_validator_idle_for_user_action(&validator);
        // `accounts_with_pending_unstake` is the validator-side index used by epoch / withdraw scheduling;
        // ensure this account is listed whenever they still carry tranches (idempotent if already present).
        if !validator
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
            return self.payout_user_withdraw(account_id, validator_id);
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
}

// ---------------------------------------------------------------------------
// Tranche math, bucket claim (no transfer), and payout orchestration
// ---------------------------------------------------------------------------

impl Contract {
    /// Total yocto across **all** tranches for `(account, validator)` (includes not-yet-claimable epochs).
    fn sum_user_unstake_tranches_yocto(
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

    /// Yocto the user may claim at `at_epoch`: tranches with `available_epoch_height <= at_epoch`.
    fn sum_claimable_user_tranches_yocto(
        &self,
        account_validator_key: &(AccountId, ValidatorId),
        at_epoch: u64,
    ) -> u128 {
        self.user_pending_unstake
            .get(account_validator_key)
            .map(|tranches| {
                tranches
                    .iter()
                    .filter(|tranche| at_epoch >= tranche.available_epoch_height)
                    .map(|tranche| tranche.amount.as_yoctonear())
                    .fold(0u128, |sum, yocto| sum.saturating_add(yocto))
            })
            .unwrap_or(0)
    }

    /// Applies `deduct_yocto` against **claimable-at-`at_epoch`** tranches in vector order (FIFO among
    /// eligible rows). Panics if eligible tranches cannot cover `deduct_yocto` (caller must size the claim).
    ///
    /// Returns `true` when the `(account, validator)` tranche list is now empty (remove from worklists).
    fn deduct_claim_from_user_tranches_fifo(
        &mut self,
        account_validator_key: &(AccountId, ValidatorId),
        at_epoch: u64,
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
            if at_epoch < tranche.available_epoch_height {
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

    /// Moves up to `min(claimable user tranches at current epoch, pool withdraw bucket)` from
    /// [`Validator::pending_to_withdraw`] into this user’s hands **as a [`NearToken`] return value**:
    /// updates tranches, `pending_user_unstake_total`, and emits `log_withdraw`. Does **not** attach
    /// `Promise::transfer` — use [`Contract::payout_user_withdraw`] for the full user-facing payout.
    pub(crate) fn claim_from_withdraw_bucket(
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
        let eligible_yocto = self.sum_claimable_user_tranches_yocto(&account_validator_key, eh);
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
            .expect("pending_to_withdraw accounting mismatch; contact the contract maintainers");
        validator.pending_user_unstake_total = validator
            .pending_user_unstake_total
            .checked_sub(NearToken::from_yoctonear(credit_yocto))
            .expect(
                "pending_user_unstake_total accounting mismatch; contact the contract maintainers",
            );

        let user_done =
            self.deduct_claim_from_user_tranches_fifo(&account_validator_key, eh, credit_yocto);
        if user_done {
            validator
                .accounts_with_pending_unstake
                .retain(|a| *a != account_id);
        }

        // Defensive second check: FIFO path should already clear the worklist when tranches vanish,
        // but retain here if storage was cleared without `user_done` (should not happen in normal flows).
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

    /// Claim from `pending_to_withdraw` and transfer to the user. Pool → contract withdraw runs in the
    /// shared per-epoch settlement chain before this ([`crate::epoch::Contract::promise_validator_per_epoch_settlement_then`]).
    ///
    /// Called from [`Contract::withdraw`] (non-WASM / tests) and from [`crate::epoch::Contract::on_epoch_settlement_dispatch_continue`] on WASM.
    pub(crate) fn payout_user_withdraw(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        let credit = self.claim_from_withdraw_bucket(account_id.clone(), validator_id);
        Promise::new(account_id).transfer(credit)
    }
}
