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

/// One day's macro sample: employment rate (per-mille of citizens),
/// seekers, mean posted price (mills), treasury receipts (mills), total
/// citizen money (mills), and cumulative units consumed by citizens.
type MacroSample = (i64, i64, i64, i64, i64, i64);
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

    // Integrated: a PROVEN A→C→A cycle inside the live schedule, under
    // the daily audit, with the cycled citizen's needs checked against
    // the all-Tier-A twin (ADR 0011 §8, as amended). Caps 15/0 make the
    // displacement direct (A↔C, no B stop), and a spotlight pin forces
    // the cycle deterministically: pinning a Tier C citizen pushes the
    // BACK of the embodied front out to C; the pin's expiry brings them
    // home to A.
    let tier_of = |sim: &sim_time::Simulation, citizen: core_ecs::Entity| {
        sim.world()
            .get::<LodTier>(citizen)
            .expect("query")
            .map(|row| row.tier)
    };
    let build = |a_cap: u32| {
        let mut defs = pinned_defs();
        defs.lod.tier_a_cap = a_cap;
        defs.lod.tier_b_cap = 0;
        runner::build_simulation(&WorldSpec::town(Seed::new(227), 120), &defs).expect("build")
    };
    let (mut sim, mut schedule) = build(15);
    // The all-Tier-A twin: same seed, same data, caps beyond the town.
    let (mut twin, mut twin_schedule) = build(1_000_000);
    sim.run_ticks(&mut schedule, 2 * TICKS_PER_DAY)
        .expect("a halted run means a daily audit failed");
    twin.run_ticks(&mut twin_schedule, 2 * TICKS_PER_DAY)
        .expect("twin");
    // The victim: the back of the embodied front (highest-index Tier A
    // citizen) — the first displaced when a pin jumps the queue.
    let victim = sim
        .world()
        .iter::<LodTier>()
        .expect("query")
        .filter(|(_, row)| matches!(row.tier, Tier::A))
        .map(|(citizen, _)| citizen)
        .max_by_key(|citizen| citizen.index())
        .expect("an embodied citizen exists");
    let pinned = sim
        .world()
        .iter::<LodTier>()
        .expect("query")
        .find(|(_, row)| matches!(row.tier, Tier::C))
        .map(|(citizen, _)| citizen)
        .expect("a statistical citizen exists");
    assert_eq!(tier_of(&sim, victim), Some(Tier::A));
    // Pin through day 2's boundary; expiry lands exactly on day 3's
    // (the assignment drops pins with `until_tick <= boundary`).
    sim.world_mut()
        .insert(
            pinned,
            Spotlight {
                until_tick: 3 * TICKS_PER_DAY,
            },
        )
        .expect("insert");
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
    twin.run_ticks(&mut twin_schedule, TICKS_PER_DAY)
        .expect("twin");
    assert_eq!(tier_of(&sim, pinned), Some(Tier::A), "the pin promoted C→A");
    assert_eq!(
        tier_of(&sim, victim),
        Some(Tier::C),
        "the displaced front citizen demoted A→C"
    );
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
    twin.run_ticks(&mut twin_schedule, TICKS_PER_DAY)
        .expect("twin");
    assert_eq!(
        tier_of(&sim, victim),
        Some(Tier::A),
        "the pin expired and the cycle closed: A→C→A inside the live \
         schedule, every boundary audited"
    );
    assert_eq!(tier_of(&sim, pinned), Some(Tier::C));
    assert!(
        debug_tools::audit_economy(sim.world()).expect("audit"),
        "every conservation identity holds through the forced cycle"
    );
    // Needs within the data tolerance after re-promotion: the cycled
    // citizen against their all-Tier-A twin self at the same tick.
    let needs_of = |sim: &sim_time::Simulation, citizen: core_ecs::Entity| -> Vec<i64> {
        sim.world()
            .get::<sim_people::Needs>(citizen)
            .expect("query")
            .map(|needs| needs.levels.iter().map(|level| level.raw()).collect())
            .unwrap_or_default()
    };
    let tolerance = i64::from(pinned_defs().lod.macro_tolerance_per_mille);
    let max = core_ecs::sim_interface::NeedLevel::MAX.raw();
    for (need, (cycled, embodied)) in needs_of(&sim, victim)
        .iter()
        .zip(needs_of(&twin, victim).iter())
        .enumerate()
    {
        // A need's instantaneous level swings on the TIMING of its
        // last satisfaction alone — bread lands once, hours apart, in
        // two honest micro-timelines; an hour at the fireside does the
        // same for venue needs. The band allows the acceptance
        // tolerance plus one satisfaction quantum (a unit's gain, or
        // one hour-block at the best satisfier).
        let unit_quantum = sim
            .world()
            .iter::<RetailOffer>()
            .expect("query")
            .filter(|(_, offer)| offer.need_index == need as u32)
            .map(|(_, offer)| offer.gain_per_unit)
            .max()
            .unwrap_or(0);
        let tables = data_defs::resolve_ai(&pinned_defs());
        let block_quantum = (0..tables.kind_satisfiers.len() as u32)
            .filter_map(|kind| tables.satisfier_rate(kind, need as u32))
            .max()
            .unwrap_or(0)
            * 60;
        let quantum = unit_quantum.max(block_quantum);
        assert!(
            (cycled - embodied).abs() <= tolerance * max / 1000 + quantum,
            "need {need}: cycled {cycled} vs embodied twin {embodied} \
             exceeds the tolerance band plus one purchase quantum"
        );
    }
    // Statistical citizens live real economic lives: some hold jobs.
    let world = sim.world();
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
    let population = world.iter::<Identity>().expect("query").count().max(1) as i64;
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
    let consumed: i64 = world
        .iter::<core_ecs::sim_interface::EconCounters>()
        .expect("query")
        .next()
        .map(|(_, counters)| counters.consumed_by_citizens.iter().sum())
        .unwrap_or(0);
    (
        employed * 1000 / population,
        seeking,
        mean_price,
        receipts,
        citizen_money,
        consumed,
    )
}

