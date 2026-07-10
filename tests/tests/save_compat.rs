//! Save-format compatibility suite (SPEC §9: "Old saves must load forever").
//!
//! Every historical format version has a committed, byte-frozen fixture
//! here. Each must keep loading — through the migration pipeline — and
//! keep producing its recorded golden state, or the change that broke it
//! must ship a migration that makes it pass again. Regenerating a fixture
//! to make a red test green is forbidden without that migration.
//!
//! These suites load the FROZEN data snapshot (`fixtures/data_v8/`), never
//! the live `data/` directory, so balance edits cannot shift the goldens.
//!
//! Golden-hash policy (ADR 0004 §10): when a phase legitimately extends
//! the hash domain or the registration set, the same commit re-records the
//! goldens, says why, and proves content continuity (the persistence unit
//! tests `v*_fixture_content_survives_migration_verbatim` check migrated
//! bytes byte-for-byte). History: re-recorded in Phase 1 (event state
//! joined the hash), Phase 2 (+people registrations, format v3 —
//! ADR 0005 §9), Phase 3 (+world/AI registrations, format v4 —
//! ADR 0006 §8), Phase 4 (+economy registrations and events, format v5 —
//! ADR 0007 §9), Phase 5 (+labor registrations and events, format v6 —
//! ADR 0008 §8), Phase 6 (+money registrations and events, format v7 —
//! ADR 0009 §6), and Phase 7 (+social registrations and events, format
//! v8 — ADR 0010 §6; the pinned snapshot moved to `data_v8`, whose
//! skills/social/fertility files and schedules changed this phase, so
//! resume trajectories moved with it).

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

/// Golden hash of the migrated v1 fixture at load (re-recorded Phase 6:
/// registration grew, ADR 0009 §6).
const V1_GOLDEN_HASH_AT_LOAD: u64 = 0xdfa2_e71d_cea9_f1c4;

/// Golden hash after resuming the migrated v1 world 100 ticks.
const V1_GOLDEN_HASH_AFTER_100: u64 = 0x269d_273e_097a_f8ea;

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

/// Golden hash of the migrated v2 fixture at load (re-recorded Phase 6).
const V2_GOLDEN_HASH_AT_LOAD: u64 = 0xa39c_7766_3a99_a81d;

/// Golden hash after resuming the migrated v2 fixture 100 ticks.
const V2_GOLDEN_HASH_AFTER_100: u64 = 0x823e_24f1_b633_768f;

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

/// Golden hash of the migrated v3 fixture at load (re-recorded Phase 6).
const V3_GOLDEN_HASH_AT_LOAD: u64 = 0x66ff_2c05_7df1_d604;

/// Golden hash after resuming the migrated v3 fixture 1,500 ticks
/// (crossing a day boundary so mortality and needs decay both run again;
/// under the Phase 3+ schedule the AI also runs — a migrated pre-location
/// town has no places, so its citizens idle, honestly and deterministically;
/// the Phase 4 economy systems no-op over its empty stores and the auditor
/// honestly reports nothing to audit).
const V3_GOLDEN_HASH_AFTER_1500: u64 = 0x203a_bc34_1272_f322;

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

/// Golden hash of the v4 fixture at load (re-recorded Phase 6).
const V4_GOLDEN_HASH_AT_LOAD: u64 = 0x00c9_79e4_3edf_9a3d;

/// Golden hash after resuming the v4 fixture 700 ticks (mid-flight
/// actions complete, new decisions land, needs decay and satisfy — under
/// the v5 data snapshot, whose satisfiers changed; a migrated pre-economy
/// town has no wallets or shops, so nobody buys anything, honestly).
const V4_GOLDEN_HASH_AFTER_700: u64 = 0x9126_b7ad_c823_b3df;

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

/// Golden hash of the v5 fixture at load (re-recorded Phase 6; the
/// migrated v5 town has no working-age markers or labor stats until its
/// first day boundary — nobody is hired out of thin air; the promotion
/// stamps real adults, the ledger grows its stats row, and the market
/// hires — proven semantically by
/// `v5_migrated_economy_catches_up_with_the_labor_market`).
const V5_GOLDEN_HASH_AT_LOAD: u64 = 0x6e5c_4473_30d2_d264;

