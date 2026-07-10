//! The fixed-timestep tick loop and the calendar (SPEC §5).
//!
//! Phase 1 scope: multi-rate driving (tick/hour/day/season/year) via the
//! `Calendar`, and the start-of-tick event transition (scheduled events
//! fire, queues rotate; ADR 0004). The catch-up controller is Phase 8
//! scope and does not exist yet.
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
use core_types::calendar::{SEASONS_PER_YEAR, TICKS_PER_DAY};
use core_types::{CalendarTime, Season, Seed, StableHasher, Ticks, WorldHash};
use thiserror::Error;

/// Errors constructing time infrastructure.
#[derive(Debug, Error)]
pub enum TimeError {
    /// A calendar configuration value is out of its valid range.
    #[error("invalid calendar config: {0}")]
    InvalidConfig(String),
}

/// Converts ticks to calendar coordinates (SPEC §5, ADR 0004 §6).
///
/// Invariants:
/// - Structure is definitional and fixed (1440 ticks/day, 4 seasons/year);
///   the one tunable, `days_per_season`, comes from
///   `data/balance/calendar.ron` and is ≥ 1.
/// - Pure configuration: not world state, not saved; the application
///   reconstructs it on load exactly like component registrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calendar {
    days_per_season: u32,
}

impl Calendar {
    /// Creates a calendar. Errors if `days_per_season` is zero.
    pub fn new(days_per_season: u32) -> Result<Calendar, TimeError> {
        if days_per_season == 0 {
            return Err(TimeError::InvalidConfig(
                "days_per_season must be >= 1".to_owned(),
            ));
        }
        Ok(Calendar { days_per_season })
    }

    /// Days in one season (the tunable).
    pub fn days_per_season(&self) -> u32 {
        self.days_per_season
    }

    /// Decomposes a tick into calendar coordinates.
    pub fn time_of(&self, tick: Ticks) -> CalendarTime {
        let t = tick.raw();
        let day = t / TICKS_PER_DAY;
        let tick_of_day = t % TICKS_PER_DAY;
        let season_length = u64::from(self.days_per_season);
        let season_ordinal = day / season_length;
        CalendarTime {
            year: season_ordinal / SEASONS_PER_YEAR,
            season: Season::from_index(season_ordinal % SEASONS_PER_YEAR),
            day_of_season: (day % season_length) as u32,
            // Casts are total: tick_of_day < 1440 bounds both fields.
            hour: (tick_of_day / 60) as u8,
            minute: (tick_of_day % 60) as u8,
        }
    }
}

/// A running simulation: the world, its tick counter, and its calendar.
///
/// Invariants:
/// - `tick` counts *completed* ticks; it is also the index the next tick
///   will execute as. Systems running during tick T observe
///   `ctx.tick == T`.
/// - The schedule and calendar are configuration, passed in / reconstructed
///   by the application (they hold no world state and are not saved).
pub struct Simulation {
    world: World,
    calendar: Calendar,
    tick: Ticks,
}

impl Simulation {
    /// Wraps an assembled world into a fresh simulation at tick 0.
    pub fn new(world: World, calendar: Calendar) -> Self {
        Simulation {
            world,
            calendar,
            tick: Ticks::ZERO,
        }
    }

