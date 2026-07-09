//! World assembly and deterministic runs with periodic hashing.

use core_ecs::{EcsError, Rate, Schedule, World};
use core_types::calendar::{SEASONS_PER_YEAR, TICKS_PER_DAY};
use core_types::{Seed, Ticks, WorldHash};
use data_defs::DataDefs;
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

/// How to assemble a world. Tunables come separately from [`DataDefs`]
/// (loaded from `data/` by the CLI, pinned explicitly by tests).
///
/// Invariant: equal `(WorldSpec, DataDefs)` pairs assemble bit-identical
/// simulations — together they are the "same inputs" half of determinism
/// invariant 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldSpec {
    /// Master seed.
    pub seed: Seed,
    /// Whether the determinism fixture is populated and its systems
    /// scheduled (ADR 0002 §10).
    pub fixture: bool,
    /// Initial fixture population (ignored unless `fixture`).
    pub fixture_entities: u32,
    /// Initial citizen count (0 = no town; Phase 2, ADR 0005).
    pub citizens: u32,
}

impl WorldSpec {
    /// An empty world: no entities, no systems.
    pub fn empty(seed: Seed) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
            citizens: 0,
        }
    }

    /// A fixture world with `entities` initial fixture entities.
    pub fn fixture(seed: Seed, entities: u32) -> Self {
        WorldSpec {
            seed,
            fixture: true,
            fixture_entities: entities,
            citizens: 0,
        }
    }

    /// A town of `citizens` citizens (composable with the fixture).
    pub fn town(seed: Seed, citizens: u32) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
            citizens,
        }
    }
}

/// The calendar year length in ticks for the loaded data (definitional
/// structure × the `days_per_season` tunable).
pub fn ticks_per_year(defs: &DataDefs) -> u64 {
    SEASONS_PER_YEAR * u64::from(defs.calendar.days_per_season) * TICKS_PER_DAY
}

/// The values `persistence::load_from_bytes` needs to reconstruct a world.
pub fn load_config(defs: &DataDefs) -> Result<LoadConfig, RunnerError> {
    Ok(LoadConfig {
        calendar: Calendar::new(defs.calendar.days_per_season)?,
        event_log_capacity: defs.engine.event_log_capacity as usize,
    })
}

/// Registers every component and event this application knows, in the
/// fixed order that is part of the save/hash format (ADR 0002 §6,
/// ADR 0004 §4). Fresh worlds and loaded worlds both go through this
/// exact function.
///
/// Invariant (ADR 0005 §9): this list only ever GROWS AT THE END, and any
/// growth bumps `persistence::FORMAT_VERSION` with a list-extension
/// migration. History: v2 = fixture set; v3 = + people set.
pub fn register_world(world: &mut World) -> Result<(), EcsError> {
    world.register::<fixture::FixtureWealth>()?;
    world.register::<fixture::FixtureTag>()?;
    world.register::<sim_people::Identity>()?;
    world.register::<sim_people::Needs>()?;
    world.register::<sim_people::Personality>()?;
    world.register::<sim_people::HouseholdMember>()?;
    world.register::<sim_people::Household>()?;
    world.register_event::<fixture::FixtureChurn>()?;
    world.register_event::<fixture::FixtureAlarm>()?;
    world.register_event::<sim_people::PersonDied>()?;
    Ok(())
}

/// Derives the schedule-relevant world composition from a LOADED world's
/// actual content, so a resumed run always reconstructs exactly the
/// systems the world was saved under (SPEC §9: loading a save and running
/// N ticks must equal running the original those same N ticks). The save
/// is authoritative — for its composition just as for its seed; CLI flags
/// never override it.
///
/// Known degenerate case (documented): a fixture world saved with zero
/// `FixtureWealth` rows (possible only via `--fixture --entities 0`)
/// derives `fixture: false` and drops the alarm chain on resume.
pub fn derive_spec_from_world(world: &World) -> Result<WorldSpec, EcsError> {
    let citizens = world.iter::<sim_people::Identity>()?.count() as u32;
    let fixture_rows = world.iter::<fixture::FixtureWealth>()?.count() as u32;
    Ok(WorldSpec {
        seed: world.seed(),
        fixture: fixture_rows > 0,
        fixture_entities: fixture_rows,
        citizens,
    })
}

/// Builds the schedule for a spec: the explicit ordered system lists
/// (SPEC §6). Order within each rate:
/// - Tick: fixture alarm chain, fixture walk (fixture worlds only).
/// - Hour: needs decay (towns only).
/// - Day: mortality (towns only).
pub fn build_schedule(spec: &WorldSpec, defs: &DataDefs) -> Schedule {
    let mut schedule = Schedule::new();
    if spec.fixture {
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureAlarmSystem));
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureWalkSystem));
    }
    if spec.citizens > 0 {
        let decays = defs
            .people
            .needs
            .needs
            .iter()
            .map(|need| need.decay_per_hour)
            .collect();
        schedule.add_system(
            Rate::Hour,
            Box::new(sim_people::NeedsDecaySystem::new(decays)),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_people::MortalitySystem::new(
                defs.people.mortality.clone(),
                ticks_per_year(defs),
            )),
        );
    }
    schedule
}

/// Assembles a fresh simulation (registrations, populations, calendar) and
/// its schedule.
pub fn build_simulation(
    spec: &WorldSpec,
    defs: &DataDefs,
) -> Result<(Simulation, Schedule), RunnerError> {
    let calendar = Calendar::new(defs.calendar.days_per_season)?;
    let mut world = World::new(spec.seed, defs.engine.event_log_capacity as usize);
    register_world(&mut world)?;
    if spec.fixture {
        fixture::populate(&mut world, spec.fixture_entities)?;
    }
    if spec.citizens > 0 {
        sim_people::genesis::populate(
            &mut world,
            &defs.people,
            spec.citizens,
            ticks_per_year(defs),
        )?;
    }
    Ok((Simulation::new(world, calendar), build_schedule(spec, defs)))
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
    defs: &DataDefs,
    ticks: u64,
    hash_interval: u64,
) -> Result<(HashSequence, HashSequence), RunnerError> {
    let (mut sim_a, mut schedule_a) = build_simulation(spec, defs)?;
    let (mut sim_b, mut schedule_b) = build_simulation(spec, defs)?;
    let a = run_with_hashes(&mut sim_a, &mut schedule_a, ticks, hash_interval)?;
    let b = run_with_hashes(&mut sim_b, &mut schedule_b, ticks, hash_interval)?;
    Ok((a, b))
}