/// Golden hash after resuming the v5 fixture 700 ticks (purchases,
/// trades, repricing, and the daily audit all run again).
const V5_GOLDEN_HASH_AFTER_700: u64 = 0x38a5_101f_266b_0d04;

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

/// The migrated v5 economy CATCHES UP (ADR 0008 §8): at load nobody is
/// working-age or employed (migrations never invent state), and after
/// the first day boundary the promotion stamps real adults, the ledger
/// grows its labor stats, and the market genuinely hires — a migrated
/// town's firms keep producing instead of idling forever.
#[test]
fn v5_migrated_economy_catches_up_with_the_labor_market() {
    let mut sim = persistence::load_from_bytes(V5_FIXTURE, load_config(), runner::register_world)
        .expect("committed v5 save no longer loads");
    let world = sim.world();
    assert_eq!(
        world
            .iter::<core_ecs::sim_interface::WorkingAge>()
            .expect("query")
            .count(),
        0,
        "migrations never invent state"
    );
    assert_eq!(
        world
            .iter::<core_ecs::sim_interface::Employment>()
            .expect("query")
            .count(),
        0
    );
    let derived = runner::derive_spec_from_world(world).expect("derive");
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    // Tick 2600 → 3400 crosses the day boundary at 2880: promotion,
    // clearing, and the first shift hours.
    sim.run_ticks(&mut schedule, 800).expect("resume");
    let world = sim.world();
    assert!(
        world
            .iter::<core_ecs::sim_interface::WorkingAge>()
            .expect("query")
            .count()
            > 100,
        "the promotion stamped the town's real adults"
    );
    assert!(
        world
            .iter::<core_ecs::sim_interface::Employment>()
            .expect("query")
            .count()
            > 10,
        "the market hired out of the catch-up"
    );
    assert!(debug_tools::audit_economy(world).expect("audit"));
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

// --- v6 (Phase 5: labor — employment, wages, payroll, unemployment) -----

/// Committed v6 fixture: seed 37, 30 fixture entities + 250 citizens +
/// the working economy, 2,000 ticks — 09:20 on day 1, MID-SHIFT: the
/// clearing has hired, wages have been paid, and citizens are at work
/// (`Work`/`WorkTravel` in flight) when the save lands.
const V6_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v6_seed37_fixture30_citizens250_tick2000.embersave");

/// Golden hash of the v6 fixture at load (re-recorded Phase 6:
/// registration grew, ADR 0009 §6).
const V6_GOLDEN_HASH_AT_LOAD: u64 = 0xef85_afde_9e1e_f155;

/// Golden hash after resuming the v6 fixture 1,000 ticks (the rest of
/// the shift, then the 2,880 day boundary's audit/payroll/clearing —
/// under the Phase 6 day schedule the money systems also run; a
/// migrated town has no bank or treasury, so they no-op, honestly).
const V6_GOLDEN_HASH_AFTER_1000: u64 = 0x3769_290b_30a6_88c1;

#[test]
fn v6_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V6_FIXTURE, load_config(), runner::register_world)
        .expect("committed v6 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(2000));
    assert_eq!(sim.seed(), Seed::new(37));
    // The fixture was saved with a LIVE labor market: jobs held, wages
    // cleared, the stats written.
    let world = sim.world();
    let employed = world
        .iter::<core_ecs::sim_interface::Employment>()
        .expect("query")
        .count();
    assert!(employed > 10, "citizens hold jobs in the fixture");
    let stats = world
        .iter::<core_ecs::sim_interface::LaborStats>()
        .expect("query")
        .next()
        .map(|(_, s)| *s)
        .expect("the labor ledger exists");
    assert!(stats.hires > 0 && stats.seeking > 0);
    // The Phase 5 action variants are in flight at the save point, so the
    // resume golden genuinely covers their (de)serialization and
    // continuation (SPEC §9) — the fixture is saved mid-shift.
    let working = world
        .iter::<sim_ai::CurrentAction>()
        .expect("query")
        .filter(|(_, action)| {
            matches!(
                action,
                sim_ai::CurrentAction::Work { .. } | sim_ai::CurrentAction::WorkTravel { .. }
            )
        })
        .count();
    assert!(
        working > 0,
        "the v6 fixture must have work actions in flight"
    );
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "the loaded fixture must satisfy every conservation identity"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V6_GOLDEN_HASH_AT_LOAD),
        "loaded v6 state differs from the state that was saved"
    );
}

