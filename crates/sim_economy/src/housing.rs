//! Housing (Phase 6, ADR 0009 §3): daily rent, the rental clearing, and
//! the periodic purchase clearing with cash+mortgage financing. Both
//! markets reuse the labor market's double-auction shape (SPEC §12:
//! "the same matching machinery").

use std::cmp::Reverse;
use std::collections::BTreeSet;

use core_ecs::sim_interface::{
    FirmBooks, HomeSold, Location, Needs, Ownership, Residence, Tenancy, TenancyStarted, Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::Money;
use core_types::calendar::TICKS_PER_DAY;

use crate::bank::{bank_entity, deposit_balance, vault_withdraw};
use crate::config::EconTables;

/// Occupied homes: everyone's `Residence` target plus every tenancy.
fn occupied_homes(world: &World) -> Result<BTreeSet<u32>, EcsError> {
    let mut occupied = BTreeSet::new();
    for (_, residence) in world.iter::<Residence>()? {
        occupied.insert(residence.home.index());
    }
    for (_, tenancy) in world.iter::<Tenancy>()? {
        occupied.insert(tenancy.home.index());
    }
    Ok(occupied)
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
                // tenancy ends rather than paying nobody.
                world.remove::<Tenancy>(tenant)?;
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

/// Day-rate system: the rental clearing. Vacant owned homes ask a
/// cost-plus rent; homeless citizens bid a data share of their cash.
/// Asks ascending, bids descending, ties by entity index; matches clear
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
        let occupied = occupied_homes(world)?;

        // Asks: vacant owned homes, entity order; one flat cost-plus ask
        // (a vacancy controller is later texture).
        let ask = housing.upkeep_mills_per_day * (1000 + housing.rent_margin_per_mille) / 1000;
        let vacant: Vec<Entity> = world
            .iter::<Ownership>()?
            .map(|(home, _)| home)
            .filter(|home| !occupied.contains(&home.index()))
            .collect();
        let vacant: Vec<Entity> = vacant
            .into_iter()
            .filter(|home| {
                world
                    .get::<Location>(*home)
                    .ok()
                    .flatten()
                    .is_some_and(|location| location.kind == home_kind)
            })
            .collect();

        // Bids: homeless citizens (no residence), entity order.
        let mut bids: Vec<(Entity, i64)> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if world.get::<Residence>(citizen)?.is_some() {
                continue;
            }
            let cash = world
                .get::<Wallet>(citizen)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let bid = cash * housing.rent_bid_per_mille / 1000;
            if bid >= 1 {
                bids.push((citizen, bid));
            }
        }
        bids.sort_by_key(|(citizen, bid)| (Reverse(*bid), citizen.index()));

        for ((tenant, bid), home) in bids.iter().zip(&vacant) {
            if *bid < ask {
                break;
            }
            let rent = Money::from_mills((ask + bid) / 2);
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

/// Every-N-days system (ADR 0009 §3): the purchase clearing. Non-citizen
/// owners (builders, the treasury) offer their vacant homes at the data
/// price; citizens with sufficient savings buy, cash first and the
/// remainder as a bank mortgage. Ownership and every money leg move in
/// one atomic call.
pub struct PurchaseMarketSystem {
    tables: EconTables,
}

impl PurchaseMarketSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        PurchaseMarketSystem { tables }
    }
}

impl System for PurchaseMarketSystem {
    fn name(&self) -> &'static str {
        "econ.purchase_market"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let housing = &self.tables.money.housing;
        let day = ctx.tick.raw() / TICKS_PER_DAY;
        if housing.purchase_period_days == 0 || !day.is_multiple_of(housing.purchase_period_days) {
            return Ok(());
        }
        let Some(bank) = bank_entity(world)? else {
            return Ok(()); // mortgages need a bank; without one, no market
        };
        let home_kind = self.tables.money.home_location_kind;
        let occupied = occupied_homes(world)?;
        let price = Money::from_mills(housing.home_price_mills);

