//! Housing, rent side (Phase 6, ADR 0009 §3): daily rent collection and
//! the rental clearing. The purchase clearing lives in
//! `housing_market.rs` (SPEC §3 module-size rule). Both reuse the labor
//! market's double-auction shape (SPEC §12: "the same matching
//! machinery").

use std::cmp::Reverse;
use std::collections::BTreeSet;

use core_ecs::sim_interface::{
    FirmBooks, HousingBook, Location, Needs, Ownership, Residence, Tenancy, TenancyStarted, Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::Money;

use crate::bank::bank_entity;
use crate::config::EconTables;
use crate::transact::{add, mul};
use crate::vault::deposit_balance;

/// Occupied homes: everyone's `Residence` target plus every tenancy.
pub(crate) fn occupied_homes(world: &World) -> Result<BTreeSet<u32>, EcsError> {
    let mut occupied = BTreeSet::new();
    for (_, residence) in world.iter::<Residence>()? {
        occupied.insert(residence.home.index());
    }
    for (_, tenancy) in world.iter::<Tenancy>()? {
        occupied.insert(tenancy.home.index());
    }
    Ok(occupied)
}

/// The ledger's housing book, if this world has one (migrated pre-v7
/// worlds honestly do not).
pub(crate) fn housing_book_entity(world: &World) -> Result<Option<Entity>, EcsError> {
    Ok(world
        .iter::<HousingBook>()?
        .next()
        .map(|(entity, _)| entity))
}

/// A citizen's savings: pocket cash plus their vault row (ADR 0009 §2:
/// wealth signals read savings, not pocket cash).
pub(crate) fn savings_mills(
    world: &World,
    bank: Option<Entity>,
    citizen: Entity,
) -> Result<i64, EcsError> {
    let cash = world
        .get::<Wallet>(citizen)?
        .map(|wallet| wallet.cash.mills())
        .unwrap_or(0);
    let vault = match bank {
        Some(bank) => deposit_balance(world, bank, citizen)?.mills(),
        None => 0,
    };
    add(cash, vault, "savings")
}

/// Day-rate system: every tenancy pays its rent to the home's CURRENT
/// owner (live lookup, so inheritance keeps working); a tenant who
/// cannot pay is evicted — tenancy and residence end, and tomorrow's
/// clearing sees them homeless (measured, not hidden).
pub struct RentSystem;

impl System for RentSystem {
    fn name(&self) -> &'static str {
        "econ.rent"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let tenancies: Vec<(Entity, Tenancy)> = world
            .iter::<Tenancy>()?
            .map(|(tenant, tenancy)| (tenant, *tenancy))
            .collect();
        for (tenant, tenancy) in tenancies {
            let owner = world
                .get::<Ownership>(tenancy.home)?
                .map(|ownership| ownership.owner);
            let cash = world
                .get::<Wallet>(tenant)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            let Some(owner) = owner.filter(|owner| world.is_alive(*owner)) else {
                // Ownerless home (should not happen; defensive): the
                // arrangement ends entirely — tenancy AND residence —
                // so the ex-tenant re-enters the rental market instead
                // of squatting rent-free forever.
                world.remove::<Tenancy>(tenant)?;
                if world
                    .get::<Residence>(tenant)?
                    .is_some_and(|residence| residence.home == tenancy.home)
                {
                    world.remove::<Residence>(tenant)?;
                }
                continue;
            };
            if cash >= tenancy.rent_per_day {
                if let Some(wallet) = world.get_mut::<Wallet>(tenant)? {
                    wallet.cash = wallet.cash.try_sub(tenancy.rent_per_day)?;
                }
                if let Some(wallet) = world.get_mut::<Wallet>(owner)? {
                    wallet.cash = wallet.cash.try_add(tenancy.rent_per_day)?;
                }
                if let Some(books) = world.get_mut::<FirmBooks>(owner)? {
                    books.revenue = books.revenue.try_add(tenancy.rent_per_day)?;
                }
                // A bank-owned home (repossession awaiting resale): the
                // rent is the bank's own income — equity keeps the
                // vault identity, exactly like sale proceeds.
                if let Some(book) = world.get_mut::<core_ecs::sim_interface::BankBook>(owner)? {
                    book.equity = book.equity.try_add(tenancy.rent_per_day)?;
                }
            } else {
                world.remove::<Tenancy>(tenant)?;
                if world
                    .get::<Residence>(tenant)?
                    .is_some_and(|residence| residence.home == tenancy.home)
                {
                    world.remove::<Residence>(tenant)?;
                }
            }
        }
        Ok(())
    }
}

