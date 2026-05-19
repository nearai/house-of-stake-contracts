//! Validator pool pipeline: balance refresh, withdraw-if-ready, net stake/unstake (one mutating pool op per NEAR epoch per pool).
//! Step-by-step promise chain: [`docs/EPOCH_SETTLEMENT_CHAIN.md`](../docs/EPOCH_SETTLEMENT_CHAIN.md). Product goals: `docs/LAZY_EPOCH_PIPELINE.md`.
//!
//! **Entry:** [`Contract::promise_validator_per_epoch_settlement_then`] (**0**, sets **`Busy`**) — shared by `lock`, `unlock`, `withdraw`, `epoch_settle`.
//! Fast path when [`Validator::last_settlement_epoch`] ≥ current epoch: **0** → **4** only.
//! User tails **5a–5c** live in `lock.rs`, `unlock.rs`, `withdraw.rs`; **6** clears **`Idle`**.

use crate::events;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::{U64, U128};
use near_sdk::{
    AccountId, NearToken, Promise, PromiseOrValue, env, is_promise_success, near, require,
};

// =============================================================================
// External interfaces (cross-contract) — trait order follows pipeline steps
// =============================================================================

/// Self callbacks for epoch / pool work. Names are **stable promise targets** (`#[private]`); rename only
/// with a coordinated ABI / off-chain migration.
#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    /// **[Pipeline 1]** After pool `get_account`: refresh balance, optional **2a–2c**, then **3**.
    fn on_epoch_settlement_after_pool_account(
        &mut self,
        #[callback] pool_account: PoolAccountView,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// **[Pipeline 2b]** Credits `pending_to_withdraw` after pool `withdraw` (stays **`Busy`**).
    fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool>;
    /// **[Pipeline 2c]** After pool `withdraw`: tail [`Contract::try_epoch_stake_or_unstake`] (**3**, `None`) or settlement **3** + **4** (`Some(cont)`).
    fn on_after_pool_withdraw_maybe_settle(
        &mut self,
        validator_id: ValidatorId,
        cont: Option<PerEpochContinue>,
    ) -> PromiseOrValue<bool>;
    /// **[Pipeline 3b]** Pool `deposit_and_stake` result: updates pending queues, bumps `last_settlement_epoch`.
    fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool;
    /// **[Pipeline 3c]** Pool `unstake` result: records `last_unstake_epoch`, adjusts pending queues.
    fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool;
    /// **[Pipeline 3′]** After async **3**; forwards to **4** (ignores `_settle_ok` for routing).
    fn on_epoch_settlement_after_try_epoch_stake_or_unstake(
        &mut self,
        #[callback] _settle_ok: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise;
    /// **[Pipeline 4]** Fan-out to user tail (**5a** / **5b** / **5c**), then **6**.
    fn on_epoch_settlement_dispatch_continue(&mut self, cont: PerEpochContinue) -> Promise;
    /// **[Pipeline 5a]** Catalog mint; may re-enter **3** (`lock.rs`).
    fn on_lock_finally_mint_and_maybe_post_settle(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> PromiseOrValue<()>;
    /// **[Pipeline 5d]** Subscription upgrade after pre-user settlement (`subscriptions.rs`).
    fn on_subscription_upgrade_after_settle(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        new_price_id: PriceId,
        subscription_id: SubscriptionId,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<LockId>;
    /// **[Pipeline 5b]** Share exit after pre-user settlement (`unlock.rs`).
    fn on_unlock_tail_after_pre_user_settle(
        &mut self,
        lock_id: LockId,
        account_id: AccountId,
        validator_id: ValidatorId,
        shares_remove: u128,
    ) -> Promise;
    /// **[Pipeline 6]** After **4** tail promise completes: sets pipeline **`Idle`**.
    fn on_epoch_pipeline_terminal_release(&mut self, validator_id: ValidatorId);
}

#[ext_contract(ext_staking_pool)]
pub trait ExtStakingPool {
    /// Staking-pool view: on-chain source of truth for who may operate the pool off this contract.
    fn get_owner_id(&self) -> AccountId;
    fn deposit_and_stake(&mut self);
    fn unstake(&mut self, amount: NearToken);
    /// Move unstaked NEAR from the pool to `account_id` (this contract).
    fn withdraw(&mut self, amount: NearToken);
    /// Staked + unstaked balances and pool withdraw flag (NEAR core staking-pool `get_account`).
    fn get_account(&self, account_id: AccountId) -> PoolAccountView;
}

// =============================================================================
// Public epoch API
// =============================================================================

#[near]
impl Contract {
    /// Public entry → **[Pipeline 0]** with [`PerEpochContinue::SettleOnly`] (tail **5** = no-op, then **6**).
    pub fn epoch_settle(&mut self, validator_id: ValidatorId) -> Promise {
        self.assert_not_paused();
        self.promise_validator_per_epoch_settlement_then(
            validator_id.clone(),
            PerEpochContinue::SettleOnly { validator_id },
        )
    }
}

// =============================================================================
// Epoch pipeline (`#[near]` impl — ordered by pipeline step; see module doc)
// =============================================================================

#[near]
impl Contract {
    // --- [Pipeline 0] ---

    /// **[Pipeline 0]** Entry: set **`Busy`**, then **1–3** (full) or **4** (fast path). See module doc step map.
    pub(crate) fn promise_validator_per_epoch_settlement_then(
        &mut self,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        let mut validator = self.require_validator(&validator_id);
        self.assert_validator_idle_for_user_action(&validator);
        // Fast path when this pool already consumed its one settle slot for the current NEAR epoch.
        let needs_pre_user_settlement_pipeline =
            validator.last_settlement_epoch < env::epoch_height();
        validator.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), validator);

        if !needs_pre_user_settlement_pipeline {
            return ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
                .on_epoch_settlement_dispatch_continue(cont);
        }
        self.promise_get_validator_pool_account(validator_id.clone())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_AFTER_POOL_ACCOUNT)
                    .on_epoch_settlement_after_pool_account(validator_id, cont),
            )
    }

    /// Pool `get_account` before **[Pipeline 1]**.
    pub(crate) fn promise_get_validator_pool_account(&self, validator_id: ValidatorId) -> Promise {
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_ACCOUNT)
            .get_account(env::current_account_id())
    }

    // --- [Pipeline 1] ---

    #[private]
    /// **[Pipeline 1]** After pool `get_account`: refresh balance, optional **2a–2c**, then **3**.
    pub fn on_epoch_settlement_after_pool_account(
        &mut self,
        #[callback] pool_account: PoolAccountView,
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
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy for settlement after get_account"
        );
        validator.total_staked_balance = pool_account.total_balance();
        validator.last_balance_refresh_ns = U64(env::block_timestamp());
        self.validators.insert(validator_id.clone(), validator);

        let unstaked = pool_account.unstaked();

        if unstaked.as_yoctonear() > 0 && pool_account.can_withdraw {
            return self
                .try_epoch_withdraw_known_unstaked(
                    validator_id.clone(),
                    unstaked,
                    pool_account.can_withdraw,
                )
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                        .on_after_pool_withdraw_maybe_settle(validator_id, Some(cont)),
                );
        }

        self.try_epoch_stake_or_unstake(validator_id, Some(cont))
    }

    // --- [Pipeline 2a] ---

    /// **[Pipeline 2a]** Pull unstaked NEAR into [`Validator::pending_to_withdraw`] (from **1**).
    pub(crate) fn try_epoch_withdraw_known_unstaked(
        &mut self,
        validator_id: ValidatorId,
        unstaked_balance: NearToken,
        pool_can_withdraw: bool,
    ) -> Promise {
        require!(
            unstaked_balance.as_yoctonear() > 0,
            "Withdraw requires positive unstaked balance from the pool"
        );
        require!(
            pool_can_withdraw,
            "Pool reports unstaked balance is not yet withdrawable"
        );

        events::log_epoch_operation("epoch_withdraw", &validator_id);

        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy for this withdraw step"
        );

        self.promise_epoch_withdraw_unstaked(validator_id, unstaked_balance)
    }

    /// **[Pipeline 2a]** Pool `withdraw` promise → callback **2b**.
    pub(crate) fn promise_epoch_withdraw_unstaked(
        &self,
        validator_id: ValidatorId,
        unstaked_balance: NearToken,
    ) -> Promise {
        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::WITHDRAW)
            .withdraw(unstaked_balance)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_WITHDRAW_TRANSFER)
                    .on_epoch_withdraw_transfer_done(validator_id, unstaked_balance),
            )
    }

    // --- [Pipeline 2b] ---

    #[private]
    /// **[Pipeline 2b]** After pool `withdraw`: credit [`Validator::pending_to_withdraw`] (stays **`Busy`**).
    pub fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: ValidatorId,
        withdrawn: NearToken,
    ) -> PromiseOrValue<bool> {
        let ok = is_promise_success();
        let mut validator = self.require_validator(&validator_id);
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

    // --- [Pipeline 2c] ---

    #[private]
    /// **[Pipeline 2c]** After **2b**: unlock tail runs **3** with `None` when pending; settlement runs **3** → **4** with `Some(cont)`.
    pub fn on_after_pool_withdraw_maybe_settle(
        &mut self,
        validator_id: ValidatorId,
        cont: Option<PerEpochContinue>,
    ) -> PromiseOrValue<bool> {
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy for post-withdraw settle"
        );
        self.try_epoch_stake_or_unstake(validator_id, cont).into()
    }

    // --- [Pipeline 3 / 3a] ---

    fn promise_epoch_settlement_dispatch(&self, cont: PerEpochContinue) -> Promise {
        ext_self_epoch::ext(env::current_account_id())
            .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
            .on_epoch_settlement_dispatch_continue(cont)
    }

    /// **[Pipeline 3]** At most one pool `deposit_and_stake` or `unstake` per NEAR epoch (**3a** net-zero inline).
    /// `dispatch_after: Some(cont)` → skip to **4** when nothing pending or slot used; else pool op → **3′** → **4**.
    /// `None` — tail-only (**2c**, **5a**); no-op when nothing queued (same as slot already used).
    pub(crate) fn try_epoch_stake_or_unstake(
        &mut self,
        validator_id: ValidatorId,
        dispatch_after: Option<PerEpochContinue>,
    ) -> Promise {
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy for this settle step"
        );

        let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
        let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();
        let has_pending = pending_stake_yocto > 0 || pending_unstake_yocto > 0;
        let can_settle = validator.last_settlement_epoch < env::epoch_height();

        if let Some(ref cont) = dispatch_after {
            if !has_pending || !can_settle {
                return self.promise_epoch_settlement_dispatch(cont.clone());
            }
        } else if !has_pending {
            self.validators.insert(validator_id, validator);
            return Promise::new(env::current_account_id());
        } else if !can_settle {
            return Promise::new(env::current_account_id());
        }

        if pending_stake_yocto == pending_unstake_yocto && pending_stake_yocto > 0 {
            events::log_epoch_operation("epoch_settle_net_zero", &validator_id);
            self.apply_net_zero_pending_matched_clear(&validator_id, pending_stake_yocto);
            if let Some(cont) = dispatch_after {
                return self.promise_epoch_settlement_dispatch(cont);
            }
            return Promise::new(env::current_account_id());
        }

        require!(
            can_settle,
            "This pool already completed a stake or unstake this epoch; try again next epoch"
        );

        let pool_settle = if pending_stake_yocto > pending_unstake_yocto {
            let net = NearToken::from_yoctonear(pending_stake_yocto - pending_unstake_yocto);
            events::log_epoch_operation("epoch_settle_stake", &validator_id);
            self.validators.insert(validator_id.clone(), validator);
            ext_staking_pool::ext(validator_id.clone())
                .with_static_gas(staking_pool::DEPOSIT_AND_STAKE)
                .with_attached_deposit(net)
                .deposit_and_stake()
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_DEPOSIT_AND_STAKE)
                        .on_deposit_and_stake(
                            validator_id.clone(),
                            net,
                            U128(pending_unstake_yocto),
                        ),
                )
        } else {
            if validator.last_unstake_epoch > 0 {
                require!(
                    self.validator_unstake_waiting_finished(&validator),
                    "Wait until the previous unstake has finished its settle period before unstaking again"
                );
            }
            let net = NearToken::from_yoctonear(pending_unstake_yocto - pending_stake_yocto);
            events::log_epoch_operation("epoch_settle_unstake", &validator_id);
            ext_staking_pool::ext(validator_id.clone())
                .with_static_gas(staking_pool::UNSTAKE)
                .unstake(net)
                .then(
                    ext_self_epoch::ext(env::current_account_id())
                        .with_static_gas(callbacks::ON_UNSTAKE)
                        .on_unstake(validator_id.clone(), net, U128(pending_stake_yocto)),
                )
        };

        if let Some(cont) = dispatch_after {
            pool_settle.then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(
                        callbacks::ON_EPOCH_SETTLEMENT_AFTER_TRY_EPOCH_STAKE_OR_UNSTAKE,
                    )
                    .on_epoch_settlement_after_try_epoch_stake_or_unstake(validator_id, cont),
            )
        } else {
            pool_settle
        }
    }

    // --- [Pipeline 3a] ---

    /// **[Pipeline 3a]** Inline net-zero clear (no pool `deposit` / `unstake`).
    pub(crate) fn apply_net_zero_pending_matched_clear(
        &mut self,
        validator_id: &ValidatorId,
        matched_pending_yocto: u128,
    ) {
        let mut validator = self.require_validator(validator_id);
        require!(
            validator.pending_to_stake.as_yoctonear() == matched_pending_yocto
                && validator.pending_to_unstake.as_yoctonear() == matched_pending_yocto,
            "Net zero settle state changed; retry next epoch"
        );
        validator.pending_to_stake = NearToken::from_near(0);
        // User tranches are unchanged; re-queue pool unstake for remaining user exit liability.
        validator.pending_to_unstake = validator.pending_user_unstake_total;
        validator.last_settlement_epoch = env::epoch_height();
        self.validators.insert(validator_id.clone(), validator);
    }

    // --- [Pipeline 3b] ---

    #[private]
    /// **[Pipeline 3b]** `deposit_and_stake` callback (stays **`Busy`**).
    pub fn on_deposit_and_stake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_unstake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator(&validator_id);

        if ok {
            let net_stake_yocto = amount.as_yoctonear();
            let absorbed_unstake_yocto = absorb_unstake_yocto.0;
            let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
            let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();
            require!(
                pending_stake_yocto >= net_stake_yocto.saturating_add(absorbed_unstake_yocto),
                "After deposit: pending_to_stake underflow vs callback (contract accounting error)"
            );
            require!(
                pending_unstake_yocto >= absorbed_unstake_yocto,
                "After deposit: pending_to_unstake underflow vs absorb (contract accounting error)"
            );
            validator.pending_to_stake = NearToken::from_yoctonear(
                pending_stake_yocto
                    .saturating_sub(net_stake_yocto)
                    .saturating_sub(absorbed_unstake_yocto),
            );
            validator.pending_to_unstake = NearToken::from_yoctonear(
                pending_unstake_yocto.saturating_sub(absorbed_unstake_yocto),
            );
            if validator.pending_to_unstake.as_yoctonear() == 0
                && validator.pending_user_unstake_total.as_yoctonear() > 0
            {
                validator.pending_to_unstake = validator.pending_user_unstake_total;
            }
            validator.total_staked_balance = validator
                .total_staked_balance
                .checked_add(NearToken::from_yoctonear(net_stake_yocto))
                .expect("total_staked_balance overflow after stake");
            validator.last_settlement_epoch = env::epoch_height();
        }
        self.validators.insert(validator_id, validator);
        ok
    }

    // --- [Pipeline 3c] ---

    #[private]
    /// **[Pipeline 3c]** `unstake` callback (stays **`Busy`**).
    pub fn on_unstake(
        &mut self,
        validator_id: ValidatorId,
        amount: NearToken,
        absorb_stake_yocto: U128,
    ) -> bool {
        let ok = is_promise_success();
        let mut validator = self.require_validator(&validator_id);
        if ok {
            let current_epoch = env::epoch_height();
            validator.last_unstake_epoch = current_epoch;
            validator.last_settlement_epoch = current_epoch;
            let net_unstake_yocto = amount.as_yoctonear();
            let absorbed_stake_yocto = absorb_stake_yocto.0;
            let pending_stake_yocto = validator.pending_to_stake.as_yoctonear();
            let pending_unstake_yocto = validator.pending_to_unstake.as_yoctonear();
            require!(
                pending_unstake_yocto >= net_unstake_yocto.saturating_add(absorbed_stake_yocto),
                "After unstake: pending_to_unstake underflow vs callback (contract accounting error)"
            );
            require!(
                pending_stake_yocto >= absorbed_stake_yocto,
                "After unstake: pending_to_stake underflow vs absorb (contract accounting error)"
            );
            validator.pending_to_unstake = NearToken::from_yoctonear(
                pending_unstake_yocto
                    .saturating_sub(net_unstake_yocto)
                    .saturating_sub(absorbed_stake_yocto),
            );
            validator.pending_to_stake =
                NearToken::from_yoctonear(pending_stake_yocto.saturating_sub(absorbed_stake_yocto));
        }
        self.validators.insert(validator_id, validator);
        ok
    }

    // --- [Pipeline 3′] ---

    #[private]
    /// **[Pipeline 3′]** After async **3**; forwards to **4** (pool outcome already recorded on validator state).
    #[allow(unused_variables)]
    pub fn on_epoch_settlement_after_try_epoch_stake_or_unstake(
        &mut self,
        #[callback] _settle_ok: bool,
        validator_id: ValidatorId,
        cont: PerEpochContinue,
    ) -> Promise {
        ext_self_epoch::ext(env::current_account_id())
            .with_static_gas(callbacks::ON_EPOCH_SETTLEMENT_DISPATCH)
            .on_epoch_settlement_dispatch_continue(cont)
    }

    // --- [Pipeline 4] ---

    #[private]
    /// **[Pipeline 4]** Fan-out to **5a** / **5b** / **5c**, then chain **6**.
    pub fn on_epoch_settlement_dispatch_continue(&mut self, cont: PerEpochContinue) -> Promise {
        let validator_id = cont.validator_id().clone();
        let tail = match cont {
            // Catalog purchase: mint shares / usage after validator state is fresh for this epoch.
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
            PerEpochContinue::SubscriptionUpgrade {
                validator_id,
                buyer,
                deposit,
                new_price_id,
                subscription_id,
            } => ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_SUBSCRIPTION_UPGRADE_AFTER_SETTLE)
                .on_subscription_upgrade_after_settle(
                    buyer,
                    deposit,
                    new_price_id,
                    subscription_id,
                    validator_id,
                ),
            // Share exit: burn shares and queue `pending_to_unstake` (pool `unstake` on a later settlement).
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
            } => self.payout_user_withdraw(account_id, validator_id),
            PerEpochContinue::SettleOnly { .. } => Promise::new(env::current_account_id()),
        };
        tail.then(
            ext_self_epoch::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_EPOCH_PIPELINE_TERMINAL_RELEASE)
                .on_epoch_pipeline_terminal_release(validator_id),
        )
    }

    // --- [Pipeline 6] ---

    /// Used by **6** (and error paths in `unlock.rs`).
    pub(crate) fn release_validator_pool_pipeline(&mut self, validator_id: &ValidatorId) {
        let mut validator = self.require_validator(validator_id);
        validator.tx_status = TransactionStatus::Idle;
        self.validators.insert(validator_id.clone(), validator);
    }

    #[private]
    /// **[Pipeline 6]** After **4** tail completes: clear pipeline **`Busy`**.
    pub fn on_epoch_pipeline_terminal_release(&mut self, validator_id: ValidatorId) {
        self.release_validator_pool_pipeline(&validator_id);
    }
}