        // Offers: vacant homes owned by non-citizens (no Needs).
        let offers: Vec<(Entity, Entity)> = {
            let mut list = Vec::new();
            for (home, ownership) in world.iter::<Ownership>()? {
                if occupied.contains(&home.index()) {
                    continue;
                }
                let is_home = world
                    .get::<Location>(home)?
                    .is_some_and(|location| location.kind == home_kind);
                let seller_is_citizen = world.get::<Needs>(ownership.owner)?.is_some();
                if is_home && !seller_is_citizen && world.is_alive(ownership.owner) {
                    list.push((home, ownership.owner));
                }
            }
            list
        };
        if offers.is_empty() {
            return Ok(());
        }

        // Buyers: citizens who do NOT already own a home (ADR 0009 §3:
        // renters bid — one roof per household is Phase 6's honest
        // margin) and whose savings (wallet + deposits) clear the data
        // threshold, richest first (ties: entity index).
        let owners: BTreeSet<u32> = world
            .iter::<Ownership>()?
            .map(|(_, ownership)| ownership.owner.index())
            .collect();
        let mut buyers: Vec<(Entity, i64)> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if owners.contains(&citizen.index()) {
                continue;
            }
            let cash = world
                .get::<Wallet>(citizen)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let savings = cash + deposit_balance(world, bank, citizen)?.mills();
            if savings >= price.mills() * housing.buyer_savings_per_mille / 1000 {
                buyers.push((citizen, savings));
            }
        }
        buyers.sort_by_key(|(citizen, savings)| (Reverse(*savings), citizen.index()));

        for ((buyer, savings), (home, seller)) in buyers.iter().zip(&offers) {
            // Financing: the LTV-capped mortgage plus a cash down
            // payment (the savings screen above guarantees the buyer
            // covers the down payment).
            let mortgage = Money::from_mills(price.mills() * housing.mortgage_ltv_per_mille / 1000);
            let cash_part = price.mills() - mortgage.mills();
            let bank_cash = world
                .get::<Wallet>(bank)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            if *savings < cash_part || bank_cash < mortgage {
                continue;
            }
            // Pull the cash part up from the vault if the wallet is short.
            let wallet_cash = world
                .get::<Wallet>(*buyer)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            if wallet_cash < cash_part {
                vault_withdraw(
                    world,
                    bank,
                    *buyer,
                    Money::from_mills(cash_part - wallet_cash),
                )?;
            }
            // The atomic sale: buyer cash → seller; mortgage vault →
            // seller (a loan in the buyer's name); ownership + shelter.
            if let Some(wallet) = world.get_mut::<Wallet>(*buyer)? {
                wallet.cash = wallet.cash.try_sub(Money::from_mills(cash_part))?;
            }
            if let Some(wallet) = world.get_mut::<Wallet>(*seller)? {
                wallet.cash = wallet
                    .cash
                    .try_add(Money::from_mills(cash_part))?
                    .try_add(mortgage)?;
            }
            if let Some(books) = world.get_mut::<FirmBooks>(*seller)? {
                books.revenue = books.revenue.try_add(price)?;
            }
            if mortgage > Money::ZERO {
                if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
                    wallet.cash = wallet.cash.try_sub(mortgage)?;
                }
                let rate = world
                    .get::<core_ecs::sim_interface::BankBook>(bank)?
                    .map(|book| book.policy_rate_per_million_daily)
                    .unwrap_or(0)
                    + self.tables.money.bank.risk_premium_per_million_daily;
                let term = self.tables.money.bank.repay_term_days;
                let day_payment = Money::from_mills(
                    (mortgage.mills() / term.max(1) + mortgage.mills() * rate / 1_000_000).max(1),
                );
                if let Some(book) = world.get_mut::<core_ecs::sim_interface::BankBook>(bank)? {
                    book.loans.push(core_ecs::sim_interface::Loan {
                        borrower: *buyer,
                        principal: mortgage,
                        rate_per_million_daily: rate,
                        day_payment,
                    });
                }
                world.emit(&core_ecs::sim_interface::LoanGranted {
                    borrower: *buyer,
                    principal: mortgage,
                    rate_per_million_daily: rate,
                })?;
            }
            world.insert(*home, Ownership { owner: *buyer })?;
            if world.get::<Tenancy>(*buyer)?.is_some() {
                world.remove::<Tenancy>(*buyer)?;
            }
            world.insert(*buyer, Residence { home: *home })?;
            world.emit(&HomeSold {
                home: *home,
                seller: *seller,
                buyer: *buyer,
                price,
            })?;
        }
        Ok(())
    }
}
