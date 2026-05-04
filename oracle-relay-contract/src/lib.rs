//! Deploy this contract at [`staking_contract::Config::oracle_account_id`]. Users call
//! [`OracleRelay::forward`] with attached deposit; the relay invokes `oracle_on_call` on the staking
//! contract with `sender_id` set to the **caller** (Burrow test-oracle pattern).

use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{env, ext_contract, near, AccountId, Gas, Promise};

/// Must match `staking-contract` `OraclePriceData` JSON (subset of Burrow `PriceData`).
#[derive(Clone, Deserialize, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OraclePriceData {
    pub timestamp: u64,
    pub recency_duration_sec: u32,
    pub prices: Vec<OracleAssetOptionalPrice>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OracleAssetOptionalPrice {
    pub asset_id: String,
    pub price: Option<OracleBurrowPrice>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OracleBurrowPrice {
    pub multiplier: String,
    pub decimals: u8,
}

#[ext_contract(ext_staking)]
pub trait StakingOracle {
    fn oracle_on_call(
        &mut self,
        sender_id: AccountId,
        price_data: OraclePriceData,
        msg: String,
    );
}

const FORWARD_GAS: Gas = Gas::from_tgas(200);

#[near(contract_state)]
pub struct OracleRelay {}

#[near]
impl OracleRelay {
    #[init]
    pub fn new() -> Self {
        Self {}
    }

    /// `receiver_id` = house-of-stake staking contract. Forwards full attached deposit and `msg` JSON.
    #[payable]
    pub fn forward(
        &mut self,
        receiver_id: AccountId,
        price_data: OraclePriceData,
        msg: String,
    ) -> Promise {
        let sender_id = env::predecessor_account_id();
        ext_staking::ext(receiver_id)
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(FORWARD_GAS)
            .oracle_on_call(sender_id, price_data, msg)
    }
}
