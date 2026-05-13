//! Share minting and pricing helpers.
//!
//! Time constants: [`NS_PER_DAY`] is `u128` for fixed-point price math; [`NS_PER_DAY_TIMESTAMP`] is the same
//! nanosecond length as `u64` for block timestamps (subscription billing anchors in `lock.rs`).

use crate::Price;
use common::U256;
use near_sdk::NearToken;

/// Fixed-point denominator for `Price.lock_factor_near_months`.
pub const LOCK_FACTOR_DENOM: u128 = 1_000_000_000_000_000_000_000_000;

/// Nanoseconds in one Gregorian day (`86400 * 10^9`), as `u128` (price lock and share math).
pub const NS_PER_DAY: u128 = 86_400_000_000_000;
/// Same interval as [`NS_PER_DAY`], as `u64`, for `u64` block timestamps (e.g. billing anchor day).
pub const NS_PER_DAY_TIMESTAMP: u64 = 86_400_000_000_000;
/// Average Gregorian month length in nanoseconds: `30.4375` days = `(487 / 16) * NS_PER_DAY`.
pub const AVG_MONTH_NS: u128 = NS_PER_DAY * 487 / 16;

/// Pro-rata credit from one withdraw batch toward a user: `floor(remaining * eligible / liability)`
/// capped by `eligible` and `remaining`, plus a **1 yocto** minimum when all inputs are positive but the
/// floor rounds to zero (matches [`crate::withdraw::Contract::claim_unlocked_near`]).
pub fn withdraw_batch_credit_yocto(
    batch_remaining_yocto: u128,
    user_eligible_yocto: u128,
    liability_at_fund_yocto: u128,
) -> u128 {
    if batch_remaining_yocto == 0 || user_eligible_yocto == 0 || liability_at_fund_yocto == 0 {
        return 0;
    }
    let credit_raw = (U256::from(batch_remaining_yocto) * U256::from(user_eligible_yocto))
        / U256::from(liability_at_fund_yocto);
    let mut credit_yocto = credit_raw
        .as_u128()
        .min(user_eligible_yocto)
        .min(batch_remaining_yocto);
    if credit_yocto == 0 {
        credit_yocto = 1.min(user_eligible_yocto).min(batch_remaining_yocto);
    }
    credit_yocto
}

pub fn effective_stake_yocto(total_staked_balance: NearToken, pending_to_stake: NearToken) -> u128 {
    total_staked_balance
        .as_yoctonear()
        .saturating_add(pending_to_stake.as_yoctonear())
}

/// NEAR backing **remaining** circulating shares: gross effective stake minus **all** user exit liability
/// ([`crate::validators::Validator::pending_user_unstake_total`]) — NEAR already allocated to burned shares
/// until users claim, whether it still sits in `pending_to_unstake`, unstaked in the pool, or in
/// `pending_to_withdraw`.
///
/// **Solvency:** [`crate::internal::near_from_shares`] must not use gross `effective_stake_yocto` alone after
/// shares burn down. Subtracting only [`crate::validators::Validator::pending_to_unstake`] is insufficient:
/// that field drops after a successful pool unstake while user liability remains until claims, which would
/// let later exits re-price against the same gross. Using the full user liability total keeps exits and
/// mints aligned with the same net backing.
pub fn effective_stake_for_share_exit(
    total_staked_balance: NearToken,
    pending_to_stake: NearToken,
    pending_user_unstake_total: NearToken,
) -> u128 {
    effective_stake_yocto(total_staked_balance, pending_to_stake)
        .saturating_sub(pending_user_unstake_total.as_yoctonear())
}

