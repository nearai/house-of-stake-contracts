//! Share minting and pricing helpers.

use crate::{Currency, Price};
use common::{Fraction, U256};
use near_sdk::NearToken;

/// Fixed-point denominator for `Price.lock_factor_near_months`.
pub const LOCK_FACTOR_DENOM: u128 = 1_000_000_000_000_000_000_000_000;

/// Nanoseconds in one Gregorian day (`86400 * 10^9`).
pub const NS_PER_DAY: u128 = 86_400_000_000_000;
/// Average Gregorian month length in nanoseconds: `30.4375` days = `(487 / 16) * NS_PER_DAY`.
pub const AVG_MONTH_NS: u128 = NS_PER_DAY * 487 / 16;

pub fn effective_stake_yocto(total_staked_balance: NearToken, pending_to_stake: NearToken) -> u128 {
    total_staked_balance
        .as_yoctonear()
        .saturating_add(pending_to_stake.as_yoctonear())
}

/// Mint shares for a new deposit. First deposit: 1:1 shares to yocto.
pub fn mint_shares(total_shares: u128, effective_total: u128, deposit_yocto: u128) -> u128 {
    if total_shares == 0 || effective_total == 0 {
        return deposit_yocto;
    }
    let num = U256::from(deposit_yocto) * U256::from(total_shares);
    let den = U256::from(effective_total);
    (num / den).as_u128()
}

pub fn near_from_shares(shares: u128, effective_total: u128, total_shares: u128) -> u128 {
    if total_shares == 0 {
        return 0;
    }
    let num = U256::from(shares) * U256::from(effective_total);
    let den = U256::from(total_shares);
    (num / den).as_u128()
}

/// Enforces `locked_yocto * duration_ns >= required_near_months * AVG_MONTH_NS`
/// where `required_near_months = amount * lock_factor / LOCK_FACTOR_DENOM` for [`Currency::Near`].
pub fn check_near_price_lock(
    price: &Price,
    locked_yocto: u128,
    duration_ns: u128,
) -> Result<(), &'static str> {
    if price.currency != Currency::Near {
        return Err("expected Near price");
    }
    let required_nm = price
        .amount
        .0
        .saturating_mul(price.lock_factor_near_months.0)
        / LOCK_FACTOR_DENOM;
    let lhs = U256::from(locked_yocto) * U256::from(duration_ns);
    let rhs = U256::from(required_nm) * U256::from(AVG_MONTH_NS);
    if lhs >= rhs {
        Ok(())
    } else {
        Err("Insufficient locked NEAR or duration for this price")
    }
}

/// USD-priced: first convert USD amount to NEAR yocto via oracle fraction, then same inequality.
pub fn check_usd_price_lock(
    price: &Price,
    locked_yocto: u128,
    duration_ns: u128,
    near_per_usd: &Fraction,
) -> Result<(), &'static str> {
    if price.currency != Currency::Usd {
        return Err("expected Usd price");
    }
    let near_equiv = *near_per_usd * price.amount.0;
    let required_nm =
        near_equiv.saturating_mul(price.lock_factor_near_months.0) / LOCK_FACTOR_DENOM;
    let lhs = U256::from(locked_yocto) * U256::from(duration_ns);
    let rhs = U256::from(required_nm) * U256::from(AVG_MONTH_NS);
    if lhs >= rhs {
        Ok(())
    } else {
        Err("Insufficient locked NEAR or duration for this USD price")
    }
}

/// USD lock using a Burrow-style oracle row: `multiplier / 10^decimals` = **whole NEAR per 1.0 USD**.
/// Catalog [`Price::amount`] is **micro-USD** (10⁻⁶ USD), matching typical fiat pricing units.
pub fn check_usd_price_lock_burrow_row(
    price: &Price,
    locked_yocto: u128,
    duration_ns: u128,
    multiplier: u128,
    decimals: u8,
) -> Result<(), &'static str> {
    if price.currency != Currency::Usd {
        return Err("expected Usd price");
    }
    if decimals > 38 {
        return Err("oracle decimals too large");
    }
    let mut den = U256::from(1u8);
    for _ in 0..decimals {
        den = den * U256::from(10u8);
    }
    let den = den.max(U256::from(1u8));
    let near_per_usd_yocto =
        U256::from(multiplier).saturating_mul(U256::from(10u128.pow(24))) / den;
    let near_equiv =
        near_per_usd_yocto.saturating_mul(U256::from(price.amount.0)) / U256::from(1_000_000u128);
    let near_equiv_u128 = near_equiv.as_u128();
    let required_nm =
        near_equiv_u128.saturating_mul(price.lock_factor_near_months.0) / LOCK_FACTOR_DENOM;
    let lhs = U256::from(locked_yocto) * U256::from(duration_ns);
    let rhs = U256::from(required_nm) * U256::from(AVG_MONTH_NS);
    if lhs >= rhs {
        Ok(())
    } else {
        Err("Insufficient locked NEAR or duration for this USD price")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_first_deposit_one_to_one() {
        assert_eq!(
            mint_shares(0, 0, 1_000_000_000_000_000_000_000_000),
            1_000_000_000_000_000_000_000_000
        );
    }

    #[test]
    fn mint_pro_rata() {
        let s = mint_shares(2_000, 4_000, 1_000);
        assert_eq!(s, 500);
    }

    #[test]
    fn usd_burrow_row_rejects_huge_decimals() {
        use crate::types::{CatalogStatus, Currency, Price, PriceType};
        use near_sdk::json_types::U128;

        let price = Price {
            price_id: "p1".to_string(),
            product_id: "pr1".to_string(),
            name: String::new(),
            description: String::new(),
            currency: Currency::Usd,
            amount: U128(1_000_000),
            price_type: PriceType::OneOff,
            billing_period: None,
            lock_factor_near_months: U128(LOCK_FACTOR_DENOM / 10),
            status: CatalogStatus::Active,
            usage_count: 0,
        };
        let r = check_usd_price_lock_burrow_row(
            &price,
            1_000_000_000_000_000_000_000_000,
            AVG_MONTH_NS,
            1,
            50,
        );
        assert_eq!(r, Err("oracle decimals too large"));
    }

    #[test]
    fn usd_burrow_row_satisfied_on_generous_lock() {
        use crate::types::{CatalogStatus, Currency, Price, PriceType};
        use near_sdk::json_types::U128;

        let price = Price {
            price_id: "p1".to_string(),
            product_id: "pr1".to_string(),
            name: String::new(),
            description: String::new(),
            currency: Currency::Usd,
            amount: U128(1_000_000),
            price_type: PriceType::OneOff,
            billing_period: None,
            lock_factor_near_months: U128(LOCK_FACTOR_DENOM / 10),
            status: CatalogStatus::Active,
            usage_count: 0,
        };
        // 1 NEAR per 1 USD, 1 micro-USD notional, long duration → pass
        assert!(
            check_usd_price_lock_burrow_row(&price, 10u128.pow(24), AVG_MONTH_NS * 12, 1, 0,)
                .is_ok()
        );
    }
}
