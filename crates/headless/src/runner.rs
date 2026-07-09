//! World assembly and deterministic runs with periodic hashing.

use core_ecs::{EcsError, Rate, Schedule, World};
use core_types::{Seed, Ticks, WorldHash};
use persistence::LoadConfig;
use sim_time::{Calendar, Simulation, TimeError};
use thiserror::Error;

use crate::fixture;

/// Errors assembling or running a simulation.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// ECS/world failure.
    #[error(transparent)]
    Ecs(#[from] EcsError),
    /// Time/calendar configuration failure.
    #[error(transparent)]
    Time(#[from] TimeError),
}

/// A run's observable: `(tick, hash)` checkpoints in chronological order.
pub type HashSequence = Vec<(Ticks, WorldHash)>;

/// World-assembly tunables, sourced from `data/` via `data_defs` in the
/// CLI (SPEC §8) or pinned explicitly by tests (test inputs, not hidden
/// defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    /// Days per season (`data/balance/calendar.ron`).
    pub days_per_season: u32,
    /// Event log ring capacity (`data/balance/engine.ron`).
    pub event_log_capacity: u32,
}

impl SimConfig {
    /// Builds from loaded, validated data definitions.
    pub fn from_data(defs: &data_defs::DataDefs) -> SimConfig {
        SimConfig {
            days_per_season: defs.calendar.days_per_season,
            event_log_capacity: defs.engine.event_log_capacity,
        }
    }
}

/// How to assemble a world and its schedule.
///
/// Invariant: two `WorldSpec`s that compare equal assemble bit-identical
/// simulations for the same seed — the spec is the "same inputs" half of
/// determinism invariant 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldSpec {
    /// Master seed.
    pub seed: Seed,
    /// Whether the determinism fixture is populated and its systems
    /// scheduled (ADR 0002 §10).
    pub fixture: bool,
    /// Initial fixture population (ignored unless `fixture`).
    pub fixture_entities: u32,
    /// World-assembly tunables.
    pub config: SimConfig,
}

impl WorldSpec {
    /// An empty world: no entities, no systems.
    pub fn empty(seed: Seed, config: SimConfig) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
            config,
        }
    }

    /// A fixture world with `entities` initial fixture entities.
    pub fn fixture(seed: Seed, entities: u32, config: SimConfig) -> Self {
        WorldSpec {
            seed,
            fixture: true,
            fixture_entities: entities,
            config,
        }
    }

    /// The values `persistence::load_from_bytes` needs to reconstruct a
    /// world assembled from this spec.
    pub fn load_config(&self) -> Result<LoadConfig, RunnerError> {
        Ok(LoadConfig {
            calendar: Calendar::new(self.config.days_per_season)?,
            event_log_capacity: self.config.event_log_capacity as usize,
        })
    }
}

/// Registers every component and event this application knows, in the
/// fixed order that is part of the save/hash format (ADR 0002 §6,
/// ADR 0004 §4). Fresh worlds and loaded worlds both go through this exact
/// function.
pub fn register_world(world: &mut World) -> Result<(), EcsError> {
    world.register::<fixture::FixtureWealth>()?;
    world.register::<fixture::FixtureTag>()?;
    world.register_event::<fixture::FixtureChurn>()?;
    world.register_event::<fixture::FixtureAlarm>()?;
    Ok(())
}

/// Builds the schedule for a spec: the explicit ordered system lists
/// (SPEC §6). Phase 1 fixture order (tick rate): alarm chain, then walk.
pub fn build_schedule(spec: &WorldSpec) -> Schedule {
    let mut schedule = Schedule::new();
    if spec.fixture {
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureAlarmSystem));
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureWalkSystem));
    }
    schedule
}

/// Assembles a fresh simulation (registrations, fixture population,
/// calendar) and its schedule.
pub fn build_simulation(spec: &WorldSpec) -> Result<(Simulation, Schedule), RunnerError> {
    let calendar = Calendar::new(spec.config.days_per_season)?;
    let mut world = World::new(spec.seed, spec.config.event_log_capacity as usize);
    register_world(&mut world)?;
    if spec.fixture {
        fixture::populate(&mut world, spec.fixture_entities)?;
    }
    Ok((Simulation::new(world, calendar), build_schedule(spec)))
}

/// Runs `ticks` ticks, recording `(tick, hash)` at every multiple of
/// `hash_interval` (including tick 0 if the run starts there) and after the
/// final tick. A zero interval means "hash only at the end".
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
) -> Result<(HashSequence, HashSequence), RunnerError> {
    let (mut sim_a, mut schedule_a) = build_simulation(spec)?;
    let (mut sim_b, mut schedule_b) = build_simulation(spec)?;
    let a = run_with_hashes(&mut sim_a, &mut schedule_a, ticks, hash_interval)?;
    let b = run_with_hashes(&mut sim_b, &mut schedule_b, ticks, hash_interval)?;
    Ok((a, b))
}
