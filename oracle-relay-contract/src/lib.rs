//! Deploy at the staking contract’s configured `oracle_account_id`. [`OracleRelay::forward`] forwards
//! attached deposit and user-supplied `price_data`—**unsafe** if arbitrary users can call it with forged rows.
//!
//! **Mitigations:** set [`OracleRelay::forward_caller`] at deploy time to a single bot account that pulls
//! verified quotes off-chain or via XCC before calling `forward`; use NEAR access keys on the relay account
//! to restrict callers; or replace `forward` with an implementation that validates signatures / calls Burrow
//! on-chain before `oracle_on_call`.

use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{AccountId, Gas, Promise, env, ext_contract, near, require};

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
pub struct OracleRelay {
    /// If `Some`, only this account may invoke [`Self::forward`]. If `None`, any account may forward
    /// (localnet / tests only).
    pub forward_caller: Option<AccountId>,
}

impl Default for OracleRelay {
    fn default() -> Self {
        Self {
            forward_caller: None,
        }
    }
}

#[near]
impl OracleRelay {
    #[init]
    pub fn new(forward_caller: Option<AccountId>) -> Self {
        Self { forward_caller }
    }

    /// `receiver_id` = house-of-stake staking contract. Forwards full attached deposit and `msg` JSON.
    #[payable]
    pub fn forward(
        &mut self,
        receiver_id: AccountId,
        price_data: OraclePriceData,
        msg: String,
    ) -> Promise {
        if let Some(ref allowed) = self.forward_caller {
            require!(
                env::predecessor_account_id() == *allowed,
                "Only configured forward caller"
            );
        }
        let sender_id = env::predecessor_account_id();
        ext_staking::ext(receiver_id)
            .with_attached_deposit(env::attached_deposit())
            .with_static_gas(FORWARD_GAS)
            .oracle_on_call(sender_id, price_data, msg)
    }
}
