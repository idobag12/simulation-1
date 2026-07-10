//! Unit tests for the economy systems (split file for the SPEC §3
//! module-size rule). The pricing-controller direction tests here are
//! the glut-side half of the ADR 0007 §8 exit criterion; the integration
//! suite covers the shock (shortage) side end to end.

use core_ecs::sim_interface::{
    EconCounters, Employment, FirmBooks, GoodsPurchased, Inventory, Position, PriceChanged,
    RetailOffer, Wallet,
};
use core_ecs::{CommandBuffer, System, TickContext, World};
use core_types::{CalendarTime, Money, Seed, Ticks};

use crate::components::{Firm, Production};
use crate::config::{EconTables, EconomyConfig, FirmKindTable, LaborTables, RecipeTable};

fn ctx() -> TickContext {
    TickContext {
        tick: Ticks::ZERO,
        time: CalendarTime::START,
    }
}

/// Two goods (0 = grain, 1 = flour), two recipes (0: harvest 10 grain in
/// 2h; 1: 4 grain → 12 flour in 2h).
fn tables() -> EconTables {
    EconTables {
        goods: 2,
        spoil_per_mille: vec![0, 0],
        recipes: vec![
            RecipeTable {
                inputs: vec![],
                output_good: 0,
                output_quantity: 10,
                batch_hours: 2,
            },
            RecipeTable {
                inputs: vec![(0, 4)],
                output_good: 1,
                output_quantity: 12,
                batch_hours: 2,
            },
        ],
        firm_kinds: vec![
            FirmKindTable {
                count: 1,
                recipe: 0,
                initial_cash: Money::from_mills(10_000),
                initial_inventory: vec![0, 0],
                initial_price: Money::from_mills(40),
                location_kind: 0,
                positions: 2,
                min_workers: 1,
                retail: None,
            },
            FirmKindTable {
                count: 1,
                recipe: 1,
                initial_cash: Money::from_mills(10_000),
                initial_inventory: vec![8, 0],
                initial_price: Money::from_mills(90),
                location_kind: 1,
                positions: 2,
                min_workers: 1,
                retail: None,
            },
        ],
        economy: EconomyConfig {
            markup_per_mille: 300,
            overhead_mills_per_batch: 200,
            controller_step_per_mille: 50,
            inventory_target_batches: 3,
            min_price_mills: 1,
            max_price_mills: 5_000,
        },
        labor: LaborTables {
            shift_start_hour: 9,
            shift_end_hour: 17,
            min_working_age_years: 16,
            reservation_base_mills: 150,
            reservation_wealth_per_mille: 300,
            reservation_half_wealth_mills: 20_000,
            reservation_trait: 0,
            reservation_trait_discount_per_mille: 400,
            bid_fraction_per_mille: 600,
        },
    }
}

fn world_with_firms(tables: &EconTables) -> World {
    let mut world = World::new(Seed::new(5), 32);
    world.register::<Wallet>().expect("register");
    world.register::<Inventory>().expect("register");
    world.register::<RetailOffer>().expect("register");
    world.register::<Firm>().expect("register");
    world.register::<FirmBooks>().expect("register");
    world.register::<EconCounters>().expect("register");
    world.register::<Production>().expect("register");
    world
        .register::<core_ecs::sim_interface::Location>()
        .expect("register");
    world
        .register::<core_ecs::sim_interface::LaborStats>()
        .expect("register");
    world.register::<Employment>().expect("register");
    world.register::<Position>().expect("register");
    world
        .register::<core_ecs::sim_interface::Personality>()
        .expect("register");
    world
        .register::<core_ecs::sim_interface::WorkingAge>()
        .expect("register");
    world.register_event::<GoodsPurchased>().expect("register");
    world.register_event::<PriceChanged>().expect("register");
    world
        .register_event::<core_ecs::sim_interface::Hired>()
        .expect("register");
    world
        .register_event::<core_ecs::sim_interface::Fired>()
        .expect("register");
    crate::genesis::populate(&mut world, tables).expect("genesis");
    world
}

/// Staffs `firm` with one present worker (a bare employed entity standing
/// at the firm) so labor-gated production can start (ADR 0008 §1).
fn staff(world: &mut World, firm: core_ecs::Entity) -> core_ecs::Entity {
    let worker = world.spawn();
    world
        .insert(worker, Wallet { cash: Money::ZERO })
        .expect("insert");
    world
        .insert(
            worker,
            Employment {
                employer: firm,
                wage_per_day: Money::from_mills(100),
            },
        )
        .expect("insert");
    world.insert(worker, Position { at: firm }).expect("insert");
    worker
}