#[test]
fn v6_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V6_FIXTURE, load_config(), runner::register_world)
        .expect("committed v6 save no longer loads");
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    assert_eq!(derived.citizens, 250);
    assert!(derived.fixture);
    assert!(derived.economy);
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    sim.run_ticks(&mut schedule, 1000).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V6_GOLDEN_HASH_AFTER_1000),
        "resumed evolution diverged from the recording"
    );
}

// --- v7 (Phase 6: money — the bank, taxes, treasury, housing) -----------

/// Committed v7 fixture: seed 41, 30 fixture entities + 250 citizens +
/// the full money layer, 29,360 ticks — 09:20 on day 20, MID-SHIFT and
/// mid-loan: deposit rows populated, working-capital loans outstanding,
/// income and sales tax collected, the policy rate moved off neutral.
const V7_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v7_seed41_fixture30_citizens250_tick29360.embersave");

/// Golden hash of the v7 fixture at load.
const V7_GOLDEN_HASH_AT_LOAD: u64 = 0x0f9b_7ae7_60b0_419d;

/// Golden hash after resuming the v7 fixture 1,000 ticks (the rest of
/// the shift, then the day boundary's audit, bank service/origination,
/// payroll withholding, clearings, and markets).
const V7_GOLDEN_HASH_AFTER_1000: u64 = 0x29f5_2e8d_43f2_7834;

#[test]
fn v7_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V7_FIXTURE, load_config(), runner::register_world)
        .expect("committed v7 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(29360));
    assert_eq!(sim.seed(), Seed::new(41));
    // The fixture was saved with a LIVE money layer: savings in the
    // vault, credit outstanding, both taxes collected — so the golden
    // genuinely covers (de)serialization of every Phase 6 store.
    let world = sim.world();
    let book = world
        .iter::<core_ecs::sim_interface::BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.clone())
        .expect("the v7 fixture has a bank");
    assert!(!book.deposits.is_empty(), "citizens hold deposits");
    assert!(
        !book.loans.is_empty(),
        "working-capital loans are in flight"
    );
    let treasury = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .expect("query")
        .next()
        .map(|(_, book)| *book)
        .expect("the v7 fixture has a treasury");
    assert!(treasury.income_tax_received.mills() > 0);
    assert!(treasury.sales_tax_received.mills() > 0);
    assert!(
        world
            .iter::<core_ecs::sim_interface::Ownership>()
            .expect("query")
            .count()
            > 0,
        "genesis homes are owned"
    );
    assert!(
        book.loans.iter().all(|loan| loan.collateral.is_some()),
        "the fixture's credit is all mortgages — every loan secured"
    );
    assert_ne!(
        book.policy_rate_per_million_daily,
        pinned_defs().bank.policy_neutral_per_million_daily,
        "the Taylor rule has moved the rate off neutral"
    );
    let housing = world
        .iter::<core_ecs::sim_interface::HousingBook>()
        .expect("query")
        .next()
        .map(|(_, book)| *book)
        .expect("the v7 fixture has a housing ledger");
    assert!(
        housing.last_home_price_mills > 0,
        "the purchase clearing has measured a market price"
    );
    // Saved mid-shift: the Phase 5 action variants are in flight, so
    // the resume golden covers their (de)serialization too (SPEC §9).
    let working = world
        .iter::<sim_ai::CurrentAction>()
        .expect("query")
        .filter(|(_, action)| {
            matches!(
                action,
                sim_ai::CurrentAction::Work { .. } | sim_ai::CurrentAction::WorkTravel { .. }
            )
        })
        .count();
    assert!(working > 0, "the v7 fixture is saved mid-shift");
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "the loaded fixture must satisfy every conservation identity,
         bank vault and treasury included"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V7_GOLDEN_HASH_AT_LOAD),
        "loaded v7 state differs from the state that was saved"
    );
}

