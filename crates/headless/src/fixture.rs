//! The Phase 0 determinism fixture (ADR 0002 §10).
//!
//! Phase 0 has no simulation content by design, but the determinism suite
//! needs state that actually evolves: components mutating, entities
//! spawning and despawning (exercising free-list recycling), and RNG
//! streams advancing. This module provides exactly that — a random-walk
//! exerciser. It is **test tooling, not simulation content**: it stubs no
//! future feature, it is registered only when explicitly requested
//! (`--fixture` on the CLI, or directly by tests), and its constants are
//! fixture parameters, not balance tunables (SPEC §8 applies to sim
//! crates; this is the tooling layer).

use core_ecs::{
    CommandBuffer, Component, EcsError, Event, StorageKind, System, TickContext, World,
};
use core_rng::RngCore;
use core_types::{Money, Ticks};

/// Dense-store fixture component: a random-walking balance plus a step
/// counter. Dense because (nearly) every fixture entity carries it.
///
/// Invariant: mutated only by [`FixtureWalkSystem`], via checked arithmetic.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FixtureWealth {
    /// Random-walking balance (not conserved — this is a fixture, not an
    /// economy; conservation audits arrive with real ledgers in Phase 4).
    pub amount: Money,
    /// Ticks this entity has been walked.
    pub steps: u64,
}

impl Component for FixtureWealth {
    const NAME: &'static str = "fixture.wealth";
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Sparse-store fixture component carried by a subset of entities, so saves
/// and hashes exercise the sparse layout too. Sparse by construction: only
/// entities spawned from even draws carry it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FixtureTag {
    /// Arbitrary payload preserved verbatim across save/load.
    pub label: u64,
}

impl Component for FixtureTag {
    const NAME: &'static str = "fixture.tag";
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// Fixture event emitted by [`FixtureWalkSystem`] on any tick whose
/// population changed — exercises the bus (emit → next-tick read) in every
/// fixture run. Fixture tooling, not simulation content (ADR 0002 §10).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FixtureChurn {
    /// Entities spawned this tick.
    pub spawned: u32,
    /// Entities despawned this tick.
    pub despawned: u32,
}

impl Event for FixtureChurn {
    const NAME: &'static str = "fixture.churn";
}

/// Fixture event delivered by the scheduler; [`FixtureAlarmSystem`]
/// re-schedules the next one, so a self-perpetuating alarm chain exercises
/// schedule → fire → read → re-schedule in every fixture run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FixtureAlarm {
    /// How many alarms have fired before this one.
    pub generation: u64,
}

impl Event for FixtureAlarm {
    const NAME: &'static str = "fixture.alarm";
}

// Fixture parameters (not balance tunables, ADR 0002 §10):
// each tick, every entity's balance moves by a draw in
// [-WALK_SPAN/2, +WALK_SPAN/2] mills…
const WALK_SPAN: u64 = 2001;
// …spawns happen on draw % SPAWN_MOD == 0 while below POP_MAX, and despawns
// on draw % DESPAWN_MOD == 0 while above POP_MIN, keeping the population
// churning inside a bounded band so long runs neither explode nor die out.
const SPAWN_MOD: u64 = 53;
const DESPAWN_MOD: u64 = 59;
const POP_MIN: usize = 100;
const POP_MAX: usize = 400;
// Initial balances are seeded in [0, INITIAL_WEALTH_SPAN) mills.
const INITIAL_WEALTH_SPAN: u64 = 100_000;
// Ticks between self-perpetuating fixture alarms.
const ALARM_INTERVAL: u64 = 97;

/// Name of the RNG stream the walk system draws from.
pub const WALK_STREAM: &str = "fixture.walk";
/// Name of the RNG stream used to seed initial fixture entities.
pub const SEED_STREAM: &str = "fixture.seed";

/// Spawns `count` fixture entities with RNG-derived balances; entities from
/// even draws also get a [`FixtureTag`]. Schedules the first
/// [`FixtureAlarm`] so the alarm chain starts. Deterministic given the
/// world's seed. Call once at world assembly, before any ticks.
pub fn populate(world: &mut World, count: u32) -> Result<(), EcsError> {
    for _ in 0..count {
        let draw = world.rng(SEED_STREAM).next_u64();
        let entity = world.spawn();
        world.insert(
            entity,
            FixtureWealth {
                amount: Money::from_mills((draw % INITIAL_WEALTH_SPAN) as i64),
                steps: 0,
            },
        )?;
        if draw.is_multiple_of(2) {
            world.insert(entity, FixtureTag { label: draw })?;
        }
    }
    world.schedule_event(Ticks::new(ALARM_INTERVAL), &FixtureAlarm { generation: 0 })?;
    Ok(())
}

/// Fixture system that reads this tick's [`FixtureAlarm`]s and schedules
/// the next link of the chain, exercising the scheduler continuously.
///
/// Invariant: stateless between runs (SPEC §6); the chain's state is the
/// scheduled entry itself.
pub struct FixtureAlarmSystem;

impl System for FixtureAlarmSystem {
    fn name(&self) -> &'static str {
        "fixture.alarm"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        for alarm in world.events::<FixtureAlarm>()? {
            let next = FixtureAlarm {
                generation: alarm.generation.saturating_add(1),
            };
            world.schedule_event(ctx.tick.try_add(ALARM_INTERVAL)?, &next)?;
        }
        Ok(())
    }
}

/// The fixture's per-tick system: random-walks every balance and churns the
/// population inside `[POP_MIN, POP_MAX]`.
///
/// Invariants:
/// - Stateless between runs (SPEC §6): everything lives in the world.
/// - Iterates in entity-index order; draws one RNG value per entity from
///   [`WALK_STREAM`]; all structural changes go through the command buffer.
pub struct FixtureWalkSystem;

impl System for FixtureWalkSystem {
    fn name(&self) -> &'static str {
        "fixture.walk"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pre-draw one value per entity (entity-index order), because the
        // RNG and the mutable component iterator both borrow the world.
        // (A combined split-borrow query API is deliberately deferred to
        // Phase 3, when real systems define what it must look like.)
        let population = world.iter::<FixtureWealth>()?.count();
        let draws: Vec<u64> = {
            let rng = world.rng(WALK_STREAM);
            (0..population).map(|_| rng.next_u64()).collect()
        };

        let mut spawned = 0usize;
        let mut despawned = 0usize;
        for ((entity, wealth), draw) in world.iter_mut::<FixtureWealth>()?.zip(draws) {
            wealth.steps = wealth.steps.saturating_add(1);
            // Signed step in [-1000, +1000] mills.
            let delta = (draw % WALK_SPAN) as i64 - (WALK_SPAN as i64 / 2);
            wealth.amount = wealth.amount.try_add(Money::from_mills(delta))?;

            if draw.is_multiple_of(DESPAWN_MOD) && population + spawned - despawned > POP_MIN {
                cmd.despawn(entity);
                despawned += 1;
            } else if draw.is_multiple_of(SPAWN_MOD) && population + spawned - despawned < POP_MAX {
                let amount = Money::from_mills((draw % INITIAL_WEALTH_SPAN) as i64);
                let tagged = draw.is_multiple_of(2);
                cmd.spawn(move |w, e| {
                    w.insert(e, FixtureWealth { amount, steps: 0 })?;
                    if tagged {
                        w.insert(e, FixtureTag { label: draw })?;
                    }
                    Ok(())
                });
                spawned += 1;
            }
        }
        if spawned > 0 || despawned > 0 {
            world.emit(&FixtureChurn {
                spawned: spawned as u32,
                despawned: despawned as u32,
            })?;
        }
        Ok(())
    }
}
