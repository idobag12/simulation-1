//! World assembly and deterministic runs with periodic hashing.

use core_ecs::{EcsError, Schedule, World};
use core_types::{Seed, Ticks, WorldHash};
use sim_time::Simulation;

use crate::fixture;

/// A run's observable: `(tick, hash)` checkpoints in chronological order.
pub type HashSequence = Vec<(Ticks, WorldHash)>;

/// How to assemble a world and its schedule.
///
/// Invariant: two `WorldSpec`s that compare equal assemble bit-identical
/// simulations for the same seed — the spec is the "same inputs" half of
/// determinism invariant 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldSpec {
    /// Master seed.
    pub seed: Seed,
    /// Whether the Phase 0 determinism fixture is populated and its walk
    /// system scheduled (ADR 0002 §10).
    pub fixture: bool,
    /// Initial fixture population (ignored unless `fixture`).
    pub fixture_entities: u32,
}

impl WorldSpec {
    /// An empty world: no entities, no systems.
    pub fn empty(seed: Seed) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
        }
    }

    /// A fixture world with `entities` initial fixture entities.
    pub fn fixture(seed: Seed, entities: u32) -> Self {
        WorldSpec {
            seed,
            fixture: true,
            fixture_entities: entities,
        }
    }
}

/// Registers every component this application knows, in the fixed order
/// that is part of the save/hash format (ADR 0002 §6). Fresh worlds and
/// loaded worlds both go through this exact function.
pub fn register_components(world: &mut World) -> Result<(), EcsError> {
    world.register::<fixture::FixtureWealth>()?;
    world.register::<fixture::FixtureTag>()?;
    Ok(())
}

/// Builds the schedule for a spec. The schedule is the explicit ordered
/// system list (SPEC §6); Phase 0 has at most the fixture walk system.
pub fn build_schedule(spec: &WorldSpec) -> Schedule {
    let mut schedule = Schedule::new();
    if spec.fixture {
        schedule.add_tick_system(Box::new(fixture::FixtureWalkSystem));
    }
    schedule
}

/// Assembles a fresh simulation (registrations, fixture population) and its
/// schedule.
pub fn build_simulation(spec: &WorldSpec) -> Result<(Simulation, Schedule), EcsError> {
    let mut sim = Simulation::new(spec.seed);
    register_components(sim.world_mut())?;
    if spec.fixture {
        fixture::populate(sim.world_mut(), spec.fixture_entities)?;
    }
    Ok((sim, build_schedule(spec)))
}

/// Runs `ticks` ticks, recording `(tick, hash)` at every multiple of
/// `hash_interval` (including tick 0 if the run starts there) and after the
/// final tick.
///
/// Invariant: for the same simulation state and arguments, the returned
/// sequence is identical across runs — it is the determinism suite's
/// observable.
pub fn run_with_hashes(
    sim: &mut Simulation,
    schedule: &mut Schedule,
    ticks: u64,
    hash_interval: u64,
) -> Result<HashSequence, EcsError> {
    // A zero interval means "hash only at the end".
    let mut hashes = Vec::new();
    for _ in 0..ticks {
        if hash_interval != 0 && sim.tick().raw().is_multiple_of(hash_interval) {
            hashes.push((sim.tick(), sim.state_hash()?));
        }
        sim.step(schedule)?;
    }
    hashes.push((sim.tick(), sim.state_hash()?));
    Ok(hashes)
}

/// Runs the same spec twice from scratch and returns both hash sequences.
/// Equal sequences are the SPEC §14(a) determinism check.
pub fn verify_two_fresh_runs(
    spec: &WorldSpec,
    ticks: u64,
    hash_interval: u64,
) -> Result<(HashSequence, HashSequence), EcsError> {
    let (mut sim_a, mut schedule_a) = build_simulation(spec)?;
    let (mut sim_b, mut schedule_b) = build_simulation(spec)?;
    let a = run_with_hashes(&mut sim_a, &mut schedule_a, ticks, hash_interval)?;
    let b = run_with_hashes(&mut sim_b, &mut schedule_b, ticks, hash_interval)?;
    Ok((a, b))
}
