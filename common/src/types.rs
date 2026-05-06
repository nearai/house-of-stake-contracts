use crate::venear::VenearGrowthConfig;
use crate::*;
use near_sdk::json_types::{U128, U64};
use near_sdk::require;
use std::cmp::Ordering;
use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};

#[allow(clippy::manual_div_ceil)]
mod uints {
    uint::construct_uint!(
        pub struct U256(4);
    );

    uint::construct_uint!(
        pub struct U384(6);
    );
}

pub use uints::{U256, U384};

/// The timestamp in nanoseconds. It serializes as a string for JSON.
pub type TimestampNs = U64;

/// The version of the contract. It is a monotonically increasing number.
pub type Version = u64;

/// Represents balance of NEAR and veNEAR tokens. NEAR tokens grow over time, while veNEAR tokens
/// do not.
#[derive(Copy, Clone, Default)]
#[near(serializers=[borsh, json])]
pub struct VenearBalance {
    /// The balance in NEAR tokens. This balance doesn't grow over time.
    pub near_balance: NearToken,

    /// The balance in veNEAR tokens. This balance does grow over time.
    pub extra_venear_balance: NearToken,
}

impl VenearBalance {
    pub fn total(&self) -> NearToken {
        near_add(self.near_balance, self.extra_venear_balance)
    }

    pub fn update(
        &mut self,
        previous_timestamp: TimestampNs,
        current_timestamp: TimestampNs,
        venear_growth_config: &VenearGrowthConfig,
    ) {
        self.extra_venear_balance = near_add(
            self.extra_venear_balance,
            venear_growth_config.calculate(
                previous_timestamp,
                current_timestamp,
                self.near_balance,
            ),
        );
    }

    pub fn from_near(near_balance: NearToken) -> Self {
        Self {
            near_balance,
            extra_venear_balance: NearToken::from_yoctonear(0),
        }
    }

    pub fn scale_by_bps(&self, bps: u16) -> Self {
        require!(bps <= 10_000, "bps must be <= 10000");
        if bps == 0 {
            return Self::default();
        }
        if bps == 10_000 {
            return *self;
        }
        let bps_u128 = bps as u128;
        Self {
            near_balance: NearToken::from_yoctonear(
                self.near_balance.as_yoctonear() * bps_u128 / 10_000
            ),
            extra_venear_balance: NearToken::from_yoctonear(
                self.extra_venear_balance.as_yoctonear() * bps_u128 / 10_000
            ),
        }
    }
}

impl Add<Self> for VenearBalance {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            near_balance: near_add(self.near_balance, rhs.near_balance),
            extra_venear_balance: near_add(self.extra_venear_balance, rhs.extra_venear_balance),
        }
    }
}

impl Sub<Self> for VenearBalance {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            near_balance: near_sub(self.near_balance, rhs.near_balance),
            extra_venear_balance: near_sub(self.extra_venear_balance, rhs.extra_venear_balance),
        }
    }
}

impl AddAssign<Self> for VenearBalance {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign<Self> for VenearBalance {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

/// Represents balance of NEAR and veNEAR tokens that are pooled together. The `near_balance` is
/// truncated to milliNEAR for every added `VenearBalance` to avoid rounding errors
/// during `extra_venear_balance` growth calculations. The truncated `near_balance` is added to
/// `extra_venear_balance` to ensure that the total balance remains consistent.
#[derive(Copy, Clone, Default)]
#[near(serializers=[borsh, json])]
pub struct PooledVenearBalance(VenearBalance);

impl PooledVenearBalance {
    pub fn total(&self) -> NearToken {
        self.0.total()
    }

    pub fn update(
        &mut self,
        previous_timestamp: TimestampNs,
        current_timestamp: TimestampNs,
        venear_growth_config: &VenearGrowthConfig,
    ) {
        self.0
            .update(previous_timestamp, current_timestamp, venear_growth_config);
    }

    pub fn pooled_add(&self, other: &VenearBalance) -> Self {
        let truncated_near_balance = truncate_near_to_millis(other.near_balance);
        let difference = near_sub(other.near_balance, truncated_near_balance);
        Self(VenearBalance {
            near_balance: near_add(self.0.near_balance, truncated_near_balance),
            extra_venear_balance: near_add(
                self.0.extra_venear_balance,
                near_add(other.extra_venear_balance, difference),
            ),
        })
    }

    pub fn pooled_sub(&self, other: &VenearBalance) -> Self {
        let truncated_near_balance = truncate_near_to_millis(other.near_balance);
        let difference = near_sub(other.near_balance, truncated_near_balance);
        Self(VenearBalance {
            near_balance: near_sub(self.0.near_balance, truncated_near_balance),
            extra_venear_balance: near_sub(
                self.0.extra_venear_balance,
                near_add(other.extra_venear_balance, difference),
            ),
        })
    }

    pub fn pooled_add_scaled(&self, other: &VenearBalance, bps: u16) -> Self {
        let scaled = other.scale_by_bps(bps);
        self.pooled_add(&scaled)
    }

