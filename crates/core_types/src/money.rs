//! Fixed-point monetary amounts.

use serde::{Deserialize, Serialize};

use crate::error::ArithmeticError;

/// A monetary amount in **mills** (1/1000 of one currency unit), stored as
/// an exact 64-bit integer (SPEC §2: fixed-point only, floats never touch
/// conserved quantities).
///
/// Invariants:
/// - Arithmetic is exact; every fallible operation is checked and returns a
///   typed error instead of wrapping or saturating, so conservation audits
///   can never be silently corrupted by overflow.
/// - Negative amounts are representable (debts, deltas); whether a negative
///   balance is *valid* is the owning ledger's invariant, not this type's.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Money(i64);

impl Money {
    /// Zero mills.
    pub const ZERO: Money = Money(0);

    /// Constructs an amount from a raw mill count. Total (never fails).
    pub const fn from_mills(mills: i64) -> Self {
        Money(mills)
    }

    /// Returns the raw mill count. Total (never fails).
    pub const fn mills(self) -> i64 {
        self.0
    }

    /// Checked addition. Errors on 64-bit overflow; never wraps.
    pub fn try_add(self, rhs: Money) -> Result<Money, ArithmeticError> {
        self.0
            .checked_add(rhs.0)
            .map(Money)
            .ok_or(ArithmeticError::Overflow {
                op: "Money::try_add",
            })
    }

    /// Checked subtraction. Errors on 64-bit overflow; never wraps.
    pub fn try_sub(self, rhs: Money) -> Result<Money, ArithmeticError> {
        self.0
            .checked_sub(rhs.0)
            .map(Money)
            .ok_or(ArithmeticError::Overflow {
                op: "Money::try_sub",
            })
    }
}

impl std::fmt::Display for Money {
    /// Formats as a decimal currency amount, e.g. `-1.500` for -1500 mills.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        // 1000 mills per currency unit is the definition of the type, not a tunable.
        write!(f, "{sign}{}.{:03}", abs / 1000, abs % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_sub_are_exact() {
        let a = Money::from_mills(1_500);
        let b = Money::from_mills(-2_250);
        assert_eq!(a.try_add(b).unwrap(), Money::from_mills(-750));
        assert_eq!(a.try_sub(b).unwrap(), Money::from_mills(3_750));
    }

    #[test]
    fn overflow_is_a_typed_error_not_a_wrap() {
        let max = Money::from_mills(i64::MAX);
        let one = Money::from_mills(1);
        assert_eq!(
            max.try_add(one),
            Err(ArithmeticError::Overflow {
                op: "Money::try_add"
            })
        );
        let min = Money::from_mills(i64::MIN);
        assert_eq!(
            min.try_sub(one),
            Err(ArithmeticError::Overflow {
                op: "Money::try_sub"
            })
        );
    }

    #[test]
    fn display_handles_signs_and_extremes() {
        assert_eq!(Money::from_mills(0).to_string(), "0.000");
        assert_eq!(Money::from_mills(1_500).to_string(), "1.500");
        assert_eq!(Money::from_mills(-1_500).to_string(), "-1.500");
        assert_eq!(Money::from_mills(-7).to_string(), "-0.007");
        // i64::MIN must not panic (unsigned_abs path).
        assert_eq!(
            Money::from_mills(i64::MIN).to_string(),
            "-9223372036854775.808"
        );
    }
}