/// Exit criterion — macro time-series statistically indistinguishable
/// (the DEFINED tolerance, from `balance/lod.ron`) between an
/// all-Tier-A small town and the same town under tiny caps: same seed,
/// same data, only the tier caps differ; per-day series averaged over
/// the run must agree within the tolerance.
#[test]
fn tiered_macro_series_match_the_all_tier_a_twin() {
    let run = |seed: u64, a_cap: u32, b_cap: u32| -> Vec<MacroSample> {
        let mut defs = pinned_defs();
        defs.lod.tier_a_cap = a_cap;
        defs.lod.tier_b_cap = b_cap;
        let (mut sim, mut schedule) =
            runner::build_simulation(&WorldSpec::town(Seed::new(seed), 150), &defs).expect("build");
        // Four unmeasured warm-up days: genesis needs are full, so the
        // integrators' cold starts differ by construction (embodied
        // citizens top up opportunistically; the day model waits for a
        // deficit) — the criterion is the STEADY state's agreement.
        sim.run_ticks(&mut schedule, 4 * TICKS_PER_DAY)
            .expect("run");
        let mut series = Vec::new();
        let mut previous = macro_sample(sim.world());
        for _ in 0..12 {
            sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
            let sample = macro_sample(sim.world());
            // The cumulative counters (receipts, consumption) compare
            // as PER-DAY deltas — a level comparison would smear one
            // early divergence over every later day.
            series.push((
                sample.0,
                sample.1,
                sample.2,
                sample.3 - previous.3,
                sample.4,
                sample.5 - previous.5,
            ));
            previous = sample;
        }
        series
    };
    let tolerance = i64::from(pinned_defs().lod.macro_tolerance_per_mille);
    let series: [(&str, &SeriesPick); 6] = [
        ("employment rate", &|sample| sample.0),
        ("seekers", &|sample| sample.1),
        ("mean price", &|sample| sample.2),
        ("treasury receipts per day", &|sample| sample.3),
        ("citizen money", &|sample| sample.4),
        ("units consumed per day", &|sample| sample.5),
    ];
    // Several seeds, compared PER DAY (a run-mean comparison would let
    // opposite-sign daily divergences cancel): the mean daily relative
    // difference of each series must sit inside the data band.
    for seed in [233u64, 331, 433] {
        let embodied = run(seed, 1_000_000, 0);
        let tiered = run(seed, 20, 40);
        for (name, pick) in series {
            let mut relative_sum = 0i64;
            for (a_day, b_day) in embodied.iter().zip(tiered.iter()) {
                let a = pick(a_day);
                let b = pick(b_day);
                let scale = a.abs().max(b.abs()).max(1);
                relative_sum += (a - b).abs() * 1000 / scale;
            }
            let mean_relative = relative_sum / embodied.len() as i64;
            assert!(
                mean_relative <= tolerance,
                "seed {seed}, {name}: mean daily divergence {mean_relative}/1000 \
                 exceeds the {tolerance}/1000 band"
            );
        }
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
        ticks_per_sec >= 300.0,
        "the Phase 9 HEADROOM floor (ADR 0012 §5; the SPEC §10 budget \
         is 200): {ticks_per_sec:.0} ticks/sec < 300 at 10k citizens \
         with the map wired"
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
        elapsed.as_secs_f64() < 2.5,
        "the Phase 9 HEADROOM floor (ADR 0012 §5; the SPEC §15 budget \
         is 5 s): a simulated week took {elapsed:?} at 10k citizens"
    );
    assert!(debug_tools::audit_economy(sim.world()).expect("audit"));
}
