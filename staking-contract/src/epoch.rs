use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::{env, near, require, AccountId, NearToken, Promise};

#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    fn on_deposit_and_stake(&mut self, validator_pool: AccountId, amount: NearToken) -> bool;
    fn on_refresh_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_pool: AccountId,
    );
}

#[ext_contract(ext_staking_pool)]
pub trait ExtStakingPool {
    fn deposit_and_stake(&mut self);
    fn unstake(&mut self, amount: NearToken);
    fn withdraw_all(&mut self);
    fn get_account_total_balance(&self, account_id: AccountId) -> NearToken;
}

#[near]
impl Contract {
    /// Operator (or anyone if operators list empty): push pending stake to the pool.
    pub fn epoch_stake(&mut self, validator_pool: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.tx_status == TransactionStatus::Idle,
            "validator pool busy"
        );
        let amt = v.pending_to_stake;
        require!(amt.as_yoctonear() > 0, "nothing to stake");

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_pool.clone(), v);

        ext_staking_pool::ext(validator_pool.clone())
            .with_static_gas(staking_pool::DEPOSIT_AND_STAKE)
            .with_attached_deposit(amt)
            .deposit_and_stake()
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_DEPOSIT_AND_STAKE)
                    .on_deposit_and_stake(validator_pool, amt),
            )
    }

    pub fn refresh_validator_balance(&mut self, validator_pool: AccountId) -> Promise {
        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.tx_status == TransactionStatus::Idle,
            "validator pool busy"
        );
        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_pool.clone(), v);

        ext_staking_pool::ext(validator_pool.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_TOTAL_BALANCE)
            .get_account_total_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_TOTAL_BALANCE)
                    .on_refresh_total_balance(validator_pool),
            )
    }
}
