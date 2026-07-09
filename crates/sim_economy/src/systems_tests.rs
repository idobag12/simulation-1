//! Unit tests for the economy systems (split file for the SPEC §3
//! module-size rule). The pricing-controller direction tests here are
//! the glut-side half of the ADR 0007 §8 exit criterion; the integration
//! suite covers the shock (shortage) side end to end.

use core_ecs::sim_interface::{
    EconCounters, FirmBooks, GoodsPurchased, Inventory, PriceChanged, RetailOffer, Wallet,
};
use core_ecs::{CommandBuffer, System, TickContext, World};
use core_types::{CalendarTime, Money, Seed, Ticks};

use crate::components::{Firm, Production};
use crate::config::{EconTables, EconomyConfig, FirmKindTable, RecipeTable};

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
                retail: None,
            },
            FirmKindTable {
                count: 1,
                recipe: 1,
                initial_cash: Money::from_mills(10_000),
                initial_inventory: vec![8, 0],
                initial_price: Money::from_mills(90),
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
    world.register_event::<GoodsPurchased>().expect("register");
    world.register_event::<PriceChanged>().expect("register");
    crate::genesis::populate(&mut world, tables).expect("genesis");
    world
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
