use crate::epoch::ext_staking_pool;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::{U64, U128};
use near_sdk::{
    AccountId, NearToken, Promise, assert_one_yocto, env, is_promise_success, near, require,
};

#[ext_contract(ext_self_validators)]
#[allow(dead_code)] // Used by `ext_self_validators::ext` generated code; rustc does not count it as a read.
trait ExtSelfValidators {
    fn sync_validator_owner_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        pool_account_id: AccountId,
    );
}

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Validator {
    pub pool_account_id: AccountId,
    /// Snapshot of the staking pool's `get_owner_id()` at enrollment (`add_validator`) or after
    /// `sync_validator_owner_from_pool`. Catalog ops re-verify the pool on each call.
    pub owner_account_id: AccountId,
    pub status: ValidatorStatus,

    pub total_shares: U128,
    pub total_staked_balance: NearToken,
    pub last_balance_refresh_ns: U64,

    pub pending_to_stake: NearToken,
    pub pending_to_unstake: NearToken,
    /// Epoch height recorded after the last successful `epoch_unstake` callback; gates further unstakes.
    pub last_unstake_epoch: u64,
    /// Epoch height of the last successful `on_deposit_and_stake`; enforces at most one successful `epoch_stake` per epoch per pool.
    pub last_stake_epoch: u64,
    /// NEAR returned from the pool (`epoch_withdraw`) not yet claimed into user accounts.
    pub pending_to_withdraw: NearToken,
    /// Sum of `user_pending_unstake` for this pool; used with `pending_to_withdraw` for pro-rata claims.
    pub pending_user_unstake_total: NearToken,

    pub tx_status: TransactionStatus,
}

#[near]
impl Contract {
    /// Contract owner: add a validator to the allowlist.
    #[payable]
    pub fn add_validator(
        &mut self,
        pool_account_id: AccountId,
        validator_owner_account_id: AccountId,
    ) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        require!(
            self.validators.get(&pool_account_id).is_none(),
            "Validator already exists"
        );

        let v = Validator {
            pool_account_id: pool_account_id.clone(),
            owner_account_id: validator_owner_account_id,
            status: ValidatorStatus::Active,
            total_shares: U128(0),
            total_staked_balance: NearToken::from_near(0),
            last_balance_refresh_ns: U64(env::block_timestamp()),
            pending_to_stake: NearToken::from_near(0),
            pending_to_unstake: NearToken::from_near(0),
            last_unstake_epoch: 0,
            last_stake_epoch: 0,
            pending_to_withdraw: NearToken::from_near(0),
            pending_user_unstake_total: NearToken::from_near(0),
            tx_status: TransactionStatus::Idle,
        };
        self.validators.insert(pool_account_id.clone(), v);
        self.validator_ids.push(pool_account_id.clone());
        crate::events::log_validator_added(&pool_account_id);
    }

    pub fn get_validator(&self, pool_account_id: AccountId) -> Option<Validator> {
        self.validators.get(&pool_account_id).cloned()
    }

    pub fn list_validator_ids(&self, from_index: u64, limit: u64) -> Vec<AccountId> {
        let len_u64 = self.validator_ids.len() as u64;
        let mut out = Vec::new();
        let mut i = from_index;
        while i < len_u64 && (out.len() as u64) < limit {
            if let Some(id) = self.validator_ids.get(i as u32) {
                out.push(id.clone());
            }
            i += 1;
        }
        out
    }

    /// Paginated validator records (same ordering as [`Contract::list_validator_ids`]).
    pub fn get_validators(&self, from_index: u64, limit: u64) -> Vec<Validator> {
        self.list_validator_ids(from_index, limit)
            .into_iter()
            .filter_map(|id| self.validators.get(&id).cloned())
            .collect()
    }

    /// Refresh [`Validator::owner_account_id`] from the pool's `get_owner_id()` (on-chain source of truth).
    #[payable]
    pub fn sync_validator_owner_from_pool(&mut self, pool_account_id: AccountId) -> Promise {
        assert_one_yocto();
        require!(
            self.validators.get(&pool_account_id).is_some(),
            "Unknown validator"
        );
        ext_staking_pool::ext(pool_account_id.clone())
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_validators::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .sync_validator_owner_after_get_owner(pool_account_id),
            )
    }

    #[private]
    pub fn sync_validator_owner_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        pool_account_id: AccountId,
    ) {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "private"
        );
        require!(is_promise_success(), "get_owner_id failed");
        let mut v = self
            .validators
            .get(&pool_account_id)
            .cloned()
            .expect("Unknown validator");
        v.owner_account_id = pool_owner;
        self.validators.insert(pool_account_id, v);
    }

    #[payable]
    pub fn pause_validator(&mut self, pool_account_id: AccountId) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        let mut v = self
            .validators
            .get(&pool_account_id)
            .cloned()
            .expect("Unknown validator");
        v.status = ValidatorStatus::Paused;
        self.validators.insert(pool_account_id, v);
    }

    #[payable]
    pub fn remove_validator(&mut self, pool_account_id: AccountId) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        let v = self
            .validators
            .get(&pool_account_id)
            .cloned()
            .expect("Unknown validator");
        require!(
            v.total_shares.0 == 0
                && v.pending_to_stake.as_yoctonear() == 0
                && v.pending_to_unstake.as_yoctonear() == 0
                && v.pending_to_withdraw.as_yoctonear() == 0
                && v.pending_user_unstake_total.as_yoctonear() == 0,
            "Validator still has stake or pending operations"
        );
        let mut v = v;
        v.status = ValidatorStatus::Removed;
        self.validators.insert(pool_account_id, v);
    }
}

impl Contract {
    /// Pool must be on the allowlist. Catalog methods confirm the caller against the pool's
    /// `get_owner_id()` via a cross-contract call (see `products.rs`).
    pub fn assert_validator_allowlisted(&self, pool_account_id: &AccountId) {
        require!(
            self.validators.get(pool_account_id).is_some(),
            "Unknown validator"
        );
    }

    pub fn assert_validator_active_for_lock(&self, pool_account_id: &AccountId) {
        let v = self
            .validators
            .get(pool_account_id)
            .expect("Unknown validator");
        require!(
            v.status == ValidatorStatus::Active,
            "Validator not active for new locks"
        );
    }
}
