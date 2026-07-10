//! Unit tests for the labor and public-payroll systems (split from
//! `systems_tests.rs` for the SPEC §3 module-size rule). Shares the
//! fixture helpers via `super::tests`.

use core_ecs::sim_interface::{Employment, FirmBooks, Wallet};
use core_ecs::{CommandBuffer, System};
use core_types::{Money, Ticks};

use super::tests::{ctx, firm_entities, staff, tables, world_with_firms};

#[test]
fn payroll_pays_booked_wages_and_fires_when_the_wallet_runs_dry() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let farm = firms[0];
    let worker = staff(&mut world, farm);
    // Wage 100/day against 10_000 cash: pays fine today.
    let mut payroll = crate::PayrollSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    payroll.run(&mut world, &ctx(), &mut cmd).expect("run");
    assert_eq!(
        world
            .get::<Wallet>(worker)
            .expect("get")
            .expect("some")
            .cash,
        Money::from_mills(90),
        "the worker got the wage net of the 10% withholding \
         (workers spawn cashless here)"
    );
    let books = world.get::<FirmBooks>(farm).expect("get").expect("some");
    assert_eq!(
        books.expenses,
        Money::from_mills(100),
        "payroll books the GROSS wage"
    );
    let treasury_book = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .expect("query")
        .next()
        .map(|(_, book)| *book)
        .expect("treasury");
    assert_eq!(
        treasury_book.income_tax_received,
        Money::from_mills(10),
        "the withheld tax landed in the treasury"
    );

    // Drain the firm (conserving: park its cash on the worker), then the
    // next payroll fires instead of paying.
    let cash = world.get::<Wallet>(farm).expect("get").expect("some").cash;
    world
        .get_mut::<Wallet>(farm)
        .expect("get")
        .expect("some")
        .cash = Money::ZERO;
    let worker_wallet = world.get_mut::<Wallet>(worker).expect("get").expect("some");
    worker_wallet.cash = worker_wallet.cash.try_add(cash).expect("add");
    payroll.run(&mut world, &ctx(), &mut cmd).expect("run");
    assert!(
        world.get::<Employment>(worker).expect("get").is_none(),
        "an unpayable wage fires (reason Insolvent)"
    );
    world.begin_tick(Ticks::new(1));
    let fired = world
        .events::<core_ecs::sim_interface::Fired>()
        .expect("events");
    assert_eq!(fired.len(), 1);
    assert!(matches!(
        fired[0].reason,
        core_ecs::sim_interface::FiredReason::Insolvent
    ));
}

#[test]
fn the_daily_clearing_matches_bids_to_asks_and_measures_the_rest() {
    // One slot per firm: two slots for three seekers — the priciest ask
    // must stay unmatched (measured, not assigned).
    let mut tables = tables();
    tables.firm_kinds[0].positions = 1;
    tables.firm_kinds[1].positions = 1;
    let mut world = world_with_firms(&tables);
    // Three working-age seekers: cheap (industrious, poor), middling, and
    // one so wealthy the reservation outprices every bid.
    let mut seekers = Vec::new();
    for (cash, trait_per_mille) in [(0i64, 900i16), (10_000, 300), (10_000_000, 0)] {
        let citizen = world.spawn();
        world
            .insert(
                citizen,
                Wallet {
                    cash: Money::from_mills(cash),
                },
            )
            .expect("insert");
        world
            .insert(
                citizen,
                core_ecs::sim_interface::Personality {
                    weights: vec![trait_per_mille],
                },
            )
            .expect("insert");
        world
            .insert(citizen, core_ecs::sim_interface::WorkingAge)
            .expect("insert");
        seekers.push(citizen);
    }

    let mut market = crate::LaborMarketSystem::new(tables);
    let mut cmd = CommandBuffer::new();
    market.run(&mut world, &ctx(), &mut cmd).expect("run");

    // Both firms bid (2 slots each, marginal product >> reservations);
    // the two affordable seekers match, the millionaire stays out.
    assert!(
        world.get::<Employment>(seekers[0]).expect("get").is_some(),
        "the cheapest ask matches first"
    );
    assert!(world.get::<Employment>(seekers[1]).expect("get").is_some());
    assert!(
        world.get::<Employment>(seekers[2]).expect("get").is_none(),
        "with no slot left, the priciest ask stays unmatched"
    );
    let stats = world
        .iter::<core_ecs::sim_interface::LaborStats>()
        .expect("query")
        .next()
        .map(|(_, s)| *s)
        .expect("stats");
    assert_eq!(stats.working_age, 3);
    assert_eq!(stats.seeking, 3);
    assert_eq!(stats.employed, 2);
    assert_eq!(stats.unmatched, 1);
    assert_eq!(stats.hires, 2);
    // Wages: distinct (heterogeneous asks) and within the bid/ask band.
    let w0 = world
        .get::<Employment>(seekers[0])
        .expect("get")
        .expect("some")
        .wage_per_day;
    let w1 = world
        .get::<Employment>(seekers[1])
        .expect("get")
        .expect("some")
        .wage_per_day;
    // The best bid pairs with the best ask, so the cheapest seeker
    // captures the largest surplus — wages genuinely disperse.
    assert!(w0 != w1, "heterogeneous matches clear at distinct wages");
}

