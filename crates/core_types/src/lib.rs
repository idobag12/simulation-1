//! Foundational unit-safe newtypes and canonical encoding/hashing helpers
//! shared by every Embervale crate.
//!
//! Invariants owned by this crate:
//! - All economic quantities are 64-bit integers with checked arithmetic
//!   ([`Money`]); floating point never appears here (SPEC §2).
//! - There is exactly one binary encoding configuration in the codebase
//!   ([`codec`]); everything persisted or hashed goes through it.
//! - State hashing ([`hash`]) is bit-stable across platforms, builds, and
//!   Rust versions (ADR 0002 §2).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod codec;
pub mod error;
pub mod hash;
pub mod money;
pub mod ticks;

pub use error::ArithmeticError;
pub use hash::{StableHasher, WorldHash};
pub use money::Money;
pub use ticks::{Seed, Ticks};
