//! Phase 5 exit-criteria suite (SPEC §15): the labor market clears; a
//! wage distribution emerges; a firm's collapse produces measurable
//! local unemployment. Plus the payroll/conservation interplay
//! (ADR 0008 §6).

use core_ecs::sim_interface::{Employment, LaborStats, RetailOffer, Wallet};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};

const TICKS_PER_DAY: u64 = 1440;

fn labor_stats(world: &core_ecs::World) -> LaborStats {
    world
        .iter::<LaborStats>()
        .expect("query")
        .next()
        .map(|(_, stats)| *stats)
        .expect("towns carry a labor ledger")
}

fn total_positions(defs: &data_defs::DataDefs) -> u32 {
    defs.firms
        .kinds
        .iter()
        .map(|kind| kind.positions * kind.count)
        .sum::<u32>()
        // The treasury's public slots (Phase 6, ADR 0009 §4).
        + defs.taxes.public_positions
}

/// Exit criterion 1a — the market clears with excess workers: every slot
/// fills, and the leftover seekers are MEASURED as unmatched (never
/// assigned anything).
#[test]
fn labor_market_clears_when_workers_outnumber_positions() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(91), 150), &defs).expect("build");
    sim.run_ticks(&mut schedule, 3 * TICKS_PER_DAY)
        .expect("run");

    let stats = labor_stats(sim.world());
    let positions = total_positions(&defs);
    assert!(
        stats.working_age > positions,
        "the regime under test: more workers ({}) than slots ({positions})",
        stats.working_age
    );
    assert_eq!(
        stats.employed, positions,
        "every slot fills when bids cover the cheapest asks"
    );
    // Phase 6 churn (insolvency firings, reopened slots) means a few of
    // the day's seekers can be hired into freshly vacated slots; the
    // clearing proof is `employed == positions` above. What must hold:
    // the surplus is MEASURED — seekers beyond the reopened slots stay
    // unmatched, never silently assigned.
    assert!(
        stats.unmatched > 0 && stats.seeking >= stats.unmatched,
        "the surplus seekers are measured as unmatched (seeking {}, unmatched {})",
        stats.seeking,
        stats.unmatched
    );
    assert!(
        stats.seeking - stats.unmatched <= positions,
        "hires in one clearing never exceed the town's slot count"
    );
    assert!(stats.hires >= u64::from(positions));
}

/// Exit criterion 1b — the market clears with excess positions: every
/// working-age citizen finds a job and unemployment measures zero.
#[test]
fn labor_market_clears_when_positions_outnumber_workers() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(92), 30), &defs).expect("build");
    sim.run_ticks(&mut schedule, 3 * TICKS_PER_DAY)
        .expect("run");

    let stats = labor_stats(sim.world());
    assert!(
        stats.working_age < total_positions(&defs),
        "the regime under test: fewer workers than slots"
    );
    assert_eq!(
        stats.employed, stats.working_age,
        "everyone who can work is hired"
    );
    assert_eq!(stats.unmatched, 0, "unemployment measures zero");
}

/// Exit criterion 2 — a wage distribution emerges: heterogeneous
/// reservations and marginal products clear at genuinely dispersed
/// wages, not one administered number.
#[test]
fn a_wage_distribution_emerges() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(93), 150), &defs).expect("build");
    sim.run_ticks(&mut schedule, 5 * TICKS_PER_DAY)
        .expect("run");

    let wages: Vec<i64> = sim
        .world()
        .iter::<Employment>()
        .expect("query")
        .map(|(_, employment)| employment.wage_per_day.mills())
        .collect();
    assert!(wages.len() >= 20, "a real workforce to measure");
    let distinct: std::collections::BTreeSet<i64> = wages.iter().copied().collect();
    assert!(
        distinct.len() >= 5,
        "wages must disperse; got only {distinct:?}"
    );
    let (min, max) = (
        *distinct.iter().next().expect("nonempty"),
        *distinct.iter().next_back().expect("nonempty"),
    );
    assert!(
        max >= min + min / 5,
        "the spread is real (≥20% of the floor): {min}..{max}"
    );
    // Every wage is positive and every employer could book it (the daily
    // audit inside the schedule already proved payroll conserved).
    assert!(min > 0);
}

