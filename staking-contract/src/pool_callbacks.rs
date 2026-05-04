//! Callbacks from staking pool promises.
//!
//! `on_refresh_total_balance` overwrites `Validator::total_staked_balance` with the pool-reported aggregate.
//! That figure can diverge from internal share accounting (`total_shares`, `pending_to_stake`, `pending_to_unstake`)
//! when staking rewards accrue or rounding differs—use as an operator diagnostics aid unless reconciled later.
//!
//! **Share vs balance reconciliation:** pro-rata exits still use `near_from_shares` against `effective_stake`
//! (staked + pending stake). If `total_staked_balance` is refreshed upward by rewards but shares are not
//! re-minted, user NEAR-on-exit can slightly trail the pool’s notional balance; a future version could
//! periodically mint “donation” shares to the protocol or run a share/NEAR true-up. Not implemented here.
//!
//! `on_epoch_withdraw_transfer_done` credits the **requested** `withdrawn` amount on a successful
//! pool `withdraw` return. (A balance-delta approach is unsafe if multiple pool withdrawals overlap in
//! time and share the contract’s single account balance; operators should run one `epoch_withdraw` at
//! a time if their pool can short-pay.)

use crate::epoch::{ext_self_epoch, ext_staking_pool};
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::json_types::U64;
use near_sdk::{NearToken, PromiseOrValue, env, is_promise_success, near, require};

#[near]
impl Contract {
    #[private]
    pub fn on_deposit_and_stake(&mut self, validator_pool: AccountId, amount: NearToken) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let ok = is_promise_success();
        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("validator");

        v.tx_status = TransactionStatus::Idle;

        if ok {
            let pend = v.pending_to_stake.as_yoctonear();
            let consume = amount.as_yoctonear().min(pend);
            v.pending_to_stake = NearToken::from_yoctonear(pend.saturating_sub(consume));
            v.total_staked_balance = v
                .total_staked_balance
                .checked_add(NearToken::from_yoctonear(consume))
                .expect("staked balance");
        }
        self.validators.insert(validator_pool, v);
        ok
    }

    #[private]
    pub fn on_unstake(&mut self, validator_pool: AccountId, amount: NearToken) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let ok = is_promise_success();
        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("validator");
        v.tx_status = TransactionStatus::Idle;
        if ok {
            v.last_unstake_epoch = env::epoch_height();
            let pu = v.pending_to_unstake.as_yoctonear();
            let done = amount.as_yoctonear().min(pu);
            v.pending_to_unstake = NearToken::from_yoctonear(pu.saturating_sub(done));
        }
        self.validators.insert(validator_pool, v);
        ok
    }

    /// After `get_account_unstaked_balance`: if zero, release `Busy`; else `withdraw` unstaked NEAR into this contract.
    #[private]
    pub fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_pool: AccountId,
    ) -> PromiseOrValue<bool> {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        if !is_promise_success() {
            let mut v = self
                .validators
                .get(&validator_pool)
                .cloned()
                .expect("validator");
            v.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_pool, v);
            return PromiseOrValue::Value(false);
        }

        if unstaked_balance.as_yoctonear() == 0 {
            let mut v = self
                .validators
                .get(&validator_pool)
                .cloned()
                .expect("validator");
            v.tx_status = TransactionStatus::Idle;
            self.validators.insert(validator_pool, v);
            return PromiseOrValue::Value(true);
        }

        ext_staking_pool::ext(validator_pool.clone())
            .with_static_gas(staking_pool::WITHDRAW)
            .withdraw(unstaked_balance)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_WITHDRAW_TRANSFER)
                    .on_epoch_withdraw_transfer_done(validator_pool, unstaked_balance),
            )
            .into()
    }

    #[private]
    pub fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_pool: AccountId,
        withdrawn: NearToken,
    ) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let ok = is_promise_success();
        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("validator");
        v.tx_status = TransactionStatus::Idle;
        let credited_yocto = if ok { withdrawn.as_yoctonear() } else { 0 };
        if ok && credited_yocto > 0 {
            let add = NearToken::from_yoctonear(credited_yocto);
            v.pending_to_withdraw = v
                .pending_to_withdraw
                .checked_add(add)
                .expect("pending_to_withdraw overflow");
            crate::events::log_pool_withdraw_in(credited_yocto, &validator_pool);
        }
        self.validators.insert(validator_pool, v);
        ok
    }

    #[private]
    pub fn on_refresh_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_pool: AccountId,
    ) {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("validator");
        v.tx_status = TransactionStatus::Idle;
        if is_promise_success() {
            v.total_staked_balance = total_balance;
            v.last_balance_refresh_ns = U64(env::block_timestamp());
        }
        self.validators.insert(validator_pool, v);
    }
}
