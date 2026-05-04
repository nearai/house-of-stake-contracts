//! Deploy at the staking contract’s configured `oracle_account_id`. [`OracleRelay::forward`] forwards
//! attached deposit and user-supplied `price_data`—**unsafe** if arbitrary users can call it: they can
//! forge oracle rows. Production: restrict callers, verify signatures, or replace `forward` with an
//! implementation that pulls quotes via XCC from the real Burrow/oracle contract before calling
//! `oracle_on_call`.

use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{AccountId, Gas, Promise, env, ext_contract, near};

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
    fn oracle_on_call(&mut self, sender_id: AccountId, price_data: OraclePriceData, msg: String);
}

const FORWARD_GAS: Gas = Gas::from_tgas(200);

#[near(contract_state)]
pub struct OracleRelay {}

impl Default for OracleRelay {
    fn default() -> Self {
        Self::new()
    }
}

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