    pub fn pooled_sub_scaled(&self, other: &VenearBalance, bps: u16) -> Self {
        let scaled = other.scale_by_bps(bps);
        self.pooled_sub(&scaled)
    }
}

#[derive(Clone, Copy, Debug)]
#[near(serializers=[borsh, json])]
pub struct Fraction {
    pub numerator: U128,
    pub denominator: U128,
}

impl PartialEq<Self> for Fraction {
    fn eq(&self, other: &Self) -> bool {
        U256::from(self.numerator.0) * U256::from(other.denominator.0)
            == U256::from(self.denominator.0) * U256::from(other.numerator.0)
    }
}

impl PartialOrd for Fraction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (U256::from(self.numerator.0) * U256::from(other.denominator.0))
            .partial_cmp(&(U256::from(self.denominator.0) * U256::from(other.numerator.0)))
    }
}

impl Mul<u128> for Fraction {
    type Output = u128;

    fn mul(self, rhs: u128) -> Self::Output {
        let numerator = U256::from(self.numerator.0) * U256::from(rhs);
        let denominator = U256::from(self.denominator.0);
        (numerator / denominator).as_u128()
    }
}

impl Fraction {
    pub fn u384_mul(&self, a: u128, b: u128) -> u128 {
        let numerator = U384::from(self.numerator.0) * U384::from(a) * U384::from(b);
        let denominator = U384::from(self.denominator.0);
        // Ensure that the multiplication does not introduce rounding errors.
        require!(numerator % denominator == U384::from(0), "Rounding error");
        (numerator / denominator).as_u128()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_by_bps_zero_returns_zero() {
        let balance = VenearBalance {
            near_balance: NearToken::from_near(100),
            extra_venear_balance: NearToken::from_near(50),
        };
        let scaled = balance.scale_by_bps(0);
        assert_eq!(scaled.near_balance.as_yoctonear(), 0);
        assert_eq!(scaled.extra_venear_balance.as_yoctonear(), 0);
    }

    #[test]
    fn scale_by_bps_full_returns_full() {
        let balance = VenearBalance {
            near_balance: NearToken::from_near(100),
            extra_venear_balance: NearToken::from_near(50),
        };
        let scaled = balance.scale_by_bps(10_000);
        assert_eq!(scaled.near_balance.as_yoctonear(), balance.near_balance.as_yoctonear());
        assert_eq!(scaled.extra_venear_balance.as_yoctonear(), balance.extra_venear_balance.as_yoctonear());
    }

    #[test]
    fn scale_by_bps_half_returns_half() {
        let balance = VenearBalance {
            near_balance: NearToken::from_near(100),
            extra_venear_balance: NearToken::from_near(50),
        };
        let scaled = balance.scale_by_bps(5000);
        assert_eq!(scaled.near_balance.as_yoctonear(), NearToken::from_near(50).as_yoctonear());
        assert_eq!(scaled.extra_venear_balance.as_yoctonear(), NearToken::from_near(25).as_yoctonear());
    }

    #[test]
    fn scale_by_bps_99_near_3333bps() {
        let balance = VenearBalance {
            near_balance: NearToken::from_near(99),
            extra_venear_balance: NearToken::from_near(0),
        };
        let scaled = balance.scale_by_bps(3333);
        // 99 * 3333 / 10000 = 329967 / 10000 = 32 (floor)
        let expected_yocto = (99_000_000_000_000_000_000_000_000u128 * 3333) / 10_000;
        assert_eq!(scaled.near_balance.as_yoctonear(), expected_yocto);
    }

    #[test]
    fn pooled_add_scaled_then_sub_scaled_is_identity() {
        let initial = PooledVenearBalance(VenearBalance {
            near_balance: NearToken::from_near(1000),
            extra_venear_balance: NearToken::from_near(500),
        });
        let to_add = VenearBalance {
            near_balance: NearToken::from_near(100),
            extra_venear_balance: NearToken::from_near(50),
        };
        let bps = 5000;

        let after_add = initial.pooled_add_scaled(&to_add, bps);
        let after_sub = after_add.pooled_sub_scaled(&to_add, bps);

        // Due to pooled truncation, we allow 1 milliNEAR tolerance
        let tolerance = 1_000_000_000_000_000u128; // 1 milliNEAR in yocto
        assert!(
            (after_sub.0.near_balance.as_yoctonear() as i128 - initial.0.near_balance.as_yoctonear() as i128).abs() <= tolerance as i128,
            "near_balance mismatch: expected ~{}, got {}",
            initial.0.near_balance.as_yoctonear(),
            after_sub.0.near_balance.as_yoctonear()
        );
    }

    #[test]
    fn scale_by_bps_extra_venear_scaled() {
        let balance = VenearBalance {
            near_balance: NearToken::from_near(0),
            extra_venear_balance: NearToken::from_near(100),
        };
        let scaled = balance.scale_by_bps(2500);
        assert_eq!(scaled.near_balance.as_yoctonear(), 0);
        assert_eq!(scaled.extra_venear_balance.as_yoctonear(), NearToken::from_near(25).as_yoctonear());
    }
}
