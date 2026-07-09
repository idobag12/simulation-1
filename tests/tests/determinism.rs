//! The determinism suite (SPEC §14, CI-blocking from Phase 0).
//!
//! (a) Same seed, two fresh runs, hash-compare every checkpoint.
//! (b) Save at tick T, load, run to T+N, compare with the uninterrupted run.
//! (c) Replay from input log — deferred by ADR 0003 until the first
//!     external-input path exists; with no inputs, a replay is exactly (a).

use core_types::Seed;
use headless::runner::{self, SimConfig, WorldSpec};

/// Test-pinned world-assembly inputs (explicit test inputs, not hidden
/// defaults): 30-day seasons, event log ring of 4096.
const CONFIG: SimConfig = SimConfig {
    days_per_season: 30,
    event_log_capacity: 4096,
};

/// SPEC Phase 0 exit criterion: two runs of 1M empty ticks hash-identical,
/// checked every 10k ticks (SPEC §9).
#[test]
fn two_fresh_empty_runs_of_one_million_ticks_are_hash_identical() {
    let spec = WorldSpec::empty(Seed::new(0xE58E), CONFIG);
    let (a, b) =
        runner::verify_two_fresh_runs(&spec, 1_000_000, 10_000).expect("empty runs cannot fail");
    assert_eq!(a.len(), 101, "100 interval checkpoints + final");
    assert_eq!(a, b, "empty-world runs diverged");
    // Sanity: the hash actually observes time passing.
    assert_ne!(a[0].1, a[1].1, "hash must include the tick counter");
}

/// Determinism (a) with real state churn: components mutating, entities
/// spawning/despawning, RNG streams advancing, events emitted and
/// scheduled every tick (the Phase 0/1 fixture).
#[test]
fn two_fresh_fixture_runs_are_hash_identical() {
    let spec = WorldSpec::fixture(Seed::new(42), 200, CONFIG);
    let (a, b) = runner::verify_two_fresh_runs(&spec, 50_000, 5_000).expect("fixture run failed");
    assert_eq!(a, b, "fixture runs diverged");
    // Sanity: state is actually evolving between checkpoints.
    assert_ne!(a[3].1, a[4].1);
}

/// Different seeds must diverge (otherwise the hash observes nothing).
#[test]
fn different_seeds_produce_different_trajectories() {
    let (a, _) =
        runner::verify_two_fresh_runs(&WorldSpec::fixture(Seed::new(1), 200, CONFIG), 2_000, 1_000)
            .expect("run failed");
    let (b, _) =
        runner::verify_two_fresh_runs(&WorldSpec::fixture(Seed::new(2), 200, CONFIG), 2_000, 1_000)
            .expect("run failed");
    assert_ne!(a.last(), b.last());
}

/// Determinism (b), SPEC Phase 0 exit criterion: save/load/continue matches
/// the uninterrupted run — bit-identical state hash and identical re-save
/// bytes at every subsequent checkpoint. The save happens mid-flight:
/// events are pending, scheduled alarms are queued, the log ring is
/// populated — all of it must survive the round-trip exactly (SPEC §9).
#[test]
fn save_load_continue_matches_uninterrupted_run() {
    let spec = WorldSpec::fixture(Seed::new(7), 200, CONFIG);
    let total_ticks = 20_000;
    let save_at = 10_000;

    // Uninterrupted run.
    let (mut solid, mut solid_schedule) = runner::build_simulation(&spec).expect("build failed");
    solid
        .run_ticks(&mut solid_schedule, total_ticks)
        .expect("run failed");

    // Interrupted run: save at `save_at`, load, resume.
    let (mut first, mut first_schedule) = runner::build_simulation(&spec).expect("build failed");
    first
        .run_ticks(&mut first_schedule, save_at)
        .expect("run failed");
    assert!(
        first.world().event_system().scheduled_count() > 0,
        "fixture must have live scheduled entries at the save point"
    );
    let save = persistence::save_to_bytes(&first).expect("save failed");

    let load_config = spec.load_config().expect("load config");
    let mut resumed = persistence::load_from_bytes(&save, load_config, runner::register_world)
        .expect("load failed");
    assert_eq!(
        first.state_hash().expect("hash failed"),
        resumed.state_hash().expect("hash failed"),
        "loading a save must reproduce the exact saved state"
    );

    let mut resumed_schedule = runner::build_schedule(&spec);
    resumed
        .run_ticks(&mut resumed_schedule, total_ticks - save_at)
        .expect("resume failed");

    assert_eq!(
        solid.state_hash().expect("hash failed"),
        resumed.state_hash().expect("hash failed"),
        "save/load/continue diverged from the uninterrupted run"
    );
    // Stronger than hash equality: the full serialized states are identical.
    assert_eq!(
        persistence::save_to_bytes(&solid).expect("save failed"),
        persistence::save_to_bytes(&resumed).expect("save failed"),
        "serialized states differ despite equal hashes"
    );
}

/// RNG streams are restored exactly (SPEC §9): a save made mid-run resumes
/// every stream mid-sequence, including streams never touched before the
/// save (lazy creation is observationally invisible).
#[test]
fn save_load_preserves_rng_stream_positions() {
    use core_rng::RngCore;

    let spec = WorldSpec::fixture(Seed::new(11), 150, CONFIG);
    let (mut sim, mut schedule) = runner::build_simulation(&spec).expect("build failed");
    sim.run_ticks(&mut schedule, 1_000).expect("run failed");

    let save = persistence::save_to_bytes(&sim).expect("save failed");
    let load_config = spec.load_config().expect("load config");
    let mut loaded = persistence::load_from_bytes(&save, load_config, runner::register_world)
        .expect("load failed");

    let touched: Vec<u64> = (0..32)
        .map(|_| {
            sim.world_mut()
                .rng(headless::fixture::WALK_STREAM)
                .next_u64()
        })
        .collect();
    let touched_loaded: Vec<u64> = (0..32)
        .map(|_| {
            loaded
                .world_mut()
                .rng(headless::fixture::WALK_STREAM)
                .next_u64()
        })
        .collect();
    assert_eq!(
        touched, touched_loaded,
        "touched stream did not resume exactly"
    );

    let fresh: Vec<u64> = (0..32)
        .map(|_| sim.world_mut().rng("never.touched.before").next_u64())
        .collect();
    let fresh_loaded: Vec<u64> = (0..32)
        .map(|_| loaded.world_mut().rng("never.touched.before").next_u64())
        .collect();
    assert_eq!(
        fresh, fresh_loaded,
        "lazily-created stream diverged after load"
    );
}

/// The fixture population stays inside its configured band over a long run,
/// so the suite's state churn cannot silently die out or explode.
#[test]
fn fixture_population_stays_bounded_and_churning() {
    let spec = WorldSpec::fixture(Seed::new(3), 200, CONFIG);
    let (mut sim, mut schedule) = runner::build_simulation(&spec).expect("build failed");
    let initial = sim.world().entity_count();
    sim.run_ticks(&mut schedule, 20_000).expect("run failed");
    let population = sim.world().entity_count();
    assert!(
        (100..=400).contains(&population),
        "population {population} left the fixture band"
    );
    assert_ne!(initial, population, "no churn happened in 20k ticks");
}
