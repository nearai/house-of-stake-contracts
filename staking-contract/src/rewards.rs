//! Staking-farm style reward accounting for catalog locks.
//!
//! Rewards are tracked with a validator-level cumulative reward index so rate changes apply
//! prospectively without scanning all locks. Claiming is accounting-only; this contract does not
//! custody a separate reward token.

use crate::utils::block_timestamp;
use crate::*;
use near_sdk::json_types::{U64, U128};
use near_sdk::{NearToken, assert_one_yocto, env, near, require};

#[near]
impl Contract {
    /// Set the reward emission rate for a validator. Owner only; attach 1 yocto.
    ///
    /// `reward_rate_yocto_per_near_ns` emits that many reward yocto-units per 1 NEAR of active
    /// lock principal per nanosecond. Set to zero to disable future accrual.
    #[payable]
    pub fn set_validator_reward_rate(
        &mut self,
        validator_id: ValidatorId,
        reward_rate_yocto_per_near_ns: U128,
    ) {
        assert_one_yocto();
        self.assert_owner();
        self.require_validator(&validator_id);

        let mut config = self.project_validator_reward_config(&validator_id, block_timestamp());
        config.reward_rate_yocto_per_near_ns = reward_rate_yocto_per_near_ns;
        config.last_update_ns = U64(block_timestamp());
        self.internal_set_validator_reward_config(validator_id.clone(), config.clone());
        crate::events::log_reward_config_update(
            &validator_id,
            config.reward_rate_yocto_per_near_ns.0,
            config.accumulated_reward_per_near.0,
        );
    }

    pub fn get_validator_reward_config(&self, validator_id: ValidatorId) -> ValidatorRewardConfig {
        self.project_validator_reward_config(&validator_id, block_timestamp())
    }

    pub fn get_lock_reward(&self, lock_id: LockId) -> Option<LockRewardView> {
        let lock = self.internal_get_lock(&lock_id)?;
        let state = self.internal_get_lock_reward_state(&lock_id)?;
        let config = self.project_validator_reward_config(&lock.validator_id, block_timestamp());
        let projected = self.project_lock_reward_state(&lock, state, &config);
        Some(LockRewardView {
            lock_id,
            validator_id: lock.validator_id,
            account_id: lock.account_id,
            reward_rate_yocto_per_near_ns: config.reward_rate_yocto_per_near_ns,
            accumulated_reward_per_near: config.accumulated_reward_per_near,
            unclaimed_rewards: projected.unclaimed_rewards,
            claimed_rewards: projected.claimed_rewards,
            last_update_ns: projected.last_update_ns,
        })
    }

    /// Persist accrued rewards for a lock and return the updated reward state. Lock owner only.
    #[payable]
    pub fn update_lock_rewards(&mut self, lock_id: LockId) -> LockRewardState {
        assert_one_yocto();
        self.assert_not_paused();
        let caller = env::predecessor_account_id();
        let lock = self.require_lock_owned_by(
            &lock_id,
            &caller,
            "Lock not found; check the lock id",
            "Only the lock owner can update rewards",
        );
        self.settle_lock_rewards(&lock)
    }

    /// Mark all unclaimed rewards for a lock as claimed and return the claimed amount.
    ///
    /// This is accounting-only: no NEAR or reward token transfer is performed here.
    #[payable]
    pub fn claim_lock_rewards(&mut self, lock_id: LockId) -> U128 {
        assert_one_yocto();
        self.assert_not_paused();
        let caller = env::predecessor_account_id();
        let lock = self.require_lock_owned_by(
            &lock_id,
            &caller,
            "Lock not found; check the lock id",
            "Only the lock owner can claim rewards",
        );
        let mut state = self.settle_lock_rewards(&lock);
        let amount = state.unclaimed_rewards.0;
        require!(amount > 0, "No rewards are available to claim");
        state.unclaimed_rewards = U128(0);
        state.claimed_rewards = U128(state.claimed_rewards.0.saturating_add(amount));
        state.last_update_ns = U64(block_timestamp());
        self.internal_set_lock_reward_state(lock_id.clone(), state);
        crate::events::log_reward_claim(&lock_id, &caller, amount);
        U128(amount)
    }
}

