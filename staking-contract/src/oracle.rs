use crate::*;
use common::Fraction;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, ext_contract, near};

/// Oracle response: NEAR per 1 USD at `timestamp_ns`.
#[derive(Clone, Debug)]
#[near(serializers = [borsh, json])]
pub struct OraclePrice {
    pub near_per_usd: Fraction,
    pub timestamp_ns: U64,
}

#[ext_contract(ext_oracle)]
pub trait ExtOracle {
    fn get_price(&self) -> OraclePrice;
    fn get_price_30d_avg(&self) -> OraclePrice;
}

#[near]
impl Contract {
    pub fn get_oracle_account(&self) -> AccountId {
        self.config.oracle_account_id.clone()
    }
}
