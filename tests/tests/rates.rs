//! Multi-rate schedule integration (SPEC §5/§6, ADR 0004 §7): each rate
//! fires exactly on its period boundaries, driven by the real Calendar
//! through the real tick loop.

use core_ecs::{
    CommandBuffer, Component, EcsError, Rate, Schedule, StorageKind, System, TickContext, World,
};
use core_types::{Seed, Ticks};
use serde::{Deserialize, Serialize};
use sim_time::{Calendar, Simulation};

/// Per-rate firing tally, one instance on a single bookkeeping entity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Tally {
    ticks: u64,
    hours: u64,
    days: u64,
    seasons: u64,
    years: u64,
    /// `(tick, rate-marker)` of the first few firings, proving coarse→fine
    /// order within a boundary tick. Markers: 5=year … 1=tick.
    first_firings: Vec<(u64, u8)>,
}

impl Component for Tally {
    const NAME: &'static str = "test.tally";
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// Increments one tally field each firing.
struct CountAt {
    marker: u8,
}

impl System for CountAt {
    fn name(&self) -> &'static str {
        "test.count"
    }
    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        for (_, tally) in world.iter_mut::<Tally>()? {
            match self.marker {
                5 => tally.years += 1,
                4 => tally.seasons += 1,
                3 => tally.days += 1,
                2 => tally.hours += 1,
                _ => tally.ticks += 1,
            }
            if tally.first_firings.len() < 8 {
                tally.first_firings.push((ctx.tick.raw(), self.marker));
            }
        }
        Ok(())
    }
}

#[test]
fn rates_fire_exactly_on_their_boundaries_over_two_years() {
    // Test-pinned inputs: 2-day seasons so a year is only 8 days.
    let days_per_season = 2u64;
    let ticks_per_year = 4 * days_per_season * 1440;
    let total = 2 * ticks_per_year; // exactly two years

    let mut world = World::new(Seed::new(1), 16);
    world.register::<Tally>().expect("register");
    let entity = world.spawn();
    world.insert(entity, Tally::default()).expect("insert");

    let mut schedule = Schedule::new();
    schedule.add_system(Rate::Year, Box::new(CountAt { marker: 5 }));
    schedule.add_system(Rate::Season, Box::new(CountAt { marker: 4 }));
    schedule.add_system(Rate::Day, Box::new(CountAt { marker: 3 }));
    schedule.add_system(Rate::Hour, Box::new(CountAt { marker: 2 }));
    schedule.add_system(Rate::Tick, Box::new(CountAt { marker: 1 }));

    let mut sim = Simulation::new(
        world,
        Calendar::new(days_per_season as u32).expect("static test config"),
    );
    sim.run_ticks(&mut schedule, total).expect("run failed");
    assert_eq!(sim.tick(), Ticks::new(total));

    let tally = sim
        .world()
        .get::<Tally>(entity)
        .expect("query failed")
        .expect("tally exists");
    assert_eq!(tally.ticks, total);
    assert_eq!(tally.hours, total / 60);
    assert_eq!(tally.days, total / 1440);
    assert_eq!(tally.seasons, total / (days_per_season * 1440));
    assert_eq!(
        tally.years, 2,
        "years begin at ticks 0 and {ticks_per_year}"
    );

    // Tick 0 begins everything; order is coarsest-first (ADR 0004 §7).
    assert_eq!(
        tally.first_firings[..5],
        [(0, 5), (0, 4), (0, 3), (0, 2), (0, 1)]
    );
    // Tick 1 is mid-hour: only the tick rate.
    assert_eq!(tally.first_firings[5], (1, 1));
}
