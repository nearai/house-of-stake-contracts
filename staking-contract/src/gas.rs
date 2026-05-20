use near_sdk::Gas;

pub const BASE_GAS: Gas = Gas::from_gas(25_000_000_000_000);

pub mod staking_pool {
    use near_sdk::Gas;
    pub const DEPOSIT_AND_STAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const UNSTAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    /// Pull unstaked NEAR from pool into this contract.
    pub const WITHDRAW: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    /// Pool `get_account` (staked + unstaked + `can_withdraw` in one view).
    pub const GET_ACCOUNT: Gas = super::BASE_GAS;
    /// View call to the pool's `get_owner_id`.
    pub const GET_OWNER_ID: Gas = super::BASE_GAS;
}

pub mod callbacks {
    use super::BASE_GAS;
    use near_sdk::Gas;
    pub const ON_DEPOSIT_AND_STAKE: Gas = BASE_GAS;
    pub const ON_UNSTAKE: Gas = BASE_GAS;
    /// After pool `withdraw`: tail settle (`None`) or settlement `try_epoch_stake_or_unstake` + dispatch (`Some(cont)`).
    pub const ON_GET_UNSTAKED_FOR_WITHDRAW: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    pub const ON_WITHDRAW_TRANSFER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 2);
    pub const ON_TOTAL_BALANCE: Gas = BASE_GAS;
    /// Balance refresh then catalog mint, usage bumps, subscription index, and `try_epoch_stake_or_unstake`.
    pub const ON_LOCK_REFRESH_THEN_FINALIZE: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// After `get_owner_id`: callback does a few storage writes (catalog or owner cache refresh).
    pub const ON_VALIDATOR_OWNER_CHECK: Gas = Gas::from_gas(BASE_GAS.as_gas() * 2);
    /// Tail dispatch after shared per-epoch settlement (`PerEpochContinue`).
    pub const ON_EPOCH_SETTLEMENT_DISPATCH: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// After pool `get_account` during shared settlement (balance refresh + withdraw-if-ready).
    pub const ON_EPOCH_SETTLEMENT_AFTER_POOL_ACCOUNT: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// After `try_epoch_stake_or_unstake` pool call during shared settlement.
    ///
    /// This callback only bridges into the dispatch callback, so keep this budget tight.
    /// A smaller value here avoids exhausting gas in the preceding settlement callback
    /// when it schedules `deposit_and_stake/unstake` plus follow-up callbacks.
    pub const ON_EPOCH_SETTLEMENT_AFTER_TRY_EPOCH_STAKE_OR_UNSTAKE: Gas = BASE_GAS;
    /// Mint lock after shared pre-user settlement pipeline.
    pub const ON_LOCK_FINALLY_MINT: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// Unlock tail after pre-user settlement.
    pub const ON_UNLOCK_TAIL_AFTER_PRE_USER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// Withdraw tail after pre-user settlement.
    pub const ON_WITHDRAW_TAIL_AFTER_PRE_USER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// Subscription upgrade after pre-user settlement.
    pub const ON_SUBSCRIPTION_UPGRADE_AFTER_SETTLE: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// After user-flow tail promise completes: release pipeline `Busy`.
    pub const ON_EPOCH_PIPELINE_TERMINAL_RELEASE: Gas = BASE_GAS;
    /// Release pipeline `Busy` and pass lock id through to caller.
    pub const ON_EPOCH_PIPELINE_RELEASE_WITH_LOCK_ID: Gas = BASE_GAS;
}
