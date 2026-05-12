//! Operator epoch pipeline: stake / unstake / withdraw-from-pool, plus pool promise callbacks (`on_*`).
//!
//! **Pool callbacks** (`on_deposit_and_stake`, `on_unstake`, `on_get_unstaked_for_epoch_withdraw`,
//! `on_epoch_withdraw_transfer_done`, `on_refresh_total_balance`) run on this contract after staking-pool
//! promises resolve. `on_refresh_total_balance` overwrites [`crate::validators::Validator::total_staked_balance`]
//! with the pool-reported aggregate; that figure can diverge from internal share accounting when rewards
//! accrue or rounding differs.
//!
//! **Share vs balance:** exits use [`crate::internal::near_from_shares`] against
//! [`crate::internal::effective_stake_for_share_exit`] (gross + pending stake minus full
//! `pending_user_unstake_total`). `total_staked_balance` is reduced when `on_epoch_withdraw_transfer_done`
//! confirms NEAR left the pool so backing is not inflated by funds in `pending_to_withdraw`.
//!
//! `on_epoch_withdraw_transfer_done` credits the **requested** `withdrawn` on a successful pool `withdraw`.
//! Operators should run one `epoch_withdraw` at a time if the pool can short-pay.

use crate::events;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U64;
use near_sdk::{
    AccountId, NearToken, Promise, PromiseOrValue, env, is_promise_success, near, require,
};

#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    fn on_deposit_and_stake(&mut self, validator_id: AccountId, amount: NearToken) -> bool;
    fn on_unstake(&mut self, validator_id: AccountId, amount: NearToken) -> bool;
    fn on_refresh_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_id: AccountId,
    );
    /// After `get_account_unstaked_balance`; may chain `withdraw` on the pool.
    fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: AccountId,
    ) -> PromiseOrValue<bool>;
    fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_id: AccountId,
        withdrawn: NearToken,
    ) -> bool;
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

#[near]
impl Contract {
    /// Operator: stake `pending_to_stake` on the pool. At most **one successful** stake batch per epoch per
    /// validator (retry the same epoch only if the pool call failed and `tx_status` returned to idle).
    pub fn epoch_stake(&mut self, validator_id: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_stake", &validator_id);

        let mut v = self.require_validator(&validator_id);
        require!(
            v.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        require!(
            v.last_stake_epoch < env::epoch_height(),
            "A stake batch already succeeded this epoch for this validator"
        );
        let amt = v.pending_to_stake;
        require!(
            amt.as_yoctonear() > 0,
            "No NEAR is queued to stake for this validator"
        );

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), v);

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::DEPOSIT_AND_STAKE)
            .with_attached_deposit(amt)
            .deposit_and_stake()
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_DEPOSIT_AND_STAKE)
                    .on_deposit_and_stake(validator_id, amt),
            )
    }

    /// Operator: request unstake for `pending_to_unstake` (after users called [`crate::unlock::Contract::unlock`]).
    /// A new unstake is rejected until `epoch_unstake_settle_epochs` have passed since the last successful
    /// unstake callback for this pool (serialize unstake rounds before the next batch).
    pub fn epoch_unstake(&mut self, validator_id: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_unstake", &validator_id);

        let mut v = self.require_validator(&validator_id);
        require!(
            v.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        if v.last_unstake_epoch > 0 {
            let ready_epoch = v
                .last_unstake_epoch
                .saturating_add(self.config.epoch_unstake_settle_epochs);
            require!(
                env::epoch_height() >= ready_epoch,
                "Wait until the previous unstake has finished its settle period before unstaking again"
            );
        }
        let amt = v.pending_to_unstake;
        require!(
            amt.as_yoctonear() > 0,
            "No NEAR is queued to unstake for this validator"
        );

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), v);

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::UNSTAKE)
            .unstake(amt)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_UNSTAKE)
                    .on_unstake(validator_id, amt),
            )
    }

    /// Operator: after unstake settles (`epoch_unstake_settle_epochs`), pull unstaked NEAR from the pool into
    /// `pending_to_withdraw`. Users then call `claim_unlocked_near`.
    pub fn epoch_withdraw(&mut self, validator_id: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_withdraw", &validator_id);

        let mut v = self.require_validator(&validator_id);
        require!(
            v.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        require!(
            v.last_unstake_epoch > 0,
            "Run epoch_unstake for this validator before epoch_withdraw"
        );
        require!(
            env::epoch_height()
                >= v.last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs),
            "Wait until enough epochs have passed after the last unstake before withdrawing"
        );

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), v);

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_UNSTAKED_BALANCE)
            .get_account_unstaked_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                    .on_get_unstaked_for_epoch_withdraw(validator_id),
            )
    }

    /// Refresh pool-reported balance into [`Validator::total_staked_balance`]. Same access as
    /// [`Contract::epoch_stake`] — [`Contract::assert_operator`].
    pub fn refresh_validator_balance(&mut self, validator_id: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        let mut v = self.require_validator(&validator_id);
        require!(
            v.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_id.clone(), v);

        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_TOTAL_BALANCE)
            .get_account_total_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_TOTAL_BALANCE)
                    .on_refresh_total_balance(validator_id),
            )
    }
}

