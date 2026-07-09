//! Save-format compatibility suite (SPEC §9: "Old saves must load forever").
//!
//! Every historical format version has a committed, byte-frozen fixture
//! here. Each must keep loading — through the migration pipeline — and
//! keep producing its recorded golden state, or the change that broke it
//! must ship a migration that makes it pass again. Regenerating a fixture
//! to make a red test green is forbidden without that migration.
//!
//! These suites load the FROZEN data snapshot (`fixtures/data_v3/`), never
//! the live `data/` directory, so balance edits cannot shift the goldens.
//!
//! Golden-hash policy (ADR 0004 §10): when a phase legitimately extends
//! the hash domain or the registration set, the same commit re-records the
//! goldens, says why, and proves content continuity (the persistence unit
//! test `v1_fixture_content_survives_migration_verbatim` checks migrated
//! bytes byte-for-byte). History: re-recorded in Phase 1 (event state
//! joined the hash) and Phase 2 (registration grew by the people set,
//! format v3 — ADR 0005 §9).

use core_types::{Seed, Ticks, WorldHash};
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use persistence::LoadConfig;

fn load_config() -> LoadConfig {
    runner::load_config(&pinned_defs()).expect("static snapshot config")
}

/// Resume schedule for a fixture-only save (fixture systems; no citizens).
fn fixture_schedule() -> core_ecs::Schedule {
    runner::build_schedule(&WorldSpec::fixture(Seed::new(0), 0), &pinned_defs())
}

// --- v1 (Phase 0: no event state; migrates v1→v2→v3) --------------------

/// Committed v1 fixture: seed 7, 50 fixture entities, 1,000 ticks (Phase 0).
const V1_FIXTURE: &[u8] = include_bytes!("../fixtures/v1_seed7_fixture50_tick1000.embersave");

/// Golden hash of the migrated v1 fixture at load (re-recorded Phase 2:
/// registration grew, ADR 0005 §9).
const V1_GOLDEN_HASH_AT_LOAD: u64 = 0x03ca_efc5_08ca_848a;

/// Golden hash after resuming the migrated v1 world 100 ticks.
const V1_GOLDEN_HASH_AFTER_100: u64 = 0x3e8e_ca13_31ce_25f2;

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
    sim.run_ticks(&mut fixture_schedule(), 100)
        .expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V1_GOLDEN_HASH_AFTER_100),
        "resumed evolution diverged from the recording"
    );
}

// --- v2 (Phase 1: live event state; migrates v2→v3) ---------------------

/// Committed v2 fixture: seed 13, 60 fixture entities, 2,000 ticks, saved
/// mid-flight with pending emissions, a populated log ring, and live
/// scheduled alarms.
const V2_FIXTURE: &[u8] = include_bytes!("../fixtures/v2_seed13_fixture60_tick2000.embersave");

/// Golden hash of the migrated v2 fixture at load (re-recorded Phase 2).
const V2_GOLDEN_HASH_AT_LOAD: u64 = 0x1901_f482_7918_6bd9;

/// Golden hash after resuming the migrated v2 fixture 100 ticks.
const V2_GOLDEN_HASH_AFTER_100: u64 = 0x9146_9e4d_1a3d_e0ca;

#[test]
fn v2_golden_save_loads_through_migration_to_the_golden_state() {
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
        "migrated v2 state differs from the recorded golden"
    );
}

#[test]
fn v2_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V2_FIXTURE, load_config(), runner::register_world)
        .expect("committed v2 save no longer loads");
    sim.run_ticks(&mut fixture_schedule(), 100)
        .expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V2_GOLDEN_HASH_AFTER_100),
        "resumed evolution diverged from the recording"
    );
}

// --- v3 (Phase 2: citizens — identities, needs, households, deaths) -----

/// Committed v3 fixture: seed 17, 40 fixture entities + 300 citizens,
/// 3,000 ticks (2+ days: needs decayed, mortality has run), saved
/// mid-flight with live event and scheduler state.
const V3_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v3_seed17_fixture40_citizens300_tick3000.embersave");

/// Golden hash of the v3 fixture at load.
const V3_GOLDEN_HASH_AT_LOAD: u64 = 0x600d_e7e6_0b0b_67a2;

/// Golden hash after resuming the v3 fixture 1,500 ticks (crossing a day
/// boundary so mortality and needs decay both run again).
const V3_GOLDEN_HASH_AFTER_1500: u64 = 0xc915_a333_cdd8_635c;

#[test]
fn v3_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V3_FIXTURE, load_config(), runner::register_world)
        .expect("committed v3 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(3000));
    assert_eq!(sim.seed(), Seed::new(17));
    let citizens = headless::inspect::population(sim.world()).expect("count failed");
    assert!(citizens > 0, "the v3 fixture contains a town");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V3_GOLDEN_HASH_AT_LOAD),
        "loaded v3 state differs from the state that was saved"
    );
}

#[test]
fn v3_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V3_FIXTURE, load_config(), runner::register_world)
        .expect("committed v3 save no longer loads");
    let spec = WorldSpec {
        citizens: 300,
        ..WorldSpec::fixture(sim.seed(), 0)
    };
    let mut schedule = runner::build_schedule(&spec, &pinned_defs());
    sim.run_ticks(&mut schedule, 1_500).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V3_GOLDEN_HASH_AFTER_1500),
        "resumed evolution diverged from the recording"
    );
}