/// Day-rate system: the rental clearing. Vacant owned homes ask the
/// controller-steered rent (cost-plus floored; vacancy above the data
/// target walks it down, scarcity walks it up — measured, bounded,
/// ADR 0009 §3); homeless citizens bid a data share of their SAVINGS
/// (wallet + vault — the float sweep pins pocket cash, ADR 0009 §2).
/// Asks uniform, bids descending, ties by entity index; matches clear
/// at the midpoint.
pub struct RentalMarketSystem {
    tables: EconTables,
}

impl RentalMarketSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        RentalMarketSystem { tables }
    }
}

impl System for RentalMarketSystem {
    fn name(&self) -> &'static str {
        "econ.rental_market"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let housing = &self.tables.money.housing;
        let home_kind = self.tables.money.home_location_kind;
        let bank = bank_entity(world)?;
        let occupied = occupied_homes(world)?;

        // The supply side: vacant owned homes, entity order.
        let mut owned_homes: i64 = 0;
        let mut vacant: Vec<Entity> = Vec::new();
        for (home, _) in world.iter::<Ownership>()? {
            let is_home = world
                .get::<Location>(home)?
                .is_some_and(|location| location.kind == home_kind);
            if !is_home {
                continue;
            }
            owned_homes += 1;
            if !occupied.contains(&home.index()) {
                vacant.push(home);
            }
        }

        // The controller-steered ask (ADR 0009 §3): floored at the
        // cost-plus basis, stepped by measured vacancy vs the target.
        let floor = mul(
            housing.upkeep_mills_per_day,
            add(1000, housing.rent_margin_per_mille, "rent margin")?,
            "rent floor",
        )? / 1000;
        let vacancy_per_mille = if owned_homes > 0 {
            mul(vacant.len() as i64, 1000, "vacancy")? / owned_homes
        } else {
            0
        };
        let mut ask = match housing_book_entity(world)? {
            Some(ledger) => world
                .get::<HousingBook>(ledger)?
                .map(|book| book.rent_ask_mills)
                .filter(|ask| *ask > 0)
                .unwrap_or(floor),
            None => floor,
        };
        let step = (mul(ask, housing.rent_step_per_mille, "rent step")? / 1000).max(1);
        if vacancy_per_mille > housing.rent_vacancy_target_per_mille {
            ask = (ask - step).max(floor);
        } else if vacancy_per_mille < housing.rent_vacancy_target_per_mille {
            ask = add(ask, step, "rent ask")?;
        }
        if let Some(ledger) = housing_book_entity(world)?
            && let Some(book) = world.get_mut::<HousingBook>(ledger)?
        {
            book.rent_ask_mills = ask;
        }

        // Bids: homeless citizens (no residence), from savings.
        let mut bids: Vec<(Entity, i64)> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if world.get::<Residence>(citizen)?.is_some() {
                continue;
            }
            let savings = savings_mills(world, bank, citizen)?;
            let bid = mul(savings, housing.rent_bid_per_mille, "rent bid")? / 1000;
            if bid >= 1 {
                bids.push((citizen, bid));
            }
        }
        bids.sort_by_key(|(citizen, bid)| (Reverse(*bid), citizen.index()));

        for ((tenant, bid), home) in bids.iter().zip(&vacant) {
            if *bid < ask {
                break;
            }
            let rent = Money::from_mills(add(ask, *bid, "rent midpoint")? / 2);
            world.insert(
                *tenant,
                Tenancy {
                    home: *home,
                    rent_per_day: rent,
                },
            )?;
            world.insert(*tenant, Residence { home: *home })?;
            world.emit(&TenancyStarted {
                tenant: *tenant,
                home: *home,
                rent_per_day: rent,
            })?;
        }
        Ok(())
    }
}