    /// Reassembles a simulation from loaded state. For `persistence` use.
    pub fn from_parts(world: World, calendar: Calendar, tick: Ticks) -> Self {
        Simulation {
            world,
            calendar,
            tick,
        }
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

    /// The calendar.
    pub fn calendar(&self) -> &Calendar {
        &self.calendar
    }

    /// Executes exactly one tick, in the defined order (ADR 0004 §§3, 7):
    /// 1. start-of-tick event transition (due scheduled events fire, queues
    ///    rotate, log appends),
    /// 2. every rate whose period begins this tick, coarsest-first, then
    ///    tick systems — each system followed by its command buffer,
    /// 3. the counter advances.
    ///
    /// On error the tick aborts and the counter does not advance.
    pub fn step(&mut self, schedule: &mut Schedule) -> Result<(), EcsError> {
        let ctx = TickContext {
            tick: self.tick,
            time: self.calendar.time_of(self.tick),
        };
        self.world.begin_tick(self.tick);
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

    /// The catch-up integrator (SPEC §5; Phase 8, ADR 0011 §5):
    /// advances `count` ticks in HOUR strides. Boundary-rate systems
    /// run exactly where and in the order the normal loop would run
    /// them; tick-rate systems are skipped — per-tick agent behavior is
    /// what the day models replace. Scheduled events due mid-stride
    /// fire at the next stride's start (`begin_tick` drains everything
    /// due). Deterministic: a DEFINED coarse integrator, not a skip —
    /// the same state caught up the same span always lands identically,
    /// but it is NOT tick-equivalent to the full loop.
    pub fn run_ticks_coarse(
        &mut self,
        schedule: &mut Schedule,
        count: u64,
    ) -> Result<(), EcsError> {
        let ticks_per_hour = core_types::calendar::TICKS_PER_DAY / 24;
        let target = self.tick.try_add(count)?;
        while self.tick < target {
            let ctx = TickContext {
                tick: self.tick,
                time: self.calendar.time_of(self.tick),
            };
            self.world.begin_tick(self.tick);
            schedule.run_tick_coarse(&mut self.world, &ctx)?;
            // Stride to the next hour boundary (or the target).
            let next_boundary = (self.tick.raw() / ticks_per_hour + 1) * ticks_per_hour;
            self.tick = Ticks::new(next_boundary.min(target.raw()));
        }
        Ok(())
    }

    /// The canonical world-state hash (SPEC §9 `hash_world()`): tick
    /// counter, then full world state (entities, every component store in
    /// registration order, RNG stream states, event state).
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

    fn sim(seed: u64) -> Simulation {
        // Test fixture parameters: 30-day seasons, log capacity 16.
        Simulation::new(
            World::new(Seed::new(seed), 16),
            Calendar::new(30).expect("static test config"),
        )
    }

    #[test]
    fn zero_days_per_season_is_a_typed_error() {
        assert!(matches!(Calendar::new(0), Err(TimeError::InvalidConfig(_))));
    }

    #[test]
    fn calendar_golden_conversions() {
        let cal = Calendar::new(30).expect("static test config");
        assert_eq!(cal.time_of(Ticks::ZERO), CalendarTime::START);
        assert_eq!(
            cal.time_of(Ticks::new(1439)),
            CalendarTime {
                year: 0,
                season: Season::Spring,
                day_of_season: 0,
                hour: 23,
                minute: 59,
            }
        );
        assert_eq!(
            cal.time_of(Ticks::new(1440)),
            CalendarTime {
                year: 0,
                season: Season::Spring,
                day_of_season: 1,
                hour: 0,
                minute: 0,
            }
        );
        // Day 30 begins Summer; 4 seasons of 30 days begin year 1.
        assert_eq!(cal.time_of(Ticks::new(30 * 1440)).season, Season::Summer);
        let year1 = cal.time_of(Ticks::new(4 * 30 * 1440));
        assert_eq!(year1.year, 1);
        assert_eq!(year1.season, Season::Spring);
        assert!(year1.starts_year());
    }

    #[test]
    fn boundary_flags_match_tick_arithmetic() {
        let cal = Calendar::new(7).expect("static test config");
        for t in 0..(2 * 7 * 1440) {
            let time = cal.time_of(Ticks::new(t));
            assert_eq!(time.starts_hour(), t % 60 == 0, "tick {t}");
            assert_eq!(time.starts_day(), t % 1440 == 0, "tick {t}");
            assert_eq!(time.starts_season(), t % (7 * 1440) == 0, "tick {t}");
        }
    }

    #[test]
    fn stepping_advances_the_tick_counter() {
        let mut s = sim(1);
        let mut schedule = Schedule::new();
        assert_eq!(s.tick(), Ticks::ZERO);
        s.run_ticks(&mut schedule, 10).unwrap();
        assert_eq!(s.tick(), Ticks::new(10));
    }

    #[test]
    fn hash_distinguishes_ticks_but_not_runs() {
        let mut a = sim(7);
        let mut b = sim(7);
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
        assert_ne!(sim(1).state_hash().unwrap(), sim(2).state_hash().unwrap());
    }

    /// Event state is part of the hash domain (SPEC §9, ADR 0004 §§1, 5):
    /// two worlds identical except for a scheduled entry — or except for a
    /// pending emission — must hash differently. Pins
    /// `World::hash_into`'s events frame directly, independent of the
    /// re-recordable save_compat goldens.
    #[test]
    fn event_state_participates_in_the_world_hash() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Ping;
        impl core_ecs::Event for Ping {
            const NAME: &'static str = "test.ping";
        }

        let mut base = sim(1);
        let mut with_scheduled = sim(1);
        let mut with_pending = sim(1);
        for s in [&mut base, &mut with_scheduled, &mut with_pending] {
            s.world_mut().register_event::<Ping>().unwrap();
        }
        assert_eq!(
            base.state_hash().unwrap(),
            with_scheduled.state_hash().unwrap(),
            "identical worlds must hash equal before diverging"
        );

        with_scheduled
            .world_mut()
            .schedule_event(Ticks::new(500), &Ping)
            .unwrap();
        assert_ne!(
            base.state_hash().unwrap(),
            with_scheduled.state_hash().unwrap(),
            "a scheduled entry must be observable in the world hash"
        );

        with_pending.world_mut().emit(&Ping).unwrap();
        assert_ne!(
            base.state_hash().unwrap(),
            with_pending.state_hash().unwrap(),
            "a pending emission must be observable in the world hash"
        );
        assert_ne!(
            with_scheduled.state_hash().unwrap(),
            with_pending.state_hash().unwrap(),
            "scheduled and pending states must be distinguishable"
        );
    }
}
