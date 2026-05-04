//! Calendar-month helpers for subscription end dates (Stripe-style). Used by [`crate::lock`] subscription locks.

pub use crate::internal::AVG_MONTH_NS;

/// Extend `from_ns` by `months` × average Gregorian months (linear approximation).
///
/// `anchor_day` (1–31) is the Stripe-style **billing-cycle day-of-month** hint for future calendar-accurate
/// billing (end-of-month clamping, leap years). It is validated but **not yet** applied in this helper until a
/// full calendar implementation lands; see `docs/ACTION_ITEMS.md` (subscriptions section).
pub fn add_months_stripe_style(anchor_day: u8, months: u32, from_ns: u64) -> u64 {
    let _anchor_day = anchor_day.clamp(1, 31);
    let add_ns = (months as u128).saturating_mul(AVG_MONTH_NS);
    let add_u64 = u64::try_from(add_ns).unwrap_or(u64::MAX);
    from_ns.saturating_add(add_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_month_stack() {
        let out = add_months_stripe_style(15, 2, 100);
        assert_eq!(out, 100 + (2u128 * AVG_MONTH_NS) as u64);
    }
}
