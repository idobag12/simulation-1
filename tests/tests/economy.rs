//! Phase 4 exit-criteria suite (SPEC §15): money and goods conservation
//! audits pass over 100 days; prices respond to seeded supply shocks in
//! the correct direction. Plus the ADR 0007 §3 ledger identity and the
//! Phase 4 inspector surfaces.

use core_ecs::sim_interface::{EconCounters, FirmBooks, RetailOffer, Wallet};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};

const TICKS_PER_DAY: u64 = 1440;

fn counters(world: &core_ecs::World) -> EconCounters {
    world
        .iter::<EconCounters>()
        .expect("query")
        .next()
        .map(|(_, c)| c.clone())
        .expect("towns carry a conservation ledger")
}

/// Exit criterion 1: the auditors pass over 100 days. The audit system
/// runs INSIDE the schedule every day and halts the run on any drift —
/// so completing the run is the proof; the final assertions confirm the
/// economy was genuinely active the whole time (a dead economy conserves
/// trivially) and re-check every identity once more from outside.
#[test]
fn conservation_audits_pass_over_100_days_of_live_economy() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(81), 80);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");

    sim.run_ticks(&mut schedule, 100 * TICKS_PER_DAY)
        .expect("a halted run means an audit failed — conservation broke");

    let world = sim.world();
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "the final state must satisfy every conservation identity"
    );

    // The century was economically alive: every counter moved.
    let c = counters(world);
    assert!(
        c.consumed_by_citizens.iter().sum::<i64>() > 1000,
        "citizens bought goods all along ({:?})",
        c.consumed_by_citizens
    );
    assert!(
        c.consumed_in_production.iter().sum::<i64>() > 1000,
        "firms transformed inputs all along"
    );
    assert!(
        c.spoiled.iter().sum::<i64>() > 100,
        "the spoilage sink destroyed stock all along"
    );
    // Money moved from citizens to firms (sales), never in aggregate.
    let firm_revenue: i64 = world
        .iter::<FirmBooks>()
        .expect("query")
        .map(|(_, books)| books.revenue.mills())
        .sum();
    assert!(firm_revenue > 100_000, "firms earned real revenue");
}

/// ADR 0007 §3: the double-entry slice balances per firm — checked
/// directly here (the auditor also checks it daily), with books that
/// demonstrably moved on both sides.
#[test]
fn firm_books_balance_and_both_sides_move() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(82), 100);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");
    sim.run_ticks(&mut schedule, 10 * TICKS_PER_DAY)
        .expect("run");

    let world = sim.world();
    let mut earned = 0;
    let mut spent = 0;
    let mut firms = 0;
    for (entity, books) in world.iter::<FirmBooks>().expect("query") {
        firms += 1;
        let cash = world
            .get::<Wallet>(entity)
            .expect("query")
            .expect("firms carry wallets")
            .cash;
        assert_eq!(
            cash.try_sub(books.initial_cash).expect("sub"),
            books.revenue.try_sub(books.expenses).expect("sub"),
            "firm #{}: cash − initial_cash must equal revenue − expenses",
            entity.index()
        );
        if books.revenue.mills() > 0 {
            earned += 1;
        }
        if books.expenses.mills() > 0 {
            spent += 1;
        }
    }
    assert!(firms >= 5, "the town has its firms");
    assert!(earned >= 3, "several firms booked revenue");
    assert!(spent >= 3, "several firms booked expenses");
}