#[test]
fn payroll_fires_redundant_extras_down_to_positions() {
    // Two positions, three employees (as if data shrank): the highest-
    // indexed extra is fired with reason Redundant and never paid; the
    // two keepers are paid normally.
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let farm = firm_entities(&world)[0];
    let workers: Vec<core_ecs::Entity> = (0..3).map(|_| staff(&mut world, farm)).collect();
    let mut payroll = crate::PayrollSystem::new(tables);
    let mut cmd = CommandBuffer::new();
    payroll.run(&mut world, &ctx(), &mut cmd).expect("run");

    assert!(
        world.get::<Employment>(workers[2]).expect("get").is_none(),
        "the highest-indexed extra is fired"
    );
    assert_eq!(
        world
            .get::<Wallet>(workers[2])
            .expect("get")
            .expect("some")
            .cash,
        Money::ZERO,
        "a redundantly fired worker is not paid"
    );
    for keeper in &workers[..2] {
        assert!(world.get::<Employment>(*keeper).expect("get").is_some());
        assert_eq!(
            world
                .get::<Wallet>(*keeper)
                .expect("get")
                .expect("some")
                .cash,
            Money::from_mills(90),
            "keepers are paid normally (net of withholding)"
        );
    }
    world.begin_tick(Ticks::new(1));
    let fired = world
        .events::<core_ecs::sim_interface::Fired>()
        .expect("events");
    assert_eq!(fired.len(), 1);
    assert!(matches!(
        fired[0].reason,
        core_ecs::sim_interface::FiredReason::Redundant
    ));
    let stats = world
        .iter::<core_ecs::sim_interface::LaborStats>()
        .expect("query")
        .next()
        .map(|(_, s)| *s)
        .expect("stats");
    assert_eq!(stats.firings, 1);
}

#[test]
fn reservation_wages_rise_with_wealth_and_fall_with_industriousness() {
    // ADR 0008 §3's ask formula, term by term (base 150, wealth 300‰
    // saturating at half-wealth 20k, trait discount 400‰).
    let market = crate::LaborMarketSystem::new(tables());
    let broke_lazy = market.reservation(0, 0, 0).expect("ask");
    assert_eq!(broke_lazy, 150, "base only");
    let rich_lazy = market.reservation(20_000, 0, 0).expect("ask");
    assert_eq!(rich_lazy, 150 + 22, "half-wealth adds half the 45-mill cap");
    let richest_lazy = market.reservation(i64::MAX / 2_000_000, 0, 0).expect("ask");
    assert!(
        (150 + 40..=150 + 45).contains(&richest_lazy),
        "the wealth raise saturates near +45, got {richest_lazy}"
    );
    let broke_industrious = market.reservation(0, 1000, 0).expect("ask");
    assert_eq!(
        broke_industrious,
        150 - 60,
        "full trait discounts 400‰ of base"
    );
    assert!(
        market.reservation(20_000, 1000, 0).expect("ask") < rich_lazy,
        "industriousness undercuts an equally wealthy twin"
    );
}