/// Mint shares for a new deposit. First deposit: 1:1 shares to yocto.
///
/// When `total_shares > 0`, callers must ensure `effective_total > 0` or rounding will mis-price mints
/// (see [`effective_stake_for_share_exit`] and guards in [`crate::lock`]).
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
/// Smallest integer `locked_yocto` such that [`check_near_price_lock`] passes for `(price, duration_ns)`.
/// Used for tier-gap surplus when downgrading subscriptions (Phase B prorate).
pub fn min_locked_yocto_for_duration(price: &Price, duration_ns: u128) -> u128 {
    if duration_ns == 0 {
        return 0;
    }
    let required_nm = price
        .amount
        .0
        .saturating_mul(price.lock_factor_near_months.0)
        / LOCK_FACTOR_DENOM;
    let rhs = U256::from(required_nm) * U256::from(AVG_MONTH_NS);
    let duration_u256 = U256::from(duration_ns);
    let quotient = rhs / duration_u256;
    let remainder = rhs % duration_u256;
    let mut min_locked_yocto = quotient.as_u128();
    if !remainder.is_zero() {
        min_locked_yocto = min_locked_yocto.saturating_add(1);
    }
    min_locked_yocto
}

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
        Err("Locked NEAR or lock duration is too low for this catalog price")
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

    /// Two half-share exits in one epoch must not exceed one full exit (no double-count gross pool).
    #[test]
    fn sequential_share_exit_net_effective_totals_to_gross() {
        let gross = 100u128;
        let ts0 = 100u128;
        let sh = 50u128;
        let user_liab0 = 0u128;
        let eff1 = gross.saturating_sub(user_liab0);
        let n1 = near_from_shares(sh, eff1, ts0);
        let user_liab1 = user_liab0.saturating_add(n1);
        let ts1 = ts0.saturating_sub(sh);
        let eff2 = gross.saturating_sub(user_liab1);
        let n2 = near_from_shares(sh, eff2, ts1);
        assert_eq!(n1.saturating_add(n2), gross);
    }

    #[test]
    fn min_locked_matches_price_check_boundary() {
        use crate::types::{BillingPeriod, CatalogStatus, Price, PriceType};
        use near_sdk::json_types::U128;
        let price = Price {
            price_id: "price_x".into(),
            product_id: "prod_x".into(),
            name: "".into(),
            description: "".into(),
            amount: U128(100),
            price_type: PriceType::Recurring,
            billing_period: Some(BillingPeriod::Monthly),
            lock_factor_near_months: U128(LOCK_FACTOR_DENOM),
            status: CatalogStatus::Active,
            usage_count: 0,
        };
        let d: u128 = 1_000_000_000_000;
        let m = min_locked_yocto_for_duration(&price, d);
        assert!(check_near_price_lock(&price, m, d).is_ok());
        assert!(m > 0);
        if m > 1 {
            assert!(check_near_price_lock(&price, m - 1, d).is_err());
        }
    }

    #[test]
    fn withdraw_batch_credit_pro_rata_rounding() {
        assert_eq!(withdraw_batch_credit_yocto(100, 30, 100), 30);
    }

    #[test]
    fn withdraw_batch_credit_tiny_bucket_dust_minimum() {
        assert_eq!(withdraw_batch_credit_yocto(1, 1, 2), 1);
    }

    #[test]
    fn withdraw_batch_credit_large_product_uses_u256() {
        let w_y: u128 = 1u128 << 64;
        let o_y: u128 = 1u128 << 64;
        let t_y: u128 = 1u128 << 64;
        assert!(
            w_y.checked_mul(o_y).is_none(),
            "sanity: product overflows u128; math must use U256"
        );
        assert_eq!(withdraw_batch_credit_yocto(w_y, o_y, t_y), 1u128 << 64);
    }

    #[test]
    fn withdraw_batch_credit_zero_when_any_operand_zero() {
        assert_eq!(withdraw_batch_credit_yocto(0, 1, 1), 0);
        assert_eq!(withdraw_batch_credit_yocto(1, 0, 1), 0);
        assert_eq!(withdraw_batch_credit_yocto(1, 1, 0), 0);
    }
}
