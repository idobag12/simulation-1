//! The fixed-timestep tick loop (SPEC §5).
//!
//! Phase 0 scope: drive the per-tick schedule and expose the canonical
//! state hash. The `Calendar`, the future-callback `Scheduler`, the other
//! schedule rates (hour/day/season/year), and the catch-up controller are
//! Phase 1+ scope and do not exist yet (ADR 0002 §9).
//!
//! Invariants:
//! - 1 tick = 1 simulated minute; the tick is the only unit of causality.
//!   Wall-clock time never appears here or anywhere below (SPEC §5).
//! - Speed controls live entirely outside this crate; callers just request
//!   more or fewer ticks.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use core_ecs::{EcsError, Schedule, TickContext, World};
use core_types::{Seed, StableHasher, Ticks, WorldHash};

/// A running simulation: the world plus its tick counter.
///
/// Invariants:
/// - `tick` counts *completed* ticks; it is also the index the next tick
///   will execute as. Systems running during tick T observe
///   `ctx.tick == T`.
/// - The schedule is passed in by the caller (it holds no world state and
///   is not part of saved state; the application reconstructs it on load,
///   exactly as it reconstructs component registrations).
pub struct Simulation {
    world: World,
    tick: Ticks,
}

impl Simulation {
    /// Creates a fresh simulation at tick 0 with an empty world.
    pub fn new(seed: Seed) -> Self {
        Simulation {
            world: World::new(seed),
            tick: Ticks::ZERO,
        }
    }

    /// Reassembles a simulation from loaded state. For `persistence` use.
    pub fn from_parts(world: World, tick: Ticks) -> Self {
        Simulation { world, tick }
    }

    /// The world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The world, mutably. Setup/loading code only; systems receive the
    /// world through the schedule.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Completed tick count (== the index the next tick executes as).
    pub fn tick(&self) -> Ticks {
        self.tick
    }

    /// The world's master seed.
    pub fn seed(&self) -> Seed {
        self.world.seed()
    }

    /// Executes exactly one tick: every per-tick system in schedule order,
    /// then advances the counter. On error the tick is aborted and the
    /// counter does not advance.
    pub fn step(&mut self, schedule: &mut Schedule) -> Result<(), EcsError> {
        let ctx = TickContext { tick: self.tick };
        schedule.run_tick(&mut self.world, &ctx)?;
        self.tick = self.tick.try_add(1)?;
        Ok(())
    }

    /// Executes `count` ticks.
    pub fn run_ticks(&mut self, schedule: &mut Schedule, count: u64) -> Result<(), EcsError> {
        for _ in 0..count {
            self.step(schedule)?;
        }
        Ok(())
    }

    /// The canonical world-state hash (SPEC §9 `hash_world()`): tick
    /// counter, then full world state (entities, every component store in
    /// registration order, RNG stream states).
    pub fn state_hash(&self) -> Result<WorldHash, EcsError> {
        let mut hasher = StableHasher::new();
        hasher.write_u64(self.tick.raw());
        self.world.hash_into(&mut hasher)?;
        Ok(WorldHash::new(hasher.finish()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_advances_the_tick_counter() {
        let mut sim = Simulation::new(Seed::new(1));
        let mut schedule = Schedule::new();
        assert_eq!(sim.tick(), Ticks::ZERO);
        sim.run_ticks(&mut schedule, 10).unwrap();
        assert_eq!(sim.tick(), Ticks::new(10));
    }

    #[test]
    fn hash_distinguishes_ticks_but_not_runs() {
        let mut a = Simulation::new(Seed::new(7));
        let mut b = Simulation::new(Seed::new(7));
        let mut schedule = Schedule::new();

        let h0 = a.state_hash().unwrap();
        a.step(&mut schedule).unwrap();
        let h1 = a.state_hash().unwrap();
        assert_ne!(h0, h1, "tick counter must be part of the hash");

        b.step(&mut schedule).unwrap();
        assert_eq!(h1, b.state_hash().unwrap());
    }

    #[test]
    fn different_seeds_hash_differently() {
        let a = Simulation::new(Seed::new(1));
        let b = Simulation::new(Seed::new(2));
        assert_ne!(a.state_hash().unwrap(), b.state_hash().unwrap());
    }
}
