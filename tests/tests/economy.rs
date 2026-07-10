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

/// ADR 0007 §6 abort semantics, directly: a `BuyPending` whose shelf
/// emptied or whose buyer went broke between deciding and arriving
/// aborts cleanly — nothing transferred, the action simply ends.
#[test]
fn purchases_abort_cleanly_when_stock_or_cash_vanished() {
    let defs = pinned_defs();
    let bread = defs
        .goods
        .goods
        .iter()
        .position(|g| g.id == "bread")
        .expect("bread exists") as u32;

    let find_bakery = |world: &core_ecs::World| -> core_ecs::Entity {
        world
            .iter::<RetailOffer>()
            .expect("query")
            .find(|(_, offer)| offer.good == bread)
            .map(|(entity, _)| entity)
            .expect("a bakery exists")
    };
    let first_citizens = |world: &core_ecs::World, n: usize| -> Vec<core_ecs::Entity> {
        world
            .iter::<sim_people::Identity>()
            .expect("query")
            .take(n)
            .map(|(entity, _)| entity)
            .collect()
    };
    let wallet = |world: &core_ecs::World, e: core_ecs::Entity| -> i64 {
        world
            .get::<Wallet>(e)
            .expect("query")
            .expect("wallet")
            .cash
            .mills()
    };

    // Case 1: the shelf emptied (through the counted sink) before the
    // purchase executed.
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(86), 30), &defs).expect("build");
    let bakery = find_bakery(sim.world());
    let citizen = first_citizens(sim.world(), 1)[0];
    sim_goods::spoil_stock(sim.world_mut(), bakery, bread, i64::MAX).expect("empty the shelf");
    let world = sim.world_mut();
    world
        .insert(citizen, core_ecs::sim_interface::Position { at: bakery })
        .expect("insert");
    world
        .insert(citizen, sim_ai::CurrentAction::BuyPending { at: bakery })
        .expect("insert");
    let cash_before = wallet(sim.world(), citizen);
    sim.step(&mut schedule).expect("step");
    assert_eq!(
        wallet(sim.world(), citizen),
        cash_before,
        "an aborted purchase must not move money"
    );
    assert!(
        sim.world()
            .get::<sim_ai::CurrentAction>(citizen)
            .expect("query")
            .is_none(),
        "the aborted action ends; the citizen re-decides next tick"
    );
    assert_eq!(
        counters(sim.world()).consumed_by_citizens[bread as usize],
        0,
        "nothing was consumed"
    );
    assert!(debug_tools::audit_economy(sim.world()).expect("audit"));

    // Case 2: the buyer's cash vanished (moved to a neighbor — a
    // conserving intervention) before the purchase executed.
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(87), 30), &defs).expect("build");
    let bakery = find_bakery(sim.world());
    let citizens = first_citizens(sim.world(), 2);
    let (buyer, neighbor) = (citizens[0], citizens[1]);
    let world = sim.world_mut();
    let estate = world
        .get::<Wallet>(buyer)
        .expect("query")
        .expect("wallet")
        .cash;
    world
        .get_mut::<Wallet>(buyer)
        .expect("query")
        .expect("wallet")
        .cash = core_types::Money::ZERO;
    let neighbor_wallet = world
        .get_mut::<Wallet>(neighbor)
        .expect("query")
        .expect("wallet");
    neighbor_wallet.cash = neighbor_wallet.cash.try_add(estate).expect("add");
    world
        .insert(buyer, core_ecs::sim_interface::Position { at: bakery })
        .expect("insert");
    world
        .insert(buyer, sim_ai::CurrentAction::BuyPending { at: bakery })
        .expect("insert");
    let shelf_before = sim
        .world()
        .get::<core_ecs::sim_interface::Inventory>(bakery)
        .expect("query")
        .expect("inventory")
        .stock(bread);
    assert!(
        shelf_before > 0,
        "the shelf is stocked; only the cash is gone"
    );
    sim.step(&mut schedule).expect("step");
    assert_eq!(wallet(sim.world(), buyer), 0, "still broke, not negative");
    assert!(
        sim.world()
            .get::<sim_ai::CurrentAction>(buyer)
            .expect("query")
            .is_none(),
        "the penniless purchase aborts cleanly"
    );
    assert_eq!(
        counters(sim.world()).consumed_by_citizens[bread as usize],
        0,
        "nothing was sold"
    );
    assert!(debug_tools::audit_economy(sim.world()).expect("audit"));
}

