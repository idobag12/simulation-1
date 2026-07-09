//! Bit-stable state hashing (the backbone of all determinism testing,
//! SPEC §9).

use serde::{Deserialize, Serialize};

// FNV-1a 64-bit algorithmic constants (Fowler–Noll–Vo). These define the
// algorithm itself; they are not tunables (ADR 0002 §2).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A streaming FNV-1a 64-bit hasher.
///
/// Invariants:
/// - Output depends only on the exact byte sequence written: it is stable
///   across platforms, builds, and Rust versions (unlike `DefaultHasher`).
/// - The algorithm is frozen (ADR 0002 §2); changing it invalidates every
///   golden hash and is a breaking change to the determinism suite.
/// - Not cryptographic; used to detect state divergence, not adversaries.
#[derive(Debug, Clone)]
pub struct StableHasher {
    state: u64,
}

impl StableHasher {
    /// Creates a hasher at the FNV-1a offset basis.
    pub const fn new() -> Self {
        StableHasher {
            state: FNV_OFFSET_BASIS,
        }
    }

    /// Absorbs raw bytes.
    ///
    /// Invariant: callers hashing multiple variable-length fields must
    /// length-frame them (see [`StableHasher::write_frame`]) so that field
    /// boundaries cannot alias.
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    /// Absorbs a u64 as 8 little-endian bytes.
    pub fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    /// Absorbs a variable-length field unambiguously: length prefix, then
    /// bytes. Two different sequences of frames can never produce the same
    /// byte stream.
    pub fn write_frame(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.write(bytes);
    }

    /// Returns the current hash value. Does not consume; further writes may
    /// follow.
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for StableHasher {
    fn default() -> Self {
        StableHasher::new()
    }
}

/// Convenience: FNV-1a 64 of one byte slice.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = StableHasher::new();
    h.write(bytes);
    h.finish()
}

/// A canonical world-state hash (SPEC §9 `hash_world()` output).
///
/// Invariant: two `WorldHash` values are equal iff the canonical encodings
/// of the hashed states were byte-identical (modulo the non-cryptographic
/// collision caveat of ADR 0002 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldHash(u64);

impl WorldHash {
    /// Wraps a raw hash value. Total (never fails).
    pub const fn new(raw: u64) -> Self {
        WorldHash(raw)
    }

    /// Returns the raw hash value. Total (never fails).
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for WorldHash {
    /// Fixed-width lowercase hex, e.g. `00c0ffee00c0ffee`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden values freeze the algorithm (ADR 0002 §2). If this test fails,
    /// the hash function changed and every recorded world hash is invalid.
    #[test]
    fn fnv1a64_golden_values() {
        // Independently computed FNV-1a 64 reference values.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn frames_cannot_alias() {
        let mut a = StableHasher::new();
        a.write_frame(b"ab");
        a.write_frame(b"c");
        let mut b = StableHasher::new();
        b.write_frame(b"a");
        b.write_frame(b"bc");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn write_u64_is_little_endian_bytes() {
        let mut a = StableHasher::new();
        a.write_u64(0x0102_0304_0506_0708);
        let mut b = StableHasher::new();
        b.write(&[8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(a.finish(), b.finish());
    }
}
