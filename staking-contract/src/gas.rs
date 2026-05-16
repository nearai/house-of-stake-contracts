use near_sdk::Gas;

pub const BASE_GAS: Gas = Gas::from_gas(25_000_000_000_000);

pub mod staking_pool {
    use near_sdk::Gas;
    pub const DEPOSIT_AND_STAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const UNSTAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    /// Pull unstaked NEAR from pool (after `get_account_unstaked_balance`).
    pub const WITHDRAW: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const GET_ACCOUNT_TOTAL_BALANCE: Gas = super::BASE_GAS;
    pub const GET_ACCOUNT_UNSTAKED_BALANCE: Gas = super::BASE_GAS;
    /// View call to the pool's `get_owner_id`.
    pub const GET_OWNER_ID: Gas = super::BASE_GAS;
}

pub mod callbacks {
    use super::BASE_GAS;
    use near_sdk::Gas;
    pub const ON_DEPOSIT_AND_STAKE: Gas = BASE_GAS;
    pub const ON_UNSTAKE: Gas = BASE_GAS;
    /// Self-callback after net-zero stake/unstake pending (no pool deposit/unstake).
    pub const ON_SETTLE_NET_ZERO: Gas = BASE_GAS;
    /// May chain a second promise to `withdraw` on the pool.
    pub const ON_GET_UNSTAKED_FOR_WITHDRAW: Gas = Gas::from_gas(BASE_GAS.as_gas() * 4);
    pub const ON_WITHDRAW_TRANSFER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 2);
    pub const ON_TOTAL_BALANCE: Gas = BASE_GAS;
    /// Balance refresh then catalog mint, usage bumps, subscription index, and `try_epoch_settle_pool`.
    pub const ON_LOCK_REFRESH_THEN_FINALIZE: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// After `get_owner_id`: callback does a few storage writes (catalog or owner cache refresh).
    pub const ON_VALIDATOR_OWNER_CHECK: Gas = Gas::from_gas(BASE_GAS.as_gas() * 2);
    /// Tail dispatch after shared per-epoch settlement (`PerEpochContinue`).
    pub const ON_EPOCH_SETTLEMENT_DISPATCH: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// After pool `get_account_total_balance` during shared settlement (before lock / unlock).
    pub const ON_EPOCH_SETTLEMENT_AFTER_TOTAL_BALANCE: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// After pool `get_account_unstaked_balance` during shared settlement.
    pub const ON_EPOCH_SETTLEMENT_AFTER_UNSTAKED: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// After `try_epoch_settle` pool call during shared settlement.
    pub const ON_EPOCH_SETTLEMENT_AFTER_TRY_EPOCH_SETTLE: Gas =
        Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// After withdraw chain hop during shared settlement.
    pub const ON_EPOCH_SETTLEMENT_AFTER_WITHDRAW_CHAIN: Gas = Gas::from_gas(BASE_GAS.as_gas() * 6);
    /// Mint lock and optional post-settle after shared pipeline.
    pub const ON_LOCK_FINALLY_MINT: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// Unlock tail after pre-user settlement.
    pub const ON_UNLOCK_TAIL_AFTER_PRE_USER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
    /// User withdraw tail after shared per-epoch settlement (batch claim + transfer).
    pub const ON_WITHDRAW_USER_AFTER_EPOCH_SETTLEMENT: Gas = Gas::from_gas(BASE_GAS.as_gas() * 8);
}
