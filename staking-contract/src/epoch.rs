//! Validator pool pipeline: refresh cached balances, pull ready unstaked NEAR from pools into this contract,
//! net pending stake vs unstake, and perform at most one successful pool **`deposit_and_stake`** or **`unstake`**
//! per NEAR `epoch_height` per pool ([`Validator::last_settlement_epoch`]). See `docs/LAZY_EPOCH_PIPELINE.md`.
//!
//! ## Who calls what
//! - **Catalog `lock` / `unlock` / user `withdraw` (WASM):** share [`Contract::promise_validator_per_epoch_settlement_then`]
//!   so balance sync, withdraw-if-ready, and net settle run in a consistent order before user-visible tails
//!   (`lock.rs`, `unlock.rs`, `withdraw.rs`).
//! - **Anyone:** [`Contract::epoch_settle`] retries or advances [`Contract::try_epoch_settle`] for one pool under the same rules.
//!
//! ## Pre-user per-epoch settlement (`promise_validator_per_epoch_settlement_then`)
//! When [`Validator::last_settlement_epoch`] is **before** the current NEAR epoch, the contract runs:
//! 1. [`Contract::promise_get_validator_total_staked_balance`] → [`Contract::on_epoch_settlement_after_total_balance`]
//!    (updates [`Validator::total_staked_balance`]).
//! 2. [`Contract::promise_get_validator_unstaked_then_continue_settlement`] → [`Contract::on_epoch_settlement_after_unstaked`]
//!    (may call [`Contract::try_epoch_withdraw`] to top up [`Validator::pending_to_withdraw`], then continues).
//! 3. [`Contract::try_validator_settle_if_due_or_dispatch_continue`] → either [`Contract::try_epoch_settle`] (pool call) or an
//!    immediate hop to [`Contract::on_epoch_settlement_dispatch_continue`].
//! When the pool **already** settled this epoch, steps 1–3 are skipped and [`Contract::on_epoch_settlement_dispatch_continue`]
//! runs directly with the caller’s [`PerEpochContinue`] payload (cached **`total_staked_balance`** is authoritative).
//!
//! **Unlock / unstake ordering:** when a new pool `unstake` starts, if a prior round’s unstaked NEAR is still
//! withdrawable after the configured settle delay, [`Contract::try_epoch_withdraw`] runs first (same doc, section 2b).
//!
//! ## Net settle
//! [`Contract::try_epoch_settle`] compares `pending_to_stake` vs `pending_to_unstake` in yocto: stake the excess,
//! unstake the deficit, clear both without a pool call when equal (net-zero path uses [`Contract::apply_net_zero_pending_matched_clear`]),
//! or panic when nothing is queued.
//!
//! ## File layout
//! - **This file:** `ExtSelfEpoch` / `ExtStakingPool`, [`Contract::epoch_settle`], `pub(crate)` orchestration,
//!   and `#[private]` settlement / pool callbacks.
//! - **`lock.rs`:** `on_lock_finally_mint_and_maybe_post_settle`.
//! - **`unlock.rs`:** `on_unlock_tail_after_pre_user_settle`, `on_unstake_pipeline_unstaked_balance`,
//!   `on_after_withdraw_then_unstake`.
//! - **`withdraw.rs`:** `on_user_withdraw_payout_continue`.

use crate::events;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::{U64, U128};
use near_sdk::{
    AccountId, NearToken, Promise, PromiseOrValue, env, is_promise_success, near, require,
};

// =============================================================================
// External interfaces (cross-contract)
// =============================================================================

