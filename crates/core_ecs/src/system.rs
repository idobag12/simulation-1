//! Systems and the explicitly ordered schedule (SPEC §6).

use core_types::Ticks;

use crate::command::CommandBuffer;
use crate::error::EcsError;
use crate::world::World;

/// Read-only per-tick context handed to every system.
///
/// Invariant: immutable during a tick; carries only tick-derived data (the
/// calendar joins it in Phase 1). Mutable state — including RNG streams —
/// lives in the [`World`] (ADR 0002 §4).
#[derive(Debug, Clone, Copy)]
pub struct TickContext {
    /// The tick currently executing. The first tick ever executed is 0.
    pub tick: Ticks,
}

/// A simulation system: a plain struct run at a fixed point in the schedule.
///
/// Invariants:
/// - Systems hold no references into the world between runs (SPEC §6); all
///   simulation state they touch lives in the world.
/// - Structural mutations (spawn/despawn/insert/remove) made while iterating
///   must go through `cmd`; the schedule applies the buffer immediately
///   after `run` returns (SPEC §6, ADR 0002 §5).
/// - Errors abort the tick and propagate (SPEC §3); a system must never
///   panic or swallow a failure.
pub trait System {
    /// Stable diagnostic name, used to attribute failures and (later)
    /// per-system tracing spans.
    fn name(&self) -> &'static str;

    /// Executes one step of this system.
    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError>;
}

/// The explicit, ordered system list (SPEC §6: "a literal `Vec` in one
/// file — no dependency-graph magic").
///
/// Invariants:
/// - Systems run in exactly the order they were pushed; order is part of
///   the spec of the application that builds the schedule.
/// - Phase 0 carries only the per-tick rate; hour/day/season/year rates are
///   added together with the `Calendar` in Phase 1 (ADR 0002 §9) — they do
///   not exist yet, so nothing fakes them.
#[derive(Default)]
pub struct Schedule {
    tick_systems: Vec<Box<dyn System>>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Self {
        Schedule::default()
    }

    /// Appends a per-tick system. Order of calls = order of execution.
    pub fn add_tick_system(&mut self, system: Box<dyn System>) {
        self.tick_systems.push(system);
    }

    /// Names of the per-tick systems in execution order (for tooling).
    pub fn tick_system_names(&self) -> Vec<&'static str> {
        self.tick_systems.iter().map(|s| s.name()).collect()
    }

    /// Runs one tick: each system in order, applying its command buffer
    /// immediately after it returns (the defined point, SPEC §6). Failures
    /// are wrapped with the system's name and abort the tick.
    pub fn run_tick(&mut self, world: &mut World, ctx: &TickContext) -> Result<(), EcsError> {
        for system in &mut self.tick_systems {
            let mut cmd = CommandBuffer::new();
            let name = system.name();
            let wrap = |source: EcsError| EcsError::System {
                system: name,
                source: Box::new(source),
            };
            system.run(world, ctx, &mut cmd).map_err(wrap)?;
            cmd.apply(world).map_err(wrap)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Component, StorageKind};
    use core_types::Seed;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Log(Vec<u8>);
    impl Component for Log {
        const NAME: &'static str = "test.log";
        const STORAGE: StorageKind = StorageKind::Dense;
    }

    /// Appends its tag to a shared log component, proving execution order.
    struct Tagger {
        tag: u8,
    }
    impl System for Tagger {
        fn name(&self) -> &'static str {
            "test.tagger"
        }
        fn run(
            &mut self,
            world: &mut World,
            _ctx: &TickContext,
            _cmd: &mut CommandBuffer,
        ) -> Result<(), EcsError> {
            for (_, log) in world.iter_mut::<Log>()? {
                log.0.push(self.tag);
            }
            Ok(())
        }
    }

    struct Failing;
    impl System for Failing {
        fn name(&self) -> &'static str {
            "test.failing"
        }
        fn run(
            &mut self,
            world: &mut World,
            _ctx: &TickContext,
            _cmd: &mut CommandBuffer,
        ) -> Result<(), EcsError> {
            // Force a typed error via an unregistered component.
            #[derive(Debug, Serialize, Deserialize)]
            struct Ghost;
            impl Component for Ghost {
                const NAME: &'static str = "test.ghost";
                const STORAGE: StorageKind = StorageKind::Sparse;
            }
            world.iter::<Ghost>().map(|_| ())
        }
    }

    #[test]
    fn systems_run_in_registration_order() {
        let mut world = World::new(Seed::new(1));
        world.register::<Log>().unwrap();
        let e = world.spawn();
        world.insert(e, Log(Vec::new())).unwrap();

        let mut schedule = Schedule::new();
        schedule.add_tick_system(Box::new(Tagger { tag: 1 }));
        schedule.add_tick_system(Box::new(Tagger { tag: 2 }));
        schedule.add_tick_system(Box::new(Tagger { tag: 3 }));
        assert_eq!(
            schedule.tick_system_names(),
            vec!["test.tagger", "test.tagger", "test.tagger"]
        );

        let ctx = TickContext { tick: Ticks::ZERO };
        schedule.run_tick(&mut world, &ctx).unwrap();
        schedule.run_tick(&mut world, &ctx).unwrap();
        assert_eq!(
            world.get::<Log>(e).unwrap(),
            Some(&Log(vec![1, 2, 3, 1, 2, 3]))
        );
    }

    #[test]
    fn failures_are_attributed_to_the_system() {
        let mut world = World::new(Seed::new(1));
        let mut schedule = Schedule::new();
        schedule.add_tick_system(Box::new(Failing));
        let ctx = TickContext { tick: Ticks::ZERO };
        match schedule.run_tick(&mut world, &ctx) {
            Err(EcsError::System { system, .. }) => assert_eq!(system, "test.failing"),
            other => panic!("expected attributed system error, got {other:?}"),
        }
    }
}
