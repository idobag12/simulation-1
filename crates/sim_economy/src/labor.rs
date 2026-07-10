//! The Phase 5 labor systems (ADR 0008 §§3–5): daily payroll (fires
//! instead of paying when a wage can't be covered) and the daily
//! double-auction clearing that hires, sets wages, and measures
//! unemployment. All integer arithmetic; no RNG.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use core_ecs::sim_interface::{Employment, Hired, LaborStats, Wallet, WorkingAge};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::{ArithmeticError, Money};

use crate::components::Firm;
use crate::config::EconTables;

/// The ledger entity — the economy's presence signal, shared with every
/// other economy system (`EconCounters`, not `LaborStats`: a world
/// migrated from v5 HAS an economy but genesis never gave it a labor
/// ledger). The stats row is created here on first use, so migrated
/// towns catch up honestly on their first day boundary instead of
/// never hiring again (ADR 0008 §8).
pub(crate) fn ledger_entity(world: &mut World) -> Result<Option<Entity>, EcsError> {
    let Some(ledger) = world
        .iter::<core_ecs::sim_interface::EconCounters>()?
        .next()
        .map(|(entity, _)| entity)
    else {
        return Ok(None);
    };
    if world.get::<LaborStats>(ledger)?.is_none() {
        world.insert(ledger, LaborStats::default())?;
    }
    Ok(Some(ledger))
}

pub(crate) fn bump_stats(
    world: &mut World,
    ledger: Entity,
    update: impl FnOnce(&mut LaborStats),
) -> Result<(), EcsError> {
    if let Some(stats) = world.get_mut::<LaborStats>(ledger)? {
        update(stats);
    }
    Ok(())
}

/// Day-rate system (ADR 0008 §3), after payroll: the daily double
/// auction. Firms bid a data-defined share of a worker's expected daily
/// marginal product (clamped to their wallet); unemployed working-age
/// citizens ask their reservation wage (wealth raises it, the
/// data-defined trait lowers it). Bids sort descending, asks ascending
/// (ties: entity index); matches clear at the midpoint. Afterwards the
/// clearing writes [`LaborStats`] — unemployment is measured, never
/// assigned (SPEC §12).
pub struct LaborMarketSystem {
    tables: EconTables,
}