/// Self callbacks for epoch / pool work. Names are **stable promise targets** (`#[private]`); rename only
/// with a coordinated ABI / off-chain migration.
#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    /// Pool `deposit_and_stake` result: updates pending queues, bumps `last_settlement_epoch` on success.
    /// Returns whether the pool call succeeded (boolean tail for some callers).
    fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool;
    /// Pool `unstake` result: records `last_unstake_epoch`, adjusts pending queues on success.
    fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool;
    /// Internal hop: net-zero stake/unstake pending cleared without pool `deposit` / `unstake`.
    fn on_settle_net_zero_done(&mut self, validator_id: ValidatorId, matched_pending_yocto: U128);
    /// Epoch withdraw helper: after `get_account_unstaked_balance`, optionally `withdraw` everything
    /// spendable into [`Validator::pending_to_withdraw`].
    fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool>;
    /// Credits `pending_to_withdraw` after a successful pool `withdraw` transfer (or clears `Busy` on failure).
    fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool>;
    /// After pool withdraw completes during unlock pipeline: continue with pool unstake if needed.
    fn on_after_withdraw_then_unstake(&mut self, validator_id: ValidatorId)
    -> PromiseOrValue<bool>;
    // --- Shared pre-user settlement chain (see module doc) ---
    /// Step 1: total-balance callback; persists [`Validator::total_staked_balance`] then chains unstaked probe.
    fn on_epoch_settlement_after_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// Step 2: unstaked-balance callback; may run [`Contract::try_epoch_withdraw`] before net settle.
    fn on_epoch_settlement_after_unstaked(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// Step 3: after optional withdraw-during-settlement hop; resumes [`Contract::try_validator_settle_if_due_or_dispatch_continue`].
    fn on_epoch_settlement_after_withdraw_chain(
        &mut self,
        #[callback] _prior: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// Step 4: after [`Contract::try_epoch_settle`] from the settlement chain; forwards to dispatch (pool
    /// success/failure is reflected in validator state via earlier callbacks, not `_settle_ok`).
    fn on_epoch_settlement_after_try_epoch_settle(
        &mut self,
        #[callback] _settle_ok: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// Tail: run catalog mint or unlock queue after shared settlement steps.
    fn on_epoch_settlement_dispatch_continue(&mut self, cont: PerEpochContinue) -> Promise;
    /// Mint lock and run [`Contract::try_epoch_settle`] if the epoch slot is still free.
    fn on_lock_finally_mint_and_maybe_post_settle(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> PromiseOrValue<()>;
    /// After shared settlement: set pool `Busy`, queue unlock shares, unstake pipeline.
    fn on_unlock_tail_after_pre_user_settle(
        &mut self,
        lock_id: LockId,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> Promise;
    /// Private callback: resumes [`Contract::payout_user_withdraw`] after shared per-epoch settlement.
    fn on_user_withdraw_payout_continue(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise;
    /// After querying unstaked balance: withdraw first if needed, else unstake.
    fn on_unstake_pipeline_unstaked_balance(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool>;
}

#[ext_contract(ext_staking_pool)]
pub trait ExtStakingPool {
    /// Staking-pool view: on-chain source of truth for who may operate the pool off this contract.
    fn get_owner_id(&self) -> AccountId;
    fn deposit_and_stake(&mut self);
    fn unstake(&mut self, amount: NearToken);
    /// Unstaked balance available to transfer to this contract (query).
    fn get_account_unstaked_balance(&self, account_id: AccountId) -> NearToken;
    /// Move unstaked NEAR from the pool to `account_id` (this contract).
    fn withdraw(&mut self, amount: NearToken);
    fn get_account_total_balance(&self, account_id: AccountId) -> NearToken;
}

// =============================================================================
// Public epoch API
// =============================================================================

#[near]
impl Contract {
    /// Public entry: run [`Contract::try_epoch_settle`] for one allowlisted pool (manual retry or advance
    /// pending stake/unstake when automatic promises did not complete). Same per-epoch mutex as automatic flows.
    pub fn epoch_settle(&mut self, validator_id: ValidatorId) -> Promise {
        self.assert_not_paused();
        self.try_epoch_settle(validator_id, false)
    }
}

// =============================================================================
// Internal epoch orchestration (`pub(crate)`)
// =============================================================================

impl Contract {
    // ---------------------------------------------------------------------------
    // Pool view calls (staking pool → `#[callback]` on this contract)
    // ---------------------------------------------------------------------------

    /// Issues `get_account_total_balance` for this contract on `validator_id` (the pool account).
    /// The callback persists the result into [`Validator::total_staked_balance`] (see settlement step 1).
    pub(crate) fn promise_get_validator_total_staked_balance(
        &self,
        validator_id: ValidatorId,
    ) -> Promise {
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_ACCOUNT_TOTAL_BALANCE)
            .get_account_total_balance(env::current_account_id())
    }

    /// Shared per-epoch pipeline for a validator row before catalog **`lock`**, **`unlock`**, or user
    /// **`withdraw`**:
    /// When [`Validator::last_settlement_epoch`] is **before** the current NEAR epoch: (1)
    /// [`Contract::promise_get_validator_total_staked_balance`], (2) withdraw ready unstaked NEAR from the pool if allowed
    /// ([`Contract::promise_get_validator_unstaked_then_continue_settlement`]), (3) net settle via
    /// [`Contract::try_validator_settle_if_due_or_dispatch_continue`] / [`Contract::try_epoch_settle`], then dispatch **`cont`**.
    /// When the pool **already** settled this epoch (`last_settlement_epoch` ≥ current height), **skip** that
    /// pre-user pipeline and run [`Contract::on_epoch_settlement_dispatch_continue`] immediately (mint / unlock /
    /// withdraw payout uses cached **`total_staked_balance`**). Callers must ensure the pool row is **`Idle`**.
    pub(crate) fn promise_validator_per_epoch_settlement_then(
        &self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        // Fast path when this pool already consumed its one settle slot for the current NEAR epoch.
        let needs_pre_user_settlement_pipeline = self
            .validators
            .get(&validator_id)
            .map(|row| row.last_settlement_epoch < env::epoch_height())
            .unwrap_or(true);
        if !needs_pre_user_settlement_pipeline {
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }
        self.promise_get_validator_total_staked_balance(validator_id.clone())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_TOTAL_BALANCE)
                    .on_epoch_settlement_after_total_balance(validator_id, cont),
            )
    }

    /// Settlement step 2: query pool unstaked balance, then resume in [`Contract::on_epoch_settlement_after_unstaked`]
    /// (withdraw-if-ready, then [`Contract::try_validator_settle_if_due_or_dispatch_continue`]).
    pub(crate) fn promise_get_validator_unstaked_then_continue_settlement(
        &self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_UNSTAKED_BALANCE)
            .get_account_unstaked_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_UNSTAKED)
                    .on_epoch_settlement_after_unstaked(validator_id, cont),
            )
    }

    /// Clears equal `pending_to_stake` / `pending_to_unstake` without a pool call when both match
    /// `matched_pending_yocto` (net-zero queue). Re-roots `pending_to_unstake` at [`Validator::pending_user_unstake_total`]
    /// so user exit liability continues to drive the next unstake round. Shared by [`Contract::on_settle_net_zero_done`]
    /// and the inline net-zero branch inside [`Contract::try_validator_settle_if_due_or_dispatch_continue`].
    pub(crate) fn apply_net_zero_pending_matched_clear(
        &mut self,
        validator_id: &ValidatorId,
        matched_pending_yocto: u128,
    ) {
        let mut validator = self.require_validator_callback(validator_id);
        require!(
            validator.pending_to_stake.as_yoctonear() == matched_pending_yocto
                && validator.pending_to_unstake.as_yoctonear() == matched_pending_yocto,
            "Net zero settle state changed before callback; retry next epoch"
        );
        validator.pending_to_stake = NearToken::from_near(0);
        // User tranches are unchanged; re-queue pool unstake for remaining user exit liability.
        validator.pending_to_unstake = validator.pending_user_unstake_total;
        validator.last_settlement_epoch = env::epoch_height();
        validator.tx_status = TransactionStatus::Idle;
        self.validators.insert(validator_id.clone(), validator);
    }

    // ---------------------------------------------------------------------------
    // Net stake / unstake (at most one mutating pool op per NEAR epoch)
    // ---------------------------------------------------------------------------

    /// Net pool settlement: at most one successful **`deposit_and_stake`** or **`unstake`** per NEAR
    /// `epoch_height` per pool ([`crate::validators::Validator::last_settlement_epoch`]). Compares
    /// `pending_to_stake` vs `pending_to_unstake` in yocto.
    pub(crate) fn try_epoch_settle(
        &mut self,
        validator_id: ValidatorId,
        validator_already_busy: bool,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        // `validator_already_busy`: unlock / withdraw paths may already hold `Busy` before chaining settle.
        if validator_already_busy {
            require!(
                validator.tx_status == TransactionStatus::Busy,
                "Validator pool must be busy for this settle step"
            );
        } else {
            require!(
                validator.tx_status == TransactionStatus::Idle,
                "Validator pool is busy; wait for the in-flight pool call to finish"
            );
        }
        require!(
            validator.last_settlement_epoch < env::epoch_height(),
            "This pool already completed a stake or unstake this epoch; try again next epoch"
        );

        let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
        let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();

        // Nothing queued → caller error (epoch_settle) or upstream should not route here.
        if pending_stake_yocto == 0 && pending_unstake_yocto == 0 {
            if validator_already_busy {
                validator.tx_status = TransactionStatus::Idle;
                self.validators.insert(validator_id, validator);
            }
            env::panic_str("Nothing is queued to settle for this validator");
        }

        // Equal pending → net-zero clear via self-call (or async callback when not already busy).
        if pending_stake_yocto == pending_unstake_yocto && pending_stake_yocto > 0 {
            events::log_epoch_operation("epoch_settle_net_zero", &validator_id);
            if !validator_already_busy {
                validator.tx_status = TransactionStatus::Busy;
            }
            self.validators.insert(validator_id.clone(), validator);
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_SETTLE_NET_ZERO)
                .on_settle_net_zero_done(validator_id, U128(pending_stake_yocto));
        }

        // Excess stake → deposit_and_stake(net), absorbing matched unstake pending in callback args.
        if pending_stake_yocto > pending_unstake_yocto {
            let net = NearToken::from_yoctonear(pending_stake_yocto - pending_unstake_yocto);
            require!(
                net.as_yoctonear() > 0,
                "Net stake amount is zero after netting pending unstake"
            );
            events::log_epoch_operation("epoch_settle_stake", &validator_id);
            if !validator_already_busy {
                validator.tx_status = TransactionStatus::Busy;
            }
            self.validators.insert(validator_id.clone(), validator);
            return ext_staking_pool::ext(validator_id.clone())
                .with_static_gas(staking_pool::DEPOSIT_AND_STAKE)
                .with_attached_deposit(net)
                .deposit_and_stake()
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_DEPOSIT_AND_STAKE)
                        .on_deposit_and_stake(validator_id, net, U128(pending_unstake_yocto)),
                );
        }

        // Excess unstake → pool `unstake(net)` (subject to prior-unstake settle delay above).
        if validator.last_unstake_epoch > 0 {
            require!(
                self.validator_unstake_waiting_finished(&validator),
                "Wait until the previous unstake has finished its settle period before unstaking again"
            );
        }
        let net = NearToken::from_yoctonear(pending_unstake_yocto - pending_stake_yocto);
        require!(
            net.as_yoctonear() > 0,
            "Net unstake amount is zero after netting pending stake"
        );
        events::log_epoch_operation("epoch_settle_unstake", &validator_id);
        if !validator_already_busy {
            validator.tx_status = TransactionStatus::Busy;
            self.validators.insert(validator_id.clone(), validator);
        }

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::UNSTAKE)
            .unstake(net)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_UNSTAKE)
                    .on_unstake(validator_id, net, U128(pending_stake_yocto)),
            )
    }

    // ---------------------------------------------------------------------------
    // Pool → contract withdraw (fills `pending_to_withdraw` for user claims)
    // ---------------------------------------------------------------------------

    /// Pull unstaked NEAR from the staking pool into this contract’s [`Validator::pending_to_withdraw`].
    /// When `validator_already_busy` is true, the row must already be `Busy` (unlock pipeline); it is not flipped to `Busy` again.
    pub(crate) fn try_epoch_withdraw(
        &mut self,
        validator_id: ValidatorId,
        validator_already_busy: bool,
    ) -> Promise {
        events::log_epoch_operation("epoch_withdraw", &validator_id);

        let mut validator = self.require_validator(&validator_id);
        require!(
            self.validator_unstake_waiting_finished(&validator),
            "Wait until enough epochs have passed after the last unstake before withdrawing"
        );

        if validator_already_busy {
            require!(
                validator.tx_status == TransactionStatus::Busy,
                "Validator pool must be busy for this withdraw step"
            );
        } else {
            require!(
                validator.tx_status == TransactionStatus::Idle,
                "Validator pool is busy; wait for the in-flight pool call to finish"
            );
            validator.tx_status = TransactionStatus::Busy;
            self.validators.insert(validator_id.clone(), validator);
        }

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_UNSTAKED_BALANCE)
            .get_account_unstaked_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                    .on_get_unstaked_for_epoch_withdraw(validator_id),
            )
    }

    /// Unlock tail: after `commit_share_exit`, query unstaked balance then run the withdraw-first unstake
    /// pipeline ([`Contract::on_unstake_pipeline_unstaked_balance`] in `unlock.rs`).
    pub(crate) fn promise_post_unlock_unstaked_pipeline(
        &mut self,
        validator_id: ValidatorId,
    ) -> Promise {
        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_UNSTAKED_BALANCE)
            .get_account_unstaked_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                    .on_unstake_pipeline_unstaked_balance(validator_id),
            )
    }

    /// End of the **pre-user** balance/withdraw phase: either there is nothing left to net-settle this epoch,
    /// or we inline net-zero, or we attach [`Contract::try_epoch_settle`] and resume at
    /// [`Contract::on_epoch_settlement_after_try_epoch_settle`] before [`Contract::on_epoch_settlement_dispatch_continue`].
    pub(crate) fn try_validator_settle_if_due_or_dispatch_continue(
        &mut self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
        let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();
        let has_pending = pending_stake_yocto > 0 || pending_unstake_yocto > 0;
        let can_settle = validator.last_settlement_epoch < env::epoch_height();

        // No pending deltas, or epoch slot already consumed elsewhere → jump to catalog / unlock / withdraw tail.
        if !has_pending || !can_settle {
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }

        // Equal non-zero pending → clear synchronously (no pool `deposit` / `unstake`), then dispatch.
        if pending_stake_yocto == pending_unstake_yocto && pending_stake_yocto > 0 {
            events::log_epoch_operation("epoch_settle_net_zero", &validator_id);
            require!(
                validator.tx_status == TransactionStatus::Idle,
                "Validator pool is busy; wait for the in-flight pool call to finish"
            );
            validator.tx_status = TransactionStatus::Busy;
            self.validators.insert(validator_id.clone(), validator);
            self.apply_net_zero_pending_matched_clear(&validator_id, pending_stake_yocto);
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }

        // Asymmetric pending → one pool mutating call for this epoch (`try_epoch_settle` enforces the mutex).
        self.try_epoch_settle(validator_id.clone(), false).then(
            ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_TRY_EPOCH_SETTLE)
                .on_epoch_settlement_after_try_epoch_settle(validator_id, cont),
        )
    }
}

