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
}

pub mod oracle {
    use near_sdk::Gas;
    pub const GET_PRICE: Gas = super::BASE_GAS;
}

pub mod callbacks {
    use near_sdk::Gas;
    use super::BASE_GAS;
    pub const ON_DEPOSIT_AND_STAKE: Gas = BASE_GAS;
    pub const ON_UNSTAKE: Gas = BASE_GAS;
    /// May chain a second promise to `withdraw` on the pool.
    pub const ON_GET_UNSTAKED_FOR_WITHDRAW: Gas = Gas::from_gas(BASE_GAS.as_gas() * 4);
    pub const ON_WITHDRAW_TRANSFER: Gas = Gas::from_gas(BASE_GAS.as_gas() * 2);
    pub const ON_TOTAL_BALANCE: Gas = BASE_GAS;
}
