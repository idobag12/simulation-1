//! Save-format compatibility suite (SPEC §9: "Old saves must load forever").
//!
//! Every historical format version has a committed, byte-frozen fixture
//! here. Each must keep loading — through the migration pipeline — and
//! keep producing its recorded golden state, or the change that broke it
//! must ship a migration that makes it pass again. Regenerating a fixture
//! to make a red test green is forbidden without that migration.
//!
//! Golden-hash policy (ADR 0004 §10): when a phase legitimately extends
//! the hash domain (e.g. Phase 1 added event state), the same commit
//! re-records the goldens, says why, and proves content continuity (the
//! persistence unit test `v1_fixture_content_survives_migration_verbatim`
//! checks the migrated v1 component/entity/RNG bytes byte-for-byte).

use core_types::{Seed, Ticks, WorldHash};
use headless::runner::{self, SimConfig, WorldSpec};
use persistence::LoadConfig;

/// Test-pinned world-assembly inputs matching the fixtures' generation.
const CONFIG: SimConfig = SimConfig {
    days_per_season: 30,
    event_log_capacity: 4096,
};

fn load_config() -> LoadConfig {
    WorldSpec::empty(Seed::new(0), CONFIG)
        .load_config()
        .expect("static test config")
}

// --- v1 (Phase 0: no event state; loads through the v1→v2 migration) ----

/// Committed v1 fixture: seed 7, 50 fixture entities, 1,000 ticks (Phase 0).
const V1_FIXTURE: &[u8] = include_bytes!("../fixtures/v1_seed7_fixture50_tick1000.embersave");

/// Golden hash of the migrated v1 fixture at load (re-recorded in Phase 1
/// when event state joined the hash domain, ADR 0004 §10).
const V1_GOLDEN_HASH_AT_LOAD: u64 = 0x6d7e_d624_8e74_2d3c;

/// Golden hash after resuming the migrated v1 world 100 ticks under the
/// Phase 1 schedule.
const V1_GOLDEN_HASH_AFTER_100: u64 = 0x6756_ae2f_f8bf_ff64;

#[test]
fn v1_golden_save_loads_through_migration_to_the_golden_state() {
    let sim = persistence::load_from_bytes(V1_FIXTURE, load_config(), runner::register_world)
        .expect("committed v1 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(1000));
    assert_eq!(sim.seed(), Seed::new(7));
    assert_eq!(
        sim.world().event_system().scheduled_count(),
        0,
        "a migrated v1 world starts with no scheduled events"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V1_GOLDEN_HASH_AT_LOAD),
        "migrated v1 state differs from the recorded golden"
    );
}

#[test]
fn v1_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V1_FIXTURE, load_config(), runner::register_world)
        .expect("committed v1 save no longer loads");
    let mut schedule = runner::build_schedule(&WorldSpec::fixture(sim.seed(), 0, CONFIG));
    sim.run_ticks(&mut schedule, 100).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V1_GOLDEN_HASH_AFTER_100),
        "resumed evolution diverged from the Phase 1 recording"
    );
}

// --- v2 (Phase 1: live event state — queues, log ring, alarm chain) -----

/// Committed v2 fixture: seed 13, 60 fixture entities, 2,000 ticks, saved
/// mid-flight with pending emissions, a populated log ring, and live
/// scheduled alarms.
const V2_FIXTURE: &[u8] = include_bytes!("../fixtures/v2_seed13_fixture60_tick2000.embersave");

/// Golden hash of the v2 fixture at load.
const V2_GOLDEN_HASH_AT_LOAD: u64 = 0xea67_696f_86d8_c4ff;

/// Golden hash after resuming the v2 fixture 100 ticks.
const V2_GOLDEN_HASH_AFTER_100: u64 = 0x6784_51de_60a9_2f25;

#[test]
fn v2_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V2_FIXTURE, load_config(), runner::register_world)
        .expect("committed v2 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(2000));
    assert_eq!(sim.seed(), Seed::new(13));
    assert!(
        sim.world().event_system().scheduled_count() > 0,
        "the v2 fixture was saved with live scheduled entries"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V2_GOLDEN_HASH_AT_LOAD),
        "loaded v2 state differs from the state that was saved"
    );
}

#[test]
fn v2_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V2_FIXTURE, load_config(), runner::register_world)
        .expect("committed v2 save no longer loads");
    let mut schedule = runner::build_schedule(&WorldSpec::fixture(sim.seed(), 0, CONFIG));
    sim.run_ticks(&mut schedule, 100).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V2_GOLDEN_HASH_AFTER_100),
        "resumed evolution diverged from the Phase 1 recording"
    );
}
