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

// `NeedLevel`, `Needs`, and `Personality` moved to
// `core_ecs::sim_interface` in Phase 3 (ADR 0006 §1): `sim_ai` scores
// against them, and sim crates only share components through the
// interface (SPEC §4). NAMEs unchanged; re-exported for source
// compatibility.
pub use core_ecs::sim_interface::{NeedLevel, Needs, Personality};

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
}