#[near]
impl Contract {
    #[private]
    pub fn on_deposit_and_stake(&mut self, validator_id: AccountId, amount: NearToken) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only this staking contract may call this callback"
        );
        let ok = is_promise_success();
        let mut v = self.require_validator_pool_callback(&validator_id);

        v.tx_status = TransactionStatus::Idle;

        if ok {
            let pend = v.pending_to_stake.as_yoctonear();
            let consume = amount.as_yoctonear().min(pend);
            v.pending_to_stake = NearToken::from_yoctonear(pend.saturating_sub(consume));
            v.total_staked_balance = v
                .total_staked_balance
                .checked_add(NearToken::from_yoctonear(consume))
                .expect("total_staked_balance overflow after stake");
            v.last_stake_epoch = env::epoch_height();
        }
        self.validators.insert(validator_id, v);
        ok
    }

    #[private]
    pub fn on_unstake(&mut self, validator_id: AccountId, amount: NearToken) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only this staking contract may call this callback"
        );
        let ok = is_promise_success();
        let mut v = self.require_validator_pool_callback(&validator_id);
        v.tx_status = TransactionStatus::Idle;
        if ok {
            v.last_unstake_epoch = env::epoch_height();
            let pu = v.pending_to_unstake.as_yoctonear();
            let done = amount.as_yoctonear().min(pu);
            v.pending_to_unstake = NearToken::from_yoctonear(pu.saturating_sub(done));
        }
        self.validators.insert(validator_id, v);
        ok
    }

    /// After `get_account_unstaked_balance`: if zero, release `Busy`; else `withdraw` unstaked NEAR into this contract.
    #[private]
    pub fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_id: AccountId,
    ) -> PromiseOrValue<bool> {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only this staking contract may call this callback"
        );
        if !is_promise_success() {
            let mut v = self.require_validator_pool_callback(&validator_id);
            v.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, v);
            return PromiseOrValue::Value(false);
        }

        if unstaked_balance.as_yoctonear() == 0 {
            let mut v = self.require_validator_pool_callback(&validator_id);
            v.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_id, v);
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
        validator_id: AccountId,
        withdrawn: NearToken,
    ) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only this staking contract may call this callback"
        );
        let ok = is_promise_success();
        let mut v = self.require_validator_pool_callback(&validator_id);
        v.tx_status = TransactionStatus::Idle;
        let credited_yocto = if ok { withdrawn.as_yoctonear() } else { 0 };
        if ok && credited_yocto > 0 {
            let add = NearToken::from_yoctonear(credited_yocto);
            let bal_y = v.total_staked_balance.as_yoctonear();
            require!(
                bal_y >= credited_yocto,
                "Recorded pool balance is less than the withdrawn amount; run refresh_validator_balance for this validator, then retry"
            );
            v.total_staked_balance = NearToken::from_yoctonear(bal_y - credited_yocto);
            let k = v.withdraw_batches.len() as u32;
            let accounts_snapshot = v.accounts_with_pending_unstake.clone();
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
                "Cannot record this withdraw: no user pending unstake matches this batch. Affected users should call claim_unlocked_near once, then the operator can retry epoch_withdraw"
            );
            let l_near = NearToken::from_yoctonear(l_k);
            v.pending_to_withdraw = v
                .pending_to_withdraw
                .checked_add(add)
                .expect("pending_to_withdraw overflow after pool transfer");
            v.withdraw_batches.push(WithdrawBatch {
                remaining: add,
                liability_at_fund: l_near,
            });
            events::log_pool_withdraw_in(credited_yocto, &validator_id);
        }
        self.validators.insert(validator_id, v);
        ok
    }

    #[private]
    pub fn on_refresh_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_id: AccountId,
    ) {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only this staking contract may call this callback"
        );
        let mut v = self.require_validator_pool_callback(&validator_id);
        v.tx_status = TransactionStatus::Idle;
        if is_promise_success() {
            v.total_staked_balance = total_balance;
            v.last_balance_refresh_ns = U64(env::block_timestamp());
        }
        self.validators.insert(validator_id, v);
    }
}
