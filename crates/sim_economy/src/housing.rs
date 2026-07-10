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
                // arrangement ends entirely — tenancy AND every
                // co-resident's residence — so the household re-enters
                // the rental market instead of squatting forever.
                world.remove::<Tenancy>(tenant)?;
                let evicted: Vec<Entity> = world
                    .iter::<Residence>()?
                    .filter(|(_, residence)| residence.home == tenancy.home)
                    .map(|(resident, _)| resident)
                    .collect();
                for resident in evicted {
                    world.remove::<Residence>(resident)?;
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
                // The WHOLE household leaves — a co-resident spouse or
                // child left behind would squat rent-free on a home the
                // market thinks is occupied (Phase 7 review).
                let evicted: Vec<Entity> = world
                    .iter::<Residence>()?
                    .filter(|(_, residence)| residence.home == tenancy.home)
                    .map(|(resident, _)| resident)
                    .collect();
                for resident in evicted {
                    world.remove::<Residence>(resident)?;
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

        // The supply side: vacant owned homes with districts, entity
        // order; per-district occupancy counts feed per-district asks
        // (Phase 9, ADR 0012 §3). Un-sited homes bucket under the
        // global ask (the migrated world's whole market).
        let districts = self.tables.district_travel.len();
        let mut owned_per_district = vec![0i64; districts + 1]; // last = un-sited
        let mut vacant_per_district = vec![0i64; districts + 1];
        let mut vacant: Vec<(Entity, Option<u32>, bool)> = Vec::new();
        for (home, _) in world.iter::<Ownership>()? {
            let is_home = world
                .get::<Location>(home)?
                .is_some_and(|location| location.kind == home_kind);
            if !is_home {
                continue;
            }
            let district = world
                .get::<core_ecs::sim_interface::Sited>(home)?
                .map(|sited| sited.district);
            let bucket = district
                .map(|district| district as usize)
                .filter(|district| *district < districts)
                .unwrap_or(districts);
            owned_per_district[bucket] += 1;
            if !occupied.contains(&home.index()) {
                vacant_per_district[bucket] += 1;
                vacant.push((home, district, false));
            }
        }

        // The controller-steered asks (ADR 0009 §3, made SPATIAL in
        // ADR 0012 §3): floored at the cost-plus basis, each district's
        // ask stepped by ITS OWN measured vacancy vs the target —
        // scarce central homes genuinely cost more, idle edge homes
        // cheapen; the global ask keeps steering un-sited homes.
        let floor = mul(
            housing.upkeep_mills_per_day,
            add(1000, housing.rent_margin_per_mille, "rent margin")?,
            "rent floor",
        )? / 1000;
        let ledger = housing_book_entity(world)?;
        let (global_ask, mut district_asks) = match ledger {
            Some(ledger) => world
                .get::<HousingBook>(ledger)?
                .map(|book| {
                    (
                        Some(book.rent_ask_mills)
                            .filter(|ask| *ask > 0)
                            .unwrap_or(floor),
                        book.district_rent_ask_mills.clone(),
                    )
                })
                .unwrap_or((floor, Vec::new())),
            None => (floor, Vec::new()),
        };
        district_asks.resize(districts, 0);
        for ask in &mut district_asks {
            // Same sanitization as the global ask's `> 0` filter: a
            // corrupt or hand-edited save must never price a clearing
            // with a non-positive ask (a negative midpoint would sign
            // a REVERSED rent stream).
            if *ask <= 0 {
                *ask = global_ask;
            }
        }
        let ask_of = |district: Option<u32>| -> i64 {
            district
                .map(|district| district as usize)
                .and_then(|district| district_asks.get(district).copied())
                .unwrap_or(global_ask)
        };
        // ACCESS (ADR 0012 §3): a home is also a base for everything
        // else — mean travel from each district to every sited
        // non-home location (shops, venues, workplaces). Central homes
        // serve every errand cheaply; the edge pays for each trip.
        let access_per_district: Vec<i64> = {
            let mut destination_count = 0i64;
            let mut totals = vec![0i64; districts];
            for (location, entry) in world.iter::<Location>()? {
                let is_home = entry.kind == home_kind;
                if is_home {
                    continue;
                }
                if let Some(sited) = world.get::<core_ecs::sim_interface::Sited>(location)? {
                    destination_count += 1;
                    for (district, total) in totals.iter_mut().enumerate() {
                        *total += i64::from(
                            self.tables
                                .travel_between(Some(district as u32), Some(sited.district)),
                        );
                    }
                }
            }
            totals
                .into_iter()
                .map(|total| {
                    if destination_count > 0 {
                        total / destination_count
                    } else {
                        0
                    }
                })
                .collect()
        };
        let access_of = |district: Option<u32>| -> i64 {
            district
                .map(|district| district as usize)
                .and_then(|district| access_per_district.get(district).copied())
                .unwrap_or(0)
        };
        let min_ask = district_asks
            .iter()
            .copied()
            .chain(std::iter::once(global_ask))
            .min()
            .unwrap_or(global_ask);

        // Bids: homeless citizens (no residence), from savings, with
        // their workplace's district (Phase 9, ADR 0012 §3 — the
        // commute enters the match).
        let mut bids: Vec<(Entity, i64, Option<u32>)> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if world.get::<Residence>(citizen)?.is_some() {
                continue;
            }
            let savings = savings_mills(world, bank, citizen)?;
            let bid = mul(savings, housing.rent_bid_per_mille, "rent bid")? / 1000;
            if bid >= 1 {
                let work_district = match world
                    .get::<core_ecs::sim_interface::Employment>(citizen)?
                    .map(|employment| employment.employer)
                {
                    Some(employer) => world
                        .get::<core_ecs::sim_interface::Sited>(employer)?
                        .map(|sited| sited.district),
                    None => None,
                };
                bids.push((citizen, bid, work_district));
            }
        }
        bids.sort_by_key(|(citizen, bid, _)| (Reverse(*bid), citizen.index()));

        // The match is SPATIAL (ADR 0012 §3): each bidder, best bid
        // first, takes the AFFORDABLE vacant home with the lowest
        // EFFECTIVE cost — the home's district ask +
        // commute_mills_per_tick × (travel(home, workplace) + ACCESS).
        // Jobless bidders zero the commute term only; access weighs on
        // everyone (a home is a base for every errand).
        let mut priced_out = vec![0i64; districts + 1];
        let mut contention = vec![0i64; districts + 1];
        for (tenant, bid, work_district) in bids {
            // The bidder's FIRST choice over the whole offered stock
            // (taken or not): measured demand per district — the
            // contention signal the ask controllers price, and the
            // bucket that owns their PRICED-OUT signal too (they wanted
            // THAT market; a global count would let one unhousable
            // pauper anywhere cut every district's ask).
            let first_choice = vacant
                .iter()
                .min_by_key(|(home, district, _)| {
                    let commute = match work_district {
                        Some(_) => i64::from(self.tables.travel_between(*district, work_district)),
                        None => 0,
                    };
                    (
                        ask_of(*district).saturating_add(
                            self.tables
                                .commute_mills_per_tick
                                .saturating_mul(commute.saturating_add(access_of(*district))),
                        ),
                        home.index(),
                    )
                })
                .map(|(_, district, _)| *district);
            let first_bucket = first_choice.map(|district| {
                district
                    .map(|district| district as usize)
                    .filter(|district| *district < districts)
                    .unwrap_or(districts)
            });
            if bid < min_ask {
                if let Some(bucket) = first_bucket {
                    priced_out[bucket] += 1;
                }
                continue; // this market is out of reach for them
            }
            if let Some(bucket) = first_bucket {
                contention[bucket] += 1;
            }
            let choice = vacant
                .iter()
                .enumerate()
                .filter(|(_, (_, district, taken))| !taken && bid >= ask_of(*district))
                .min_by_key(|(_, (home, district, _))| {
                    let commute = match work_district {
                        Some(_) => i64::from(self.tables.travel_between(*district, work_district)),
                        None => 0,
                    };
                    (
                        ask_of(*district).saturating_add(
                            self.tables
                                .commute_mills_per_tick
                                .saturating_mul(commute.saturating_add(access_of(*district))),
                        ),
                        home.index(),
                    )
                })
                .map(|(index, _)| index);
            let Some(index) = choice else {
                if let Some(bucket) = first_bucket {
                    priced_out[bucket] += 1;
                }
                continue; // nothing this bidder can afford remains
            };
            let (home, district) = (vacant[index].0, vacant[index].1);
            vacant[index].2 = true;
            // The rent midpoint takes the bidder's LOCATION-ADJUSTED
            // willingness (ADR 0012 §3): every trip this home adds to
            // their life comes off what they will pay for it — commute
            // costs enter housing prices through the tenant's own
            // valuation, never as an assigned district premium.
            let commute_value = self.tables.commute_mills_per_tick.saturating_mul(
                match work_district {
                    Some(_) => i64::from(self.tables.travel_between(district, work_district)),
                    None => 0,
                }
                .saturating_add(access_of(district)),
            );
            let effective_bid = bid.saturating_sub(commute_value).max(ask_of(district));
            let rent =
                Money::from_mills(add(ask_of(district), effective_bid, "rent midpoint")? / 2);
            world.insert(
                tenant,
                Tenancy {
                    home,
                    rent_per_day: rent,
                },
            )?;
            world.insert(tenant, Residence { home })?;
            world.emit(&TenancyStarted {
                tenant,
                home,
                rent_per_day: rent,
            })?;
        }

        // Steering happens on OUTCOMES (ADR 0012 §3, amended): each
        // bucket's ask falls when its vacancy exceeds the target — or
        // when homes sat vacant while bidders were PRICED OUT (unsold
        // inventory is the market saying the price is wrong; without
        // this brake a supply-constrained town ratchets to prices
        // nobody ever pays) — and rises only when its vacancy is below
        // target. Same data step, no new tunables.
        let leftover: Vec<i64> = {
            let mut leftover = vec![0i64; districts + 1];
            for (_, district, taken) in &vacant {
                if !taken {
                    let bucket = district
                        .map(|district| district as usize)
                        .filter(|district| *district < districts)
                        .unwrap_or(districts);
                    leftover[bucket] += 1;
                }
            }
            leftover
        };
        let mut steered = vec![0i64; districts];
        for district in 0..districts {
            steered[district] = steer_ask(
                housing,
                floor,
                priced_out[district],
                district_asks[district],
                owned_per_district[district],
                vacant_per_district[district],
                leftover[district],
                contention[district],
            )?;
        }
        let steered_global = steer_ask(
            housing,
            floor,
            priced_out[districts],
            global_ask,
            owned_per_district[districts],
            vacant_per_district[districts],
            leftover[districts],
            contention[districts],
        )?;
        if let Some(ledger) = ledger
            && let Some(book) = world.get_mut::<HousingBook>(ledger)?
        {
            book.rent_ask_mills = steered_global;
            book.district_rent_ask_mills = steered;
        }
        Ok(())
    }
}