/// Exit criterion 3 — a firm's collapse produces measurable local
/// unemployment: bakeries with tiny starting capital (test-shaped data)
/// and their shelves destroyed through the counted sink every hour
/// cannot cover payroll, fire everyone (`Fired { Insolvent }`), and the
/// next clearings measure the spike — the market routes around the husk.
#[test]
fn a_firm_collapse_produces_measurable_unemployment() {
    let mut defs = pinned_defs();
    // Test-shaped balance (like the flat-mortality pattern): bakeries
    // open with ~1 day of payroll in the bank.
    for kind in &mut defs.firms.kinds {
        if kind.retail.as_ref().is_some_and(|r| r.need_id == "hunger") {
            kind.initial_cash_mills = 2_000;
        }
    }
    let bread = defs
        .goods
        .goods
        .iter()
        .position(|g| g.id == "bread")
        .expect("bread exists") as u32;
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(94), 150), &defs).expect("build");

    // Day 1 settles hiring: the bakeries staff up like everyone else.
    sim.run_ticks(&mut schedule, TICKS_PER_DAY + 60)
        .expect("run");
    let bakeries: Vec<core_ecs::Entity> = sim
        .world()
        .iter::<RetailOffer>()
        .expect("query")
        .filter(|(_, offer)| offer.good == bread)
        .map(|(entity, _)| entity)
        .collect();
    let bakery_staff = |world: &core_ecs::World| -> usize {
        world
            .iter::<Employment>()
            .expect("query")
            .filter(|(_, employment)| bakeries.contains(&employment.employer))
            .count()
    };
    let staffed_before = bakery_staff(sim.world());
    assert!(staffed_before >= 6, "the bakeries hired ({staffed_before})");
    let stats_before = labor_stats(sim.world());

    // The shock (ADR 0007 §8 path): burn the shelves hourly for nine
    // days — no revenue, payroll and procurement drain the till, wages
    // stop. (The first bakery entered the shock with a full day of the
    // whole town's bread sales banked — the tie-break favors it — so the
    // bleed-out takes about a week.)
    // (The `Fired { Insolvent }` fact itself is asserted at the unit
    // level in `sim_economy`; here the measured stats are the
    // observable.)
    let goods = defs.goods.goods.len() as u32;
    for _ in 0..(9 * 24) {
        sim.run_ticks(&mut schedule, 60).expect("run");
        for bakery in &bakeries {
            // Everything burns: inputs too, so no batch ever completes
            // and no loaf is ever sold.
            for good in 0..goods {
                sim_goods::spoil_stock(sim.world_mut(), *bakery, good, i64::MAX).expect("shock");
            }
        }
    }

    assert_eq!(bakery_staff(sim.world()), 0, "the husks employ nobody");
    let stats = labor_stats(sim.world());
    assert!(
        stats.firings >= stats_before.firings + staffed_before as u64,
        "the collapse fired the whole bakery workforce ({} -> {} firings \
         for {staffed_before} staff)",
        stats_before.firings,
        stats.firings
    );
    assert!(
        stats.unmatched > stats_before.unmatched,
        "the spike is measured: unmatched {} -> {}",
        stats_before.unmatched,
        stats.unmatched
    );
    // Conservation survived the collapse (the in-schedule auditor already
    // enforced it daily; re-check from outside).
    assert!(debug_tools::audit_economy(sim.world()).expect("audit"));
}

/// ADR 0008 §6 — wages close the loop: workers demonstrably EARN (their
/// wallets grow past the seeded maximum) while conservation holds.
#[test]
fn wages_flow_from_firms_to_workers() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(95), 100), &defs).expect("build");
    sim.run_ticks(&mut schedule, 20 * TICKS_PER_DAY)
        .expect("run");

    let world = sim.world();
    let seeded_max = defs.people.demographics.wealth_max_mills;
    // Wealth = wallet + vault row (Phase 6): the float keeps wallets
    // small while earnings accumulate in the bank.
    let vault: std::collections::BTreeMap<u32, i64> = world
        .iter::<core_ecs::sim_interface::BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| {
            book.deposits
                .iter()
                .map(|(owner, balance)| (owner.index(), balance.mills()))
                .collect()
        })
        .unwrap_or_default();
    let earners = world
        .iter::<Employment>()
        .expect("query")
        .filter(|(citizen, _)| {
            let cash = world
                .get::<Wallet>(*citizen)
                .expect("query")
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            cash + vault.get(&citizen.index()).copied().unwrap_or(0) > seeded_max
        })
        .count();
    assert!(
        earners > 5,
        "after 20 days of wages, workers hold more than any seed could ({earners})"
    );
    assert!(debug_tools::audit_economy(world).expect("audit"));
}

/// Deaths reopen slots and the market refills them (ADR 0008 §§3–4 with
/// mortality in the loop): under heavy mortality the town keeps every
/// slot staffed, rehiring as workers die, with conservation intact.
#[test]
fn deaths_reopen_slots_and_the_market_refills_them() {
    let mut defs = pinned_defs();
    defs.people.mortality.bands.clear();
    // ~3%/day: dozens of deaths across 12 days of a 150-citizen town.
    defs.people.mortality.terminal_per_day_chance_per_billion = 30_000_000;
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(96), 150), &defs).expect("build");
    sim.run_ticks(&mut schedule, 12 * TICKS_PER_DAY)
        .expect("run (the daily auditor rides along)");

    let world = sim.world();
    let population = headless::inspect::population(world).expect("count");
    assert!(population < 140, "the run must contain real deaths");
    let stats = labor_stats(world);
    // Not every slot must be filled — a churned firm's bid can honestly
    // price below every ask — but the workforce stays near capacity:
    // slots freed by death REFILL rather than leak.
    assert!(
        stats.employed * 5 >= total_positions(&defs) * 4,
        "the town stays near full staffing despite the deaths ({} of {})",
        stats.employed,
        total_positions(&defs)
    );
    assert!(
        stats.hires > u64::from(total_positions(&defs)),
        "rehiring happened ({} hires for {} slots)",
        stats.hires,
        total_positions(&defs)
    );
    // Every job belongs to a live citizen (despawn cleaned Employment).
    for (citizen, _) in world.iter::<Employment>().expect("query") {
        assert!(world.is_alive(citizen));
    }
    assert!(debug_tools::audit_economy(world).expect("audit"));
}