/// Exit criterion 2: prices respond to a seeded supply shock in the
/// correct direction. Two identical runs; at day 10 one has every
/// bakery's bread destroyed through the modeled spoilage sink (the
/// counters-consistent path — the auditor keeps passing, ADR 0007 §8).
/// Two days later the shocked town's posted bread prices are strictly
/// higher than the baseline's, and never rose faster than the
/// controller's bounded step. (The glut direction is unit-tested at the
/// controller in `sim_economy`.)
#[test]
fn prices_rise_after_a_seeded_supply_shock() {
    let defs = pinned_defs();
    let bread = defs
        .goods
        .goods
        .iter()
        .position(|g| g.id == "bread")
        .expect("bread exists") as u32;
    let spec = WorldSpec::town(Seed::new(83), 60);
    let shock_at = 10 * TICKS_PER_DAY;
    let observe_after = 2 * TICKS_PER_DAY;

    let bread_prices = |world: &core_ecs::World| -> Vec<i64> {
        world
            .iter::<RetailOffer>()
            .expect("query")
            .filter(|(_, offer)| offer.good == bread)
            .map(|(_, offer)| offer.unit_price.mills())
            .collect()
    };

    // Baseline: untouched.
    let (mut baseline, mut baseline_schedule) =
        runner::build_simulation(&spec, &defs).expect("build");
    baseline
        .run_ticks(&mut baseline_schedule, shock_at + observe_after)
        .expect("run");

    // Shocked: identical up to day 10, then the shelves burn down —
    // through the same counted sink daily spoilage uses.
    let (mut shocked, mut shocked_schedule) =
        runner::build_simulation(&spec, &defs).expect("build");
    shocked
        .run_ticks(&mut shocked_schedule, shock_at)
        .expect("run");
    let bakeries: Vec<(core_ecs::Entity, i64)> = shocked
        .world()
        .iter::<RetailOffer>()
        .expect("query")
        .filter(|(_, offer)| offer.good == bread)
        .map(|(entity, offer)| (entity, offer.unit_price.mills()))
        .collect();
    assert!(!bakeries.is_empty());
    let mut destroyed_total = 0;
    for (bakery, _) in &bakeries {
        destroyed_total +=
            sim_goods::spoil_stock(shocked.world_mut(), *bakery, bread, i64::MAX).expect("shock");
    }
    assert!(destroyed_total > 0, "the shock destroyed real stock");
    assert!(
        debug_tools::audit_economy(shocked.world()).expect("audit"),
        "a counters-consistent shock leaves conservation intact"
    );
    shocked
        .run_ticks(&mut shocked_schedule, observe_after)
        .expect("post-shock run (the daily auditor keeps passing)");

    // Direction: every shocked bakery posts a higher bread price than its
    // baseline twin.
    let baseline_prices = bread_prices(baseline.world());
    let shocked_prices = bread_prices(shocked.world());
    assert_eq!(baseline_prices.len(), shocked_prices.len());
    for (index, (base, shocked_price)) in baseline_prices.iter().zip(&shocked_prices).enumerate() {
        assert!(
            shocked_price > base,
            "bakery {index}: shocked price {shocked_price} must exceed baseline {base}"
        );
    }

    // Bounded movement: from the pre-shock posted price, two repricings
    // can raise at most (1 + step/1000)² (data-defined controller bound).
    let step = defs.economy.controller_step_per_mille;
    for ((_, pre_shock), shocked_price) in bakeries.iter().zip(&shocked_prices) {
        let mut bound = *pre_shock;
        for _ in 0..2 {
            bound += (bound * step / 1000).max(1);
        }
        assert!(
            *shocked_price <= bound,
            "price {shocked_price} exceeded the controller bound {bound} from {pre_shock}"
        );
    }
}

/// The Phase 4 observables render (SPEC §13): the economy report shows
/// firms, counters, and a passing audit; citizen dumps show the wallet.
#[test]
fn economy_report_and_wallet_are_inspectable() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(84), 40);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");
    sim.run_ticks(&mut schedule, 3 * TICKS_PER_DAY)
        .expect("run");

    let report = headless::inspect::economy(&sim, &defs).expect("report");
    for section in [
        "firm #",
        "bakery",
        "posts bread",
        "issued:",
        "counters",
        "audit: PASS",
    ] {
        assert!(
            report.contains(section),
            "missing `{section}` in:\n{report}"
        );
    }

    // Entity index 1 is the first genesis household's first member.
    let dump = headless::inspect::inspect_entity(&sim, &defs, 1).expect("inspect");
    assert!(dump.contains("wallet: "), "{dump}");
}
