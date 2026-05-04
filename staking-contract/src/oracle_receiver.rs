//! Burrow-style oracle callback: only [`crate::config::Config::oracle_account_id`] may call
//! `oracle_on_call` on this contract. Use an oracle relay (see `ACTION_ITEMS.md` in this crate) that
//! forwards the user as `sender_id` and attaches the user’s NEAR deposit to this call.

use crate::internal::check_usd_price_lock_burrow_row;
use crate::*;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{env, near, require, AccountId};

/// Compatible with Burrow `contracts/common` [`PriceData`] JSON shape (subset).
#[derive(Clone, Deserialize, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OraclePriceData {
    /// Block time of the quote (nanoseconds), as in Burrow.
    pub timestamp: u64,
    /// Carried for Burrow compatibility; not enforced beyond future-staleness checks.
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
    /// Burrow JSON uses decimal strings for large integers.
    pub multiplier: String,
    pub decimals: u8,
}

/// JSON body forwarded by the relay (see module docs).
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct LockForProductUsdMsg {
    pub price_id: PriceId,
    pub lock_duration_ns: u64,
}

impl Contract {
    pub(crate) fn validate_oracle_price_data(&self, data: &OraclePriceData) {
        let now = env::block_timestamp();
        require!(
            data.timestamp <= now,
            "Oracle price timestamp is in the future"
        );
        let max_age = self.config.oracle_max_age_ns.0;
        require!(
            now - data.timestamp <= max_age,
            "Oracle price data is too stale"
        );
    }
}

#[near]
impl Contract {
    /// Finish a USD-priced lock using oracle-supplied NEAR/USD (Burrow-style `PriceData`) + JSON intent.
    /// **Attach** the NEAR to lock (relay must forward attached deposit from the user).
    #[payable]
    pub fn oracle_on_call(&mut self, sender_id: AccountId, price_data: OraclePriceData, msg: String) {
        require!(
            env::predecessor_account_id() == self.config.oracle_account_id,
            "Only the configured oracle account may call oracle_on_call"
        );
        self.assert_not_paused();
        self.validate_oracle_price_data(&price_data);

        let intent = serde_json::from_str::<LockForProductUsdMsg>(&msg)
            .unwrap_or_else(|_| env::panic_str("invalid oracle msg JSON"));

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached deposit below min_lock_amount"
        );
        require!(
            intent.lock_duration_ns >= self.config.min_lock_duration_ns.0
                && intent.lock_duration_ns <= self.config.max_lock_duration_ns.0,
            "lock_duration_ns out of bounds"
        );

        let row = price_data
            .prices
            .iter()
            .find_map(|p| p.price.as_ref())
            .unwrap_or_else(|| env::panic_str("PriceData must include at least one price"));

        let price = self
            .prices
            .get(&intent.price_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown price"));
        let product = self
            .products
            .get(&price.product_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Unknown product"));

        require!(price.status == CatalogStatus::Active, "Price not active");
        require!(
            product.status == CatalogStatus::Active,
            "Product not active"
        );
        require!(
            price.currency == Currency::Usd,
            "Price must be USD for oracle_on_call"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);
        self.ensure_min_storage(&sender_id);

        let dur_u128 = u128::from(intent.lock_duration_ns);
        let mult = row.multiplier.parse::<u128>().unwrap_or_else(|_| {
            env::panic_str("oracle price multiplier must be a decimal integer string")
        });
        check_usd_price_lock_burrow_row(
            &price,
            locked.as_yoctonear(),
            dur_u128,
            mult,
            row.decimals,
        )
        .unwrap_or_else(|e| env::panic_str(e));

        let _ = self.finalize_product_lock(
            sender_id,
            intent.price_id,
            price,
            product,
            locked,
            dur_u128,
        );
    }
}
