use near_sdk::Gas;

pub const BASE_GAS: Gas = Gas::from_gas(25_000_000_000_000);

pub mod staking_pool {
    use near_sdk::Gas;
    pub const DEPOSIT_AND_STAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const UNSTAKE: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const WITHDRAW_ALL: Gas = Gas::from_gas(super::BASE_GAS.as_gas() * 3);
    pub const GET_ACCOUNT_TOTAL_BALANCE: Gas = super::BASE_GAS;
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
    pub const ON_WITHDRAW: Gas = BASE_GAS;
    pub const ON_TOTAL_BALANCE: Gas = BASE_GAS;
}