// =============================================================================
// Private epoch promise callbacks (settlement + pool stake/unstake/withdraw)
// =============================================================================
// Bodies are `#[private]` promise targets; keep names stable unless migrating the ABI.

#[near]
impl Contract {
    #[private]
    /// Settlement step 1 callback: persist refreshed pool total, then [`Contract::promise_get_validator_unstaked_then_continue_settlement`].
    pub fn on_epoch_settlement_after_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        if !is_promise_success() {
            env::panic_str(
                "Could not refresh validator balance from the pool; retry in a few blocks",
            );
        }
        let mut validator = self.require_validator(&validator_id);
        // TODO: should we panic here if validator is busy
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        validator.total_staked_balance = total_balance;
        validator.last_balance_refresh_ns = U64(env::block_timestamp());
        self.validators.insert(validator_id.clone(), validator);

        self.promise_get_validator_unstaked_then_continue_settlement(validator_id, cont)
    }

    #[private]
    /// Settlement step 2 callback: optionally [`Contract::try_epoch_withdraw`], else jump to
    /// [`Contract::try_validator_settle_if_due_or_dispatch_continue`].
    pub fn on_epoch_settlement_after_unstaked(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        if !is_promise_success() {
            env::panic_str("Could not read unstaked balance from the pool; retry in a few blocks");
        }

        let validator = self.require_validator(&validator_id);
        // Pool may hold spendable unstaked NEAR after a prior unstake; pull it in before net settle when allowed.
        let can_withdraw = self.validator_unstake_waiting_finished(&validator);

        if unstaked_balance.as_yoctonear() > 0 && can_withdraw {
            return self
                .try_epoch_withdraw(validator_id.clone(), false)
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                        .on_after_withdraw_then_unstake(validator_id.clone()),
                )
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_WITHDRAW_CHAIN)
                        .on_epoch_settlement_after_withdraw_chain(validator_id, cont),
                );
        }

        self.try_validator_settle_if_due_or_dispatch_continue(validator_id, cont)
    }

    #[private]
    /// Settlement step 3 callback: `_prior` is the bool tail from unlock’s withdraw→unstake hop; settlement
    /// only needs ordering, not the value.
    pub fn on_epoch_settlement_after_withdraw_chain(
        &mut self,
        #[callback] _prior: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        self.try_validator_settle_if_due_or_dispatch_continue(validator_id, cont)
    }

    #[private]
    /// Settlement step 4 callback: pool outcome is already folded into `validator` by `on_deposit_and_stake` /
    /// `on_unstake`; this hop only forwards to [`Contract::on_epoch_settlement_dispatch_continue`].
    #[allow(unused_variables)]
    pub fn on_epoch_settlement_after_try_epoch_settle(
        &mut self,
        #[callback] _settle_ok: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        ext_self_epoch::ext(env::current_account_id())
            .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
            .on_epoch_settlement_dispatch_continue(cont)
    }

    #[private]
    /// Fan-out: run the catalog mint tail, unlock unstake tail, or user withdraw payout continuation.
    pub fn on_epoch_settlement_dispatch_continue(&mut self, cont: PerEpochContinue) -> Promise {
        match cont {
            // Catalog purchase: mint shares / usage after pool row is fresh for this epoch.
            PerEpochContinue::CatalogLockMint {
                validator_id,
                buyer,
                locked,
                duration_ns,
                order,
                subscription_followup,
            } => ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_LOCK_FINALLY_MINT)
                .on_lock_finally_mint_and_maybe_post_settle(
                    buyer,
                    locked,
                    duration_ns,
                    order,
                    validator_id,
                    subscription_followup,
                ),
            // Share exit: burn shares and drive pool `unstake` / withdraw pipeline for this lock.
            PerEpochContinue::UnlockQueueUnstake {
                lock_id,
                account_id,
                validator_id,
                shares_remove,
            } => ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_UNLOCK_TAIL_AFTER_PRE_USER)
                .on_unlock_tail_after_pre_user_settle(
                    lock_id,
                    account_id,
                    validator_id,
                    shares_remove,
                ),
            // User claim: move unlocked NEAR from `pending_to_withdraw` (see `withdraw.rs`).
            PerEpochContinue::WithdrawUserTransfer {
                account_id,
                validator_id,
            } => ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_WITHDRAW_USER_AFTER_EPOCH_SETTLEMENT)
                .on_user_withdraw_payout_continue(account_id, validator_id),
        }
    }

    #[private]
    /// Async half of net-zero settle from [`Contract::try_epoch_settle`] when the row was marked `Busy` first.
    pub fn on_settle_net_zero_done(
        &mut self,
        validator_id: ValidatorId,
        matched_pending_yocto: U128,
    ) {
        let g = matched_pending_yocto.0;
        self.apply_net_zero_pending_matched_clear(&validator_id, g);
    }

    #[private]
    /// `deposit_and_stake` callback: subtracts matched pending, updates cached stake, clears `Busy`.
    pub fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator_callback(&validator_id);

        validator.tx_status = TransactionStatus::Idle;

        if ok {
            let stake_y = amount.as_yoctonear();
            let absorb_y = absorb_unstake_yocto.0;
            let pend = validator.pending_to_stake.as_yoctonear();
            let pu = validator.pending_to_unstake.as_yoctonear();
            require!(
                pend >= stake_y.saturating_add(absorb_y),
                "After deposit: pending_to_stake underflow vs callback (contract accounting error)"
            );
            require!(
                pu >= absorb_y,
                "After deposit: pending_to_unstake underflow vs absorb (contract accounting error)"
            );
            validator.pending_to_stake =
                NearToken::from_yoctonear(pend.saturating_sub(stake_y).saturating_sub(absorb_y));
            validator.pending_to_unstake = NearToken::from_yoctonear(pu.saturating_sub(absorb_y));
            if validator.pending_to_unstake.as_yoctonear() == 0
                && validator.pending_user_unstake_total.as_yoctonear() > 0
            {
                validator.pending_to_unstake = validator.pending_user_unstake_total;
            }
            validator.total_staked_balance = validator
                .total_staked_balance
                .checked_add(NearToken::from_yoctonear(stake_y))
                .expect("total_staked_balance overflow after stake");
            validator.last_settlement_epoch = env::epoch_height();
        }
        self.validators.insert(validator_id, validator);
        ok
    }

    #[private]
    /// `unstake` callback: records `last_unstake_epoch`, subtracts matched pending, clears `Busy`.
    pub fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator_callback(&validator_id);
        validator.tx_status = TransactionStatus::Idle;
        if ok {
            let eh = env::epoch_height();
            validator.last_unstake_epoch = eh;
            validator.last_settlement_epoch = eh;
            let unstake_y = amount.as_yoctonear();
            let absorb_s = absorb_stake_yocto.0;
            let pend = validator.pending_to_stake.as_yoctonear();
            let pu = validator.pending_to_unstake.as_yoctonear();
            require!(
                pu >= unstake_y.saturating_add(absorb_s),
                "After unstake: pending_to_unstake underflow vs callback (contract accounting error)"
            );
            require!(
                pend >= absorb_s,
                "After unstake: pending_to_stake underflow vs absorb (contract accounting error)"
            );
            validator.pending_to_unstake =
                NearToken::from_yoctonear(pu.saturating_sub(unstake_y).saturating_sub(absorb_s));
            validator.pending_to_stake = NearToken::from_yoctonear(pend.saturating_sub(absorb_s));
        }
        self.validators.insert(validator_id, validator);
        ok
    }

    #[private]
    /// After unstaked-balance view during [`Contract::try_epoch_withdraw`]: zero → clear `Busy`; else pool `withdraw` all.
    pub fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool> {
        if !is_promise_success() {
            let mut validator = self.require_validator_callback(&validator_id);
            validator.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, validator);
            return PromiseOrValue::Value(false);
        }

        if unstaked_balance.as_yoctonear() == 0 {
            let mut validator = self.require_validator_callback(&validator_id);
            validator.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, validator);
            return PromiseOrValue::Value(true);
        }

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::WITHDRAW)
            .withdraw(unstaked_balance)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_WITHDRAW_TRANSFER)
                    .on_epoch_withdraw_transfer_done(validator_id, unstaked_balance),
            )
            .into()
    }

    #[private]
    /// After pool `withdraw` FT transfer: credit [`Validator::pending_to_withdraw`], shrink cached pool balance.
    pub fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool> {
        let ok = is_promise_success();
        let mut validator = self.require_validator_callback(&validator_id);
        validator.tx_status = TransactionStatus::Idle;
        let credited_yocto = if ok { withdrawn.as_yoctonear() } else { 0 };
        if ok && credited_yocto > 0 {
            let add = NearToken::from_yoctonear(credited_yocto);
            let bal_y = validator.total_staked_balance.as_yoctonear();
            require!(
                bal_y >= credited_yocto,
                "Recorded pool balance is less than the withdrawn amount; retry after the next successful balance refresh from the pool"
            );
            validator.total_staked_balance = NearToken::from_yoctonear(bal_y - credited_yocto);
            require!(
                validator.pending_user_unstake_total.as_yoctonear() > 0,
                "Cannot record this withdraw: no user pending unstake for this pool; try withdraw to refresh accounting, then retry"
            );
            validator.pending_to_withdraw = validator
                .pending_to_withdraw
                .checked_add(add)
                .expect("pending_to_withdraw overflow after pool transfer");
            events::log_validator_withdraw_in(credited_yocto, &validator_id);
        }
        self.validators.insert(validator_id, validator);
        PromiseOrValue::Value(ok)
    }
}