/// One bucket's ask steering, from CLEARING OUTCOMES (ADR 0012 §3, as
/// amended): a market with nothing offered HOLDS (no inventory, no
/// price discovery — a fully-occupied district must not ratchet to
/// numbers nobody ever pays); unsold homes beside priced-out bidders
/// push DOWN (too dear); chronic slack above the vacancy target pushes
/// DOWN; more first-choice bidders than offered homes (CONTENTION —
/// the binary sold-out signal saturates when demand swamps every
/// district; the queue's length does not) probes UP. Floored at the
/// cost-plus floor; the step is the data share of the current ask.
#[allow(
    clippy::too_many_arguments,
    reason = "one clearing's full outcome vector; a params struct would only rename the locals"
)]
fn steer_ask(
    housing: &crate::config_money::HousingConfig,
    floor: i64,
    priced_out: i64,
    ask: i64,
    owned: i64,
    offered: i64,
    unsold: i64,
    wanted: i64,
) -> Result<i64, EcsError> {
    if owned == 0 || offered == 0 {
        return Ok(ask);
    }
    let vacancy_per_mille = mul(unsold, 1000, "vacancy")? / owned;
    let step = (mul(ask, housing.rent_step_per_mille, "rent step")? / 1000).max(1);
    // Down on either failure signal: unsold inventory beside priced-out
    // bidders (too dear), or chronic slack above the vacancy target.
    Ok(
        if (unsold > 0 && priced_out > 0)
            || vacancy_per_mille > housing.rent_vacancy_target_per_mille
        {
            (ask - step).max(floor)
        } else if wanted > offered {
            add(ask, step, "rent ask")?
        } else {
            ask
        },
    )
}

