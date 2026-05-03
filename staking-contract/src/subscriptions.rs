//! Calendar-month helpers for subscription end dates (Stripe-style). Used by future `lock_for_subscription`.

/// Average Gregorian month length in nanoseconds: 30.4375 days.
pub const AVG_MONTH_NS: u128 = 2_629_746_000_000_000_000;

/// Stub until calendar-month math lands: extends by `months * AVG_MONTH_NS`.
pub fn add_months_stripe_style(_anchor_day: u8, months: u32, from_ns: u64) -> u64 {
    let _ = _anchor_day;
    from_ns.saturating_add((months as u128).saturating_mul(AVG_MONTH_NS) as u64)
}
