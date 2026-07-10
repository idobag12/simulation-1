//! Deterministic economy genesis (ADR 0007 §2): the conservation ledger,
//! then every firm in kind order × instance order (deterministic entity
//! indices). No RNG — firm seeding is exact data.
//!
//! Issuance: after every wallet in the world is seeded (citizens by
//! `sim_people::genesis`, firms here), the TOTAL is recorded as
//! `EconCounters::issued` — the one explicitly modeled money source.
//! Seeded stock is likewise recorded in `produced`, so both audit
//! identities hold from tick 0.

use core_ecs::sim_interface::{
    BankBook, EconCounters, FirmBooks, HousingBook, Inventory, LaborStats, Location, RetailOffer,
    TreasuryBook, Wallet,
};
use core_ecs::{EcsError, World};
use core_types::Money;

use crate::components::Firm;
use crate::config::EconTables;

/// Spawns the ledger entity and every firm, seeds their cash and stock,
/// and records issuance from ALL wallets in the world (call after citizen
/// genesis; call exactly once per world).
pub fn populate(world: &mut World, tables: &EconTables) -> Result<(), EcsError> {
    let ledger = world.spawn();
    world.insert(ledger, EconCounters::new(tables.goods))?;
    // The escheat wallet (ADR 0007 §8a): unclaimed estates accumulate
    // here, inside the audited wallet sum, until Phase 6's treasury.
    world.insert(ledger, Wallet { cash: Money::ZERO })?;
    // The labor ledger (ADR 0008 §5), written by each daily clearing.
    world.insert(ledger, LaborStats::default())?;
    // The housing ledger (ADR 0009 §§3, 5): controller state and the
    // measured market price of homes.
    world.insert(ledger, HousingBook::default())?;

    // The bank (ADR 0009 §§1–2): one vault, seeded with equity — counted
    // in issuance below like every other genesis wallet.
    let bank = world.spawn();
    world.insert(
        bank,
        Wallet {
            cash: Money::from_mills(tables.money.bank.equity_seed_mills),
        },
    )?;
    world.insert(
        bank,
        BankBook {
            equity: Money::from_mills(tables.money.bank.equity_seed_mills),
            policy_rate_per_million_daily: tables.money.bank.policy_neutral_per_million_daily,
            last_price_index_milli: 0,
            deposits: Vec::new(),
            loans: Vec::new(),
            interest_received: Money::ZERO,
            deposit_interest_paid: Money::ZERO,
            granted: Vec::new(),
        },
    )?;

    // The treasury (ADR 0009 §4): the public employer's wallet, books
    // (payroll like any employer's; taxes book as revenue), receipt
    // counters, and its town-hall workplace.
    let treasury = world.spawn();
    world.insert(
        treasury,
        Wallet {
            cash: tables.money.treasury_seed,
        },
    )?;
    world.insert(
        treasury,
        FirmBooks {
            initial_cash: tables.money.treasury_seed,
            revenue: Money::ZERO,
            expenses: Money::ZERO,
        },
    )?;
    world.insert(treasury, TreasuryBook::default())?;
    world.insert(
        treasury,
        Location {
            kind: tables.money.public_location_kind,
        },
    )?;

    let mut seeded_goods: Vec<i64> = vec![0; tables.goods];
    for (kind_index, kind) in tables.firm_kinds.iter().enumerate() {
        let recipe =
            tables
                .recipes
                .get(kind.recipe as usize)
                .ok_or(EcsError::InternalCorruption(
                    "firm kind references a recipe outside the loaded data",
                ))?;
        for _ in 0..kind.count {
            let firm = world.spawn();
            world.insert(
                firm,
                Firm {
                    kind: kind_index as u32,
                    recipe: kind.recipe,
                    posted_price: kind.initial_price,
                },
            )?;
            world.insert(
                firm,
                Wallet {
                    cash: kind.initial_cash,
                },
            )?;
            world.insert(
                firm,
                Inventory {
                    quantities: kind.initial_inventory.clone(),
                },
            )?;
            world.insert(
                firm,
                FirmBooks {
                    initial_cash: kind.initial_cash,
                    revenue: Money::ZERO,
                    expenses: Money::ZERO,
                },
            )?;
            // Every firm is a place (ADR 0008 §1): workers commute here.
            world.insert(
                firm,
                Location {
                    kind: kind.location_kind,
                },
            )?;
            if let Some((need_index, gain_per_unit, use_ticks)) = kind.retail {
                let (output_good, _) = recipe.output.ok_or(EcsError::InternalCorruption(
                    "a retail firm kind must have a goods output (validated)",
                ))?;
                world.insert(
                    firm,
                    RetailOffer {
                        good: output_good,
                        need_index,
                        gain_per_unit,
                        use_ticks,
                        unit_price: kind.initial_price,
                    },
                )?;
            }
            for (good, seeded) in seeded_goods.iter_mut().enumerate() {
                *seeded += kind.initial_inventory.get(good).copied().unwrap_or(0);
            }
        }
    }

    // Record the sources: every wallet seeded anywhere in this world, and
    // every unit of seeded stock.
    let mut issued = Money::ZERO;
    for (_, wallet) in world.iter::<Wallet>()? {
        issued = issued.try_add(wallet.cash)?;
    }
    if let Some(counters) = world.get_mut::<EconCounters>(ledger)? {
        counters.issued = issued;
        for (produced, seeded) in counters.produced.iter_mut().zip(&seeded_goods) {
            *produced += seeded;
        }
    }
    Ok(())
}
