//! Unit tests for the bank and housing systems (ADR 0009 §§2–3): the
//! float sweep, deposit interest, the payment-priced serviceability
//! screen, default recovery + repossession + cooldown, rent, eviction,
//! the rental clearing, and vault liquidity under stress.

use core_ecs::sim_interface::{
    BankBook, BorrowerStatus, FirmBooks, Loan, Location, Needs, Ownership, Residence, Tenancy,
    TenancyStarted, Wallet,
};
use core_ecs::{CommandBuffer, Entity, System, TickContext, World};
use core_types::{CalendarTime, Money, Ticks};

use crate::systems::tests::{firm_entities, staff, tables, world_with_firms};

fn day_ctx(day: u64) -> TickContext {
    TickContext {
        tick: Ticks::new(day * 1440),
        time: CalendarTime::START,
    }
}

/// A citizen for money tests: `Needs` (the economy's citizen signal) +
/// a wallet.
fn citizen(world: &mut World, cash: i64) -> Entity {
    let entity = world.spawn();
    world
        .insert(entity, Needs { levels: Vec::new() })
        .expect("insert");
    world
        .insert(
            entity,
            Wallet {
                cash: Money::from_mills(cash),
            },
        )
        .expect("insert");
    entity
}

fn bank_of(world: &World) -> Entity {
    crate::bank::bank_entity(world)
        .expect("query")
        .expect("bank")
}

fn book_of(world: &World) -> BankBook {
    world
        .iter::<BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.clone())
        .expect("bank")
}

/// The vault identity, asserted inline (debug_tools audits it in
/// integration; unit worlds check it directly).
fn assert_vault_balances(world: &World) {
    let (bank, book) = world
        .iter::<BankBook>()
        .expect("query")
        .next()
        .map(|(entity, book)| (entity, book.clone()))
        .expect("bank");
    let deposits: i64 = book.deposits.iter().map(|(_, b)| b.mills()).sum();
    let outstanding: i64 = book.loans.iter().map(|l| l.principal.mills()).sum();
    let vault = world
        .get::<Wallet>(bank)
        .expect("query")
        .map(|wallet| wallet.cash.mills())
        .unwrap_or(0);
    assert_eq!(
        vault,
        deposits + book.equity.mills() - outstanding,
        "the vault identity must hold"
    );
    assert!(vault >= 0, "the bank wallet must never go negative");
}

/// ADR 0009 §2: the sweep banks everything above the float, refills up
/// to it, and interest accrues from equity into rows.
#[test]
fn deposits_sweep_to_the_float_and_earn_interest_from_equity() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let saver = citizen(&mut world, 10_000);
    let mut bank_system = crate::BankSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    bank_system
        .run(&mut world, &day_ctx(1), &mut cmd)
        .expect("run");

    let bank = bank_of(&world);
    let wallet = world
        .get::<Wallet>(saver)
        .expect("query")
        .expect("wallet")
        .cash;
    assert_eq!(
        wallet,
        Money::from_mills(2_000),
        "the wallet holds exactly the data float"
    );
    // The sweep banks 8_000, and the same run's interest pass pays
    // (policy 800 − spread 400)/million on it = 3 mills from equity.
    let row = crate::vault::deposit_balance(&world, bank, saver).expect("row");
    assert_eq!(
        row,
        Money::from_mills(8_003),
        "excess banked + day-one interest"
    );
    let equity_before = book_of(&world).equity;

    bank_system
        .run(&mut world, &day_ctx(2), &mut cmd)
        .expect("run");
    let book = book_of(&world);
    let row = crate::vault::deposit_balance(&world, bank, saver).expect("row");
    assert_eq!(
        row,
        Money::from_mills(8_006),
        "interest compounds in the row"
    );
    assert_eq!(
        book.equity,
        equity_before.try_sub(Money::from_mills(3)).expect("sub"),
        "interest is a transfer FROM equity, not minted"
    );
    assert_eq!(book.deposit_interest_paid, Money::from_mills(6));
    assert_vault_balances(&world);
}

