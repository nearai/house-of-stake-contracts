use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::serde::de::Error as DeError;
use near_sdk::serde::{Deserialize, Deserializer, Serialize};
use near_sdk::{NearSchema, NearToken};
use std::ops::Mul;

/// Basis points in the range `0..=10_000`. Construction and JSON deserialization both enforce
/// the upper bound; Borsh trusts on-chain storage and performs no extra check (existing state is
/// always written via a validating path).
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    NearSchema,
)]
#[abi(borsh, json)]
#[borsh(crate = "near_sdk::borsh")]
#[serde(crate = "near_sdk::serde", transparent)]
pub struct Bps(u16);

impl Bps {
    pub const ZERO: Self = Self(0);
    pub const FULL: Self = Self(10_000);
    pub const MAX_RAW: u16 = 10_000;

    pub const fn new(value: u16) -> Self {
        assert!(value <= Self::MAX_RAW, "bps must be in 0..=10000");
        Self(value)
    }

    pub fn try_new(value: u16) -> Result<Self, &'static str> {
        if value > Self::MAX_RAW {
            Err("bps must be in 0..=10000")
        } else {
            Ok(Self(value))
        }
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_full(self) -> bool {
        self.0 == Self::MAX_RAW
    }
}

impl<'de> Deserialize<'de> for Bps {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = <u16 as Deserialize>::deserialize(deserializer)?;
        Self::try_new(raw).map_err(D::Error::custom)
    }
}

impl Mul<NearToken> for Bps {
    type Output = NearToken;

    fn mul(self, rhs: NearToken) -> Self::Output {
        if self.is_zero() {
            return NearToken::from_yoctonear(0);
        }
        if self.is_full() {
            return rhs;
        }
        NearToken::from_yoctonear(
            rhs.as_yoctonear() * u128::from(self.0) / u128::from(Self::MAX_RAW),
        )
    }
}

impl Mul<Bps> for NearToken {
    type Output = NearToken;

    fn mul(self, rhs: Bps) -> Self::Output {
        rhs * self
    }
}

impl From<Bps> for u16 {
    fn from(b: Bps) -> u16 {
        b.0
    }
}

impl From<Bps> for u32 {
    fn from(b: Bps) -> u32 {
        b.0.into()
    }
}

impl From<Bps> for u128 {
    fn from(b: Bps) -> u128 {
        b.0.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::borsh;

    #[test]
    fn try_new_accepts_in_range() {
        assert_eq!(u16::from(Bps::try_new(0).unwrap()), 0);
        assert_eq!(u16::from(Bps::try_new(3_500).unwrap()), 3_500);
        assert_eq!(u16::from(Bps::try_new(10_000).unwrap()), 10_000);
    }

    #[test]
    fn try_new_rejects_out_of_range() {
        assert!(Bps::try_new(10_001).is_err());
        assert!(Bps::try_new(u16::MAX).is_err());
    }

    #[test]
    #[should_panic(expected = "bps must be in 0..=10000")]
    fn new_panics_out_of_range() {
        let _ = Bps::new(10_001);
    }

    #[test]
    fn const_constructors() {
        assert!(Bps::ZERO.is_zero());
        assert!(Bps::FULL.is_full());
        assert_eq!(u16::from(Bps::ZERO), 0);
        assert_eq!(u16::from(Bps::FULL), Bps::MAX_RAW);
    }

    #[test]
    fn json_roundtrip_serializes_transparently() {
        let bps = Bps::new(3_500);
        let s = serde_json::to_string(&bps).unwrap();
        assert_eq!(s, "3500");
        let back: Bps = serde_json::from_str("3500").unwrap();
        assert_eq!(back, bps);
    }

    #[test]
    fn json_rejects_out_of_range() {
        let err = serde_json::from_str::<Bps>("10001").unwrap_err();
        assert!(err.to_string().contains("bps must be in 0..=10000"));
    }

    #[test]
    fn borsh_roundtrip_matches_u16_layout() {
        let bps = Bps::new(1_234);
        let bytes = borsh::to_vec(&bps).unwrap();
        // Same wire format as u16: 2 bytes, little-endian.
        assert_eq!(bytes, 1_234_u16.to_le_bytes().to_vec());
        let back: Bps = borsh::from_slice(&bytes).unwrap();
        assert_eq!(back, bps);
    }

    #[test]
    fn mul_zero_returns_zero() {
        let zero = Bps::ZERO * NearToken::from_near(100);
        assert_eq!(zero.as_yoctonear(), 0);
    }

    #[test]
    fn mul_full_returns_input() {
        let full = Bps::FULL * NearToken::from_near(123);
        assert_eq!(full, NearToken::from_near(123));
    }

    #[test]
    fn mul_half_returns_half() {
        let half = Bps::new(5_000) * NearToken::from_near(100);
        assert_eq!(half, NearToken::from_near(50));
    }

    #[test]
    fn mul_commutes() {
        let a = Bps::new(2_500) * NearToken::from_near(40);
        let b = NearToken::from_near(40) * Bps::new(2_500);
        assert_eq!(a, b);
        assert_eq!(a, NearToken::from_near(10));
    }

    #[test]
    fn mul_fractional_floors() {
        // 99 NEAR * 3333 / 10000 = floor((99e24 * 3333) / 10000)
        let result = Bps::new(3_333) * NearToken::from_near(99);
        let expected = (99_u128 * 10_u128.pow(24) * 3_333) / 10_000;
        assert_eq!(result.as_yoctonear(), expected);
    }
}
