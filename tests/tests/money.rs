//! Phase 6 exit-criteria suite (SPEC §15): the full monetary loop
//! closes; the conservation audit spans every ledger including the bank
//! and the treasury; a rate change measurably shifts credit and
//! construction. Data-shaped twins follow the flat-mortality pattern
//! (ADR 0009 §8): pinned data, one knob pegged, everything else shared.

use core_ecs::sim_interface::{BankBook, FirmBooks, Ownership, TreasuryBook};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};

const TICKS_PER_DAY: u64 = 1440;

fn bank_book(world: &core_ecs::World) -> BankBook {
    world
        .iter::<BankBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.clone())
        .expect("towns carry a bank")
}

/// Exit criterion 1 — the full monetary loop closes: over twelve days a
/// mill's worth of every station moves — wages (payroll expenses),
/// income tax and sales tax into the treasury, public wages OUT of the
/// treasury, retail revenue, mortgage credit, loan repayment (interest
/// collected), and deposit interest — while `Σ wallets == issued` holds
/// DAILY (the auditor runs inside the schedule; drift halts the run, so
/// finishing is itself the conservation proof).
#[test]
fn the_full_monetary_loop_closes() {
    let defs = pinned_defs();
    let spec = WorldSpec {
        citizens: 250,
        ..WorldSpec::fixture(Seed::new(41), 30)
    };
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");
    sim.run_ticks(&mut schedule, 12 * TICKS_PER_DAY)
        .expect("a halted run means a daily audit failed");

    let world = sim.world();
    let book = bank_book(world);
    assert!(!book.deposits.is_empty(), "citizens bank their savings");
    assert!(
        book.deposits
            .iter()
            .map(|(_, balance)| balance.mills())
            .sum::<i64>()
            > 0,
        "the vault holds real balances"
    );
    assert!(
        !book.loans.is_empty(),
        "credit is outstanding (the day-10 purchase clearing wrote mortgages)"
    );
    assert!(
        book.interest_received.mills() > 0,
        "borrowers serviced their loans (repayment station)"
    );
    assert!(
        book.deposit_interest_paid.mills() > 0,
        "savers earned deposit interest (deposit station)"
    );

    let (treasury, receipts) = world
        .iter::<TreasuryBook>()
        .expect("query")
        .next()
        .map(|(entity, book)| (entity, *book))
        .expect("towns carry a treasury");
    assert!(
        receipts.income_tax_received.mills() > 0,
        "payroll withheld income tax (wage station)"
    );
    assert!(
        receipts.sales_tax_received.mills() > 0,
        "the till split out sales tax (purchase station)"
    );
    let books = world
        .get::<FirmBooks>(treasury)
        .expect("query")
        .expect("the treasury keeps payroll books");
    assert!(
        books.expenses.mills() > 0,
        "the treasury paid public wages (public-employer station)"
    );
    assert!(
        books.revenue.mills() > 0,
        "taxes booked as treasury revenue (its ledger identity audits)"
    );

    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "every conservation identity — wallets, per-good, per-firm, \
         vault, treasury — holds at the end"
    );
}

/// Exit criterion 2 — the audit spans the bank's ledger: one mill of
/// seeded drift in a deposit row (a book entry, not a wallet — the old
/// Phase 4 identities cannot see it) halts a SCHEDULED run at the next
/// day boundary with the vault identity's violation.
#[test]
fn a_scheduled_run_halts_on_seeded_deposit_drift() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(89), 40), &defs).expect("build");
    sim.run_ticks(&mut schedule, 2 * TICKS_PER_DAY)
        .expect("healthy run");

    let bank = sim
        .world()
        .iter::<BankBook>()
        .expect("query")
        .next()
        .map(|(entity, _)| entity)
        .expect("bank");
    let book = sim
        .world_mut()
        .get_mut::<BankBook>(bank)
        .expect("query")
        .expect("book");
    let row = book
        .deposits
        .first_mut()
        .expect("two days of floats created deposit rows");
    row.1 = row
        .1
        .try_add(core_types::Money::from_mills(1))
        .expect("add");

    let err = sim
        .run_ticks(&mut schedule, TICKS_PER_DAY + 10)
        .expect_err("the next daily audit must halt the run");
    let message = err.to_string();
    assert!(message.contains("vault"), "{message}");
}

/// One rate-pegged twin (ADR 0009 §8): pinned data, the Taylor rule
/// clamped to a single rate, the purchase market pushed out of the
/// horizon (so mortgages don't blur the construction channel), and the
/// builder seeded lean enough that later batches need materials credit.
fn run_pegged_twin(rate_per_million_daily: i64) -> (usize, i64, i64) {
    let mut defs = pinned_defs();
    defs.bank.policy_neutral_per_million_daily = rate_per_million_daily;
    defs.bank.policy_min_per_million_daily = rate_per_million_daily;
    defs.bank.policy_max_per_million_daily = rate_per_million_daily;
    defs.housing.purchase_period_days = 100;
    for kind in &mut defs.firms.kinds {
        if kind.id == "builder" {
            kind.initial_cash_mills = 20_000;
        }
    }
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(97), 250), &defs).expect("build");
    let homes_start = sim.world().iter::<Ownership>().expect("query").count();
    sim.run_ticks(&mut schedule, 12 * TICKS_PER_DAY)
        .expect("run");
    let world = sim.world();
    let book = bank_book(world);
    assert_eq!(
        book.policy_rate_per_million_daily, rate_per_million_daily,
        "the peg holds: min == max clamps the Taylor rule"
    );
    assert!(debug_tools::audit_economy(world).expect("audit"));
    let homes_end = world.iter::<Ownership>().expect("query").count();
    let outstanding: i64 = book.loans.iter().map(|loan| loan.principal.mills()).sum();
    (
        homes_end - homes_start,
        outstanding,
        book.interest_received.mills(),
    )
}

/// Exit criterion 3 — a rate change measurably shifts credit and
/// construction: same seed, same data, only the peg differs. Cheap
/// money: the builder exhausts its till, takes materials credit (the
/// hurdle passes), and keeps building. Dear money: once the till is
/// short, the financing surcharge pushes the hurdle past the home price
/// — no credit, no further starts.
#[test]
fn a_rate_change_shifts_credit_and_construction() {
    let (built_low, outstanding_low, interest_low) = run_pegged_twin(100);
    let (built_high, outstanding_high, _) = run_pegged_twin(150_000);

    assert!(
        built_low > built_high,
        "cheap money out-builds dear money ({built_low} vs {built_high} starts)"
    );
    assert!(
        built_high > 0,
        "dear money slows construction, it does not abolish the builder \
         (the till-funded batches still happen: {built_high})"
    );
    assert!(
        outstanding_low > outstanding_high,
        "cheap money carries more outstanding credit ({outstanding_low} \
         vs {outstanding_high} mills)"
    );
    assert!(
        outstanding_low > 0 && interest_low > 0,
        "the low twin's credit is real and serviced"
    );
    assert_eq!(
        outstanding_high, 0,
        "at the dear peg the hurdle fails while the till is short — no \
         materials loan is ever written"
    );
}