/// ADR 0009 §2: the serviceability screen prices the PAYMENT — the same
/// firm that qualifies at a low rate is refused at a dear one, and net
/// revenue excludes past principal grants (no self-ratchet).
#[test]
fn working_capital_is_screened_on_the_payment_and_net_revenue() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let farm = firm_entities(&world)[0];
    staff(&mut world, farm);
    // Give the farm a real sales history, then drain its till below the
    // 3-day floor (100/day wage × 3 = 300).
    if let Some(books) = world.get_mut::<FirmBooks>(farm).expect("query") {
        books.revenue = Money::from_mills(50_000);
    }
    let drained = world
        .get::<Wallet>(farm)
        .expect("query")
        .expect("wallet")
        .cash;
    if let Some(wallet) = world.get_mut::<Wallet>(farm).expect("query") {
        wallet.cash = Money::from_mills(100);
    }
    // Park the drained cash on the bank so the world stays conserved.
    let bank = bank_of(&world);
    if let Some(wallet) = world.get_mut::<Wallet>(bank).expect("query") {
        wallet.cash = wallet
            .cash
            .try_add(drained.try_sub(Money::from_mills(100)).expect("sub"))
            .expect("add");
    }
    if let Some(book) = world.get_mut::<BankBook>(bank).expect("query") {
        book.equity = book
            .equity
            .try_add(drained.try_sub(Money::from_mills(100)).expect("sub"))
            .expect("add");
    }

    // Dear money first: the pegged rate makes the obligation exceed
    // what 20% of revenue covers — refused.
    if let Some(book) = world.get_mut::<BankBook>(bank).expect("query") {
        book.policy_rate_per_million_daily = 900_000;
    }
    let mut bank_system = crate::BankSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    bank_system
        .run(&mut world, &day_ctx(1), &mut cmd)
        .expect("run");
    assert!(
        book_of(&world).loans.is_empty(),
        "dear money refuses the same borrower"
    );

    // Cheap money: granted (obligation ≈ principal; 20% of 50_000
    // covers the 500-mill loan's ≈ 503 obligation... use the numbers:
    // principal = 5 × 100 = 500; obligation at rate 400+400 ≈ 524;
    // screen: 50_000 × 200 / 1000 = 10_000 ≥ 524).
    if let Some(book) = world.get_mut::<BankBook>(bank).expect("query") {
        book.policy_rate_per_million_daily = 400;
    }
    bank_system
        .run(&mut world, &day_ctx(2), &mut cmd)
        .expect("run");
    let book = book_of(&world);
    assert_eq!(book.loans.len(), 1, "cheap money grants");
    assert_eq!(book.loans[0].borrower, farm);
    assert_eq!(book.loans[0].principal, Money::from_mills(500));
    assert_eq!(
        crate::vault::granted_principal(&world, bank, farm).expect("granted"),
        Money::from_mills(500),
        "the grant is remembered so revenue nets it out next time"
    );
    assert_vault_balances(&world);
}

