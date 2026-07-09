//! Discrete simulation time and seeds.

use serde::{Deserialize, Serialize};

use crate::error::ArithmeticError;

/// A count of simulation ticks (1 tick = 1 simulated minute, SPEC §5).
///
/// Invariants:
/// - The tick is the only unit of causality; wall-clock time never appears
///   in simulation code.
/// - Ordering is total and matches chronological order, so `Ticks` is a
///   valid `BTreeMap` key for deterministic scheduling.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Ticks(u64);

impl Ticks {
    /// Tick zero: the instant before the first tick executes.
    pub const ZERO: Ticks = Ticks(0);

    /// Constructs from a raw tick count. Total (never fails).
    pub const fn new(raw: u64) -> Self {
        Ticks(raw)
    }

    /// Returns the raw tick count. Total (never fails).
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Checked forward offset. Errors on u64 overflow (astronomically far in
    /// the future, but the standards ban unchecked arithmetic in sim code).
    pub fn try_add(self, delta: u64) -> Result<Ticks, ArithmeticError> {
        self.0
            .checked_add(delta)
            .map(Ticks)
            .ok_or(ArithmeticError::Overflow {
                op: "Ticks::try_add",
            })
    }
}

impl std::fmt::Display for Ticks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A master world seed. Same seed + same inputs ⇒ bit-identical world state
/// (SPEC invariant 1).
///
/// Invariant: the seed is fixed at world creation and never mutates; every
/// RNG stream derives from it deterministically (ADR 0002 §3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Seed(u64);

impl Seed {
    /// Constructs from a raw u64. Total (never fails).
    pub const fn new(raw: u64) -> Self {
        Seed(raw)
    }

    /// Returns the raw u64. Total (never fails).
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_ordering_is_chronological() {
        assert!(Ticks::new(1) < Ticks::new(2));
        assert_eq!(Ticks::ZERO.try_add(5).unwrap(), Ticks::new(5));
    }

    #[test]
    fn ticks_overflow_is_a_typed_error() {
        assert_eq!(
            Ticks::new(u64::MAX).try_add(1),
            Err(ArithmeticError::Overflow {
                op: "Ticks::try_add"
            })
        );
    }
}