#[cfg(test)]
mod steer_tests {
    use super::steer_ask;

    fn housing() -> crate::config_money::HousingConfig {
        crate::config_money::HousingConfig {
            upkeep_mills_per_day: 40,
            rent_margin_per_mille: 250,
            rent_bid_per_mille: 8,
            rent_vacancy_target_per_mille: 200,
            rent_step_per_mille: 50,
            home_price_mills: 30_000,
            purchase_period_days: 10,
            buyer_savings_per_mille: 400,
            home_bid_per_mille: 1_500,
            mortgage_ltv_per_mille: 800,
            construction_margin_per_mille: 200,
        }
    }

    /// ADR 0012 §3: every branch of the outcome steer, bitten once.
    #[test]
    fn the_ask_steers_on_clearing_outcomes() {
        let housing = housing();
        // No inventory: HOLD, whatever the demand queue looks like.
        assert_eq!(
            steer_ask(&housing, 50, 9, 700, 10, 0, 0, 9).unwrap(),
            700,
            "a market with nothing offered has no price to discover"
        );
        // Unsold beside priced-out: too dear — down one step, floored.
        assert_eq!(
            steer_ask(&housing, 50, 3, 1000, 10, 2, 2, 0).unwrap(),
            950,
            "unsold inventory beside priced-out bidders pushes down"
        );
        assert_eq!(
            steer_ask(&housing, 50, 3, 51, 10, 2, 2, 0).unwrap(),
            50,
            "the cost-plus floor holds"
        );
        // Chronic slack above the target: down.
        assert_eq!(
            steer_ask(&housing, 50, 0, 1000, 10, 3, 3, 0).unwrap(),
            950,
            "vacancy above the target cheapens"
        );
        // Contention: more first-choice bidders than offered homes.
        assert_eq!(
            steer_ask(&housing, 50, 0, 1000, 10, 1, 0, 4).unwrap(),
            1050,
            "a queue longer than the shelf probes upward"
        );
        // Balanced: hold.
        assert_eq!(
            steer_ask(&housing, 50, 0, 1000, 10, 1, 0, 1).unwrap(),
            1000,
            "a cleared market with no queue holds"
        );
    }
}
