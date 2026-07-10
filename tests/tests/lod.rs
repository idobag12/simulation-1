//! Phase 8 exit-criteria suite (SPEC §15): deterministic tier
//! assignment; A↔C cycling conserves money exactly (and needs across
//! the transitions); the catch-up controller is a deterministic,
//! conserving coarse integrator; tiered macro series match the
//! all-Tier-A twin within the data tolerance. The 10k timing budgets
//! are asserted in release builds only (`--release` in `check.sh`) —
//! debug builds still run every correctness assert.

use core_ecs::sim_interface::{
    BankBook, Employment, LaborStats, LodTier, RetailOffer, Spotlight, Tier, TreasuryBook, Wallet,
};
use core_ecs::{CommandBuffer, System, TickContext};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use sim_people::Identity;

const TICKS_PER_DAY: u64 = 1440;

/// One day's macro sample: employment, seekers, mean posted price,
/// treasury receipts, total citizen money (all mills or counts).
type MacroSample = (i64, i64, i64, i64, i64);
type SeriesPick = dyn Fn(&MacroSample) -> i64;

fn tier_counts(world: &core_ecs::World) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for (_, row) in world.iter::<LodTier>().expect("query") {
        match row.tier {
            Tier::A => counts.0 += 1,
            Tier::B => counts.1 += 1,
            Tier::C => counts.2 += 1,
        }
    }
    counts
}

/// Tier membership is deterministic, capped by data, assigned on the
/// very first day boundary (tick 0), and spotlight pins pull a citizen
/// into the embodied tier ahead of the stable front (ADR 0011 §1).
#[test]
fn tiers_assign_deterministically_and_spotlights_pin() {
    let mut defs = pinned_defs();
    defs.lod.tier_a_cap = 20;
    defs.lod.tier_b_cap = 30;
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(211), 100), &defs).expect("build");
    sim.run_ticks(&mut schedule, 10).expect("run");
    let (a, b, c) = tier_counts(sim.world());
    assert_eq!((a, b), (20, 30), "the caps bind in entity order");
    assert!(c >= 40, "the rest of the town is statistical ({c})");

    // Pin a Tier C citizen: the next day boundary promotes them.
    let pinned = sim
        .world()
        .iter::<LodTier>()
        .expect("query")
        .find(|(_, row)| matches!(row.tier, Tier::C))
        .map(|(citizen, _)| citizen)
        .expect("a C citizen exists");
    sim.world_mut()
        .insert(
            pinned,
            Spotlight {
                until_tick: 10 * TICKS_PER_DAY,
            },
        )
        .expect("insert");
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
    assert!(
        matches!(
            sim.world()
                .get::<LodTier>(pinned)
                .expect("query")
                .map(|row| row.tier),
            Some(Tier::A)
        ),
        "a spotlighted citizen is embodied ahead of the front"
    );
    // The caps still bind: someone else gave way.
    let (a, _, _) = tier_counts(sim.world());
    assert_eq!(a, 20);
}

