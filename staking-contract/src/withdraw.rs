//! User exit payouts: drop every epoch-eligible [`PendingUnstakeTranche`], pay their combined amount
//! from [`Validator::pending_to_withdraw`], then transfer to the user.
//!
//! **Flow (WASM):** `withdraw` → shared per-epoch settlement → [`Contract::payout_user_withdraw`] (claim bucket + transfer).
//!
//! **Non-WASM:** `testing_env!` skips promise chains; [`Contract::payout_user_withdraw`] runs directly from [`Contract::withdraw`].

use crate::*;
use near_sdk::{AccountId, NearToken, Promise, env, near, require};

// Public `withdraw` + private promise callback (WASM continuation).

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
        // Must have tranches from a prior `internal_unstake` / unlock on this pool.
        let user_pending_tranches_yocto =
            self.sum_user_unstake_tranches_yocto(&account_validator_key);
        require!(
            user_pending_tranches_yocto > 0,
            "You have no unlocked NEAR waiting to claim for this validator"
        );

        let mut validator = self.require_validator_idle(&validator_id);
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
            self.require_enough_gas_for_epoch_settlement();
            self.promise_validator_per_epoch_settlement_then(
                validator_id.clone(),
                PerEpochContinue::WithdrawUserTransfer {
                    validator_id,
                    account_id,
                },
            )
        }
    }

    /// **[Pipeline 5c]** User transfer tail after shared pre-user settlement (**0–4**).
    #[private]
    pub fn on_withdraw_user_transfer_after_settle(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        self.payout_user_withdraw(account_id, validator_id)
    }
}

// Tranche math, bucket claim (no transfer), and payout orchestration.

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

    /// Removes every tranche with `available_epoch_height <= at_epoch` and returns their total yocto.
    /// Non-claimable tranches are kept. Returns `true` when no tranches remain.
    fn remove_claimable_tranches(
        &mut self,
        account_validator_key: &(AccountId, ValidatorId),
        at_epoch: u64,
    ) -> (u128, bool) {
        let mut tranches = self
            .user_pending_unstake
            .get(account_validator_key)
            .cloned()
            .unwrap_or_default();
        let claimable_yocto = tranches
            .iter()
            .filter(|tranche| at_epoch >= tranche.available_epoch_height)
            .map(|tranche| tranche.amount.as_yoctonear())
            .fold(0u128, |sum, yocto| sum.saturating_add(yocto));
        tranches.retain(|tranche| at_epoch < tranche.available_epoch_height);
        if tranches.is_empty() {
            self.user_pending_unstake.remove(account_validator_key);
            (claimable_yocto, true)
        } else {
            self.user_pending_unstake
                .insert(account_validator_key.clone(), tranches);
            (claimable_yocto, false)
        }
    }

    /// Drops all epoch-eligible tranches, debits the withdraw bucket by their sum, and returns that NEAR.
    /// Use [`Contract::payout_user_withdraw`] to attach the transfer promise.
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
        let (credit_yocto, user_done) = self.remove_claimable_tranches(&account_validator_key, eh);
        require!(
            credit_yocto > 0,
            "Nothing to claim yet: wait until `epoch_height >=` your tranche's available epoch height"
        );
        require!(
            pending_withdraw_bucket_yocto >= credit_yocto,
            "Withdraw bucket cannot cover all claimable tranches yet; call withdraw again after more pool funds arrive"
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

        if user_done {
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
