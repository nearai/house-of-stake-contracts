use crate::*;
use near_sdk::json_types::U64;
use near_sdk::{env, is_promise_success, near, require, NearToken};

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

    #[private]
    pub fn on_withdraw_all_to_contract(
        &mut self,
        validator_pool: AccountId,
        #[callback] amount: NearToken,
    ) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let ok = is_promise_success();
        if !ok {
            return false;
        }
        let _ = validator_pool;
        let _ = amount;
        // Credit users proportionally: simplified v1 credits entire pending_to_withdraw split — TODO.
        true
    }

    #[private]
    pub fn on_refresh_total_balance(
        &mut self,
        validator_pool: AccountId,
        #[callback] total_balance: NearToken,
    ) {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        let mut v = self
            .validators
            .get(&validator_pool)
            .expect("validator");
        v.tx_status = TransactionStatus::Idle;
        if is_promise_success() {
            v.total_staked_balance = total_balance;
            v.last_balance_refresh_ns = U64(env::block_timestamp());
        }
        self.validators.insert(validator_pool, v);
    }
}