/// Exit criterion — A↔C cycling conserves money EXACTLY: the demotion
/// and promotion transitions move no money and do not touch needs (the
/// live rows are the single source of truth), and a tiered town under
/// daily audits keeps every conservation identity while citizens cycle.
#[test]
fn a_to_c_to_a_cycling_conserves_money_exactly() {
    // Isolated transitions: everyone embodied, then a full demote and a
    // full promote — wallets and needs byte-identical across both.
    let mut defs = pinned_defs();
    defs.lod.tier_a_cap = 1_000_000; // all-A world
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(223), 80), &defs).expect("build");
    sim.run_ticks(&mut schedule, 3 * 60).expect("run");
    let tables = data_defs::resolve_ai(&defs);

    let snapshot = |world: &core_ecs::World| -> Vec<(u32, i64, Vec<i64>)> {
        let mut rows = Vec::new();
        for (citizen, _) in world.iter::<Identity>().expect("query") {
            let cash = world
                .get::<Wallet>(citizen)
                .expect("query")
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let needs = world
                .get::<sim_people::Needs>(citizen)
                .expect("query")
                .map(|needs| needs.levels.iter().map(|level| level.raw()).collect())
                .unwrap_or_default();
            rows.push((citizen.index(), cash, needs));
        }
        rows
    };
    let before = snapshot(sim.world());
    sim_ai::demote_all_to_c(sim.world_mut(), &tables).expect("demote");
    assert_eq!(
        snapshot(sim.world()),
        before,
        "demotion moves no money and touches no need"
    );
    assert!(
        sim.world()
            .iter::<core_ecs::sim_interface::DayModel>()
            .expect("query")
            .count()
            > 0,
        "demoted citizens carry day models"
    );
    // Promote everyone back through the assignment (caps cover the town).
    let ctx = TickContext {
        tick: sim.tick(),
        time: sim.calendar().time_of(sim.tick()),
    };
    let mut assign = sim_ai::TierAssignSystem::new(tables);
    assign
        .run(sim.world_mut(), &ctx, &mut CommandBuffer::new())
        .expect("assign");
    assert_eq!(
        snapshot(sim.world()),
        before,
        "the full A→C→A cycle conserves money exactly and needs verbatim"
    );
    assert_eq!(
        sim.world()
            .iter::<core_ecs::sim_interface::DayModel>()
            .expect("query")
            .count(),
        0,
        "promotion drops the models"
    );
    let (a, b, c) = tier_counts(sim.world());
    assert_eq!((b, c), (0, 0), "everyone is embodied again ({a} A)");

    // Integrated: a tiered town cycles citizens for a week under the
    // scheduled DAILY AUDIT — finishing is the conservation proof.
    let mut defs = pinned_defs();
    defs.lod.tier_a_cap = 15;
    defs.lod.tier_b_cap = 25;
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(227), 120), &defs).expect("build");
    sim.run_ticks(&mut schedule, 7 * TICKS_PER_DAY)
        .expect("a halted run means a daily audit failed");
    let world = sim.world();
    let (a, b, c) = tier_counts(world);
    assert!(a > 0 && b > 0 && c > 0, "mixed tiers ({a}/{b}/{c})");
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "every conservation identity holds over a cycling week"
    );
    // Statistical citizens live real economic lives: some hold jobs and
    // real money moved through their wallets.
    let employed_c = world
        .iter::<LodTier>()
        .expect("query")
        .filter(|(citizen, row)| {
            matches!(row.tier, Tier::C)
                && world.get::<Employment>(*citizen).expect("query").is_some()
        })
        .count();
    assert!(employed_c > 0, "Tier C citizens hold real jobs");
}

/// Exit criterion — the catch-up controller: a DEFINED deterministic
/// coarse integrator (same save caught up twice lands identically),
/// conserving (audits green after), and resumable by the normal loop.
#[test]
fn a_week_of_catchup_is_deterministic_and_conserves() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(229), 120), &defs).expect("build");
    sim.run_ticks(&mut schedule, 5 * TICKS_PER_DAY)
        .expect("run");
    let bytes = persistence::save_to_bytes(&sim).expect("save");
    let start_tick = sim.tick().raw();

    let load = || {
        persistence::load_from_bytes(
            &bytes,
            runner::load_config(&defs).expect("config"),
            runner::register_world,
        )
        .expect("load")
    };
    let mut first = load();
    runner::catch_up(&mut first, &defs, 7).expect("catch up");
    assert_eq!(
        first.tick().raw(),
        start_tick + 7 * TICKS_PER_DAY,
        "catch-up advances exactly the requested span"
    );
    assert!(
        debug_tools::audit_economy(first.world()).expect("audit"),
        "every conservation identity holds after the coarse week"
    );
    let mut second = load();
    runner::catch_up(&mut second, &defs, 7).expect("catch up");
    assert_eq!(
        first.state_hash().expect("hash"),
        second.state_hash().expect("hash"),
        "catch-up is deterministic: same save, same week, same world"
    );
    // The normal per-tick loop resumes from caught-up state.
    let derived = runner::derive_spec_from_world(first.world()).expect("derive");
    let mut schedule = runner::build_schedule(&derived, &defs);
    first
        .run_ticks(&mut schedule, TICKS_PER_DAY)
        .expect("resume");
    assert!(debug_tools::audit_economy(first.world()).expect("audit"));
}