impl LaborMarketSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        LaborMarketSystem { tables }
    }

    /// A citizen's reservation wage (exact integer arithmetic). Skill
    /// raises it (Phase 7, ADR 0010 §1): mastery is an outside option —
    /// this is where the skill wage premium enters the double auction
    /// (the ask side knows its owner; a uniform bid cannot).
    pub(crate) fn reservation(
        &self,
        cash_mills: i64,
        trait_per_mille: i64,
        skill_per_mille: i64,
    ) -> Result<i64, EcsError> {
        let labor = &self.tables.labor;
        let base = labor.reservation_base_mills;
        let overflow = |op: &'static str| EcsError::Arithmetic(ArithmeticError::Overflow { op });
        // Wealth raise: base × per_mille/1000 × wallet/(wallet + half).
        let cash = cash_mills.max(0);
        let raise = base
            .checked_mul(labor.reservation_wealth_per_mille)
            .and_then(|x| x.checked_mul(cash))
            .ok_or_else(|| overflow("reservation raise"))?
            / 1000
            / cash
                .checked_add(labor.reservation_half_wealth_mills)
                .ok_or_else(|| overflow("reservation divisor"))?
                .max(1);
        // Trait discount: base × discount/1000 × trait/1000.
        let discount = base
            .checked_mul(labor.reservation_trait_discount_per_mille)
            .and_then(|x| x.checked_mul(trait_per_mille.clamp(0, 1000)))
            .ok_or_else(|| overflow("reservation discount"))?
            / 1_000_000;
        // Skill premium: base × weight/1000 × skill/1000.
        let premium = base
            .checked_mul(self.tables.labor_skill_weight_per_mille)
            .and_then(|x| x.checked_mul(skill_per_mille.clamp(0, 1000)))
            .ok_or_else(|| overflow("reservation premium"))?
            / 1_000_000;
        Ok(base
            .checked_add(raise)
            .and_then(|x| x.checked_sub(discount))
            .and_then(|x| x.checked_add(premium))
            .ok_or_else(|| overflow("reservation total"))?
            .max(1))
    }

    /// A firm's bid for one worker: `bid_fraction` of the expected daily
    /// marginal product (output per worker-day at the posted price),
    /// clamped so the wallet could cover the wages already committed to
    /// incumbents PLUS this day's open slots (ADR 0008 §3).
    fn bid(
        &self,
        kind: &crate::config::FirmKindTable,
        posted_price: Money,
        cash: Money,
        committed_wages: i64,
        open_slots: u32,
    ) -> Result<i64, EcsError> {
        let overflow = |op: &'static str| EcsError::Arithmetic(ArithmeticError::Overflow { op });
        let recipe =
            self.tables
                .recipes
                .get(kind.recipe as usize)
                .ok_or(EcsError::InternalCorruption(
                    "firm kind references a recipe outside the loaded data",
                ))?;
        let shift_hours = i64::from(self.tables.labor.shift_end_hour)
            - i64::from(self.tables.labor.shift_start_hour);
        // Expected daily value per worker: batch value × shift hours /
        // (batch hours × positions) — fractional in hours, so multi-day
        // batches (construction) still value their workers. A builder's
        // batch value is the home it finishes (ADR 0009 §5).
        let batch_value = match recipe.output {
            Some((_, quantity)) => quantity
                .checked_mul(posted_price.mills())
                .ok_or_else(|| overflow("bid product"))?,
            None => self.tables.money.housing.home_price_mills,
        };
        let marginal = batch_value
            .checked_mul(shift_hours)
            .ok_or_else(|| overflow("bid hours"))?
            / (i64::from(recipe.batch_hours.max(1)) * i64::from(kind.positions.max(1)));
        let bid = marginal
            .checked_mul(self.tables.labor.bid_fraction_per_mille)
            .ok_or_else(|| overflow("bid fraction"))?
            / 1000;
        // Affordability: incumbents' committed wages plus every open
        // slot must be payable for a day out of cash on hand.
        let free = cash
            .mills()
            .checked_sub(committed_wages)
            .ok_or_else(|| overflow("bid free cash"))?
            .max(0);
        let affordable = free / i64::from(open_slots.max(1));
        Ok(bid.min(affordable))
    }
}

impl System for LaborMarketSystem {
    fn name(&self) -> &'static str {
        "econ.labor_market"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let Some(ledger) = ledger_entity(world)? else {
            return Ok(()); // no economy at all (migrated pre-v5 world)
        };

        // Deposit balances (Phase 6, ADR 0009 §2): the reservation's
        // wealth term reads wallet + vault.
        let mut vault: BTreeMap<u32, i64> = BTreeMap::new();
        if let Some((_, book)) = world.iter::<core_ecs::sim_interface::BankBook>()?.next() {
            for (owner, balance) in &book.deposits {
                vault.insert(owner.index(), balance.mills());
            }
        }

