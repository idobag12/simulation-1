//! Citizen and household components (ADR 0005 §3).

use core_ecs::{Component, Entity, StorageKind};
use core_types::Ticks;
use serde::{Deserialize, Serialize};

/// Biological sex (drives name-list choice and later Phase 7 reproduction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sex {
    /// Female.
    Female,
    /// Male.
    Male,
}

/// Who a citizen is. Dense: every citizen carries it (it is the marker of
/// citizenship — a query over `Identity` enumerates the town).
///
/// Invariants:
/// - `birth_tick` is a signed offset from tick 0 (negative for citizens
///   born before the world began); age is always derived from it, never
///   stored (ADR 0005 §2).
/// - Names are authored-data draws fixed at genesis (until Phase 7 births).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Given name (from the sex-matched data list).
    pub given_name: String,
    /// Family name (shared within a genesis household).
    pub family_name: String,
    /// Sex.
    pub sex: Sex,
    /// Birth instant as a signed tick offset from tick 0.
    pub birth_tick: i64,
}

impl Identity {
    /// Age in whole world years at tick `now`, given the calendar's year
    /// length. Saturates at 0 for not-yet-born (cannot happen in Phase 2)
    /// and uses exact integer division.
    pub fn age_years(&self, now: Ticks, ticks_per_year: u64) -> u32 {
        // The cast is total: data_defs validates that the largest
        // data-defined age times the calendar's year length fits i64
        // (`validate_calendar_age_fit`), so `lived / ticks_per_year` fits
        // u32 for every age genesis can mint or mortality can look up.
        (self.lived_ticks(now) as u64 / ticks_per_year) as u32
    }

    /// Days into the current age-year at tick `now` (for display: an
    /// inspector age of "49y 87d").
    pub fn age_days_into_year(&self, now: Ticks, ticks_per_year: u64) -> u32 {
        let into_year = self.lived_ticks(now) as u64 % ticks_per_year;
        (into_year / core_types::calendar::TICKS_PER_DAY) as u32
    }

    fn lived_ticks(&self, now: Ticks) -> i64 {
        (now.raw() as i64).saturating_sub(self.birth_tick).max(0)
    }
}

impl Component for Identity {
    const NAME: &'static str = "people.identity";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// One need level in per-million units (ADR 0005 §2): 0 = fully depleted,
/// [`NeedLevel::MAX`] = fully satisfied.
///
/// Invariants: always within `0..=1_000_000`; all mutation goes through
/// the clamping constructors/operations, so an out-of-range level cannot
/// exist. Serializes transparently as its inner `i64` (identical bytes to
/// a bare level, so introducing the newtype was not a save-format break).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NeedLevel(i64);

impl NeedLevel {
    /// Fully satisfied (per-million scale). Unit definition, not a tunable.
    pub const MAX: NeedLevel = NeedLevel(1_000_000);
    /// Fully depleted.
    pub const ZERO: NeedLevel = NeedLevel(0);

    /// Constructs a level, clamping into the valid range.
    pub const fn new_clamped(raw: i64) -> NeedLevel {
        if raw < 0 {
            NeedLevel::ZERO
        } else if raw > NeedLevel::MAX.0 {
            NeedLevel::MAX
        } else {
            NeedLevel(raw)
        }
    }

    /// The raw per-million value (always in range).
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Decays by `amount` per-million units, clamping at zero (exact
    /// integer arithmetic; SPEC §2).
    pub fn decay(self, amount: i64) -> NeedLevel {
        NeedLevel::new_clamped(self.0.saturating_sub(amount))
    }
}

/// A citizen's need levels, one per data-defined need, in data order
/// (ADR 0005 §§2–3).
///
/// Invariant: the vector's length and order equal `NeedsConfig::needs`
/// (validated at load and guarded by `NeedsDecaySystem`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Needs {
    /// Need levels, data order.
    pub levels: Vec<NeedLevel>,
}

impl Component for Needs {
    const NAME: &'static str = "people.needs";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// A citizen's personality-trait weights, one per data-defined trait, in
/// data order. Weights are per-mille (ADR 0005 §2); they become utility
/// scoring inputs in Phase 3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Personality {
    /// Trait weights in per-mille units, data order.
    pub weights: Vec<i16>,
}

impl Component for Personality {
    const NAME: &'static str = "people.personality";
    // Dense: every citizen carries it.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Membership edge: citizen → household entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseholdMember {
    /// The household this citizen belongs to.
    pub household: Entity,
}

impl Component for HouseholdMember {
    const NAME: &'static str = "people.household_member";
    // Dense: every citizen carries it in Phase 2.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// A household: the entity that groups co-living citizens.
///
/// Invariant: `members` is kept sorted by entity index (deterministic
/// iteration, SPEC §3) and consistent with the members'
/// [`HouseholdMember`] edges; an emptied household is despawned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Household {
    /// Member citizens, ascending entity index.
    pub members: Vec<Entity>,
}

impl Component for Household {
    // Sparse: households are ~1/3 the entity population and interleaved
    // with citizens in index space.
    const NAME: &'static str = "people.household";
    const STORAGE: StorageKind = StorageKind::Sparse;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_derivation_is_exact_and_total() {
        let identity = Identity {
            given_name: "A".into(),
            family_name: "B".into(),
            sex: Sex::Female,
            birth_tick: -(3 * 172_800), // born 3 years (120-day years) before tick 0
        };
        let ticks_per_year = 172_800; // 4 seasons × 30 days × 1440
        assert_eq!(identity.age_years(Ticks::new(0), ticks_per_year), 3);
        assert_eq!(identity.age_years(Ticks::new(172_799), ticks_per_year), 3);
        assert_eq!(identity.age_years(Ticks::new(172_800), ticks_per_year), 4);
        assert_eq!(
            identity.age_days_into_year(Ticks::new(2 * 1440), ticks_per_year),
            2
        );

        let newborn = Identity {
            birth_tick: 100,
            ..identity
        };
        assert_eq!(newborn.age_years(Ticks::new(99), ticks_per_year), 0);
        assert_eq!(
            newborn.age_years(Ticks::new(100 + ticks_per_year), ticks_per_year),
            1
        );
    }

    #[test]
    fn need_levels_clamp_at_both_ends() {
        assert_eq!(NeedLevel::new_clamped(-5), NeedLevel::ZERO);
        assert_eq!(NeedLevel::new_clamped(2_000_000), NeedLevel::MAX);
        assert_eq!(NeedLevel::new_clamped(37).raw(), 37);
        assert_eq!(NeedLevel::new_clamped(100).decay(150), NeedLevel::ZERO);
        assert_eq!(NeedLevel::new_clamped(100).decay(40).raw(), 60);
        // Serializes transparently as the inner i64 (save-format identity).
        assert_eq!(
            core_types::codec::to_bytes(&NeedLevel::new_clamped(42)).unwrap(),
            core_types::codec::to_bytes(&42i64).unwrap()
        );
    }
}