/// One day's macro sample: employment, seekers, mean posted price
/// (mills), treasury receipts (mills), and total citizen money (wallet
/// + deposit rows, mills).
fn macro_sample(world: &core_ecs::World) -> MacroSample {
    let employed = world.iter::<Employment>().expect("query").count() as i64;
    let seeking = world
        .iter::<LaborStats>()
        .expect("query")
        .next()
        .map(|(_, stats)| stats.seeking as i64)
        .unwrap_or(0);
    let (offer_sum, offer_count) = world
        .iter::<RetailOffer>()
        .expect("query")
        .fold((0i64, 0i64), |(sum, count), (_, offer)| {
            (sum + offer.unit_price.mills(), count + 1)
        });
    let mean_price = if offer_count > 0 {
        offer_sum / offer_count
    } else {
        0
    };
    let receipts = world
        .iter::<TreasuryBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.income_tax_received.mills() + book.sales_tax_received.mills())
        .unwrap_or(0);
    let deposits: std::collections::BTreeMap<u32, i64> = world
        .iter::<BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| {
            book.deposits
                .iter()
                .map(|(owner, balance)| (owner.index(), balance.mills()))
                .collect()
        })
        .unwrap_or_default();
    let mut citizen_money = 0i64;
    for (citizen, _) in world.iter::<Identity>().expect("query") {
        citizen_money += world
            .get::<Wallet>(citizen)
            .expect("query")
            .map(|wallet| wallet.cash.mills())
            .unwrap_or(0);
        citizen_money += deposits.get(&citizen.index()).copied().unwrap_or(0);
    }
    (employed, seeking, mean_price, receipts, citizen_money)
}

/// Exit criterion — macro time-series statistically indistinguishable
/// (the DEFINED tolerance, from `balance/lod.ron`) between an
/// all-Tier-A small town and the same town under tiny caps: same seed,
/// same data, only the tier caps differ; per-day series averaged over
/// the run must agree within the tolerance.
#[test]
fn tiered_macro_series_match_the_all_tier_a_twin() {
    let run = |a_cap: u32, b_cap: u32| -> Vec<MacroSample> {
        let mut defs = pinned_defs();
        defs.lod.tier_a_cap = a_cap;
        defs.lod.tier_b_cap = b_cap;
        let (mut sim, mut schedule) =
            runner::build_simulation(&WorldSpec::town(Seed::new(233), 150), &defs).expect("build");
        let mut series = Vec::new();
        for _ in 0..12 {
            sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
            series.push(macro_sample(sim.world()));
        }
        series
    };
    let embodied = run(1_000_000, 0);
    let tiered = run(20, 40);

    let tolerance = i64::from(pinned_defs().lod.macro_tolerance_per_mille);
    let mean = |pick: &SeriesPick, series: &[MacroSample]| {
        series.iter().map(pick).sum::<i64>() / series.len() as i64
    };
    let series: [(&str, &SeriesPick); 5] = [
        ("employment", &|sample| sample.0),
        ("seekers", &|sample| sample.1),
        ("mean price", &|sample| sample.2),
        ("treasury receipts", &|sample| sample.3),
        ("citizen money", &|sample| sample.4),
    ];
    for (name, pick) in series {
        let a = mean(pick, &embodied);
        let b = mean(pick, &tiered);
        let scale = a.abs().max(b.abs()).max(1);
        assert!(
            (a - b).abs() * 1000 <= tolerance * scale,
            "{name}: all-A {a} vs tiered {b} exceeds the {tolerance}/1000 band"
        );
    }
}

/// Exit criterion — the 10k performance budget (release builds only:
/// debug timing says nothing about the shipped budget). Tick 0 assigns
/// tiers, so the town runs mixed from the first minute.
#[cfg(not(debug_assertions))]
#[test]
fn ten_thousand_citizens_hit_the_tick_budget() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(239), 10_000), &defs).expect("build");
    // Settle the first day (assignment, first clearings) off the clock.
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("warmup");
    let (a, b, c) = tier_counts(sim.world());
    assert!(
        a <= 200 && b <= 2000 && c >= 7_000,
        "the shipped caps shape a 10k town ({a}/{b}/{c})"
    );
    let span = 2_000u64;
    let start = std::time::Instant::now();
    sim.run_ticks(&mut schedule, span).expect("run");
    let elapsed = start.elapsed();
    let ticks_per_sec = span as f64 / elapsed.as_secs_f64();
    assert!(
        ticks_per_sec >= 200.0,
        "SPEC §10 budget: {ticks_per_sec:.0} ticks/sec < 200 at 10k citizens"
    );
}

/// Exit criterion — one week of catch-up under 5 seconds at 10k
/// citizens (release builds only).
#[cfg(not(debug_assertions))]
#[test]
fn a_week_of_catchup_fits_the_five_second_budget() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(241), 10_000), &defs).expect("build");
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("warmup");
    let start = std::time::Instant::now();
    runner::catch_up(&mut sim, &defs, 7).expect("catch up");
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "SPEC §5 budget: a simulated week took {elapsed:?} (>= 5s) at 10k citizens"
    );
    assert!(debug_tools::audit_economy(sim.world()).expect("audit"));
}
