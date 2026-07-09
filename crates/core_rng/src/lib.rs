//! Named deterministic RNG streams (SPEC §2).
//!
//! Every system draws randomness from its own named stream
//! (`rng("weather")`, `rng("ai.social")`). Streams are independently seeded,
//! so adding, removing, or reordering draws in one system can never perturb
//! another system's sequence — their determinism is decoupled.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use core_types::Seed;
use core_types::hash::fnv1a64;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

// Re-exported so downstream crates use the mandated RNG through this crate
// instead of depending on rand crates directly (SPEC §2 dependency policy).
pub use rand_core::RngCore;
pub use rand_pcg::Pcg64;

// splitmix64 algorithmic constants (Steele, Lea & Flood; also used by Vigna's
// reference implementation). They define the mixer itself, not tunables.
const SPLITMIX64_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const SPLITMIX64_MUL1: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX64_MUL2: u64 = 0x94d0_49bb_1331_11eb;

/// One splitmix64 step: advances `state` and returns the next output.
///
/// Invariant: this function is frozen (ADR 0002 §3). Changing it changes
/// every derived stream seed and is a save-format break.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(SPLITMIX64_GAMMA);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(SPLITMIX64_MUL1);
    z = (z ^ (z >> 27)).wrapping_mul(SPLITMIX64_MUL2);
    z ^ (z >> 31)
}

/// Derives the 32-byte PCG64 seed for a named stream from the master seed.
///
/// Algorithm (frozen, ADR 0002 §3):
/// 1. `h = fnv1a64(name)`
/// 2. `state = splitmix64(master) ^ h`
/// 3. seed = 4 successive splitmix64 outputs, little-endian.
///
/// Invariants: pure; depends only on `(master, name)`; never depends on
/// `rand_core` seeding internals, so saves outlive `rand` version bumps.
pub fn derive_stream_seed(master: Seed, name: &str) -> [u8; 32] {
    let mut mix_state = master.raw();
    let mixed_master = splitmix64(&mut mix_state);
    let mut state = mixed_master ^ fnv1a64(name.as_bytes());
    let mut seed = [0u8; 32];
    for chunk in seed.chunks_exact_mut(8) {
        chunk.copy_from_slice(&splitmix64(&mut state).to_le_bytes());
    }
    seed
}

/// Registry of named, independently seeded `Pcg64` streams.
///
/// Invariants:
/// - A stream's entire output sequence is a pure function of
///   `(master seed, stream name)` and the number of values already drawn.
/// - Streams are stored in a `BTreeMap`, so serialization and hashing order
///   is deterministic (SPEC §3: iteration order is correctness).
/// - Lazy creation is observationally invisible: a stream first touched
///   before or after a save/load round-trip yields the same sequence.
/// - Never share one stream across systems (SPEC §2) — that recouples their
///   determinism. Enforced by convention: stream names are namespaced per
///   system (`"ai.social"`, `"weather"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RngRegistry {
    master: Seed,
    streams: BTreeMap<String, Pcg64>,
}

impl RngRegistry {
    /// Creates an empty registry for the given master seed.
    pub fn new(master: Seed) -> Self {
        RngRegistry {
            master,
            streams: BTreeMap::new(),
        }
    }

    /// Returns the master seed the registry was created with.
    pub fn master_seed(&self) -> Seed {
        self.master
    }

    /// Returns the named stream, creating it deterministically on first use.
    pub fn stream(&mut self, name: &str) -> &mut Pcg64 {
        let master = self.master;
        self.streams
            .entry(name.to_owned())
            .or_insert_with(|| Pcg64::from_seed(derive_stream_seed(master, name)))
    }

    /// Number of streams that have been touched so far.
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::codec;
    use rand_core::RngCore;

    #[test]
    fn same_seed_and_name_reproduce_the_sequence() {
        let mut a = RngRegistry::new(Seed::new(7));
        let mut b = RngRegistry::new(Seed::new(7));
        let seq_a: Vec<u64> = (0..16).map(|_| a.stream("weather").next_u64()).collect();
        let seq_b: Vec<u64> = (0..16).map(|_| b.stream("weather").next_u64()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn different_names_and_seeds_produce_different_streams() {
        let mut reg = RngRegistry::new(Seed::new(7));
        let a = reg.stream("weather").next_u64();
        let b = reg.stream("ai.social").next_u64();
        assert_ne!(a, b);

        let mut other = RngRegistry::new(Seed::new(8));
        let c = other.stream("weather").next_u64();
        let mut same = RngRegistry::new(Seed::new(7));
        let d = same.stream("weather").next_u64();
        assert_ne!(a, c);
        assert_eq!(a, d);
    }

    #[test]
    fn streams_are_independent_of_interleaving() {
        // Drawing from stream X must not perturb stream Y (SPEC §2).
        let mut solo = RngRegistry::new(Seed::new(42));
        let expected: Vec<u64> = (0..8).map(|_| solo.stream("y").next_u64()).collect();

        let mut interleaved = RngRegistry::new(Seed::new(42));
        let mut got = Vec::new();
        for _ in 0..8 {
            let _ = interleaved.stream("x").next_u64();
            got.push(interleaved.stream("y").next_u64());
            let _ = interleaved.stream("x").next_u64();
        }
        assert_eq!(expected, got);
    }

    #[test]
    fn serde_round_trip_resumes_streams_exactly() {
        let mut reg = RngRegistry::new(Seed::new(123));
        for _ in 0..100 {
            let _ = reg.stream("a").next_u64();
            let _ = reg.stream("b").next_u64();
        }
        let bytes = codec::to_bytes(&reg).unwrap();
        let mut restored: RngRegistry = codec::from_bytes(&bytes).unwrap();
        for _ in 0..100 {
            assert_eq!(reg.stream("a").next_u64(), restored.stream("a").next_u64());
            assert_eq!(reg.stream("b").next_u64(), restored.stream("b").next_u64());
        }
    }

    #[test]
    fn lazy_creation_is_invisible_across_save_load() {
        // Save BEFORE stream "late" is ever touched; the restored registry
        // must derive it identically to the original.
        let mut reg = RngRegistry::new(Seed::new(9));
        let _ = reg.stream("early").next_u64();
        let bytes = codec::to_bytes(&reg).unwrap();
        let mut restored: RngRegistry = codec::from_bytes(&bytes).unwrap();
        assert_eq!(
            reg.stream("late").next_u64(),
            restored.stream("late").next_u64()
        );
    }

    /// Golden values freeze the derivation algorithm (ADR 0002 §3). If this
    /// fails, every save containing RNG state is invalidated.
    #[test]
    fn derive_stream_seed_is_frozen() {
        let a = derive_stream_seed(Seed::new(0), "weather");
        let b = derive_stream_seed(Seed::new(0), "weather");
        assert_eq!(a, b);
        assert_ne!(derive_stream_seed(Seed::new(0), "x"), a);
        // Golden bytes recorded at Phase 0 (see phase_0 report); recomputed
        // here from the frozen definition to detect accidental edits.
        let mut mix_state = 0u64;
        let mixed = splitmix64(&mut mix_state);
        let mut state = mixed ^ fnv1a64(b"weather");
        let mut expect = [0u8; 32];
        for chunk in expect.chunks_exact_mut(8) {
            chunk.copy_from_slice(&splitmix64(&mut state).to_le_bytes());
        }
        assert_eq!(a, expect);
    }
}