/// The one Phase 6 day-rate system the resume golden's window misses
/// (day 21 is not a purchase day): loaded state drives a REAL purchase
/// clearing at day 30 — mortgages are granted from restored books.
#[test]
fn v7_purchase_clearing_works_from_restored_state() {
    let mut sim = persistence::load_from_bytes(V7_FIXTURE, load_config(), runner::register_world)
        .expect("committed v7 save no longer loads");
    let granted_at_load: i64 = sim
        .world()
        .iter::<core_ecs::sim_interface::BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.granted.iter().map(|(_, m)| m.mills()).sum())
        .unwrap_or(0);
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    // Tick 29,360 → 43,810 crosses the day-30 boundary (tick 43,200),
    // a purchase-clearing day under the pinned 10-day cadence.
    sim.run_ticks(&mut schedule, 14_450).expect("resume failed");
    let granted_after: i64 = sim
        .world()
        .iter::<core_ecs::sim_interface::BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.granted.iter().map(|(_, m)| m.mills()).sum())
        .unwrap_or(0);
    assert!(
        granted_after > granted_at_load,
        "the day-30 clearing wrote new mortgages from restored state          ({granted_at_load} → {granted_after})"
    );
    assert!(
        debug_tools::audit_economy(sim.world()).expect("audit"),
        "every identity still holds ten days past the restore"
    );
}

#[test]
fn v7_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V7_FIXTURE, load_config(), runner::register_world)
        .expect("committed v7 save no longer loads");
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    assert!(derived.fixture);
    assert!(derived.economy);
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    sim.run_ticks(&mut schedule, 1000).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V7_GOLDEN_HASH_AFTER_1000),
        "resumed evolution diverged from the recording"
    );
}

// --- v8 (Phase 7: the social layer — skills, bonds, lifecycle) ----------

/// Committed v8 fixture: seed 43, 30 fixture entities + 250 citizens +
/// the full social layer, 29,360 ticks — day 20, mid-shift: skills
/// learned at school and on the job, thousands of live edges, marriages
/// concluded, all alongside the Phase 6 money layer.
const V8_FIXTURE: &[u8] =
    include_bytes!("../fixtures/v8_seed43_fixture30_citizens250_tick29360.embersave");

/// Golden hash of the v8 fixture at load.
const V8_GOLDEN_HASH_AT_LOAD: u64 = 0x75e9_6817_ef65_0330;

/// Golden hash after resuming the v8 fixture 1,000 ticks (the social
/// hour drifts bonds, then the day boundary's decay/marriage/fertility
/// pass runs with everything else).
const V8_GOLDEN_HASH_AFTER_1000: u64 = 0xa340_d2d0_f9ca_6f63;

#[test]
fn v8_golden_save_loads_to_the_exact_golden_state() {
    let sim = persistence::load_from_bytes(V8_FIXTURE, load_config(), runner::register_world)
        .expect("committed v8 save no longer loads: save-format break without a migration");
    assert_eq!(sim.tick(), Ticks::new(29360));
    assert_eq!(sim.seed(), Seed::new(43));
    let world = sim.world();
    assert!(
        world
            .iter::<core_ecs::sim_interface::Skills>()
            .expect("query")
            .filter(|(_, skills)| skills.levels.iter().any(|level| *level > 0))
            .count()
            > 0,
        "school and work have taught real skills"
    );
    let mut spouses = 0;
    let mut edges = 0;
    for (_, relationships) in world
        .iter::<core_ecs::sim_interface::Relationships>()
        .expect("query")
    {
        edges += relationships.edges.len();
        spouses += relationships
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, core_ecs::sim_interface::RelKind::Spouse))
            .count();
    }
    assert!(edges > 100, "the social graph is alive ({edges} edges)");
    assert!(spouses > 0, "marriages have concluded");
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "every conservation identity still holds under the social layer"
    );
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V8_GOLDEN_HASH_AT_LOAD),
        "loaded v8 state differs from the state that was saved"
    );
}

#[test]
fn v8_golden_save_resumes_deterministically() {
    let mut sim = persistence::load_from_bytes(V8_FIXTURE, load_config(), runner::register_world)
        .expect("committed v8 save no longer loads");
    let derived = runner::derive_spec_from_world(sim.world()).expect("derive");
    assert!(derived.fixture);
    assert!(derived.economy);
    let mut schedule = runner::build_schedule(&derived, &pinned_defs());
    sim.run_ticks(&mut schedule, 1000).expect("resume failed");
    assert_eq!(
        sim.state_hash().expect("hash failed"),
        WorldHash::new(V8_GOLDEN_HASH_AFTER_1000),
        "resumed evolution diverged from the recording"
    );
}