/// ADR 0009 §§2–3: a hopeless borrower defaults — the bank seizes the
/// deposit row and remaining cash (partial recovery), repossesses the
/// collateral home (evicting the owner-occupier), writes off only the
/// residual, and stamps the cooldown that blocks new credit.
#[test]
fn defaults_recover_repossess_and_cool_down() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let bank = bank_of(&world);
    let borrower = citizen(&mut world, 100);
    // A home the borrower owns and lives in.
    let home = world.spawn();
    world.insert(home, Location { kind: 0 }).expect("insert");
    world
        .insert(home, Ownership { owner: borrower })
        .expect("insert");
    world.insert(borrower, Residence { home }).expect("insert");
    // A deposit row of 200 and a 1_000-mill mortgage the borrower
    // cannot possibly service (due exceeds wallet + row).
    crate::vault::vault_deposit(&mut world, bank, borrower, Money::from_mills(50)).expect("dep");
    if let Some(wallet) = world.get_mut::<Wallet>(bank).expect("query") {
        wallet.cash = wallet.cash.try_sub(Money::from_mills(1_000)).expect("sub");
    }
    if let Some(wallet) = world.get_mut::<Wallet>(borrower).expect("query") {
        wallet.cash = wallet.cash.try_add(Money::from_mills(1_000)).expect("add");
    }
    // (the borrower then "spent" the principal — burn it into the farm)
    let farm = firm_entities(&world)[0];
    if let Some(wallet) = world.get_mut::<Wallet>(borrower).expect("query") {
        wallet.cash = Money::from_mills(30);
    }
    if let Some(wallet) = world.get_mut::<Wallet>(farm).expect("query") {
        wallet.cash = wallet.cash.try_add(Money::from_mills(1_020)).expect("add");
    }
    if let Some(books) = world.get_mut::<FirmBooks>(farm).expect("query") {
        books.revenue = books
            .revenue
            .try_add(Money::from_mills(1_020))
            .expect("add");
    }
    if let Some(book) = world.get_mut::<BankBook>(bank).expect("query") {
        book.loans.push(Loan {
            borrower,
            principal: Money::from_mills(1_000),
            rate_per_million_daily: 800,
            day_payment: Money::from_mills(500),
            collateral: Some(home),
        });
    }
    let equity_before = book_of(&world).equity;

    let mut bank_system = crate::BankSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    bank_system
        .run(&mut world, &day_ctx(3), &mut cmd)
        .expect("run");

    let book = book_of(&world);
    assert!(book.loans.is_empty(), "the defaulted loan is gone");
    // Recovery: 50 from the row (netting) + 30 cash seized; residual
    // 920 written off.
    assert_eq!(
        crate::vault::deposit_balance(&world, bank, borrower).expect("row"),
        Money::ZERO,
        "the deposit row was seized"
    );
    assert_eq!(
        world
            .get::<Wallet>(borrower)
            .expect("query")
            .expect("wallet")
            .cash,
        Money::ZERO,
        "remaining cash was seized"
    );
    assert_eq!(
        book.equity,
        equity_before.try_sub(Money::from_mills(920)).expect("sub"),
        "only the unrecovered residual hits equity"
    );
    assert_eq!(
        world
            .get::<Ownership>(home)
            .expect("query")
            .expect("owned")
            .owner,
        bank,
        "the collateral home was repossessed"
    );
    assert!(
        world.get::<Residence>(borrower).expect("query").is_none(),
        "the defaulting owner-occupier is out — measured homelessness"
    );
    let status = world
        .get::<BorrowerStatus>(borrower)
        .expect("query")
        .expect("cooldown stamped");
    assert_eq!(
        status.uncreditworthy_until_day,
        3 + tables.money.bank.default_cooldown_days
    );
    assert_vault_balances(&world);
}

/// C1 (review): a stretched vault SHORT-FILLS float refills instead of
/// going negative — the run survives a liquidity crunch honestly.
#[test]
fn float_refills_short_fill_when_the_vault_is_stretched() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let bank = bank_of(&world);
    let saver = citizen(&mut world, 10_000);
    // Bank the excess (row 8_000), then lend the vault down to almost
    // nothing (a legal grant: the bank lends deposits, ADR 0009 §1).
    let mut bank_system = crate::BankSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    bank_system
        .run(&mut world, &day_ctx(1), &mut cmd)
        .expect("run");
    let farm = firm_entities(&world)[0];
    let vault_cash = world
        .get::<Wallet>(bank)
        .expect("query")
        .expect("wallet")
        .cash;
    let loan = vault_cash.try_sub(Money::from_mills(500)).expect("sub");
    crate::vault::grant_loan(&mut world, bank, farm, loan, 800, 60, None).expect("grant");

    // The borrower goes broke (its whole till moves to the treasury —
    // conserved and booked), the saver's wallet empties too, and the
    // refill wants 2_000 while the vault holds only 500: the loan will
    // default with nothing to recover, and the refill must short-fill.
    let treasury = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .expect("query")
        .next()
        .map(|(entity, _)| entity)
        .expect("treasury");
    let farm_cash = world
        .get::<Wallet>(farm)
        .expect("query")
        .expect("wallet")
        .cash;
    if let Some(wallet) = world.get_mut::<Wallet>(farm).expect("query") {
        wallet.cash = Money::ZERO;
    }
    if let Some(books) = world.get_mut::<FirmBooks>(farm).expect("query") {
        books.expenses = books.expenses.try_add(farm_cash).expect("add");
    }
    let saver_cash = world
        .get::<Wallet>(saver)
        .expect("query")
        .expect("wallet")
        .cash;
    if let Some(wallet) = world.get_mut::<Wallet>(saver).expect("query") {
        wallet.cash = Money::ZERO;
    }
    if let Some(wallet) = world.get_mut::<Wallet>(treasury).expect("query") {
        wallet.cash = wallet
            .cash
            .try_add(farm_cash)
            .expect("add")
            .try_add(saver_cash)
            .expect("add");
    }
    if let Some(books) = world.get_mut::<FirmBooks>(treasury).expect("query") {
        books.revenue = books
            .revenue
            .try_add(farm_cash)
            .expect("add")
            .try_add(saver_cash)
            .expect("add");
    }
    bank_system
        .run(&mut world, &day_ctx(2), &mut cmd)
        .expect("a liquidity crunch must not halt the run");
    let refilled = world
        .get::<Wallet>(saver)
        .expect("query")
        .expect("wallet")
        .cash;
    assert_eq!(
        refilled,
        Money::from_mills(500),
        "the refill short-fills to exactly what the vault holds"
    );
    assert_vault_balances(&world);
}

