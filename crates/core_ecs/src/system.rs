//! Systems and the explicitly ordered multi-rate schedule (SPEC §6, §5;
//! rate semantics in ADR 0004 §7).

use core_types::{CalendarTime, Ticks};

use crate::command::CommandBuffer;
use crate::error::EcsError;
use crate::world::World;

/// Read-only per-tick context handed to every system.
///
/// Invariant: immutable during a tick; carries only tick-derived data.
/// Mutable state — including RNG streams and the event bus — lives in the
/// [`World`] (ADR 0002 §4, ADR 0004 §1).
#[derive(Debug, Clone, Copy)]
pub struct TickContext {
    /// The tick currently executing. The first tick ever executed is 0.
    pub tick: Ticks,
    /// The tick's calendar coordinates, computed by the tick loop's
    /// `Calendar`. Its boundary flags decide which rates fire (ADR 0004 §7).
    pub time: CalendarTime,
}

/// A simulation rate: how often a system list runs (SPEC §5/§6).
///
/// Invariant: a rate fires on the tick that *begins* its period; boundary
/// detection is `CalendarTime`'s `starts_*` flags (ADR 0004 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rate {
    /// Every tick.
    Tick,
    /// The first tick of every hour (minute 0).
    Hour,
    /// The first tick of every day (00:00).
    Day,
    /// The first tick of every season (day 0, 00:00).
    Season,
    /// The first tick of every year (Spring, day 0, 00:00).
    Year,
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

/// The explicit, ordered system lists per rate (SPEC §6: "a literal `Vec`
/// in one file — no dependency-graph magic").
///
/// Invariants:
/// - Within a rate, systems run in exactly the order they were added; the
///   application's schedule-building function is the single documented
///   source of that order.
/// - On a boundary tick, rate lists run coarsest-first — year → season →
///   day → hour → tick — so coarse bookkeeping (payroll, rent) settles
///   before finer-grained systems act within the same tick (ADR 0004 §7).
#[derive(Default)]
pub struct Schedule {
    tick_systems: Vec<Box<dyn System>>,
    hour_systems: Vec<Box<dyn System>>,
    day_systems: Vec<Box<dyn System>>,
    season_systems: Vec<Box<dyn System>>,
    year_systems: Vec<Box<dyn System>>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Self {
        Schedule::default()
    }

    /// Appends a system to `rate`'s list. Order of calls = order of
    /// execution within the rate.
    pub fn add_system(&mut self, rate: Rate, system: Box<dyn System>) {
        self.list_mut(rate).push(system);
    }

