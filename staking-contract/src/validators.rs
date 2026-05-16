use crate::*;
use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, NearToken, env, near, require};

#[derive(Clone)]
#[near(serializers = [borsh, json])]
pub struct Validator {
    /// Staking pool contract account for this validator row (= catalog `validator_id` / lock `validator_id`).
    pub validator_id: ValidatorId,
    /// Whether new locks are allowed (**`Active`**) or blocked (**`Paused`**), or the row is **`Removed`**.
    pub status: ValidatorStatus,

    /// Total issued stake.dao **share units** for this pool (integer; same scale as per-user shares).
    pub total_shares: U128,
    /// Cached **staked NEAR** for this contract’s account on the pool, from the last successful
    /// `get_account_total_balance` (plus bookkeeping updates on stake/unstake/withdraw paths). Used with
    /// `pending_*` for share mint/burn pricing—not updated until the next pool read or accounting step.
    pub total_staked_balance: NearToken,
    /// `block_timestamp` (nanoseconds) when `total_staked_balance` was last synced from the pool (or row created).
    pub last_balance_refresh_ns: U64,

    /// NEAR waiting to be sent to the pool via **`deposit_and_stake`** (aggregated locks; net-settled vs
    /// `pending_to_unstake` in `Contract::try_epoch_settle` in `epoch.rs`).
    pub pending_to_stake: NearToken,
    /// NEAR queued to leave the pool via **`unstake`** (user unlocks etc.; net-settled vs `pending_to_stake`).
    pub pending_to_unstake: NearToken,
    /// Epoch height recorded after the last successful pool `unstake` callback; gates further unstakes
    /// (with [`crate::config::Config::epoch_unstake_settle_epochs`]).
    pub last_unstake_epoch: u64,
    /// Last NEAR `epoch_height` for which this row completed the **pre–user-action** pipeline for a request:
    /// **sync** `total_staked_balance` from the pool (at most once per epoch for catalog flows), **withdraw**
    /// from the pool when eligible, then at most one **net** pool `deposit_and_stake` / `unstake` / net-zero
    /// clearance for that epoch (same mutex the staking pool enforces per account). Successful callbacks
    /// on stake, unstake, and net-zero settlement set this to `env::epoch_height()`. When this equals the
    /// current epoch, user flows **skip** another `get_account_total_balance` for this validator until the
    /// next NEAR epoch.
    pub last_settlement_epoch: u64,
    /// NEAR that has been **`withdraw`**n from the pool into this contract and sits in the claim bucket until
    /// users call **`withdraw`** (epoch-gated tranches).
    pub pending_to_withdraw: NearToken,
    /// Sum of all user **`user_pending_unstake`** tranche amounts for this pool; must stay consistent with
    /// claims and **`pending_to_withdraw`**.
    pub pending_user_unstake_total: NearToken,
    /// Accounts that currently have at least one non-empty tranche in **`user_pending_unstake`** for this pool.
    pub accounts_with_pending_unstake: Vec<AccountId>,

    /// At most one in-flight cross-contract **mutating** pool pipeline for this row (`Idle` vs `Busy`).
    pub tx_status: TransactionStatus,
}

#[near]
impl Contract {
    /// Contract owner: add a validator pool to the allowlist. Pool ownership for catalog operations is
    /// always verified via `get_owner_id()` on the pool ([`crate::products`]).
    #[payable]
    pub fn add_validator(&mut self, validator_id: ValidatorId) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        require!(
            self.validators.get(&validator_id).is_none(),
            "Validator already exists"
        );

        let new_validator = Validator {
            validator_id: validator_id.clone(),
            status: ValidatorStatus::Active,
            total_shares: U128(0),
            total_staked_balance: NearToken::from_near(0),
            last_balance_refresh_ns: U64(env::block_timestamp()),
            pending_to_stake: NearToken::from_near(0),
            pending_to_unstake: NearToken::from_near(0),
            last_unstake_epoch: 0,
            last_settlement_epoch: 0,
            pending_to_withdraw: NearToken::from_near(0),
            pending_user_unstake_total: NearToken::from_near(0),
            accounts_with_pending_unstake: Vec::new(),
            tx_status: TransactionStatus::Idle,
        };
        self.validators.insert(validator_id.clone(), new_validator);
        self.validator_ids.push(validator_id.clone());
        crate::events::log_validator_added(&validator_id);
    }

    pub fn get_validator(&self, validator_id: ValidatorId) -> Option<Validator> {
        self.validators.get(&validator_id).cloned()
    }

    /// Paginated validator records (stable allowlist order in [`Contract::validator_ids`]).
    pub fn get_validators(&self, from_index: u64, limit: u64) -> Vec<Validator> {
        let len_u64 = self.validator_ids.len() as u64;
        let mut out = Vec::new();
        let mut i = from_index;
        while i < len_u64 && (out.len() as u64) < limit {
            if let Some(id) = self.validator_ids.get(i as u32) {
                if let Some(validator) = self.validators.get(id).cloned() {
                    out.push(validator);
                }
            }
            i += 1;
        }
        out
    }

    #[payable]
    pub fn pause_validator(&mut self, validator_id: ValidatorId) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        let mut validator = self.require_validator(&validator_id);
        validator.status = ValidatorStatus::Paused;
        self.validators.insert(validator_id, validator);
    }

    #[payable]
    pub fn remove_validator(&mut self, validator_id: ValidatorId) {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        let mut validator = self.require_validator(&validator_id);
        require!(
            validator.total_shares.0 == 0
                && validator.pending_to_stake.as_yoctonear() == 0
                && validator.pending_to_unstake.as_yoctonear() == 0
                && validator.pending_to_withdraw.as_yoctonear() == 0
                && validator.pending_user_unstake_total.as_yoctonear() == 0,
            "Cannot remove this validator: all stake, pending stake and unstake, withdraw bucket, and user claims must be cleared first"
        );
        validator.status = ValidatorStatus::Removed;
        self.validators.insert(validator_id, validator);
    }
}

impl Contract {
    /// Pool must be on the allowlist. Catalog methods confirm the caller against the pool's
    /// `get_owner_id()` via a cross-contract call (see `products.rs` and `prices.rs`).
    pub fn assert_validator_allowlisted(&self, validator_id: &ValidatorId) {
        require!(
            self.validators.get(validator_id).is_some(),
            "Validator not found on the allowlist"
        );
    }

    pub fn assert_validator_active_for_lock(&self, validator_id: &ValidatorId) {
        let validator = self.require_validator(validator_id);
        require!(
            validator.status == ValidatorStatus::Active,
            "This validator is paused or removed; new locks are not allowed on it"
        );
    }

    pub(crate) fn require_validator(&self, validator_id: &ValidatorId) -> Validator {
        self.validators
            .get(validator_id)
            .cloned()
            .expect("Validator not found on the allowlist")
    }

    pub(crate) fn require_validator_callback(&self, validator_id: &ValidatorId) -> Validator {
        self.validators
            .get(validator_id)
            .cloned()
            .expect("Validator not found on allowlist (validator callback)")
    }

    /// True once a pool `unstake` is on record and [`crate::config::Config::epoch_unstake_settle_epochs`]
    /// have passed since [`Validator::last_unstake_epoch`] (gates withdraw-from-pool and further unstakes).
    pub(crate) fn validator_unstake_waiting_finished(&self, validator: &Validator) -> bool {
        validator.last_unstake_epoch > 0
            && env::epoch_height()
                >= validator
                    .last_unstake_epoch
                    .saturating_add(self.config.epoch_unstake_settle_epochs)
    }
}