/// ADR 0009 §3: rent flows tenant → live owner daily; a broke tenant is
/// evicted (tenancy AND residence end — measured homelessness).
#[test]
fn rent_flows_daily_and_broke_tenants_are_evicted() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let landlord = citizen(&mut world, 0);
    let tenant = citizen(&mut world, 500);
    let home = world.spawn();
    world.insert(home, Location { kind: 0 }).expect("insert");
    world
        .insert(home, Ownership { owner: landlord })
        .expect("insert");
    world
        .insert(
            tenant,
            Tenancy {
                home,
                rent_per_day: Money::from_mills(60),
            },
        )
        .expect("insert");
    world.insert(tenant, Residence { home }).expect("insert");

    let mut rent = crate::RentSystem;
    let mut cmd = CommandBuffer::new();
    rent.run(&mut world, &day_ctx(1), &mut cmd).expect("run");
    assert_eq!(
        world
            .get::<Wallet>(landlord)
            .expect("query")
            .expect("wallet")
            .cash,
        Money::from_mills(60),
        "rent reached the owner"
    );

    // Broke: the next collection evicts instead of paying.
    if let Some(wallet) = world.get_mut::<Wallet>(tenant).expect("query") {
        wallet.cash = Money::from_mills(10);
    }
    if let Some(wallet) = world.get_mut::<Wallet>(landlord).expect("query") {
        wallet.cash = wallet.cash.try_add(Money::from_mills(430)).expect("add");
    }
    rent.run(&mut world, &day_ctx(2), &mut cmd).expect("run");
    assert!(
        world.get::<Tenancy>(tenant).expect("query").is_none(),
        "the tenancy ended"
    );
    assert!(
        world.get::<Residence>(tenant).expect("query").is_none(),
        "the residence ended — tomorrow's clearing sees them homeless"
    );
}

/// ADR 0009 §3: the rental clearing matches a homeless citizen's
/// savings-based bid to a vacant owned home at the midpoint and emits
/// the fact.
#[test]
fn the_rental_clearing_creates_tenancies_at_the_midpoint() {
    let tables = tables();
    let mut world = world_with_firms(&tables);
    let bank = bank_of(&world);
    let landlord = citizen(&mut world, 2_000);
    let seeker = citizen(&mut world, 10_000);
    // The seeker's wealth sits mostly in the vault (the float sweep's
    // world): the bid MUST read savings, not pocket cash.
    crate::vault::vault_deposit(&mut world, bank, seeker, Money::from_mills(8_000)).expect("dep");
    let home = world.spawn();
    world.insert(home, Location { kind: 0 }).expect("insert");
    world
        .insert(home, Ownership { owner: landlord })
        .expect("insert");
    let landlord_home = world.spawn();
    world
        .insert(
            landlord,
            Residence {
                home: landlord_home,
            },
        )
        .expect("insert");

    let mut market = crate::RentalMarketSystem::new(tables.clone());
    let mut cmd = CommandBuffer::new();
    market.run(&mut world, &day_ctx(1), &mut cmd).expect("run");

    let tenancy = world
        .get::<Tenancy>(seeker)
        .expect("query")
        .expect("the clearing matched");
    assert_eq!(tenancy.home, home);
    // Bid = savings 10_000 × 8/1000 = 80 (pocket cash alone — 2_000 ×
    // 8/1000 = 16 — would never clear the 50-mill floor: the bid reads
    // SAVINGS); ask = the cost-plus floor 50; midpoint 65.
    assert_eq!(tenancy.rent_per_day, Money::from_mills(65));
    world.begin_tick(Ticks::new(1));
    let started = world.events::<TenancyStarted>().expect("events");
    assert_eq!(started.len(), 1);
}
