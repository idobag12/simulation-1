//! The purchase clearing (Phase 6, ADR 0009 §3): a double auction over
//! homes, financed cash-first with the remainder as an LTV-capped bank
//! mortgage secured by the home. The clearing average is the measured
//! "market price of homes" the construction hurdle reads (SPEC §12:
//! prices measured from micro activity, never set). Split from
//! `housing.rs` for the SPEC §3 module-size rule.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use core_ecs::sim_interface::{
    BankBook, BorrowerStatus, FirmBooks, HomeSold, HousingBook, Location, Needs, Ownership,
    Residence, Tenancy, Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::Money;
use core_types::calendar::TICKS_PER_DAY;

use crate::bank::bank_entity;
use crate::config::EconTables;
use crate::housing::{housing_book_entity, occupied_homes, savings_mills};
use crate::transact::{add, mul};
use crate::vault::{deposit_balance, grant_mortgage, vault_withdraw};

/// The measured market price of homes: the last clearing's average, or
/// the data floor before any sale (ADR 0009 §5).
pub(crate) fn home_price_anchor(world: &World, floor_mills: i64) -> Result<i64, EcsError> {
    let anchor = match housing_book_entity(world)? {
        Some(ledger) => world
            .get::<HousingBook>(ledger)?
            .map(|book| book.last_home_price_mills)
            .filter(|price| *price > 0)
            .unwrap_or(floor_mills),
        None => floor_mills,
    };
    Ok(anchor.max(floor_mills))
}

