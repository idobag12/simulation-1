//! Shared error types for unit-safe arithmetic.

use thiserror::Error;

/// Error returned when a checked arithmetic operation on a unit newtype
/// would overflow its 64-bit representation.
///
/// Invariant: carries the operation name so an overflow deep in a system is
/// attributable without a debugger.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArithmeticError {
    /// The named operation over- or underflowed.
    #[error("{op} overflowed its 64-bit representation")]
    Overflow {
        /// The operation that overflowed, e.g. `"Money::try_add"`.
        op: &'static str,
    },
}