#[test]
fn bids_reserve_committed_payroll_before_funding_new_slots() {
    // A firm with an expensive incumbent and thin cash must bid low on
    // its open slot instead of hiring into a guaranteed next-morning
    // insolvency firing (ADR 0008 §3).
    let mut tables = tables();
    tables.firm_kinds[1].positions = 0; // only the farm bids
    let mut world = world_with_firms(&tables);
    let farm = firm_entities(&world)[0]; // positions 2
    let incumbent = staff(&mut world, farm);
    world
        .get_mut::<Employment>(incumbent)
        .expect("get")
        .expect("some")
        .wage_per_day = Money::from_mills(9_900);
    // Cash 10_000: 100 mills free after the incumbent's committed wage.
    let seeker = world.spawn();
    world
        .insert(seeker, Wallet { cash: Money::ZERO })
        .expect("insert");
    world
        .insert(
            seeker,
            core_ecs::sim_interface::Personality {
                weights: vec![1000],
            },
        )
        .expect("insert");
    world
        .insert(seeker, core_ecs::sim_interface::WorkingAge)
        .expect("insert");

    let mut market = crate::LaborMarketSystem::new(tables);
    let mut cmd = CommandBuffer::new();
    market.run(&mut world, &ctx(), &mut cmd).expect("run");

    // A refusal to hire (bid below the ask) is equally correct here.
    if let Some(employment) = world.get::<Employment>(seeker).expect("get") {
        assert!(
            employment.wage_per_day.mills() + 9_900 <= 10_000,
            "a hire may only happen if the whole payroll stays payable (wage {})",
            employment.wage_per_day
        );
    }
}

/// The public employer end to end (ADR 0009 §4): the treasury bids its
/// data wage at the clearing, its hires SURVIVE the redundancy check
/// (its slot count is `public_positions`, not a missing `Firm`'s zero),
/// and payroll pays them GROSS from the treasury wallet — its own
/// payroll is not taxed back into itself.
#[test]
fn the_treasury_employs_and_pays_public_workers() {
    let mut tables = tables();
    tables.money.public_positions = 2;
    tables.money.public_wage_bid_mills = 200;
    // Close the firms' slots so the treasury is the only bidder.
    tables.firm_kinds[0].positions = 0;
    tables.firm_kinds[1].positions = 0;
    let mut world = world_with_firms(&tables);

    // One cashless, maximally industrious seeker: ask = 150 − 60 = 90.
    let seeker = world.spawn();
    world
        .insert(seeker, Wallet { cash: Money::ZERO })
        .expect("insert");
    world
        .insert(
            seeker,
            core_ecs::sim_interface::Personality {
                weights: vec![1000],
            },
        )
        .expect("insert");
    world
        .insert(seeker, core_ecs::sim_interface::WorkingAge)
        .expect("insert");

    let mut market = crate::LaborMarketSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    market.run(&mut world, &ctx(), &mut cmd).expect("run");
    let employment = world
        .get::<Employment>(seeker)
        .expect("get")
        .expect("the treasury's bid clears against the cheap ask");
    let treasury = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .expect("query")
        .next()
        .map(|(entity, _)| entity)
        .expect("treasury");
    assert_eq!(employment.employer, treasury, "hired by the treasury");
    let wage = employment.wage_per_day;

    let mut payroll = crate::PayrollSystem::new(tables);
    payroll.run(&mut world, &ctx(), &mut cmd).expect("run");
    assert!(
        world.get::<Employment>(seeker).expect("get").is_some(),
        "a public worker within `public_positions` is NOT redundant"
    );
    assert_eq!(
        world
            .get::<Wallet>(seeker)
            .expect("get")
            .expect("some")
            .cash,
        wage,
        "paid gross — the treasury does not withhold from itself"
    );
    let receipts = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .expect("query")
        .next()
        .map(|(_, book)| *book)
        .expect("treasury");
    assert_eq!(receipts.income_tax_received, Money::ZERO);
    let books = world
        .get::<FirmBooks>(treasury)
        .expect("get")
        .expect("the treasury keeps payroll books");
    assert_eq!(books.expenses, wage, "the public wage is booked");
}