    /// Names of `rate`'s systems in execution order (for tooling).
    pub fn system_names(&self, rate: Rate) -> Vec<&'static str> {
        self.list(rate).iter().map(|s| s.name()).collect()
    }

    fn list(&self, rate: Rate) -> &Vec<Box<dyn System>> {
        match rate {
            Rate::Tick => &self.tick_systems,
            Rate::Hour => &self.hour_systems,
            Rate::Day => &self.day_systems,
            Rate::Season => &self.season_systems,
            Rate::Year => &self.year_systems,
        }
    }

    fn list_mut(&mut self, rate: Rate) -> &mut Vec<Box<dyn System>> {
        match rate {
            Rate::Tick => &mut self.tick_systems,
            Rate::Hour => &mut self.hour_systems,
            Rate::Day => &mut self.day_systems,
            Rate::Season => &mut self.season_systems,
            Rate::Year => &mut self.year_systems,
        }
    }

    /// Runs one tick: every rate whose period begins at `ctx.time`,
    /// coarsest-first (year → season → day → hour → tick); within each
    /// rate, systems in order, each followed immediately by its
    /// command-buffer application (the defined point, SPEC §6). Failures
    /// are wrapped with the system's name and abort the tick.
    pub fn run_tick(&mut self, world: &mut World, ctx: &TickContext) -> Result<(), EcsError> {
        if ctx.time.starts_year() {
            Self::run_list(&mut self.year_systems, world, ctx)?;
        }
        if ctx.time.starts_season() {
            Self::run_list(&mut self.season_systems, world, ctx)?;
        }
        if ctx.time.starts_day() {
            Self::run_list(&mut self.day_systems, world, ctx)?;
        }
        if ctx.time.starts_hour() {
            Self::run_list(&mut self.hour_systems, world, ctx)?;
        }
        Self::run_list(&mut self.tick_systems, world, ctx)
    }

    /// Runs one COARSE tick (Phase 8, ADR 0011 §5 — the catch-up
    /// integrator): every boundary rate exactly as [`Self::run_tick`]
    /// orders them, but the tick-rate list is skipped — per-tick agent
    /// behavior is what the day models replace.
    pub fn run_tick_coarse(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
    ) -> Result<(), EcsError> {
        if ctx.time.starts_year() {
            Self::run_list(&mut self.year_systems, world, ctx)?;
        }
        if ctx.time.starts_season() {
            Self::run_list(&mut self.season_systems, world, ctx)?;
        }
        if ctx.time.starts_day() {
            Self::run_list(&mut self.day_systems, world, ctx)?;
        }
        if ctx.time.starts_hour() {
            Self::run_list(&mut self.hour_systems, world, ctx)?;
        }
        Ok(())
    }

    fn run_list(
        systems: &mut [Box<dyn System>],
        world: &mut World,
        ctx: &TickContext,
    ) -> Result<(), EcsError> {
        for system in systems {
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
    use core_types::{Season, Seed};
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

    fn ctx_at(tick: u64, time: CalendarTime) -> TickContext {
        TickContext {
            tick: Ticks::new(tick),
            time,
        }
    }

    fn world_with_log() -> (World, crate::Entity) {
        let mut world = World::new(Seed::new(1), 16);
        world.register::<Log>().unwrap();
        let e = world.spawn();
        world.insert(e, Log(Vec::new())).unwrap();
        (world, e)
    }

    #[test]
    fn systems_run_in_registration_order_within_a_rate() {
        let (mut world, e) = world_with_log();
        let mut schedule = Schedule::new();
        schedule.add_system(Rate::Tick, Box::new(Tagger { tag: 1 }));
        schedule.add_system(Rate::Tick, Box::new(Tagger { tag: 2 }));
        schedule.add_system(Rate::Tick, Box::new(Tagger { tag: 3 }));
        assert_eq!(
            schedule.system_names(Rate::Tick),
            vec!["test.tagger", "test.tagger", "test.tagger"]
        );

        // Mid-hour tick: only the tick rate fires.
        let mid_hour = CalendarTime {
            minute: 30,
            ..CalendarTime::START
        };
        schedule
            .run_tick(&mut world, &ctx_at(30, mid_hour))
            .unwrap();
        schedule
            .run_tick(&mut world, &ctx_at(31, mid_hour))
            .unwrap();
        assert_eq!(
            world.get::<Log>(e).unwrap(),
            Some(&Log(vec![1, 2, 3, 1, 2, 3]))
        );
    }

    #[test]
    fn rates_fire_coarsest_first_on_boundary_ticks() {
        let (mut world, e) = world_with_log();
        let mut schedule = Schedule::new();
        schedule.add_system(Rate::Tick, Box::new(Tagger { tag: 1 }));
        schedule.add_system(Rate::Hour, Box::new(Tagger { tag: 2 }));
        schedule.add_system(Rate::Day, Box::new(Tagger { tag: 3 }));
        schedule.add_system(Rate::Season, Box::new(Tagger { tag: 4 }));
        schedule.add_system(Rate::Year, Box::new(Tagger { tag: 5 }));

        // Tick 0 begins everything: year, season, day, hour, tick.
        schedule
            .run_tick(&mut world, &ctx_at(0, CalendarTime::START))
            .unwrap();
        assert_eq!(
            world.get::<Log>(e).unwrap(),
            Some(&Log(vec![5, 4, 3, 2, 1]))
        );

        // An hour boundary mid-day: hour then tick only.
        let hour_boundary = CalendarTime {
            hour: 13,
            ..CalendarTime::START
        };
        schedule
            .run_tick(&mut world, &ctx_at(780, hour_boundary))
            .unwrap();
        assert_eq!(
            world.get::<Log>(e).unwrap(),
            Some(&Log(vec![5, 4, 3, 2, 1, 2, 1]))
        );

        // A season boundary that is not a year boundary.
        let winter = CalendarTime {
            season: Season::Winter,
            ..CalendarTime::START
        };
        schedule.run_tick(&mut world, &ctx_at(999, winter)).unwrap();
        assert_eq!(
            world.get::<Log>(e).unwrap(),
            Some(&Log(vec![5, 4, 3, 2, 1, 2, 1, 4, 3, 2, 1]))
        );
    }

    #[test]
    fn failures_are_attributed_to_the_system() {
        let mut world = World::new(Seed::new(1), 16);
        let mut schedule = Schedule::new();
        schedule.add_system(Rate::Tick, Box::new(Failing));
        match schedule.run_tick(&mut world, &ctx_at(0, CalendarTime::START)) {
            Err(EcsError::System { system, .. }) => assert_eq!(system, "test.failing"),
            other => panic!("expected attributed system error, got {other:?}"),
        }
    }
}