impl Contract {
    pub(crate) fn initialize_lock_rewards(&mut self, lock: &Lock) {
        let config = self.advance_validator_reward_config(&lock.validator_id);
        let state = LockRewardState {
            lock_id: lock.lock_id.clone(),
            accumulated_reward_per_near_paid: config.accumulated_reward_per_near,
            unclaimed_rewards: U128(0),
            claimed_rewards: U128(0),
            last_update_ns: U64(block_timestamp()),
        };
        self.internal_set_lock_reward_state(lock.lock_id.clone(), state);
    }

    pub(crate) fn settle_lock_rewards(&mut self, lock: &Lock) -> LockRewardState {
        let config = self.advance_validator_reward_config(&lock.validator_id);
        let state = self
            .internal_get_lock_reward_state(&lock.lock_id)
            .unwrap_or_else(|| {
                let state = LockRewardState {
                    lock_id: lock.lock_id.clone(),
                    accumulated_reward_per_near_paid: config.accumulated_reward_per_near,
                    unclaimed_rewards: U128(0),
                    claimed_rewards: U128(0),
                    last_update_ns: U64(block_timestamp()),
                };
                self.internal_set_lock_reward_state(lock.lock_id.clone(), state.clone());
                state
            });
        let next = self.project_lock_reward_state(lock, state, &config);
        self.internal_set_lock_reward_state(lock.lock_id.clone(), next.clone());
        next
    }

    pub(crate) fn internal_get_validator_reward_config(
        &self,
        validator_id: &ValidatorId,
    ) -> Option<ValidatorRewardConfig> {
        self.validator_reward_configs
            .get(validator_id)
            .cloned()
            .map(Into::into)
    }

    pub(crate) fn internal_set_validator_reward_config(
        &mut self,
        validator_id: ValidatorId,
        config: ValidatorRewardConfig,
    ) {
        self.validator_reward_configs
            .insert(validator_id, config.into());
    }

    pub(crate) fn internal_get_lock_reward_state(
        &self,
        lock_id: &LockId,
    ) -> Option<LockRewardState> {
        self.lock_reward_states
            .get(lock_id)
            .cloned()
            .map(Into::into)
    }

    pub(crate) fn internal_set_lock_reward_state(
        &mut self,
        lock_id: LockId,
        state: LockRewardState,
    ) {
        self.lock_reward_states.insert(lock_id, state.into());
    }

    fn advance_validator_reward_config(
        &mut self,
        validator_id: &ValidatorId,
    ) -> ValidatorRewardConfig {
        let config = self.project_validator_reward_config(validator_id, block_timestamp());
        self.internal_set_validator_reward_config(validator_id.clone(), config.clone());
        config
    }

    fn project_validator_reward_config(
        &self,
        validator_id: &ValidatorId,
        now_ns: u64,
    ) -> ValidatorRewardConfig {
        let mut config = self
            .internal_get_validator_reward_config(validator_id)
            .unwrap_or(ValidatorRewardConfig {
                validator_id: validator_id.clone(),
                reward_rate_yocto_per_near_ns: U128(0),
                accumulated_reward_per_near: U128(0),
                last_update_ns: U64(now_ns),
            });
        if now_ns <= config.last_update_ns.0 {
            return config;
        }
        let elapsed = now_ns.saturating_sub(config.last_update_ns.0);
        let reward_delta = config
            .reward_rate_yocto_per_near_ns
            .0
            .saturating_mul(u128::from(elapsed));
        config.accumulated_reward_per_near = U128(
            config
                .accumulated_reward_per_near
                .0
                .saturating_add(reward_delta),
        );
        config.last_update_ns = U64(now_ns);
        config
    }

    fn project_lock_reward_state(
        &self,
        lock: &Lock,
        mut state: LockRewardState,
        config: &ValidatorRewardConfig,
    ) -> LockRewardState {
        let reward_index_delta = config
            .accumulated_reward_per_near
            .0
            .saturating_sub(state.accumulated_reward_per_near_paid.0);
        if reward_index_delta == 0 {
            state.last_update_ns = U64(block_timestamp());
            return state;
        }

        let accrued = proportional_rewards(lock.amount_near, reward_index_delta);
        state.unclaimed_rewards = U128(state.unclaimed_rewards.0.saturating_add(accrued));
        state.accumulated_reward_per_near_paid = config.accumulated_reward_per_near;
        state.last_update_ns = U64(block_timestamp());
        state
    }
}

fn proportional_rewards(amount_near: NearToken, reward_index_delta: u128) -> u128 {
    amount_near
        .as_yoctonear()
        .saturating_mul(reward_index_delta)
        / REWARD_NEAR_DENOMINATOR
}