fn firm_entities(world: &World) -> Vec<core_ecs::Entity> {
    world
        .iter::<Firm>()
        .expect("query")
        .map(|(entity, _)| entity)
        .collect()
}

fn counters(world: &World) -> EconCounters {
    world
        .iter::<EconCounters>()
        .expect("query")
        .next()
        .map(|(_, c)| c.clone())
        .expect("ledger exists")
}

#[test]
fn genesis_records_issuance_and_seeded_stock() {
    let world = world_with_firms(&tables());
    let c = counters(&world);
    assert_eq!(c.issued, Money::from_mills(20_000));
    assert_eq!(c.produced, vec![8, 0], "seeded stock counts as produced");
    assert_eq!(c.spoiled, vec![0, 0]);
}

#[test]
fn production_consumes_inputs_when_starting_and_lands_outputs_counted() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let mill = firms[1];
    staff(&mut world, firms[0]);
    staff(&mut world, firms[1]);
    let mut system = crate::ProductionSystem::new(tables);
    let mut cmd = CommandBuffer::new();

    // Hour 1: harvest starts (no inputs); mill starts, consuming 4 grain.
    system.run(&mut world, &ctx(), &mut cmd).expect("run");
    let inventory = world.get::<Inventory>(mill).expect("get").expect("some");
    assert_eq!(inventory.quantities, vec![4, 0]);
    assert_eq!(counters(&world).consumed_in_production, vec![4, 0]);
    assert!(world.get::<Production>(mill).expect("get").is_some());

    // Hour 2: batches tick down. Hour 3: both land, produced counts.
    system.run(&mut world, &ctx(), &mut cmd).expect("run");
    system.run(&mut world, &ctx(), &mut cmd).expect("run");
    let inventory = world.get::<Inventory>(mill).expect("get").expect("some");
    assert_eq!(inventory.quantities[1], 12, "flour landed");
    let c = counters(&world);
    assert_eq!(
        c.produced,
        vec![8 + 10, 12],
        "seeded 8 + harvest 10; flour 12"
    );
    // The finished firm restarts NEXT hour (finish-or-start, not both):
    // the mill consumed 4 of its remaining 4 grain on hour 4.
    system.run(&mut world, &ctx(), &mut cmd).expect("run");
    assert!(world.get::<Production>(mill).expect("get").is_some());
    assert_eq!(counters(&world).consumed_in_production, vec![8, 0]);
}

#[test]
fn trade_transfers_goods_money_and_books_atomically() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let (farm, mill) = (firms[0], firms[1]);
    staff(&mut world, farm);
    staff(&mut world, mill);
    // Give the farm sellable grain (through the modeled path: a batch).
    let mut production = crate::ProductionSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    for _ in 0..7 {
        production.run(&mut world, &ctx(), &mut cmd).expect("run");
    }
    let farm_stock = world
        .get::<Inventory>(farm)
        .expect("get")
        .expect("some")
        .stock(0);
    assert!(farm_stock >= 10, "farm has harvested grain to sell");

    let mut trade = crate::TradeSystem::new(tables);
    trade.run(&mut world, &ctx(), &mut cmd).expect("run");

    // Mill wanted up to 4 × 3 = 12 grain; it buys the deficit at the
    // farm's posted 40 mills.
    let mill_inventory = world.get::<Inventory>(mill).expect("get").expect("some");
    assert!(mill_inventory.stock(0) >= 1, "the mill restocked");
    // Exact bookkeeping identities on both sides.
    let farm_books = world.get::<FirmBooks>(farm).expect("get").expect("some");
    let mill_books = world.get::<FirmBooks>(mill).expect("get").expect("some");
    assert_eq!(farm_books.revenue.mills(), mill_books.expenses.mills());
    assert!(farm_books.revenue.mills() > 0);
    let farm_wallet = world.get::<Wallet>(farm).expect("get").expect("some");
    let mill_wallet = world.get::<Wallet>(mill).expect("get").expect("some");
    assert_eq!(
        farm_wallet.cash.mills() + mill_wallet.cash.mills(),
        20_000,
        "trade moves money, never makes it"
    );
    assert_eq!(
        farm_wallet.cash.mills() - 10_000,
        farm_books.revenue.mills() - farm_books.expenses.mills(),
        "ledger identity holds after the trade"
    );
}

