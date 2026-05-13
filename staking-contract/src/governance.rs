use crate::*;
use near_sdk::assert_one_yocto;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, NearToken, env, near, require};

#[near]
impl Contract {
    #[payable]
    pub fn propose_new_owner_account_id(&mut self, new_owner_account_id: Option<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.proposed_new_owner_account_id = new_owner_account_id;
    }

    #[payable]
    pub fn accept_ownership(&mut self) {
        assert_one_yocto();
        let predecessor = env::predecessor_account_id();
        require!(
            self.config.proposed_new_owner_account_id.as_ref() == Some(&predecessor),
            "Only the proposed new owner can call this method"
        );
        self.config.owner_account_id = predecessor;
        self.config.proposed_new_owner_account_id = None;
    }

    #[payable]
    pub fn set_guardians(&mut self, guardians: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        self.config.guardians = guardians;
    }

    #[payable]
    pub fn set_per_lock_storage_stake(&mut self, per_lock_storage_stake: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.per_lock_storage_stake = per_lock_storage_stake;
    }

    #[payable]
    pub fn set_lock_bounds(&mut self, min_lock_duration_ns: U64, max_lock_duration_ns: U64) {
        assert_one_yocto();
        self.assert_owner();
        self.config.min_lock_duration_ns = min_lock_duration_ns;
        self.config.max_lock_duration_ns = max_lock_duration_ns;
    }

    #[payable]
    pub fn set_min_lock_amount(&mut self, min_lock_amount: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.min_lock_amount = min_lock_amount;
    }

    #[payable]
    pub fn set_min_storage_deposit(&mut self, min_storage_deposit: NearToken) {
        assert_one_yocto();
        self.assert_owner();
        self.config.min_storage_deposit = min_storage_deposit;
    }

    #[payable]
    pub fn set_epoch_unstake_settle_epochs(&mut self, epochs: u64) {
        assert_one_yocto();
        self.assert_owner();
        self.config.epoch_unstake_settle_epochs = epochs;
    }
}

impl Contract {
    pub fn assert_owner(&self) {
        require!(
            env::predecessor_account_id() == self.config.owner_account_id,
            "Only the contract owner can call this method"
        );
    }

    pub fn assert_guardian(&self) {
        let caller_id = env::predecessor_account_id();
        require!(
            self.config.guardians.contains(&caller_id) || caller_id == self.config.owner_account_id,
            "Only a guardian or the contract owner can call this method"
        );
    }
}