        // Pass 1 (immutable): asks — unemployed working-age citizens, in
        // entity order.
        let mut asks: Vec<(Entity, i64)> = Vec::new();
        let mut working_age: u32 = 0;
        for (citizen, _) in world.iter::<WorkingAge>()? {
            working_age += 1;
            if world.get::<Employment>(citizen)?.is_some() {
                continue;
            }
            let cash = world
                .get::<Wallet>(citizen)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0)
                + vault.get(&citizen.index()).copied().unwrap_or(0);
            let trait_per_mille = world
                .get::<core_ecs::sim_interface::Personality>(citizen)?
                .and_then(|personality| {
                    personality
                        .weights
                        .get(self.tables.labor.reservation_trait as usize)
                        .copied()
                })
                .unwrap_or(0);
            let skill = world
                .get::<core_ecs::sim_interface::Skills>(citizen)?
                .map(|skills| skills.levels.iter().copied().max().unwrap_or(0))
                .unwrap_or(0);
            asks.push((
                citizen,
                self.reservation(cash, i64::from(trait_per_mille), i64::from(skill))?,
            ));
        }
        let seeking = asks.len() as u32;
        asks.sort_by_key(|(citizen, ask)| (*ask, citizen.index()));

        // Bids — one per open slot, in firm entity order.
        let mut headcount: BTreeMap<u32, u32> = BTreeMap::new();
        let mut committed: BTreeMap<u32, i64> = BTreeMap::new();
        for (_, employment) in world.iter::<Employment>()? {
            *headcount.entry(employment.employer.index()).or_insert(0) += 1;
            *committed.entry(employment.employer.index()).or_insert(0) +=
                employment.wage_per_day.mills();
        }
        let mut bids: Vec<(Entity, i64)> = Vec::new();
        for (firm_entity, firm) in world.iter::<Firm>()? {
            let Some(kind) = self.tables.firm_kinds.get(firm.kind as usize) else {
                continue;
            };
            let employees = headcount.get(&firm_entity.index()).copied().unwrap_or(0);
            let open = kind.positions.saturating_sub(employees);
            if open == 0 {
                continue;
            }
            let cash = world
                .get::<Wallet>(firm_entity)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            let committed_wages = committed.get(&firm_entity.index()).copied().unwrap_or(0);
            let bid = self.bid(kind, firm.posted_price, cash, committed_wages, open)?;
            if bid < 1 {
                continue;
            }
            for _ in 0..open {
                bids.push((firm_entity, bid));
            }
        }
        // The public employer (ADR 0009 §4): the treasury bids its data
        // wage for its open town-hall slots, affordability-clamped like
        // any firm.
        if let Some((treasury, _)) = world
            .iter::<core_ecs::sim_interface::TreasuryBook>()?
            .next()
        {
            let employees = headcount.get(&treasury.index()).copied().unwrap_or(0);
            let open = self.tables.money.public_positions.saturating_sub(employees);
            if open > 0 {
                let cash = world
                    .get::<Wallet>(treasury)?
                    .map(|wallet| wallet.cash.mills())
                    .unwrap_or(0);
                let committed_wages = committed.get(&treasury.index()).copied().unwrap_or(0);
                let free = cash.checked_sub(committed_wages).unwrap_or(0).max(0);
                let bid = self
                    .tables
                    .money
                    .public_wage_bid_mills
                    .min(free / i64::from(open));
                if bid >= 1 {
                    for _ in 0..open {
                        bids.push((treasury, bid));
                    }
                }
            }
        }
        bids.sort_by_key(|(firm, bid)| (Reverse(*bid), firm.index()));

        // Clearing: match while the best bid covers the best ask; the
        // wage is the midpoint, rounded down to whole mills.
        let mut hires: Vec<(Entity, Entity, Money)> = Vec::new();
        for ((citizen, ask), (firm, bid)) in asks.iter().zip(&bids) {
            if bid < ask {
                break;
            }
            let midpoint =
                ask.checked_add(*bid)
                    .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow {
                        op: "wage midpoint",
                    }))?
                    / 2;
            hires.push((*citizen, *firm, Money::from_mills(midpoint)));
        }
        let hired = hires.len() as u32;
        for (citizen, employer, wage_per_day) in hires {
            world.insert(
                citizen,
                Employment {
                    employer,
                    wage_per_day,
                },
            )?;
            world.emit(&Hired {
                citizen,
                employer,
                wage_per_day,
            })?;
        }

        // Measurement (ADR 0008 §5): written from what actually happened.
        let employed = world.iter::<Employment>()?.count() as u32;
        bump_stats(world, ledger, |stats| {
            stats.working_age = working_age;
            stats.employed = employed;
            stats.seeking = seeking;
            stats.unmatched = seeking - hired;
            stats.hires += u64::from(hired);
        })?;
        Ok(())
    }
}
