//! Share minting and pricing helpers.

use crate::Price;
use common::U256;
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
/// where `required_near_months = amount * lock_factor / LOCK_FACTOR_DENOM`.
///
/// [`Price::amount`] is **yoctoNEAR** for the catalog line item.
pub fn check_near_price_lock(
    price: &Price,
    locked_yocto: u128,
    duration_ns: u128,
) -> Result<(), &'static str> {
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
}
