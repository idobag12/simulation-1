//! Phase 1 exit-criterion suite (SPEC §15): scheduled events fire
//! deterministically across save/load, and the event log ring is pure
//! observability (its capacity cannot affect trajectories, ADR 0004 §5).

use core_types::{Seed, Ticks};
use embervale_tests::pinned_defs;
use headless::fixture::FixtureAlarm;
use headless::runner::{self, WorldSpec};

/// Steps `sim` for `ticks`, recording every delivered [`FixtureAlarm`] as
/// `(tick, generation)`.
fn record_alarms(
    sim: &mut sim_time::Simulation,
    schedule: &mut core_ecs::Schedule,
    ticks: u64,
) -> Vec<(Ticks, u64)> {
    let mut deliveries = Vec::new();
    for _ in 0..ticks {
        let tick = sim.tick();
        sim.step(schedule).expect("step failed");
        for alarm in sim.world().events::<FixtureAlarm>().expect("read failed") {
            deliveries.push((tick, alarm.generation));
        }
    }
    deliveries
}

/// SPEC §15 Phase 1 exit criterion: scheduled events fire deterministically
/// across save/load. The fixture's alarm chain (each delivery schedules the
/// next, 97 ticks out) is recorded delivery-by-delivery in an uninterrupted
/// run and in a run that saves/loads mid-chain; the delivery logs must be
/// identical.
#[test]
fn scheduled_events_fire_deterministically_across_save_load() {
    let spec = WorldSpec::fixture(Seed::new(21), 120);
    let total = 600u64;
    let split = 250u64; // mid-chain: not a multiple of the alarm interval

    // Uninterrupted recording.
    let (mut solid, mut solid_schedule) =
        runner::build_simulation(&spec, &pinned_defs()).expect("build failed");
    let solid_log = record_alarms(&mut solid, &mut solid_schedule, total);

    // Interrupted recording.
    let (mut first, mut first_schedule) =
        runner::build_simulation(&spec, &pinned_defs()).expect("build failed");
    let mut interrupted_log = record_alarms(&mut first, &mut first_schedule, split);
    let save = persistence::save_to_bytes(&first).expect("save failed");
    let mut resumed = persistence::load_from_bytes(
        &save,
        runner::load_config(&pinned_defs()).expect("load config"),
        runner::register_world,
    )
    .expect("load failed");
    let mut resumed_schedule = runner::build_schedule(&spec, &pinned_defs());
    interrupted_log.extend(record_alarms(
        &mut resumed,
        &mut resumed_schedule,
        total - split,
    ));

    assert!(
        !solid_log.is_empty(),
        "the alarm chain must actually deliver in {total} ticks"
    );
    // The chain fires every 97 ticks: generations 0.. at ticks 97, 194, …
    assert_eq!(solid_log[0], (Ticks::new(97), 0));
    assert_eq!(
        solid_log, interrupted_log,
        "scheduled-event deliveries diverged across save/load"
    );
}

/// ADR 0004 §5: the event log ring is observability only. Two identical
/// runs under wildly different log capacities must produce byte-identical
/// entities, component stores, and RNG state. (The log contents themselves
/// legitimately differ — that is the ring bound working.)
#[test]
fn event_log_capacity_cannot_affect_trajectories() {
    let spec = WorldSpec::fixture(Seed::new(5), 150);
    let mut small_defs = pinned_defs();
    small_defs.engine.event_log_capacity = 8; // the varied test input
    let large_defs = pinned_defs(); // capacity 4096 from the snapshot

    let (mut sim_small, mut sched_small) =
        runner::build_simulation(&spec, &small_defs).expect("build failed");
    let (mut sim_large, mut sched_large) =
        runner::build_simulation(&spec, &large_defs).expect("build failed");
    sim_small
        .run_ticks(&mut sched_small, 2_000)
        .expect("run failed");
    sim_large
        .run_ticks(&mut sched_large, 2_000)
        .expect("run failed");

    let world_small = sim_small.world();
    let world_large = sim_large.world();
    assert_eq!(
        world_small.entities_to_bytes().expect("bytes"),
        world_large.entities_to_bytes().expect("bytes"),
        "log capacity leaked into entity state"
    );
    assert_eq!(
        world_small.component_blobs().expect("blobs"),
        world_large.component_blobs().expect("blobs"),
        "log capacity leaked into component state"
    );
    assert_eq!(
        world_small.rng_to_bytes().expect("bytes"),
        world_large.rng_to_bytes().expect("bytes"),
        "log capacity leaked into RNG state"
    );
    // And the ring bound itself is visible where it should be: the logs differ.
    assert_ne!(
        world_small.event_system().log().count(),
        world_large.event_system().log().count(),
        "capacities this different should retain different log lengths"
    );
}