#[test]
fn pricing_cuts_in_glut_raises_in_shortage_and_respects_the_floor() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let (farm, mill) = (firms[0], firms[1]);
    // Farm: huge stock (glut). Mill: no flour stock (shortage).
    world
        .insert(
            farm,
            Inventory {
                quantities: vec![500, 0],
            },
        )
        .expect("insert");

    let mut pricing = crate::PricingSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    pricing.run(&mut world, &ctx(), &mut cmd).expect("run");

    // Glut: 40 → down by max(1, 40×50/1000) = 2, floored at cost-plus
    // ceil(200×1.3/10) = 26.
    let farm_price = world
        .get::<Firm>(farm)
        .expect("get")
        .expect("some")
        .posted_price
        .mills();
    assert_eq!(farm_price, 38, "glut cuts by the bounded step");

    // Shortage: 90 → up by max(1, 90×50/1000) = 4.
    let mill_price = world
        .get::<Firm>(mill)
        .expect("get")
        .expect("some")
        .posted_price
        .mills();
    assert_eq!(mill_price, 94, "shortage raises by the bounded step");

    // Price-change facts were emitted for the next tick's readers.
    world.begin_tick(Ticks::new(1));
    let changes = world.events::<PriceChanged>().expect("events");
    assert_eq!(changes.len(), 2);

    // Iterated gluts never cut below the cost-plus floor.
    for _ in 0..60 {
        pricing.run(&mut world, &ctx(), &mut cmd).expect("run");
    }
    let farm_price = world
        .get::<Firm>(farm)
        .expect("get")
        .expect("some")
        .posted_price
        .mills();
    assert_eq!(farm_price, 26, "the cost-plus floor holds under a glut");
}

#[test]
fn trade_is_bounded_by_buyer_cash_and_starved_firms_idle() {
    // A cash-poor mill (90 mills) with an empty inventory: production
    // must idle (input starvation — no batch, nothing consumed), and the
    // daily trade must buy only what the wallet affords.
    let mut tables = tables();
    tables.firm_kinds[1].initial_cash = Money::from_mills(90);
    tables.firm_kinds[1].initial_inventory = vec![0, 0];
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let (farm, mill) = (firms[0], firms[1]);
    staff(&mut world, farm);
    staff(&mut world, mill);

    let mut production = crate::ProductionSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    for _ in 0..7 {
        production.run(&mut world, &ctx(), &mut cmd).expect("run");
    }
    // Starvation: the mill never started a batch and consumed nothing.
    assert!(
        world.get::<Production>(mill).expect("get").is_none(),
        "an input-starved firm must idle, not fake a batch"
    );
    assert_eq!(counters(&world).consumed_in_production, vec![0, 0]);

    let mut trade = crate::TradeSystem::new(tables);
    trade.run(&mut world, &ctx(), &mut cmd).expect("run");

    // Deficit is 4 × 3 = 12 grain and the farm has plenty, but at the
    // posted 40 mills only 90 / 40 = 2 units are affordable.
    assert_eq!(
        world
            .get::<Inventory>(mill)
            .expect("get")
            .expect("some")
            .stock(0),
        2,
        "the purchase is bounded by the buyer's cash"
    );
    let mill_wallet = world.get::<Wallet>(mill).expect("get").expect("some");
    assert_eq!(mill_wallet.cash, Money::from_mills(10));
    assert!(mill_wallet.cash >= Money::ZERO, "wallets never go negative");
    let farm_books = world.get::<FirmBooks>(farm).expect("get").expect("some");
    assert_eq!(farm_books.revenue, Money::from_mills(80));
}

#[test]
fn unstaffed_firms_never_start_batches() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let firms = firm_entities(&world);
    let mut system = crate::ProductionSystem::new(tables);
    let mut cmd = CommandBuffer::new();
    for _ in 0..5 {
        system.run(&mut world, &ctx(), &mut cmd).expect("run");
    }
    for firm in firms {
        assert!(
            world.get::<Production>(firm).expect("get").is_none(),
            "no workers present — no batch (ADR 0008 §1)"
        );
    }
    assert_eq!(counters(&world).produced, vec![8, 0], "seed only");
}

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
        Money::from_mills(100),
        "the worker got paid (workers spawn cashless here)"
    );
    let books = world.get::<FirmBooks>(farm).expect("get").expect("some");
    assert_eq!(books.expenses, Money::from_mills(100), "payroll is booked");

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
