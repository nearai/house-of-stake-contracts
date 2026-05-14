//! Pool pipeline: stake / unstake / withdraw-from-pool, plus promise callbacks (`on_*`).
//!
//! Most pool work is **`pub(crate) try_*`** and user flows (`lock`, `unlock`, `withdraw`).
//! **`epoch_settle`** is the public manual / retry entry for net settle on one pool.
//! Catalog **`lock`**, **`unlock`**, and (on **WASM**) user **`withdraw`** share
//! [`Contract::promise_validator_per_epoch_settlement_then`] (see `withdraw.rs`). **When**
//! [`Validator::last_settlement_epoch`] is behind the current NEAR epoch, the contract runs balance
//! sync, withdraw-if-ready, then [`Contract::try_epoch_settle`] on existing pending; when the pool already
//! settled this epoch, that pre-user pipeline is skipped and mint / unlock / withdraw payout runs directly (cached
//! **`total_staked_balance`**).
//! See `docs/LAZY_EPOCH_PIPELINE.md`.
//!
//! **Withdraw before new unstake:** when starting a new pool `unstake`, if a prior round’s unstaked NEAR
//! is still withdrawable from the pool after the settle window, **`try_epoch_withdraw`** runs first.
//!
//! **Net settle per NEAR epoch:** [`Contract::try_epoch_settle`] runs at most one successful pool
//! **`deposit_and_stake`** or **`unstake`** per `epoch_height` per pool (see [`crate::validators::Validator::last_settlement_epoch`]).
//! Pending stake and unstake are compared in yocto: stake the excess, unstake the deficit, or clear both
//! without a pool call when equal. Anyone may call [`Contract::epoch_settle`] to run or retry net settle for one pool (same per-epoch rules).

use crate::events;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::{U64, U128};
use near_sdk::{
    AccountId, NearToken, Promise, PromiseOrValue, env, is_promise_success, near, require,
};