/// Every-N-days system (ADR 0009 §3): the purchase clearing. Any live
/// owner's VACANT home is on offer (builders, the bank's
/// repossessions, the treasury's escheats, citizens' inherited second
/// homes); the ask is the measured anchor. Buyers are citizens without
/// an owned home whose savings clear the data screen; they bid a data
/// share of savings, richest bid first. Each match clears at the
/// midpoint, financed cash-first with the remainder as a collateralized
/// mortgage. Ownership and every money leg move in one pass.
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
        let bank_config = &self.tables.money.bank;
        let day = ctx.tick.raw() / TICKS_PER_DAY;
        if housing.purchase_period_days == 0 || !day.is_multiple_of(housing.purchase_period_days) {
            return Ok(());
        }
        let Some(bank) = bank_entity(world)? else {
            return Ok(()); // mortgages need a bank; without one, no market
        };
        let home_kind = self.tables.money.home_location_kind;
        let occupied = occupied_homes(world)?;
        let ask = home_price_anchor(world, housing.home_price_mills)?;

        // Offers: vacant homes with a live owner, home entity order,
        // with districts (Phase 9 — buyers weigh the commute).
        let mut offers: Vec<(Entity, Entity, Option<u32>, bool)> = {
            let mut list = Vec::new();
            for (home, ownership) in world.iter::<Ownership>()? {
                if occupied.contains(&home.index()) {
                    continue;
                }
                let is_home = world
                    .get::<Location>(home)?
                    .is_some_and(|location| location.kind == home_kind);
                if is_home && world.is_alive(ownership.owner) {
                    list.push((home, ownership.owner, None, false));
                }
            }
            for entry in &mut list {
                entry.2 = world
                    .get::<core_ecs::sim_interface::Sited>(entry.0)?
                    .map(|sited| sited.district);
            }
            list
        };
        if offers.is_empty() {
            return Ok(());
        }

        // Buyers: citizens without an owned home (ADR 0009 §3: renters
        // bid) whose savings clear the screen; bid = data share of
        // savings, richest bid first (ties: entity index).
        let owners: BTreeSet<u32> = world
            .iter::<Ownership>()?
            .map(|(_, ownership)| ownership.owner.index())
            .collect();
        let screen = mul(ask, housing.buyer_savings_per_mille, "savings screen")? / 1000;
        let mut bids: Vec<(Entity, i64)> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if owners.contains(&citizen.index()) {
                continue;
            }
            let savings = savings_mills(world, Some(bank), citizen)?;
            if savings < screen {
                continue;
            }
            let bid = mul(savings, housing.home_bid_per_mille, "home bid")? / 1000;
            if bid >= 1 {
                bids.push((citizen, bid));
            }
        }
        bids.sort_by_key(|(citizen, bid)| (Reverse(*bid), citizen.index()));

        let mut sale_prices: Vec<i64> = Vec::new();
        for (buyer, bid) in bids.iter() {
            if *bid < ask {
                break;
            }
            // The buyer takes the untaken home with the SHORTEST
            // commute to their workplace (ADR 0012 §3; jobless buyers
            // take entity order) — assortative, like the rental match.
            let work_district = match world
                .get::<core_ecs::sim_interface::Employment>(*buyer)?
                .map(|employment| employment.employer)
            {
                Some(employer) => world
                    .get::<core_ecs::sim_interface::Sited>(employer)?
                    .map(|sited| sited.district),
                None => None,
            };
            let Some(choice) = offers
                .iter()
                .enumerate()
                .filter(|(_, (_, _, _, taken))| !taken)
                .min_by_key(|(_, (home, _, district, _))| {
                    let commute = match work_district {
                        Some(_) => i64::from(self.tables.travel_between(*district, work_district)),
                        None => 0,
                    };
                    (commute, home.index())
                })
                .map(|(index, _)| index)
            else {
                break; // no vacancy left
            };
            let (home, seller) = (&offers[choice].0.clone(), &offers[choice].1.clone());
            offers[choice].3 = true;
            let price = add(ask, *bid, "home midpoint")? / 2;

            // Financing, cash-first (ADR 0009 §3): savings cover what
            // they can — LESS a living reserve (the cash float): a buyer
            // stripped to zero would default on the very first payment,
            // turning every sale into a foreclosure. The remainder is a
            // mortgage, capped by LTV.
            let savings = savings_mills(world, Some(bank), *buyer)?;
            let reserve = bank_config.target_cash_float_mills;
            let cash_part = (savings - reserve).max(0).min(price);
            let mortgage = Money::from_mills(price - cash_part);
            let ltv_cap = mul(price, housing.mortgage_ltv_per_mille, "ltv cap")? / 1000;
            if mortgage.mills() > ltv_cap {
                continue;
            }
            if mortgage > Money::ZERO {
                // Mortgages pass the same credit gates as any loan:
                // no stacking, no lending into a default cooldown.
                let already_borrowing = world
                    .get::<BankBook>(bank)?
                    .is_some_and(|book| book.loans.iter().any(|loan| loan.borrower == *buyer));
                if already_borrowing {
                    continue;
                }
                if world
                    .get::<BorrowerStatus>(*buyer)?
                    .is_some_and(|status| status.uncreditworthy_until_day > day)
                {
                    continue;
                }
            }
            // The vault funds the mortgage AND tops up the buyer's cash
            // part — check the WHOLE draw before any leg moves, so the
            // bank wallet can never go negative here.
            let wallet_cash = world
                .get::<Wallet>(*buyer)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let top_up = (cash_part - wallet_cash).max(0);
            let vault_need = add(mortgage.mills(), top_up, "vault need")?;
            let bank_cash = world
                .get::<Wallet>(bank)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            if bank_cash < vault_need {
                continue;
            }
            let row = deposit_balance(world, bank, *buyer)?;
            if row.mills() < top_up {
                continue; // savings moved since the bid snapshot; skip
            }

            // The atomic sale: vault top-up, buyer cash → seller,
            // mortgage vault → seller (secured by the home), books,
            // ownership, shelter, the fact.
            if top_up > 0 {
                vault_withdraw(world, bank, *buyer, Money::from_mills(top_up))?;
            }
            if let Some(wallet) = world.get_mut::<Wallet>(*buyer)? {
                wallet.cash = wallet.cash.try_sub(Money::from_mills(cash_part))?;
            }
            if let Some(wallet) = world.get_mut::<Wallet>(*seller)? {
                wallet.cash = wallet.cash.try_add(Money::from_mills(cash_part))?;
            }
            if let Some(books) = world.get_mut::<FirmBooks>(*seller)? {
                books.revenue = books.revenue.try_add(Money::from_mills(price))?;
            }
            if *seller == bank {
                // The bank selling a repossession: proceeds are the
                // bank's own recovery — equity keeps the vault identity
                // (wallet grew with no deposit/loan row moving).
                if let Some(book) = world.get_mut::<BankBook>(bank)? {
                    book.equity = book.equity.try_add(Money::from_mills(price))?;
                }
            }
            if mortgage > Money::ZERO {
                let rate = world
                    .get::<BankBook>(bank)?
                    .map(|book| book.policy_rate_per_million_daily)
                    .unwrap_or(bank_config.policy_neutral_per_million_daily)
                    + bank_config.risk_premium_per_million_daily;
                grant_mortgage(
                    world,
                    bank,
                    *buyer,
                    *seller,
                    mortgage,
                    rate,
                    bank_config.repay_term_days,
                    *home,
                )?;
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
                price: Money::from_mills(price),
            })?;
            sale_prices.push(price);
        }

        // The measured market price: this clearing's average, persisted
        // for the construction hurdle and the next clearing's ask.
        if !sale_prices.is_empty() {
            let mut sum: i64 = 0;
            for price in &sale_prices {
                sum = add(sum, *price, "clearing average")?;
            }
            let average = sum / sale_prices.len() as i64;
            if let Some(ledger) = housing_book_entity(world)?
                && let Some(book) = world.get_mut::<HousingBook>(ledger)?
            {
                book.last_home_price_mills = average;
            }
        }
        Ok(())
    }
}
