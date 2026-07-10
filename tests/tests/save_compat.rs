//! Save-format compatibility suite (SPEC §9: "Old saves must load forever").
//!
//! Every historical format version has a committed, byte-frozen fixture
//! here. Each must keep loading — through the migration pipeline — and
//! keep producing its recorded golden state, or the change that broke it
//! must ship a migration that makes it pass again. Regenerating a fixture
//! to make a red test green is forbidden without that migration.
//!
//! These suites load the FROZEN data snapshot (`fixtures/data_v5/`), never
//! the live `data/` directory, so balance edits cannot shift the goldens.
//!
//! Golden-hash policy (ADR 0004 §10): when a phase legitimately extends
//! the hash domain or the registration set, the same commit re-records the
//! goldens, says why, and proves content continuity (the persistence unit
//! tests `v*_fixture_content_survives_migration_verbatim` check migrated
//! bytes byte-for-byte). History: re-recorded in Phase 1 (event state
//! joined the hash), Phase 2 (+people registrations, format v3 —
//! ADR 0005 §9), Phase 3 (+world/AI registrations, format v4 —
//! ADR 0006 §8), and Phase 4 (+economy registrations and events, format
//! v5 — ADR 0007 §9; the pinned snapshot moved to `data_v5`, whose
//! balance changed this phase, so resume trajectories moved with it).

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

/// Golden hash of the migrated v1 fixture at load (re-recorded Phase 4:
/// registration grew, ADR 0007 §9).
const V1_GOLDEN_HASH_AT_LOAD: u64 = 0x13dd_e875_9501_1db0;

/// Golden hash after resuming the migrated v1 world 100 ticks.
const V1_GOLDEN_HASH_AFTER_100: u64 = 0xe7c8_9be3_b95b_cc05;

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

/// Golden hash of the migrated v2 fixture at load (re-recorded Phase 4).
const V2_GOLDEN_HASH_AT_LOAD: u64 = 0x7f6d_a385_51c9_c153;

/// Golden hash after resuming the migrated v2 fixture 100 ticks.
const V2_GOLDEN_HASH_AFTER_100: u64 = 0x3fe4_e88c_9608_6228;

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

/// Golden hash of the migrated v3 fixture at load (re-recorded Phase 4).
const V3_GOLDEN_HASH_AT_LOAD: u64 = 0x809e_5206_6ad1_f80c;

/// Golden hash after resuming the migrated v3 fixture 1,500 ticks
/// (crossing a day boundary so mortality and needs decay both run again;
/// under the Phase 3+ schedule the AI also runs — a migrated pre-location
/// town has no places, so its citizens idle, honestly and deterministically;
/// the Phase 4 economy systems no-op over its empty stores and the auditor
/// honestly reports nothing to audit).
const V3_GOLDEN_HASH_AFTER_1500: u64 = 0x7321_d7bd_8366_c7d8;

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

// --- v4 (Phase 3: locations + utility AI — actions, plans, decisions) ---

/// Committed v4 fixture: seed 23, 30 fixture entities + 250 citizens with
/// full AI, 2,500 ticks (past a day boundary and a plan compile), saved
/// with actions in flight, compiled plans, decision dumps, and live
/// event/scheduler state.
const V4_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v4_seed23_fixture30_citizens250_tick2500.embersave");

/// Golden hash of the v4 fixture at load (re-recorded Phase 4).
const V4_GOLDEN_HASH_AT_LOAD: u64 = 0xa263_fbfb_1678_aa87;

/// Golden hash after resuming the v4 fixture 700 ticks (mid-flight
/// actions complete, new decisions land, needs decay and satisfy — under
/// the v5 data snapshot, whose satisfiers changed; a migrated pre-economy
/// town has no wallets or shops, so nobody buys anything, honestly).
const V4_GOLDEN_HASH_AFTER_700: u64 = 0x3bbc_ff88_2ef0_789f;

#[test]
fn v4_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V4_FIXTURE, load_config(), runner::register_world)
        .expect("committed v4 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(2500));
    assert_eq!(sim.seed(), Seed::new(23));
    assert!(
        sim.world()
            .iter::<sim_ai::CurrentAction>()
            .expect("query")
            .count()
            > 0,
        "the v4 fixture was saved with actions in flight"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V4_GOLDEN_HASH_AT_LOAD),
        "loaded v4 state differs from the state that was saved"
    );
}

#[test]
fn v4_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V4_FIXTURE, load_config(), runner::register_world)
        .expect("committed v4 save no longer loads");
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    assert_eq!(derived.citizens, 250);
    assert!(derived.fixture);
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    sim.run_ticks(&mut schedule, 700).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V4_GOLDEN_HASH_AFTER_700),
        "resumed evolution diverged from the recording"
    );
}

// --- v5 (Phase 4: the economy — wallets, firms, market, auditors) -------

/// Committed v5 fixture: seed 29, 30 fixture entities + 250 citizens +
/// the full economy, 2,600 ticks (past a day boundary: the auditor,
/// trade, pricing, and spoilage have all run; citizens have bought bread
/// and firewood; batches are mid-production), saved with actions in
/// flight and live event/scheduler state.
const V5_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v5_seed29_fixture30_citizens250_tick2600.embersave");

/// Golden hash of the v5 fixture at load.
const V5_GOLDEN_HASH_AT_LOAD: u64 = 0xe539_63d4_e920_f129;

/// Golden hash after resuming the v5 fixture 700 ticks (purchases,
/// trades, repricing, and the daily audit all run again).
const V5_GOLDEN_HASH_AFTER_700: u64 = 0x6b22_229b_df9e_2bba;

#[test]
fn v5_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V5_FIXTURE, load_config(), runner::register_world)
        .expect("committed v5 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(2600));
    assert_eq!(sim.seed(), Seed::new(29));
    // The fixture was saved with a LIVE economy: purchases counted,
    // conservation intact.
    let world = sim.world();
    let counters = world
        .iter::<core_ecs::sim_interface::EconCounters>()
        .expect("query")
        .next()
        .map(|(_, c)| c.clone())
        .expect("the v5 fixture has a conservation ledger");
    assert!(
        counters.consumed_by_citizens.iter().sum::<i64>() > 0,
        "citizens had bought goods when the fixture was saved"
    );
    // The Phase 4 action variants are in flight at the save point, so the
    // resume golden genuinely covers their (de)serialization and
    // continuation (SPEC §9).
    let buying = world
        .iter::<sim_ai::CurrentAction>()
        .expect("query")
        .filter(|(_, action)| {
            matches!(
                action,
                sim_ai::CurrentAction::BuyTravel { .. }
                    | sim_ai::CurrentAction::BuyPending { .. }
                    | sim_ai::CurrentAction::Consume { .. }
            )
        })
        .count();
    assert!(
        buying > 0,
        "the v5 fixture must have purchase actions in flight"
    );
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "the loaded fixture must satisfy every conservation identity"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V5_GOLDEN_HASH_AT_LOAD),
        "loaded v5 state differs from the state that was saved"
    );
}

#[test]
fn v5_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V5_FIXTURE, load_config(), runner::register_world)
        .expect("committed v5 save no longer loads");
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    assert_eq!(derived.citizens, 250);
    assert!(derived.fixture);
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    sim.run_ticks(&mut schedule, 700).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V5_GOLDEN_HASH_AFTER_700),
        "resumed evolution diverged from the recording"
    );
}