#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool;
    fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool;
    /// Internal hop: net-zero stake/unstake pending cleared without pool `deposit` / `unstake`.
    fn on_settle_net_zero_done(&mut self, validator_id: ValidatorId, matched_pending_yocto: U128);
    /// After `get_account_unstaked_balance`; may chain `withdraw` on the pool.
    fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool>;
    fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool>;
    /// After pool withdraw completes during unlock pipeline: continue with pool unstake if needed.
    fn on_after_withdraw_then_unstake(&mut self, validator_id: ValidatorId)
    -> PromiseOrValue<bool>;
    /// After `get_account_total_balance` during shared per-epoch settlement (before lock / unlock).
    fn on_epoch_settlement_after_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// After `get_account_unstaked_balance` during shared settlement: withdraw-if-ready, then net settle.
    fn on_epoch_settlement_after_unstaked(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// After withdraw → `on_after_withdraw_then_unstake` during shared settlement.
    fn on_epoch_settlement_after_withdraw_chain(
        &mut self,
        #[callback] _prior: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// After `try_epoch_settle` during shared settlement (deposit / unstake path).
    fn on_epoch_settlement_after_try_epoch_pool(
        &mut self,
        #[callback] _pool_ok: bool,
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
    /// After shared settlement: user withdraw (batch claim + transfer; may prefetch pool withdraw).
    fn on_withdraw_user_after_epoch_settlement(
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

impl Contract {
    /// Starts `get_account_total_balance` for `validator_id` (read-only; unlock sets `Busy` before calling).
    pub(crate) fn promise_pool_get_total_balance(&self, validator_id: ValidatorId) -> Promise {
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_ACCOUNT_TOTAL_BALANCE)
            .get_account_total_balance(env::current_account_id())
    }

    /// Shared per-epoch pipeline for a validator row before catalog **`lock`**, **`unlock`**, or user
    /// **`withdraw`**:
    /// When [`Validator::last_settlement_epoch`] is **before** the current NEAR epoch: (1)
    /// `get_account_total_balance`, (2) withdraw ready unstaked NEAR from the pool if allowed, (3)
    /// [`Contract::try_epoch_settle`] on existing pending, then **`cont`**. When the pool **already**
    /// settled this epoch (`last_settlement_epoch` ≥ current height), **skip** that pre-user pipeline and
    /// run **`cont`** immediately (mint / unlock / withdraw payout uses cached **`total_staked_balance`**). Callers must
    /// ensure the pool row is **`Idle`**.
    pub(crate) fn promise_validator_per_epoch_settlement_then(
        &self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
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
        self.promise_pool_get_total_balance(validator_id.clone())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_TOTAL_BALANCE)
                    .on_epoch_settlement_after_total_balance(validator_id, cont),
            )
    }

    pub(crate) fn promise_epoch_withdraw_then_try_settle(
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

    /// Removes `yocto` from [`Validator::pending_user_unstake_total`] and matching `user_pending_unstake`
    /// tranches (FIFO over [`Validator::accounts_with_pending_unstake`]). Caller must keep
    /// `pending_to_unstake` consistent with the same reduction.
    pub(crate) fn absorb_unstake_liability_yocto_from_users(
        &mut self,
        validator: &mut Validator,
        validator_id: &ValidatorId,
        mut remaining: u128,
    ) {
        if remaining == 0 {
            return;
        }
        let pending_user_unstake_total_yocto = validator.pending_user_unstake_total.as_yoctonear();
        require!(
            remaining <= pending_user_unstake_total_yocto,
            "Net settle: absorb exceeds pending user unstake liability"
        );
        validator.pending_user_unstake_total =
            NearToken::from_yoctonear(pending_user_unstake_total_yocto.saturating_sub(remaining));

        for account_id in validator.accounts_with_pending_unstake.clone() {
            if remaining == 0 {
                break;
            }
            let account_validator_key = (account_id.clone(), validator_id.clone());
            let Some(mut pending_tranches) = self
                .user_pending_unstake
                .get(&account_validator_key)
                .cloned()
            else {
                continue;
            };
            for tranche in pending_tranches.iter_mut() {
                if remaining == 0 {
                    break;
                }
                let tranche_amount_yocto = tranche.amount.as_yoctonear();
                if tranche_amount_yocto == 0 {
                    continue;
                }
                let take_yocto = tranche_amount_yocto.min(remaining);
                tranche.amount = NearToken::from_yoctonear(tranche_amount_yocto - take_yocto);
                remaining = remaining.saturating_sub(take_yocto);
            }
            pending_tranches.retain(|tranche| tranche.amount.as_yoctonear() > 0);
            if pending_tranches.is_empty() {
                self.user_pending_unstake.remove(&account_validator_key);
            } else {
                self.user_pending_unstake
                    .insert(account_validator_key, pending_tranches);
            }
        }
        require!(
            remaining == 0,
            "Net settle: absorb could not be mapped to user unstake tranches (accounting mismatch)"
        );
        validator
            .accounts_with_pending_unstake
            .retain(|account_id| {
                let account_validator_key = (account_id.clone(), validator_id.clone());
                self.user_pending_unstake
                    .get(&account_validator_key)
                    .map(|tranches| !tranches.is_empty())
                    .unwrap_or(false)
            });
    }

    /// Net-zero pending clear (no pool `deposit` / `unstake`): shared by [`Contract::on_settle_net_zero_done`]
    /// and the catalog-lock pre-user inline path.
    pub(crate) fn apply_validator_net_zero_settle_internal(
        &mut self,
        validator_id: &ValidatorId,
        matched_pending_yocto: u128,
    ) {
        let mut validator = self.require_validator_pool_callback(validator_id);
        require!(
            validator.pending_to_stake.as_yoctonear() == matched_pending_yocto
                && validator.pending_to_unstake.as_yoctonear() == matched_pending_yocto,
            "Net zero settle state changed before callback; retry next epoch"
        );
        self.absorb_unstake_liability_yocto_from_users(
            &mut validator,
            validator_id,
            matched_pending_yocto,
        );
        validator.pending_to_stake = NearToken::from_near(0);
        validator.pending_to_unstake = NearToken::from_near(0);
        validator.last_settlement_epoch = env::epoch_height();
        validator.tx_status = TransactionStatus::Idle;
        self.validators.insert(validator_id.clone(), validator);
    }

    /// Net pool settlement: at most one successful **`deposit_and_stake`** or **`unstake`** per NEAR
    /// `epoch_height` per pool ([`crate::validators::Validator::last_settlement_epoch`]). Compares
    /// `pending_to_stake` vs `pending_to_unstake` in yocto.
    pub(crate) fn try_epoch_settle(
        &mut self,
        validator_id: ValidatorId,
        pool_already_busy: bool,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        if pool_already_busy {
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

        if pending_stake_yocto == 0 && pending_unstake_yocto == 0 {
            if pool_already_busy {
                validator.tx_status = TransactionStatus::Idle;
                self.validators.insert(validator_id, validator);
            }
            env::panic_str("Nothing is queued to settle for this validator");
        }

        if pending_stake_yocto == pending_unstake_yocto && pending_stake_yocto > 0 {
            events::log_epoch_operation("epoch_settle_net_zero", &validator_id);
            if !pool_already_busy {
                validator.tx_status = TransactionStatus::Busy;
            }
            self.validators.insert(validator_id.clone(), validator);
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_SETTLE_NET_ZERO)
                .on_settle_net_zero_done(validator_id, U128(pending_stake_yocto));
        }

        if pending_stake_yocto > pending_unstake_yocto {
            let net = NearToken::from_yoctonear(pending_stake_yocto - pending_unstake_yocto);
            require!(
                net.as_yoctonear() > 0,
                "Net stake amount is zero after netting pending unstake"
            );
            events::log_epoch_operation("epoch_settle_stake", &validator_id);
            if !pool_already_busy {
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

        // pending_unstake_yocto > pending_stake_yocto
        if validator.last_unstake_epoch > 0 {
            let ready_epoch = validator
                .last_unstake_epoch
                .saturating_add(self.config.epoch_unstake_settle_epochs);
            require!(
                env::epoch_height() >= ready_epoch,
                "Wait until the previous unstake has finished its settle period before unstaking again"
            );
        }
        let net = NearToken::from_yoctonear(pending_unstake_yocto - pending_stake_yocto);
        require!(
            net.as_yoctonear() > 0,
            "Net unstake amount is zero after netting pending stake"
        );
        events::log_epoch_operation("epoch_settle_unstake", &validator_id);
        if !pool_already_busy {
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

    /// Pull unstaked NEAR from pool into `pending_to_withdraw`.
    /// When `pool_already_busy` is true, the row must already be `Busy` (unlock pipeline); it is not flipped to `Busy` again.
    pub(crate) fn try_epoch_withdraw(
        &mut self,
        validator_id: ValidatorId,
        pool_already_busy: bool,
    ) -> Promise {
        events::log_epoch_operation("epoch_withdraw", &validator_id);

        let mut validator = self.require_validator(&validator_id);
        require!(
            validator.last_unstake_epoch > 0,
            "No unstake has been recorded for this validator yet; wait until after an unstake completes"
        );
        require!(
            env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs),
            "Wait until enough epochs have passed after the last unstake before withdrawing"
        );

        if pool_already_busy {
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

    /// After an unlock refresh: apply pool balance, then run withdraw-first unstake pipeline (share exit
    /// already committed by `commit_share_exit` in the unlock tail).
    pub(crate) fn promise_unstake_pipeline_after_unlock_queue(
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

    /// Catalog lock: shared per-epoch settlement then mint (see [`Contract::promise_validator_per_epoch_settlement_then`]).
    /// WASM-only: the host triple uses synchronous mint in [`crate::lock`] instead of this promise entrypoint.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn promise_lock_refresh_then_finalize(
        &self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> Promise {
        self.promise_validator_per_epoch_settlement_then(
            validator_id.clone(),
            PerEpochContinue::CatalogLockMint {
                validator_id,
                buyer,
                locked,
                duration_ns,
                order,
                subscription_followup,
            },
        )
    }

    /// Net settle tail for shared per-epoch pipeline (withdraw path already ran when applicable).
    pub(crate) fn finish_validator_epoch_try_settle_tail(
        &mut self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
        let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();
        let has_pending = pending_stake_yocto > 0 || pending_unstake_yocto > 0;
        let can_settle = validator.last_settlement_epoch < env::epoch_height();

        if !has_pending || !can_settle {
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }

        if pending_stake_yocto == pending_unstake_yocto && pending_stake_yocto > 0 {
            events::log_epoch_operation("epoch_settle_net_zero", &validator_id);
            require!(
                validator.tx_status == TransactionStatus::Idle,
                "Validator pool is busy; wait for the in-flight pool call to finish"
            );
            validator.tx_status = TransactionStatus::Busy;
            self.validators.insert(validator_id.clone(), validator);
            self.apply_validator_net_zero_settle_internal(&validator_id, pending_stake_yocto);
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }

        self.try_epoch_settle(validator_id.clone(), false).then(
            ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_TRY_EPOCH_POOL)
                .on_epoch_settlement_after_try_epoch_pool(validator_id, cont),
        )
    }
}

#[near]
impl Contract {
    #[private]
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
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        validator.total_staked_balance = total_balance;
        validator.last_balance_refresh_ns = U64(env::block_timestamp());
        self.validators.insert(validator_id.clone(), validator);

        self.promise_epoch_withdraw_then_try_settle(validator_id, cont)
    }

    #[private]
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
        let can_withdraw = validator.last_unstake_epoch > 0
            && env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs);

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

        self.finish_validator_epoch_try_settle_tail(validator_id, cont)
    }

    #[private]
    pub fn on_epoch_settlement_after_withdraw_chain(
        &mut self,
        #[callback] _prior: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        self.finish_validator_epoch_try_settle_tail(validator_id, cont)
    }

    #[private]
    pub fn on_epoch_settlement_after_try_epoch_pool(
        &mut self,
        #[callback] _pool_ok: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        let _ = validator_id;
        ext_self_epoch::ext(env::current_account_id())
            .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
            .on_epoch_settlement_dispatch_continue(cont)
    }

    #[private]
    pub fn on_epoch_settlement_dispatch_continue(&mut self, cont: PerEpochContinue) -> Promise {
        match cont {
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
            PerEpochContinue::WithdrawUserTransfer {
                account_id,
                validator_id,
            } => ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_WITHDRAW_USER_AFTER_EPOCH_SETTLEMENT)
                .on_withdraw_user_after_epoch_settlement(account_id, validator_id),
        }
    }

    #[private]
    pub fn on_lock_finally_mint_and_maybe_post_settle(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> PromiseOrValue<()> {
        let _lock_id = self.commit_catalog_lock(
            buyer,
            locked,
            duration_ns,
            order,
            validator_id.clone(),
            subscription_followup,
        );
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        let has_p = validator.pending_to_stake.as_yoctonear() > 0
            || validator.pending_to_unstake.as_yoctonear() > 0;
        if has_p && validator.last_settlement_epoch < env::epoch_height() {
            PromiseOrValue::Promise(self.try_epoch_settle(validator_id, false))
        } else {
            PromiseOrValue::Value(())
        }
    }

    #[private]
    pub fn on_unlock_tail_after_pre_user_settle(
        &mut self,
        lock_id: LockId,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        validator.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), validator);

        let mut lock = self
            .locks
            .get(&lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Lock not found"));
        require!(
            lock.account_id == account_id,
            "Unlock no longer matches the lock owner; retry"
        );
        require!(
            lock.status == LockStatus::Active,
            "Lock is no longer active; nothing to unlock"
        );
        require!(lock.validator_id == validator_id, "Lock validator mismatch");
        require!(lock.shares.0 == shares_remove, "Lock shares changed; retry");

        let account_log = lock.account_id.clone();
        let validator_log = lock.validator_id.clone();
        self.commit_share_exit(account_id.clone(), validator_id.clone(), shares_remove);
        lock.status = LockStatus::UnlockRequested;
        self.locks.insert(lock_id.clone(), lock);

        crate::events::log_unlock(lock_id.as_str(), &account_log, &validator_log);

        self.promise_unstake_pipeline_after_unlock_queue(validator_id)
    }

    #[private]
    pub fn on_withdraw_user_after_epoch_settlement(
        &mut self,
        account_id: AccountId,
        validator_id: ValidatorId,
    ) -> Promise {
        self.withdraw_user_transfer_tail(account_id, validator_id)
    }

    #[private]
    pub fn on_unstake_pipeline_unstaked_balance(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool> {
        if !is_promise_success() {
            let mut validator = self.require_validator_pool_callback(&validator_id);
            validator.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, validator);
            env::panic_str("Could not read unstaked balance from the pool; retry in a few blocks");
        }

        let validator = self.require_validator(&validator_id);
        let can_withdraw = validator.last_unstake_epoch > 0
            && env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs);

        if unstaked_balance.as_yoctonear() > 0 && can_withdraw {
            return self
                .try_epoch_withdraw(validator_id.clone(), true)
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                        .on_after_withdraw_then_unstake(validator_id),
                )
                .into();
        }

        let validator = self.require_validator(&validator_id);
        if validator.last_settlement_epoch < env::epoch_height() {
            return self.try_epoch_settle(validator_id, true).into();
        }
        PromiseOrValue::Value(true)
    }

    #[private]
    pub fn on_after_withdraw_then_unstake(
        &mut self,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool> {
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        if validator.pending_to_stake.as_yoctonear() == 0
            && validator.pending_to_unstake.as_yoctonear() == 0
        {
            return PromiseOrValue::Value(true);
        }
        self.try_epoch_settle(validator_id, false).into()
    }

    #[private]
    pub fn on_settle_net_zero_done(
        &mut self,
        validator_id: ValidatorId,
        matched_pending_yocto: U128,
    ) {
        let g = matched_pending_yocto.0;
        self.apply_validator_net_zero_settle_internal(&validator_id, g);
    }

    #[private]
    pub fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator_pool_callback(&validator_id);

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
            if absorb_y > 0 {
                self.absorb_unstake_liability_yocto_from_users(
                    &mut validator,
                    &validator_id,
                    absorb_y,
                );
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
    pub fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator_pool_callback(&validator_id);
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
    pub fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<bool> {
        if !is_promise_success() {
            let mut validator = self.require_validator_pool_callback(&validator_id);
            validator.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, validator);
            return PromiseOrValue::Value(false);
        }

        if unstaked_balance.as_yoctonear() == 0 {
            let mut validator = self.require_validator_pool_callback(&validator_id);
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
    pub fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool> {
        let ok = is_promise_success();
        let mut validator = self.require_validator_pool_callback(&validator_id);
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
            let k = validator.withdraw_batches.len() as u32;
            let accounts_snapshot = validator.accounts_with_pending_unstake.clone();
            let mut l_k = 0u128;
            for uid in &accounts_snapshot {
                let ukey = (uid.clone(), validator_id.clone());
                if let Some(trs) = self.user_pending_unstake.get(&ukey) {
                    for t in trs {
                        if t.min_withdraw_batch_index <= k {
                            l_k = l_k.saturating_add(t.amount.as_yoctonear());
                        }
                    }
                }
            }
            require!(
                l_k > 0,
                "Cannot record this withdraw: no user pending unstake matches this batch; try withdraw to refresh accounting, then retry"
            );
            let l_near = NearToken::from_yoctonear(l_k);
            validator.pending_to_withdraw = validator
                .pending_to_withdraw
                .checked_add(add)
                .expect("pending_to_withdraw overflow after pool transfer");
            validator.withdraw_batches.push(WithdrawBatch {
                remaining: add,
                liability_at_fund: l_near,
            });
            events::log_pool_withdraw_in(credited_yocto, &validator_id);
        }
        self.validators.insert(validator_id, validator);
        PromiseOrValue::Value(ok)
    }

    /// Public entry: run [`Contract::try_epoch_settle`] for one allowlisted pool (manual retry or advance
    /// pending stake/unstake when automatic promises did not complete). Same per-epoch mutex as automatic flows.
    pub fn epoch_settle(&mut self, validator_id: ValidatorId) -> Promise {
        self.assert_not_paused();
        self.try_epoch_settle(validator_id, false)
    }
}
