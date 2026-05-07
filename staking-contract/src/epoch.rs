use crate::events;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::{AccountId, NearToken, Promise, PromiseOrValue, env, near, require};

#[ext_contract(ext_self_epoch)]
pub trait ExtSelfEpoch {
    fn on_deposit_and_stake(&mut self, validator_pool: AccountId, amount: NearToken) -> bool;
    fn on_unstake(&mut self, validator_pool: AccountId, amount: NearToken) -> bool;
    fn on_refresh_total_balance(
        &mut self,
        #[callback] total_balance: NearToken,
        validator_pool: AccountId,
    );
    /// After `get_account_unstaked_balance`; may chain `withdraw` on the pool.
    fn on_get_unstaked_for_epoch_withdraw(
        &mut self,
        #[callback] unstaked_balance: NearToken,
        validator_pool: AccountId,
    ) -> PromiseOrValue<bool>;
    fn on_epoch_withdraw_transfer_done(
        &mut self,
        validator_pool: AccountId,
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
    pub fn epoch_stake(&mut self, validator_pool: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_stake", &validator_pool);

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.tx_status == TransactionStatus::Idle,
            "validator pool busy"
        );
        require!(
            v.last_stake_epoch < env::epoch_height(),
            "already completed a stake batch this epoch"
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

    /// Operator: request unstake for `pending_to_unstake` (after users called [`crate::unlock::Contract::unlock`]).
    /// A new unstake is rejected until `epoch_unstake_settle_epochs` have passed since the last successful
    /// unstake callback for this pool (serialize unstake rounds before the next batch).
    pub fn epoch_unstake(&mut self, validator_pool: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_unstake", &validator_pool);

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.tx_status == TransactionStatus::Idle,
            "validator pool busy"
        );
        if v.last_unstake_epoch > 0 {
            let ready_epoch = v
                .last_unstake_epoch
                .saturating_add(self.config.epoch_unstake_settle_epochs);
            require!(
                env::epoch_height() >= ready_epoch,
                "wait until previous unstake has settled before unstaking again"
            );
        }
        let amt = v.pending_to_unstake;
        require!(amt.as_yoctonear() > 0, "nothing to unstake");

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_pool.clone(), v);

        ext_staking_pool::ext(validator_pool.clone())
            .with_static_gas(staking_pool::UNSTAKE)
            .unstake(amt)
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_UNSTAKE)
                    .on_unstake(validator_pool, amt),
            )
    }

    /// Operator: after unstake settles (`epoch_unstake_settle_epochs`), pull unstaked NEAR from the pool into
    /// `pending_to_withdraw`. Users then call `claim_unlocked_near`.
    pub fn epoch_withdraw(&mut self, validator_pool: AccountId) -> Promise {
        self.assert_not_paused();
        self.assert_operator();
        events::log_epoch_operation("epoch_withdraw", &validator_pool);

        let mut v = self
            .validators
            .get(&validator_pool)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.tx_status == TransactionStatus::Idle,
            "validator pool busy"
        );
        require!(v.last_unstake_epoch > 0, "run epoch_unstake first");
        require!(
            env::epoch_height()
                >= v.last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs),
            "wait unstake settlement epochs"
        );

        v.tx_status = TransactionStatus::Busy;
        self.validators.insert(validator_pool.clone(), v);

        ext_staking_pool::ext(validator_pool.clone())
            .with_static_gas(staking_pool::GET_ACCOUNT_UNSTAKED_BALANCE)
            .get_account_unstaked_balance(env::current_account_id())
            .then(
                ext_self_epoch::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_GET_UNSTAKED_FOR_WITHDRAW)
                    .on_get_unstaked_for_epoch_withdraw(validator_pool),
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