/// Two buyers, one loaf, same tick: the purchase pass re-checks LIVE
/// stock sequentially in entity order, so the shelf can never oversell —
/// the first buyer eats, the second aborts.
#[test]
fn same_tick_contention_never_oversells_the_last_unit() {
    let defs = pinned_defs();
    let bread = defs
        .goods
        .goods
        .iter()
        .position(|g| g.id == "bread")
        .expect("bread exists") as u32;
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(88), 30), &defs).expect("build");
    let bakery = sim
        .world()
        .iter::<RetailOffer>()
        .expect("query")
        .find(|(_, offer)| offer.good == bread)
        .map(|(entity, _)| entity)
        .expect("a bakery exists");
    let stock = sim
        .world()
        .get::<core_ecs::sim_interface::Inventory>(bakery)
        .expect("query")
        .expect("inventory")
        .stock(bread);
    sim_goods::spoil_stock(sim.world_mut(), bakery, bread, stock - 1).expect("down to one loaf");
    let citizens: Vec<core_ecs::Entity> = sim
        .world()
        .iter::<sim_people::Identity>()
        .expect("query")
        .take(2)
        .map(|(entity, _)| entity)
        .collect();
    let world = sim.world_mut();
    for citizen in &citizens {
        world
            .insert(*citizen, core_ecs::sim_interface::Position { at: bakery })
            .expect("insert");
        world
            .insert(*citizen, sim_ai::CurrentAction::BuyPending { at: bakery })
            .expect("insert");
    }
    sim.step(&mut schedule).expect("step");

    let world = sim.world();
    let winner_action = world
        .get::<sim_ai::CurrentAction>(citizens[0])
        .expect("query");
    assert!(
        matches!(winner_action, Some(sim_ai::CurrentAction::Consume { .. })),
        "the lower-indexed buyer got the loaf, got {winner_action:?}"
    );
    assert!(
        world
            .get::<sim_ai::CurrentAction>(citizens[1])
            .expect("query")
            .is_none(),
        "the second buyer aborted cleanly"
    );
    assert_eq!(
        world
            .get::<core_ecs::sim_interface::Inventory>(bakery)
            .expect("query")
            .expect("inventory")
            .stock(bread),
        0,
        "exactly one loaf left the shelf"
    );
    assert_eq!(counters(world).consumed_by_citizens[bread as usize], 1);
    assert!(debug_tools::audit_economy(world).expect("audit"));
}

/// The halt mechanism itself, end to end (ADR 0007 §5): seeded drift in
/// a wallet makes the SCHEDULED run error out at the next day boundary —
/// the auditor is wired in and its violation aborts the tick.
#[test]
fn a_scheduled_run_halts_on_seeded_money_drift() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(89), 40), &defs).expect("build");
    sim.run_ticks(&mut schedule, 2 * TICKS_PER_DAY)
        .expect("healthy run");

    // Mint one mill from nothing — precisely what the auditor exists to
    // catch.
    let victim = sim
        .world()
        .iter::<Wallet>()
        .expect("query")
        .next()
        .map(|(entity, _)| entity)
        .expect("wallets exist");
    let wallet = sim
        .world_mut()
        .get_mut::<Wallet>(victim)
        .expect("query")
        .expect("wallet");
    wallet.cash = wallet
        .cash
        .try_add(core_types::Money::from_mills(1))
        .expect("add");

    let err = sim
        .run_ticks(&mut schedule, TICKS_PER_DAY + 10)
        .expect_err("the next daily audit must halt the run");
    let message = err.to_string();
    assert!(message.contains("money conservation"), "{message}");
    assert!(message.contains("debug.audit"), "{message}");
}
